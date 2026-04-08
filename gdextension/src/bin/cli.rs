//! margins_grip CLI — generate terrain artifacts for Margin's Grip.
//!
//! Commands:
//!   generate layers <SEED> <TAG>
//!       Run the full macro pipeline (512×512, erosion + rivers).
//!       Saves BiomeMap, RiverNetwork, and all debug PNGs to
//!       ~/.margins_grip/layers/<TAG>/
//!
//!   generate level <LAYERS_TAG> <X> <Y> <LEVEL_TAG>
//!       Generate a 512×512 micro chunk at world coords (X, Y).
//!       Saves BiomeMap to ~/.margins_grip/levels/<LEVEL_TAG>/
//!
//!   inspect level-presentation <LEVEL_TAG>
//!       Load a saved micro chunk artifact and compute the runtime presentation
//!       summary directly from the stored BiomeMap.
//!
//!   inspect layer-presentation-grid <LAYERS_TAG> <STEP>
//!       Sample many micro chunk summaries from a parent layers artifact using a
//!       world-space step size, then save a CSV and summary report under
//!       ~/.margins_grip/layers/<LAYERS_TAG>/inspections/
//!
//!   inspect chunk-presentation <SEED> <WORLD_X> <WORLD_Y>
//!       Compute a runtime presentation summary directly from generated chunk
//!       data without saving a level artifact.

use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use mg_artifacts::{ArtifactStore, LayerManifest, LevelManifest};
use mg_noise::{
    rasterize_to_tile, BiomeMap, NoiseLayer, RiverNetwork, RuntimeChunkPresentation,
    RuntimeChunkPresentationBundle, RuntimeChunkPresentationGrids, LOD_THRESHOLD_MACRO,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

// ─── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "margins_grip",
    version,
    about = "Margin's Grip terrain generator"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate terrain artifacts
    Generate {
        #[command(subcommand)]
        kind: GenerateKind,
    },
    /// Inspect saved terrain artifacts
    Inspect {
        #[command(subcommand)]
        kind: InspectKind,
    },
}

#[derive(Subcommand)]
enum GenerateKind {
    /// Generate macro layers artifact (full pipeline)
    Layers {
        /// World seed (u32)
        seed: u32,
        /// Artifact tag (alphanumeric, hyphens, underscores)
        tag: String,
    },
    /// Generate a micro level chunk artifact
    Level {
        /// Source layers artifact tag
        layers_tag: String,
        /// World X coordinate
        x: f64,
        /// World Y coordinate
        y: f64,
        /// Level artifact tag
        level_tag: String,
    },
}

#[derive(Subcommand)]
enum InspectKind {
    /// Compute the runtime presentation summary from a saved level artifact
    LevelPresentation {
        /// Level artifact tag
        level_tag: String,
    },
    /// Sample many runtime presentation summaries across a layers artifact
    LayerPresentationGrid {
        /// Source layers artifact tag
        layers_tag: String,
        /// World-space sampling step between chunk origins
        step: u32,
        /// Require broad world coverage and water-state diversity; exits non-zero on failure
        #[arg(long)]
        audit_defaults: bool,
        /// Compare the generated summary against a RON golden fixture; exits non-zero on mismatch
        #[arg(long)]
        golden: Option<String>,
    },
    /// Compute a runtime presentation summary directly from generated chunk data
    ChunkPresentation {
        /// World seed
        seed: u32,
        /// Generator-space world X coordinate
        world_x: f64,
        /// Generator-space world Y coordinate
        world_y: f64,
        /// Output format
        #[arg(long, value_enum, default_value_t = ChunkPresentationFormat::Text)]
        format: ChunkPresentationFormat,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ChunkPresentationFormat {
    Text,
    Json,
    Ron,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PresentationGridSummary {
    layers_tag: String,
    seed: u32,
    step: u32,
    world_width: u32,
    world_height: u32,
    sample_count: usize,
    planet_zone_counts: BTreeMap<String, usize>,
    atmosphere_class_counts: BTreeMap<String, usize>,
    water_state_counts: BTreeMap<String, usize>,
    landform_class_counts: BTreeMap<String, usize>,
    surface_palette_class_counts: BTreeMap<String, usize>,
    interestingness_min: f32,
    interestingness_avg: f32,
    interestingness_max: f32,
}

#[derive(Debug)]
struct PresentationGridScan {
    summary: PresentationGridSummary,
    csv: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReducedGridMetadata {
    width: usize,
    height: usize,
    digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReducedGridReport {
    water_state_grid: ReducedGridMetadata,
    landform_grid: ReducedGridMetadata,
    surface_palette_grid: ReducedGridMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeChunkPresentationReport {
    #[serde(flatten)]
    summary: RuntimeChunkPresentation,
    reduced_grids: ReducedGridReport,
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Generate { kind } => match kind {
            GenerateKind::Layers { seed, tag } => run_generate_layers(seed, &tag),
            GenerateKind::Level {
                layers_tag,
                x,
                y,
                level_tag,
            } => run_generate_level(&layers_tag, x, y, &level_tag),
        },
        Commands::Inspect { kind } => match kind {
            InspectKind::LevelPresentation { level_tag } => {
                run_inspect_level_presentation(&level_tag)
            }
            InspectKind::LayerPresentationGrid {
                layers_tag,
                step,
                audit_defaults,
                golden,
            } => run_inspect_layer_presentation_grid(
                &layers_tag,
                step,
                audit_defaults,
                golden.as_deref().map(Path::new),
            ),
            InspectKind::ChunkPresentation {
                seed,
                world_x,
                world_y,
                format,
            } => run_inspect_chunk_presentation(seed, world_x, world_y, format),
        },
    }
}

// ─── generate layers ─────────────────────────────────────────────────────────

// World layout constants (matching biome_map.rs and spec)
const WORLD_WIDTH: f64 = 1024.0;
const WORLD_HEIGHT: f64 = 512.0;
// Tile grid: 16×8 macro tiles of 64×64 world units each, rendered at 256px.
// Gives 4096×2048 final image at 0.25 world-units/pixel.
// 128 total dispatches vs 8192 for 32px tiles — critical for GPU efficiency.
const TILE_WORLD_SIZE: f64 = 64.0;
const TILE_PX: usize = 256;
const TILES_X: usize = (WORLD_WIDTH / TILE_WORLD_SIZE) as usize; // 16
const TILES_Y: usize = (WORLD_HEIGHT / TILE_WORLD_SIZE) as usize; // 8
const FULL_W: usize = TILES_X * TILE_PX; // 4096
const FULL_H: usize = TILES_Y * TILE_PX; // 2048
const MICRO_CHUNK_WORLD_SIZE: f64 = 1.0;
const MICRO_TILE_RESOLUTION: usize = 512;
const MICRO_DETAIL_LEVEL: u32 = 2;
const MICRO_FREQUENCY_SCALE: f64 = 8.0;

fn run_generate_layers(seed: u32, tag: &str) {
    let store = ArtifactStore::new().unwrap_or_else(|e| {
        eprintln!("error: failed to open artifact store: {e}");
        std::process::exit(1);
    });

    println!("Generating world — seed={seed}, tag={tag}");
    println!("  output:  {FULL_W}×{FULL_H} ({TILES_X}×{TILES_Y} tiles of {TILE_PX}px)");

    let t0 = Instant::now();

    // ── Step 1: Macro map for erosion + global river network ──────────────────
    // The first generate() call also initialises the GPU context if available.
    let pb = spinner("Macro pass (erosion + rivers)…");
    let macro_map = BiomeMap::generate(
        seed,
        0.0,
        0.0,
        WORLD_WIDTH,
        WORLD_HEIGHT,
        512,
        256,
        0,
        true,
        true,
        1.0,
    );
    pb.finish_and_clear();
    println!("  macro pass: {:.1}s", t0.elapsed().as_secs_f64());

    let river_network: RiverNetwork = macro_map
        .river_network
        .as_ref()
        .map(|arc| arc.as_ref().clone())
        .unwrap_or_else(|| RiverNetwork::empty(512, 256));

    // ── Step 2: Tile the world at meso detail → 4096×2048 ────────────────────
    let pb = spinner("Tiling world (128×64 tiles)…");

    // Allocate full RGBA buffers per layer (stitched in place)
    let n_pixels = FULL_W * FULL_H;
    let mut layer_bufs: Vec<Vec<u8>> = NoiseLayer::all()
        .iter()
        .map(|_| vec![0u8; n_pixels * 4])
        .collect();

    let total_tiles = TILES_X * TILES_Y;
    let mut tiles_done = 0usize;

    for ty in 0..TILES_Y {
        for tx in 0..TILES_X {
            let wx = tx as f64 * TILE_WORLD_SIZE;
            let wy = ty as f64 * TILE_WORLD_SIZE;

            let mut tile = BiomeMap::generate(
                seed,
                wx,
                wy,
                TILE_WORLD_SIZE,
                TILE_WORLD_SIZE,
                TILE_PX,
                TILE_PX,
                1,
                false,
                false,
                1.0,
            );

            // Project the macro river network onto this tile's rivers layer.
            // Each macro pixel (2 world units wide) expands to an 8×8 meso pixel footprint.
            tile.rivers = rasterize_to_tile(
                &river_network,
                TILE_PX,
                TILE_PX,
                wx,
                wy,
                TILE_WORLD_SIZE,
                TILE_WORLD_SIZE,
                WORLD_WIDTH,
                WORLD_HEIGHT,
                LOD_THRESHOLD_MACRO,
            );

            // Blit each layer from this tile into the full image
            for (li, &layer) in NoiseLayer::all().iter().enumerate() {
                let rgba = tile.layer_to_rgba(layer);
                let dst = &mut layer_bufs[li];
                let ox = tx * TILE_PX;
                let oy = ty * TILE_PX;
                for py in 0..TILE_PX {
                    for px in 0..TILE_PX {
                        let src_idx = (py * TILE_PX + px) * 4;
                        let dst_idx = ((oy + py) * FULL_W + (ox + px)) * 4;
                        dst[dst_idx..dst_idx + 4].copy_from_slice(&rgba[src_idx..src_idx + 4]);
                    }
                }
            }

            tiles_done += 1;
            if tiles_done % 512 == 0 {
                pb.set_message(format!("Tiling world… {tiles_done}/{total_tiles} tiles"));
            }
        }
    }
    pb.finish_and_clear();
    println!("  tiling: {:.1}s", t0.elapsed().as_secs_f64());

    // ── Step 3: Build images HashMap and manifest ─────────────────────────────
    let pb = spinner("Saving artifact…");
    let mut images: HashMap<String, (u32, u32, Vec<u8>)> = HashMap::new();
    for (layer, buf) in NoiseLayer::all().iter().zip(layer_bufs.into_iter()) {
        images.insert(
            format!("{}.png", layer.name()),
            (FULL_W as u32, FULL_H as u32, buf),
        );
    }

    let layer_images: Vec<String> = NoiseLayer::all()
        .iter()
        .map(|l| format!("{}.png", l.name()))
        .collect();

    let manifest = LayerManifest {
        seed,
        created: Utc::now().to_rfc3339(),
        world_width: WORLD_WIDTH as u32,
        world_height: WORLD_HEIGHT as u32,
        tile_width: FULL_W as u32,
        tile_height: FULL_H as u32,
        layer_images,
    };

    store
        .save_layers(tag, &macro_map, &river_network, &images, &manifest)
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save layers: {e}");
            std::process::exit(1);
        });
    pb.finish_and_clear();

    println!("Saved to {}/layers/{tag}/", store.base_path().display());
    println!(
        "  {FULL_W}×{FULL_H} PNGs, {} layers written",
        NoiseLayer::all().len()
    );
    println!("  total: {:.1}s", t0.elapsed().as_secs_f64());
}

// ─── generate level ───────────────────────────────────────────────────────────

fn run_generate_level(layers_tag: &str, world_x: f64, world_y: f64, level_tag: &str) {
    let store = ArtifactStore::new().unwrap_or_else(|e| {
        eprintln!("error: failed to open artifact store: {e}");
        std::process::exit(1);
    });

    // Load parent manifest for seed
    let parent_manifest = store.load_layer_manifest(layers_tag).unwrap_or_else(|e| {
        eprintln!("error: could not load layers artifact '{layers_tag}': {e}");
        std::process::exit(1);
    });

    let seed = parent_manifest.seed;
    println!("Generating micro level — seed={seed}, world=({world_x},{world_y}), tag={level_tag}");

    let pb = spinner("Running micro pipeline…");
    let t0 = Instant::now();

    let map = generate_runtime_micro_map(seed, world_x, world_y);
    let runtime_presentation = map.build_runtime_chunk_presentation_bundle();

    pb.finish_and_clear();
    println!("  pipeline done in {:.1}s", t0.elapsed().as_secs_f64());

    // ── Diagnose heightmap ────────────────────────────────────────────────────
    {
        let h = &map.heightmap;
        let n = h.len() as f64;
        let min_h = h.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_h = h.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mean = h.iter().sum::<f64>() / n;
        let land = h.iter().filter(|&&v| v > -0.01).count();
        let std_dev = (h.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();

        // Biome distribution
        use mg_noise::NoiseLayer;
        let biome_rgba = map.layer_to_rgba(NoiseLayer::Biome);
        let mut biome_counts: std::collections::HashMap<[u8; 3], (usize, String)> =
            std::collections::HashMap::new();
        for i in 0..h.len() {
            let r = biome_rgba[i * 4];
            let g = biome_rgba[i * 4 + 1];
            let b = biome_rgba[i * 4 + 2];
            biome_counts
                .entry([r, g, b])
                .or_insert((0, format!("#{r:02x}{g:02x}{b:02x}")))
                .0 += 1;
        }
        let mut biome_vec: Vec<_> = biome_counts.values().collect();
        biome_vec.sort_by(|a, b| b.0.cmp(&a.0));

        println!("\n── Micro heightmap (detail_level=2, freq=8, world=({world_x},{world_y})) ──");
        println!("  range   : [{min_h:.4}, {max_h:.4}]   mean: {mean:.4}   σ: {std_dev:.4}");
        println!(
            "  land    : {:.1}%  ocean: {:.1}%",
            100.0 * land as f64 / n,
            100.0 * (h.len() - land) as f64 / n
        );

        // Height distribution buckets
        print!("  height buckets  [-1→+1 in 0.2 steps]:  ");
        for i in 0..10 {
            let lo = -1.0 + i as f64 * 0.2;
            let hi = lo + 0.2;
            let count = h.iter().filter(|&&v| v >= lo && v < hi).count();
            print!("{:.0}% ", 100.0 * count as f64 / n);
        }
        println!();

        // Light + temperature breakdown
        let light = &map.light_level;
        let temp = &map.temperature;
        let l_mean = light.iter().sum::<f64>() / n;
        let t_mean = temp.iter().sum::<f64>() / n;
        let above45 = temp.iter().filter(|&&t| t > 45.0).count();
        let l_min = light.iter().cloned().fold(f64::INFINITY, f64::min);
        let l_max = light.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let t_min = temp.iter().cloned().fold(f64::INFINITY, f64::min);
        let t_max = temp.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        println!("  light_level : [{l_min:.3}, {l_max:.3}]   mean: {l_mean:.3}");
        println!("  temperature : [{t_min:.1}°C, {t_max:.1}°C]   mean: {t_mean:.1}°C");
        println!(
            "  above 45°C  : {:.1}%  (forced Arid)",
            100.0 * above45 as f64 / n
        );
        println!();
        println!("  top biome colors (by pixel count):");
        for (count, hex) in biome_vec.iter().take(8) {
            println!("    {}  {:5.1}%", hex, 100.0 * *count as f64 / n);
        }
        println!("────────────────────────────────────────────────────────────");
    }

    // ── Save debug PNGs ───────────────────────────────────────────────────────
    let chunk_coord = (world_x as i32, world_y as i32);
    let manifest = LevelManifest {
        parent_layers_tag: Some(layers_tag.to_string()),
        seed,
        chunk_coord,
        created: Utc::now().to_rfc3339(),
    };

    store
        .save_level(level_tag, &map, &manifest)
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save level: {e}");
            std::process::exit(1);
        });

    let level_dir = store.base_path().join("levels").join(level_tag);
    let png_dir = level_dir.join("images");
    fs::create_dir_all(&png_dir).ok();
    map.save_all_debug_pngs(&png_dir)
        .unwrap_or_else(|e| eprintln!("warn: PNG save failed: {e}"));
    write_runtime_presentation_summary(&level_dir, &runtime_presentation).unwrap_or_else(|e| {
        eprintln!("warn: runtime presentation save failed: {e}");
    });

    println!(
        "Saved to {}/levels/{level_tag}/",
        store.base_path().display()
    );
    println!("  PNGs: {}", png_dir.display());
    println!(
        "  Runtime presentation: {}",
        level_dir.join("runtime_presentation.ron").display()
    );
    print_runtime_presentation(&runtime_presentation);
    println!("  total: {:.1}s", t0.elapsed().as_secs_f64());
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn run_inspect_level_presentation(level_tag: &str) {
    let store = ArtifactStore::new().unwrap_or_else(|e| {
        eprintln!("error: failed to open artifact store: {e}");
        std::process::exit(1);
    });

    let (map, manifest) = store.load_level(level_tag).unwrap_or_else(|e| {
        eprintln!("error: could not load level artifact '{level_tag}': {e}");
        std::process::exit(1);
    });

    let runtime_presentation = map.build_runtime_chunk_presentation_bundle();
    println!(
        "Inspecting level runtime presentation — tag={level_tag}, seed={}, chunk_coord=({}, {})",
        manifest.seed, manifest.chunk_coord.0, manifest.chunk_coord.1,
    );
    print_runtime_presentation(&runtime_presentation);
}

fn run_inspect_layer_presentation_grid(
    layers_tag: &str,
    step: u32,
    audit_defaults: bool,
    golden_path: Option<&Path>,
) {
    if step == 0 {
        eprintln!("error: step must be greater than zero");
        std::process::exit(1);
    }

    let store = ArtifactStore::new().unwrap_or_else(|e| {
        eprintln!("error: failed to open artifact store: {e}");
        std::process::exit(1);
    });

    let manifest = store.load_layer_manifest(layers_tag).unwrap_or_else(|e| {
        eprintln!("error: could not load layers artifact '{layers_tag}': {e}");
        std::process::exit(1);
    });

    let pb = spinner("Scanning runtime presentation grid…");
    let scan = scan_layer_presentation_grid(
        layers_tag,
        manifest.seed,
        manifest.world_width,
        manifest.world_height,
        step,
        |scanned, total| {
            if scanned == total || scanned % 4 == 0 {
                pb.set_message(format!(
                    "Scanning runtime presentation grid… {scanned}/{total}"
                ));
            }
        },
    );
    pb.finish_and_clear();

    let inspection_dir = store
        .base_path()
        .join("layers")
        .join(layers_tag)
        .join("inspections");
    fs::create_dir_all(&inspection_dir).unwrap_or_else(|e| {
        eprintln!(
            "error: failed to create inspection output dir {}: {e}",
            inspection_dir.display()
        );
        std::process::exit(1);
    });

    let csv_path = inspection_dir.join(format!("presentation_grid_step_{step}.csv"));
    fs::write(&csv_path, scan.csv.as_bytes()).unwrap_or_else(|e| {
        eprintln!("error: failed to write {}: {e}", csv_path.display());
        std::process::exit(1);
    });

    let summary_path = inspection_dir.join(format!("presentation_grid_step_{step}.ron"));
    write_pretty_ron(&summary_path, &scan.summary).unwrap_or_else(|e| {
        eprintln!("error: failed to write {}: {e}", summary_path.display());
        std::process::exit(1);
    });

    let mut failures: Vec<String> = Vec::new();
    let mut golden_matched = false;
    if audit_defaults {
        failures.extend(audit_default_presentation_grid(&scan.summary));
    }
    if let Some(path) = golden_path {
        let expected = read_presentation_grid_summary(path).unwrap_or_else(|e| {
            eprintln!(
                "error: failed to read golden fixture {}: {e}",
                path.display()
            );
            std::process::exit(1);
        });
        let golden_failures = compare_presentation_grid_summaries(&expected, &scan.summary);
        golden_matched = golden_failures.is_empty();
        failures.extend(golden_failures);
    }

    println!(
        "Scanned runtime presentation grid — layers={layers_tag}, seed={}, step={}, samples={}",
        manifest.seed, step, scan.summary.sample_count,
    );
    println!("  CSV: {}", csv_path.display());
    println!("  Summary: {}", summary_path.display());
    print_count_report("planet zones", &scan.summary.planet_zone_counts);
    print_count_report("atmosphere classes", &scan.summary.atmosphere_class_counts);
    print_count_report("water states", &scan.summary.water_state_counts);
    print_count_report("landforms", &scan.summary.landform_class_counts);
    print_count_report(
        "surface palettes",
        &scan.summary.surface_palette_class_counts,
    );
    println!(
        "  interestingness: min={:.3} avg={:.3} max={:.3}",
        scan.summary.interestingness_min,
        scan.summary.interestingness_avg,
        scan.summary.interestingness_max,
    );

    if let Some(path) = golden_path {
        if golden_matched {
            println!("  Golden: matched {}", path.display());
        }
    }
    if !failures.is_empty() {
        println!("  Checks: failed");
        for failure in &failures {
            println!("    {failure}");
        }
    } else if audit_defaults || golden_path.is_some() {
        println!("  Checks: passed");
    }
    if !failures.is_empty() {
        std::process::exit(1);
    }
}

fn run_inspect_chunk_presentation(
    seed: u32,
    world_x: f64,
    world_y: f64,
    format: ChunkPresentationFormat,
) {
    let runtime_presentation = build_chunk_presentation_bundle(seed, world_x, world_y);
    match format {
        ChunkPresentationFormat::Text => {
            println!(
                "Inspecting generated chunk runtime presentation — seed={seed}, world=({world_x},{world_y})"
            );
            print_runtime_presentation(&runtime_presentation);
        }
        ChunkPresentationFormat::Json => {
            let text = serde_json::to_string_pretty(&presentation_report(&runtime_presentation))
                .unwrap_or_else(|e| {
                    eprintln!("error: failed to serialize chunk presentation as JSON: {e}");
                    std::process::exit(1);
                });
            println!("{text}");
        }
        ChunkPresentationFormat::Ron => {
            let text = ron::ser::to_string_pretty(
                &presentation_report(&runtime_presentation),
                ron::ser::PrettyConfig::default(),
            )
            .unwrap_or_else(|e| {
                eprintln!("error: failed to serialize chunk presentation as RON: {e}");
                std::process::exit(1);
            });
            println!("{text}");
        }
    }
}

fn scan_layer_presentation_grid<F>(
    layers_tag: &str,
    seed: u32,
    world_width: u32,
    world_height: u32,
    step: u32,
    mut on_progress: F,
) -> PresentationGridScan
where
    F: FnMut(usize, usize),
{
    let sample_coords = sampled_chunk_origins(world_width, world_height, step);
    let total_samples = sample_coords.len();
    let mut csv = String::from(
        "world_x,world_y,planet_zone,atmosphere_class,water_state,landform_class,surface_palette_class,interestingness_score,average_light_level,average_temperature,average_humidity,average_aridity,average_snowpack,average_water_table\n",
    );
    let mut planet_zone_counts = BTreeMap::new();
    let mut atmosphere_class_counts = BTreeMap::new();
    let mut water_state_counts = BTreeMap::new();
    let mut landform_class_counts = BTreeMap::new();
    let mut surface_palette_class_counts = BTreeMap::new();
    let mut interestingness_min = f32::INFINITY;
    let mut interestingness_max = f32::NEG_INFINITY;
    let mut interestingness_sum = 0.0_f64;

    for (index, (world_x, world_y)) in sample_coords.iter().copied().enumerate() {
        let summary = build_chunk_presentation(seed, f64::from(world_x), f64::from(world_y));

        increment_count(&mut planet_zone_counts, summary.planet_zone.as_str());
        increment_count(
            &mut atmosphere_class_counts,
            summary.atmosphere_class.as_str(),
        );
        increment_count(&mut water_state_counts, summary.water_state.as_str());
        increment_count(
            &mut surface_palette_class_counts,
            summary.surface_palette_class.as_str(),
        );
        increment_count(&mut landform_class_counts, summary.landform_class.as_str());
        interestingness_min = interestingness_min.min(summary.interestingness_score);
        interestingness_max = interestingness_max.max(summary.interestingness_score);
        interestingness_sum += f64::from(summary.interestingness_score);

        csv.push_str(&format!(
            "{world_x},{world_y},{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\n",
            summary.planet_zone.as_str(),
            summary.atmosphere_class.as_str(),
            summary.water_state.as_str(),
            summary.landform_class.as_str(),
            summary.surface_palette_class.as_str(),
            summary.interestingness_score,
            summary.average_light_level,
            summary.average_temperature,
            summary.average_humidity,
            summary.average_aridity,
            summary.average_snowpack,
            summary.average_water_table,
        ));
        on_progress(index + 1, total_samples);
    }

    PresentationGridScan {
        summary: PresentationGridSummary {
            layers_tag: layers_tag.to_string(),
            seed,
            step,
            world_width,
            world_height,
            sample_count: total_samples,
            planet_zone_counts,
            atmosphere_class_counts,
            water_state_counts,
            landform_class_counts,
            surface_palette_class_counts,
            interestingness_min: if total_samples > 0 {
                interestingness_min
            } else {
                0.0
            },
            interestingness_avg: if total_samples > 0 {
                (interestingness_sum / total_samples as f64) as f32
            } else {
                0.0
            },
            interestingness_max: if total_samples > 0 {
                interestingness_max
            } else {
                0.0
            },
        },
        csv,
    }
}

fn audit_default_presentation_grid(summary: &PresentationGridSummary) -> Vec<String> {
    let mut failures = Vec::new();
    if !summary
        .planet_zone_counts
        .keys()
        .any(|zone| is_dayside_zone(zone.as_str()))
    {
        failures.push("missing dayside presentation coverage".to_string());
    }
    if !summary
        .planet_zone_counts
        .keys()
        .any(|zone| is_terminus_zone(zone.as_str()))
    {
        failures.push("missing terminus presentation coverage".to_string());
    }
    if !summary
        .planet_zone_counts
        .keys()
        .any(|zone| is_nightside_zone(zone.as_str()))
    {
        failures.push("missing nightside presentation coverage".to_string());
    }
    if summary.water_state_counts.len() < 2 {
        failures.push("water-state scan collapsed to fewer than two distinct states".to_string());
    }
    if summary.landform_class_counts.len() < 4 {
        failures.push("landform scan produced fewer than four distinct classes".to_string());
    }
    if summary.interestingness_max - summary.interestingness_min < 0.10 {
        failures.push("interestingness spread was too narrow across the sampled grid".to_string());
    }
    failures
}

fn compare_presentation_grid_summaries(
    expected: &PresentationGridSummary,
    actual: &PresentationGridSummary,
) -> Vec<String> {
    let mut failures = Vec::new();
    if expected.layers_tag != actual.layers_tag {
        failures.push(format!(
            "golden layers_tag mismatch: expected {}, got {}",
            expected.layers_tag, actual.layers_tag
        ));
    }
    if expected.seed != actual.seed {
        failures.push(format!(
            "golden seed mismatch: expected {}, got {}",
            expected.seed, actual.seed
        ));
    }
    if expected.step != actual.step {
        failures.push(format!(
            "golden step mismatch: expected {}, got {}",
            expected.step, actual.step
        ));
    }
    if expected.world_width != actual.world_width || expected.world_height != actual.world_height {
        failures.push(format!(
            "golden world dimensions mismatch: expected {}x{}, got {}x{}",
            expected.world_width, expected.world_height, actual.world_width, actual.world_height
        ));
    }
    if expected.sample_count != actual.sample_count {
        failures.push(format!(
            "golden sample_count mismatch: expected {}, got {}",
            expected.sample_count, actual.sample_count
        ));
    }
    failures.extend(diff_count_maps(
        "planet_zone_counts",
        &expected.planet_zone_counts,
        &actual.planet_zone_counts,
    ));
    failures.extend(diff_count_maps(
        "atmosphere_class_counts",
        &expected.atmosphere_class_counts,
        &actual.atmosphere_class_counts,
    ));
    failures.extend(diff_count_maps(
        "water_state_counts",
        &expected.water_state_counts,
        &actual.water_state_counts,
    ));
    failures.extend(diff_count_maps(
        "landform_class_counts",
        &expected.landform_class_counts,
        &actual.landform_class_counts,
    ));
    failures.extend(diff_count_maps(
        "surface_palette_class_counts",
        &expected.surface_palette_class_counts,
        &actual.surface_palette_class_counts,
    ));
    if (expected.interestingness_min - actual.interestingness_min).abs() > 1.0e-6 {
        failures.push(format!(
            "golden interestingness_min mismatch: expected {:.6}, got {:.6}",
            expected.interestingness_min, actual.interestingness_min
        ));
    }
    if (expected.interestingness_avg - actual.interestingness_avg).abs() > 1.0e-6 {
        failures.push(format!(
            "golden interestingness_avg mismatch: expected {:.6}, got {:.6}",
            expected.interestingness_avg, actual.interestingness_avg
        ));
    }
    if (expected.interestingness_max - actual.interestingness_max).abs() > 1.0e-6 {
        failures.push(format!(
            "golden interestingness_max mismatch: expected {:.6}, got {:.6}",
            expected.interestingness_max, actual.interestingness_max
        ));
    }
    failures
}

fn diff_count_maps(
    label: &str,
    expected: &BTreeMap<String, usize>,
    actual: &BTreeMap<String, usize>,
) -> Vec<String> {
    if expected == actual {
        return Vec::new();
    }
    vec![format!(
        "golden {} mismatch: expected {:?}, got {:?}",
        label, expected, actual
    )]
}

fn build_chunk_presentation(seed: u32, world_x: f64, world_y: f64) -> RuntimeChunkPresentation {
    build_chunk_presentation_bundle(seed, world_x, world_y).summary
}

fn build_chunk_presentation_bundle(
    seed: u32,
    world_x: f64,
    world_y: f64,
) -> RuntimeChunkPresentationBundle {
    generate_runtime_micro_map(seed, world_x, world_y).build_runtime_chunk_presentation_bundle()
}

fn presentation_report(bundle: &RuntimeChunkPresentationBundle) -> RuntimeChunkPresentationReport {
    RuntimeChunkPresentationReport {
        summary: bundle.summary.clone(),
        reduced_grids: ReducedGridReport {
            water_state_grid: grid_metadata(
                bundle.reduced_grids.water_state_grid.width,
                bundle.reduced_grids.water_state_grid.height,
                bundle.reduced_grids.water_state_digest(),
            ),
            landform_grid: grid_metadata(
                bundle.reduced_grids.landform_grid.width,
                bundle.reduced_grids.landform_grid.height,
                bundle.reduced_grids.landform_digest(),
            ),
            surface_palette_grid: grid_metadata(
                bundle.reduced_grids.surface_palette_grid.width,
                bundle.reduced_grids.surface_palette_grid.height,
                bundle.reduced_grids.surface_palette_digest(),
            ),
        },
    }
}

fn grid_metadata(width: usize, height: usize, digest: String) -> ReducedGridMetadata {
    ReducedGridMetadata {
        width,
        height,
        digest,
    }
}

fn read_presentation_grid_summary(path: &Path) -> Result<PresentationGridSummary, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    ron::de::from_str(&text).map_err(|e| format!("deserializing {}: {e}", path.display()))
}

fn is_dayside_zone(zone: &str) -> bool {
    matches!(
        zone,
        "SubstellarInferno" | "ScorchBelt" | "DryDaysideMargin"
    )
}

fn is_terminus_zone(zone: &str) -> bool {
    matches!(zone, "InnerTerminus" | "OuterTerminus" | "ColdTerminus")
}

fn is_nightside_zone(zone: &str) -> bool {
    matches!(
        zone,
        "FrostMargin" | "FrozenCoast" | "DeepNightIce" | "AbyssalNight"
    )
}

fn print_runtime_presentation(bundle: &RuntimeChunkPresentationBundle) {
    let summary = &bundle.summary;
    println!("  runtime presentation:");
    println!("    planet_zone      : {}", summary.planet_zone.as_str());
    println!(
        "    atmosphere_class : {}",
        summary.atmosphere_class.as_str()
    );
    println!("    water_state      : {}", summary.water_state.as_str());
    println!("    landform_class   : {}", summary.landform_class.as_str());
    println!(
        "    surface_palette  : {}",
        summary.surface_palette_class.as_str()
    );
    println!(
        "    interestingness  : {:.3}",
        summary.interestingness_score
    );
    println!("    avg light        : {:.3}", summary.average_light_level);
    println!(
        "    avg temperature  : {:.2} C",
        summary.average_temperature
    );
    println!("    avg humidity     : {:.3}", summary.average_humidity);
    println!("    avg aridity      : {:.3}", summary.average_aridity);
    println!("    avg snowpack     : {:.3}", summary.average_snowpack);
    println!("    avg water_table  : {:.3}", summary.average_water_table);
    print_reduced_grid_report(&bundle.reduced_grids);
}

fn print_reduced_grid_report(grids: &RuntimeChunkPresentationGrids) {
    println!("    reduced_grids:");
    println!(
        "      water_state    : {}x{} {}",
        grids.water_state_grid.width,
        grids.water_state_grid.height,
        grids.water_state_digest()
    );
    println!(
        "      landform       : {}x{} {}",
        grids.landform_grid.width,
        grids.landform_grid.height,
        grids.landform_digest()
    );
    println!(
        "      surface_palette: {}x{} {}",
        grids.surface_palette_grid.width,
        grids.surface_palette_grid.height,
        grids.surface_palette_digest()
    );
}

fn write_runtime_presentation_summary(
    level_dir: &Path,
    bundle: &RuntimeChunkPresentationBundle,
) -> Result<(), String> {
    let summary_path = level_dir.join("runtime_presentation.ron");
    write_pretty_ron(&summary_path, &presentation_report(bundle))
}

fn generate_runtime_micro_map(seed: u32, world_x: f64, world_y: f64) -> BiomeMap {
    BiomeMap::generate(
        seed,
        world_x,
        world_y,
        MICRO_CHUNK_WORLD_SIZE,
        MICRO_CHUNK_WORLD_SIZE,
        MICRO_TILE_RESOLUTION,
        MICRO_TILE_RESOLUTION,
        MICRO_DETAIL_LEVEL,
        false,
        false,
        MICRO_FREQUENCY_SCALE,
    )
}

fn sampled_chunk_origins(world_width: u32, world_height: u32, step: u32) -> Vec<(u32, u32)> {
    let mut coords = Vec::new();
    for world_y in (0..world_height).step_by(step as usize) {
        for world_x in (0..world_width).step_by(step as usize) {
            coords.push((world_x, world_y));
        }
    }
    coords
}

fn increment_count(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_insert(0) += 1;
}

fn print_count_report(label: &str, counts: &BTreeMap<String, usize>) {
    println!("  {}:", label);
    for (name, count) in counts {
        println!("    {name}: {count}");
    }
}

fn write_pretty_ron<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())
        .map_err(|e| format!("serializing {}: {e}", path.display()))?;
    fs::write(path, text.as_bytes()).map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{
        audit_default_presentation_grid, compare_presentation_grid_summaries,
        read_presentation_grid_summary, scan_layer_presentation_grid,
    };
    use std::path::PathBuf;

    fn golden_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/runtime_presentation/seed42_v1_step256.ron")
    }

    #[test]
    fn default_grid_audit_passes_for_seed_42_step_256() {
        let scan = scan_layer_presentation_grid("v1", 42, 1024, 512, 256, |_scanned, _total| {});
        let failures = audit_default_presentation_grid(&scan.summary);
        assert!(
            failures.is_empty(),
            "expected default audit to pass, got {:?}",
            failures
        );
    }

    #[test]
    fn presentation_grid_matches_seed_42_golden_fixture() {
        let expected = read_presentation_grid_summary(&golden_fixture_path())
            .expect("golden fixture should load");
        let actual = scan_layer_presentation_grid("v1", 42, 1024, 512, 256, |_scanned, _total| {});
        let failures = compare_presentation_grid_summaries(&expected, &actual.summary);
        assert!(
            failures.is_empty(),
            "expected golden summary to match, got {:?}",
            failures
        );
    }
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}
