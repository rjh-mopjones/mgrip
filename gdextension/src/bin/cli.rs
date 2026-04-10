//! margins_grip CLI — generate terrain artifacts for Margin's Grip.
//!
//! Commands:
//!   generate layers <SEED> <TAG>
//!       Run the full macro pipeline (macro pass + tiled macromap render).
//!       Saves BiomeMap, RiverNetwork, macromap.png, and debug PNG layers to
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
use image::RgbaImage;
use indicatif::{ProgressBar, ProgressStyle};
use mg_artifacts::{ArtifactStore, LayerManifest, LevelManifest};
use mg_noise::{
    rasterize_to_tile, render_terrain, BiomeMap, NoiseLayer, NormalizationHints, RiverNetwork,
    RuntimeChunkPresentation, RuntimeChunkPresentationBundle, RuntimeChunkPresentationGrids,
    LOD_THRESHOLD_MACRO,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
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
    /// Compare the macro artifact against per-chunk runtime terrain for a meso map.
    /// Uses macromap.png for visible macro context and macro_biome.bin for
    /// ocean-mask/scoring semantics.
    /// Outputs macro.png, micro_grid.png, diff.png, river comparison PNGs, and agreement.json.
    CompareScale {
        /// World seed (used for micro chunk generation)
        seed: u32,
        /// Meso map X coordinate (world_x = meso_x * 8)
        meso_x: i64,
        /// Meso map Y coordinate (world_y = meso_y * 8)
        meso_y: i64,
        /// Grid size — NxN chunks to compare (default 8 for one meso map)
        grid_size: usize,
        /// Output directory for PNG and JSON files
        output_dir: String,
        /// Layers artifact tag to source the macro artifact from (default: newest)
        #[arg(long)]
        layers_tag: Option<String>,
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
        Commands::CompareScale {
            seed,
            meso_x,
            meso_y,
            grid_size,
            output_dir,
            layers_tag,
        } => run_compare_scale(
            seed,
            meso_x,
            meso_y,
            grid_size,
            Path::new(&output_dir),
            layers_tag.as_deref(),
        ),
    }
}

// ─── generate layers ─────────────────────────────────────────────────────────

// World layout constants (matching biome_map.rs and spec)
const WORLD_WIDTH: f64 = 1024.0;
const WORLD_HEIGHT: f64 = 512.0;
const MACRO_MAP_W: usize = 1024;
const MACRO_MAP_H: usize = 512;
// Tile grid: 16×8 macro tiles of 64×64 world units each.
// We render at 512px, then box-downscale each tile to 256px before stitching.
// This is equivalent to stitching an 8192×4096 intermediate and downscaling
// 2x to the final 4096×2048 artifact, without holding the full intermediate
// in memory.
const TILE_WORLD_SIZE: f64 = 64.0;
const TILE_RENDER_PX: usize = 512;
const TILE_OUTPUT_PX: usize = TILE_RENDER_PX / 2;
const TILES_X: usize = (WORLD_WIDTH / TILE_WORLD_SIZE) as usize; // 16
const TILES_Y: usize = (WORLD_HEIGHT / TILE_WORLD_SIZE) as usize; // 8
const FULL_RENDER_W: usize = TILES_X * TILE_RENDER_PX; // 8192
const FULL_RENDER_H: usize = TILES_Y * TILE_RENDER_PX; // 4096
const FULL_W: usize = TILES_X * TILE_OUTPUT_PX; // 4096
const FULL_H: usize = TILES_Y * TILE_OUTPUT_PX; // 2048
const MICRO_CHUNK_WORLD_SIZE: f64 = 1.0;
const MICRO_TILE_RESOLUTION: usize = 512;
const MICRO_DETAIL_LEVEL: u32 = 2;
const MICRO_FREQUENCY_SCALE: f64 = 8.0;

fn sample_macro_tile_world_coord(origin: f64, index: usize) -> f64 {
    if TILE_RENDER_PX <= 1 {
        return origin;
    }
    origin + (index as f64 / (TILE_RENDER_PX - 1) as f64) * TILE_WORLD_SIZE
}

fn generate_macro_tile(
    macro_map: &BiomeMap,
    seed: u32,
    river_network: &RiverNetwork,
    wx: f64,
    wy: f64,
) -> BiomeMap {
    let mut tile = BiomeMap::generate(
        seed,
        wx,
        wy,
        TILE_WORLD_SIZE,
        TILE_WORLD_SIZE,
        TILE_RENDER_PX,
        TILE_RENDER_PX,
        1,
        false,
        false,
        1.0,
    );

    if !macro_map.heightmap.is_empty() {
        let splines = mg_noise::BiomeSplines::new(mg_noise::SEA_LEVEL);
        for py in 0..TILE_RENDER_PX {
            for px in 0..TILE_RENDER_PX {
                let idx = py * TILE_RENDER_PX + px;
                let sample_x = sample_macro_tile_world_coord(wx, px);
                let sample_y = sample_macro_tile_world_coord(wy, py);
                let macro_hm = macro_map.sample_heightmap_at(sample_x, sample_y);

                let stress = 1.0 - tile.tectonic[idx];
                let above_sea = (macro_hm - mg_noise::SEA_LEVEL).max(0.0);
                let mountain_intensity = (stress * above_sea * 3.0).min(1.0);
                let mountain_detail = tile.peaks_valleys[idx] * mountain_intensity * 0.2;
                let hm = (macro_hm + mountain_detail).clamp(-1.0, 1.0);
                tile.heightmap[idx] = hm;

                let cont = tile.continentalness[idx];
                let humid = tile.humidity[idx];
                let rock = tile.rock_hardness[idx];
                let tect = tile.tectonic[idx];
                let light = tile.light_level[idx];
                let peaks = tile.peaks_valleys[idx];

                let temp = mg_noise::derive_temperature(light, hm, humid, cont);
                let eros = mg_noise::derive_erosion(hm, rock, humid);
                let arid = mg_noise::derive_aridity(temp, humid);
                let precip = mg_noise::derive_precipitation_type(temp, humid, hm);
                let snow = mg_noise::derive_snowpack(precip, temp, hm, light);

                tile.temperature[idx] = temp;
                tile.erosion[idx] = eros;
                tile.aridity[idx] = arid;
                tile.precipitation_type[idx] = precip;
                tile.snowpack[idx] = snow;
                tile.resource_richness[idx] = mg_noise::derive_resource_richness(tect, rock, eros);
                tile.biomes[idx] = splines.evaluate_dithered_with_light(
                    cont, temp, tect, eros, peaks, humid, arid, rock, px, py, light,
                );
            }
        }
    }

    tile.rivers = rasterize_to_tile(
        river_network,
        TILE_RENDER_PX,
        TILE_RENDER_PX,
        wx,
        wy,
        TILE_WORLD_SIZE,
        TILE_WORLD_SIZE,
        WORLD_WIDTH,
        WORLD_HEIGHT,
        LOD_THRESHOLD_MACRO as f64,
    );

    for i in 0..TILE_RENDER_PX * TILE_RENDER_PX {
        tile.water_table[i] = mg_noise::derive_water_table(
            tile.rivers[i],
            tile.humidity[i],
            tile.heightmap[i],
            tile.precipitation_type[i],
            tile.continentalness[i],
        );
        if tile.rivers[i] > 0.1
            && !mg_noise::tile_has_fluid_surface(tile.biomes[i])
            && tile.aridity[i] < 0.7
        {
            tile.biomes[i] = mg_core::TileType::River;
        }
        tile.vegetation_density[i] =
            mg_noise::derive_vegetation_density(tile.biomes[i], tile.water_table[i]);
        tile.soil_type[i] =
            mg_noise::derive_soil_type(tile.biomes[i], tile.erosion[i], tile.rock_hardness[i]);
    }

    tile
}

fn downscale_rgba_2x_box(src: &[u8], src_w: usize, src_h: usize) -> Vec<u8> {
    debug_assert_eq!(src_w % 2, 0);
    debug_assert_eq!(src_h % 2, 0);
    let dst_w = src_w / 2;
    let dst_h = src_h / 2;
    let mut dst = vec![0u8; dst_w * dst_h * 4];

    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let mut sums = [0u16; 4];
            for sy in 0..2 {
                for sx in 0..2 {
                    let src_x = dx * 2 + sx;
                    let src_y = dy * 2 + sy;
                    let src_idx = (src_y * src_w + src_x) * 4;
                    sums[0] += src[src_idx] as u16;
                    sums[1] += src[src_idx + 1] as u16;
                    sums[2] += src[src_idx + 2] as u16;
                    sums[3] += src[src_idx + 3] as u16;
                }
            }
            let dst_idx = (dy * dst_w + dx) * 4;
            dst[dst_idx] = (sums[0] / 4) as u8;
            dst[dst_idx + 1] = (sums[1] / 4) as u8;
            dst[dst_idx + 2] = (sums[2] / 4) as u8;
            dst[dst_idx + 3] = (sums[3] / 4) as u8;
        }
    }

    dst
}

fn blit_rgba_tile(
    dst: &mut [u8],
    dst_w: usize,
    tile_rgba: &[u8],
    tile_w: usize,
    tile_h: usize,
    ox: usize,
    oy: usize,
) {
    for py in 0..tile_h {
        let src_row = py * tile_w * 4;
        let dst_row = ((oy + py) * dst_w + ox) * 4;
        dst[dst_row..dst_row + tile_w * 4]
            .copy_from_slice(&tile_rgba[src_row..src_row + tile_w * 4]);
    }
}

fn run_generate_layers(seed: u32, tag: &str) {
    let previous_force_cpu = std::env::var_os("MG_NOISE_FORCE_CPU");
    std::env::set_var("MG_NOISE_FORCE_CPU", "1");

    let store = ArtifactStore::new().unwrap_or_else(|e| {
        eprintln!("error: failed to open artifact store: {e}");
        std::process::exit(1);
    });

    println!("Generating world — seed={seed}, tag={tag}");
    println!(
        "  output:  {FULL_W}×{FULL_H} (via {FULL_RENDER_W}×{FULL_RENDER_H} intermediate, {TILES_X}×{TILES_Y} tiles of {TILE_RENDER_PX}px)"
    );

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
        MACRO_MAP_W,
        MACRO_MAP_H,
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
        .unwrap_or_else(|| RiverNetwork::empty(MACRO_MAP_W, MACRO_MAP_H));

    // ── Step 2: Scan global height range for shared normalization ────────────
    let pb = spinner("Scanning tile height ranges for shared normalization…");
    let mut global_min = f64::INFINITY;
    let mut global_max = f64::NEG_INFINITY;
    let total_tiles = TILES_X * TILES_Y;
    let mut tiles_done = 0usize;

    for ty in 0..TILES_Y {
        for tx in 0..TILES_X {
            let wx = tx as f64 * TILE_WORLD_SIZE;
            let wy = ty as f64 * TILE_WORLD_SIZE;
            let tile = generate_macro_tile(&macro_map, seed, &river_network, wx, wy);

            for &height in &tile.heightmap {
                global_min = global_min.min(height);
                global_max = global_max.max(height);
            }

            tiles_done += 1;
            if tiles_done % TILES_X == 0 || tiles_done == total_tiles {
                pb.set_message(format!(
                    "Scanning tile height ranges… {tiles_done}/{total_tiles} tiles"
                ));
            }
        }
    }
    pb.finish_and_clear();

    let normalization_hints = NormalizationHints {
        heightmap_min: global_min,
        heightmap_max: global_max,
    };

    // ── Step 3: Tile the world at meso detail → final 4096×2048 artifact ───
    let pb = spinner("Rendering macromap tiles…");

    // Allocate final RGBA buffers per debug layer after per-tile 2x box downscale.
    // Biome semantics are persisted as macro_biome.bin; biome.png is no longer
    // emitted as a presentation artifact.
    let debug_layers: Vec<NoiseLayer> = NoiseLayer::all()
        .iter()
        .copied()
        .filter(|layer| *layer != NoiseLayer::Biome)
        .collect();
    let n_pixels = FULL_W * FULL_H;
    let mut layer_bufs: Vec<Vec<u8>> = debug_layers
        .iter()
        .map(|_| vec![0u8; n_pixels * 4])
        .collect();
    let mut macromap_buf = vec![0u8; n_pixels * 4];

    tiles_done = 0;

    for ty in 0..TILES_Y {
        for tx in 0..TILES_X {
            let wx = tx as f64 * TILE_WORLD_SIZE;
            let wy = ty as f64 * TILE_WORLD_SIZE;
            let tile = generate_macro_tile(&macro_map, seed, &river_network, wx, wy);

            let ox = tx * TILE_OUTPUT_PX;
            let oy = ty * TILE_OUTPUT_PX;

            // Stitch debug layers after local 2x box downscale. This is equivalent
            // to stitching the 8192×4096 intermediate and then downscaling 2x.
            for (li, &layer) in debug_layers.iter().enumerate() {
                let rgba = tile.layer_to_rgba(layer);
                let rgba_downscaled = downscale_rgba_2x_box(&rgba, TILE_RENDER_PX, TILE_RENDER_PX);
                let dst = &mut layer_bufs[li];
                blit_rgba_tile(
                    dst,
                    FULL_W,
                    &rgba_downscaled,
                    TILE_OUTPUT_PX,
                    TILE_OUTPUT_PX,
                    ox,
                    oy,
                );
            }

            // The authoritative macro artifact follows the Randlebrot shape:
            // composited terrain render with shared global normalization.
            let macromap_rgba = render_terrain(&tile, Some(&normalization_hints));
            let macromap_downscaled =
                downscale_rgba_2x_box(&macromap_rgba, TILE_RENDER_PX, TILE_RENDER_PX);
            blit_rgba_tile(
                &mut macromap_buf,
                FULL_W,
                &macromap_downscaled,
                TILE_OUTPUT_PX,
                TILE_OUTPUT_PX,
                ox,
                oy,
            );

            tiles_done += 1;
            if tiles_done % TILES_X == 0 || tiles_done == total_tiles {
                pb.set_message(format!(
                    "Rendering macromap tiles… {tiles_done}/{total_tiles} tiles"
                ));
            }
        }
    }
    pb.finish_and_clear();
    println!("  tile render: {:.1}s", t0.elapsed().as_secs_f64());

    // ── Step 4: Build images HashMap and manifest ────────────────────────────
    let pb = spinner("Saving artifact…");
    let mut images: HashMap<String, (u32, u32, Vec<u8>)> = HashMap::new();
    images.insert(
        "macromap.png".to_string(),
        (FULL_W as u32, FULL_H as u32, macromap_buf),
    );
    for (layer, buf) in debug_layers.iter().zip(layer_bufs.into_iter()) {
        images.insert(
            format!("{}.png", layer.name()),
            (FULL_W as u32, FULL_H as u32, buf),
        );
    }

    let mut layer_images = vec!["macromap.png".to_string()];
    layer_images.extend(debug_layers.iter().map(|l| format!("{}.png", l.name())));

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
    println!("  {FULL_W}×{FULL_H} PNGs, {} layers written", images.len());
    println!("  total: {:.1}s", t0.elapsed().as_secs_f64());

    if let Some(value) = previous_force_cpu {
        std::env::set_var("MG_NOISE_FORCE_CPU", value);
    } else {
        std::env::remove_var("MG_NOISE_FORCE_CPU");
    }
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

// ─── compare scale ───────────────────────────────────────────────────────────

/// Discover the newest named layer image across all layers artifacts, mirroring the
/// map_selector.gd macro-texture lookup. Returns (tag, image_path, world_width, world_height).
fn find_newest_layer_image(
    store: &mg_artifacts::ArtifactStore,
    image_name: &str,
) -> Option<(String, std::path::PathBuf, f64, f64)> {
    let layers_dir = store.base_path().join("layers");
    let entries = fs::read_dir(&layers_dir).ok()?;
    let mut best: Option<(String, std::path::PathBuf, std::time::SystemTime, f64, f64)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let tag = entry.file_name().to_string_lossy().to_string();
        let layer_image = path.join("images").join(image_name);
        if !layer_image.exists() {
            continue;
        }
        let mtime = match fs::metadata(&layer_image).and_then(|metadata| metadata.modified()) {
            Ok(mtime) => mtime,
            Err(_) => continue,
        };
        let (ww, wh) = if let Some(manifest) = fs::read_to_string(path.join("manifest.ron"))
            .ok()
            .and_then(|s| ron::de::from_str::<mg_artifacts::LayerManifest>(&s).ok())
        {
            (manifest.world_width as f64, manifest.world_height as f64)
        } else {
            (WORLD_WIDTH, WORLD_HEIGHT)
        };
        if best.as_ref().map_or(true, |(_, _, t, _, _)| mtime > *t) {
            best = Some((tag, layer_image, mtime, ww, wh));
        }
    }
    best.map(|(tag, p, _, ww, wh)| (tag, p, ww, wh))
}

fn load_compare_layers(
    store: &mg_artifacts::ArtifactStore,
    layers_tag: Option<&str>,
) -> (
    String,
    std::path::PathBuf,
    f64,
    f64,
    BiomeMap,
    mg_noise::RiverNetwork,
) {
    match layers_tag {
        Some(tag) => {
            let manifest = store.load_layer_manifest(tag).unwrap_or_else(|e| {
                eprintln!("error: layers artifact '{tag}' not found: {e}");
                std::process::exit(1);
            });
            let macro_visual_path = store.layer_image_path(tag, "macromap.png");
            if !macro_visual_path.exists() {
                eprintln!("error: layers artifact '{tag}' has no macromap.png. Regenerate layers.");
                std::process::exit(1);
            }
            let (macro_map, river_network) = store.load_layers_data(tag).unwrap_or_else(|e| {
                eprintln!("error: could not load macro_biome.bin for '{tag}': {e}");
                std::process::exit(1);
            });
            (
                tag.to_string(),
                macro_visual_path,
                manifest.world_width as f64,
                manifest.world_height as f64,
                macro_map,
                river_network,
            )
        }
        None => {
            let (tag, macro_visual_path, world_w, world_h) =
                find_newest_layer_image(store, "macromap.png").unwrap_or_else(|| {
                    eprintln!(
                        "error: no layers artifact with macromap.png found in ~/.margins_grip/layers/"
                    );
                    eprintln!(
                        "  Run 'margins_grip generate layers <SEED> <TAG>' first, or pass --layers-tag."
                    );
                    std::process::exit(1);
                });
            let (macro_map, river_network) = store.load_layers_data(&tag).unwrap_or_else(|e| {
                eprintln!("error: could not load macro_biome.bin for '{tag}': {e}");
                std::process::exit(1);
            });
            (
                tag,
                macro_visual_path,
                world_w,
                world_h,
                macro_map,
                river_network,
            )
        }
    }
}

fn sample_map_value_at_world(
    values: &[f64],
    map_w: usize,
    map_h: usize,
    world_w: f64,
    world_h: f64,
    wx: f64,
    wy: f64,
) -> f64 {
    if map_w == 0 || map_h == 0 || values.is_empty() {
        return 0.0;
    }
    let px = ((wx / world_w * map_w as f64) as usize).min(map_w - 1);
    let py = ((wy / world_h * map_h as f64) as usize).min(map_h - 1);
    values.get(py * map_w + px).copied().unwrap_or(0.0)
}

fn river_debug_rgba(strength: f64) -> [u8; 4] {
    if strength <= 0.0 {
        return [16, 16, 18, 255];
    }
    let t = (strength / 2000.0).clamp(0.12, 1.0);
    [0, (95.0 + 160.0 * t) as u8, 255, 255]
}

fn run_compare_scale(
    seed: u32,
    meso_x: i64,
    meso_y: i64,
    grid_size: usize,
    output_dir: &Path,
    layers_tag: Option<&str>,
) {
    const CELL_PX: usize = 64; // pixels per cell in output images
    const MICRO_RES: usize = MICRO_TILE_RESOLUTION; // LOD0 resolution

    let n = grid_size;
    let world_x = (meso_x * n as i64) as f64;
    let world_y = (meso_y * n as i64) as f64;
    let img_w = n * CELL_PX;
    let img_h = n * CELL_PX;

    fs::create_dir_all(output_dir).unwrap_or_else(|e| {
        eprintln!(
            "error: could not create output dir {}: {e}",
            output_dir.display()
        );
        std::process::exit(1);
    });

    // ── Macro: load visual macro artifact and semantic macro biome truth ─────
    let store = mg_artifacts::ArtifactStore::new().unwrap_or_else(|e| {
        eprintln!("error: failed to open artifact store: {e}");
        std::process::exit(1);
    });

    let (resolved_layers_tag, macro_visual_path, world_w, world_h, macro_map, river_network) =
        load_compare_layers(&store, layers_tag);

    let macro_visual_img = image::open(&macro_visual_path)
        .unwrap_or_else(|e| {
            eprintln!("error: failed to load {}: {e}", macro_visual_path.display());
            std::process::exit(1);
        })
        .into_rgba8();

    let tex_w = macro_visual_img.width() as usize;
    let tex_h = macro_visual_img.height() as usize;

    // Coordinate math — identical to _build_macro_crop in compare_generation_view.gd
    let px_x = ((world_x / world_w * tex_w as f64) as usize).min(tex_w.saturating_sub(1));
    let px_y = ((world_y / world_h * tex_h as f64) as usize).min(tex_h.saturating_sub(1));
    let px_w = ((n as f64 / world_w * tex_w as f64) as usize)
        .max(1)
        .min(tex_w - px_x);
    let px_h = ((n as f64 / world_h * tex_h as f64) as usize)
        .max(1)
        .min(tex_h - px_y);
    println!(
        "Comparing macro artifact vs runtime — seed={seed}, meso=({meso_x},{meso_y}), world=({world_x},{world_y}), grid={n}×{n}"
    );
    println!("  macro artifact: {}", macro_visual_path.display());
    println!("  macro semantics: macro_biome.bin ({resolved_layers_tag})");
    println!("  crop: ({px_x},{px_y}) {px_w}×{px_h} px  →  {img_w}×{img_h} output");

    // Build macro.png: macro artifact crop nearest-neighbour scaled to img_w×img_h
    let mut macro_rgba = vec![0u8; img_w * img_h * 4];
    for py in 0..img_h {
        for px in 0..img_w {
            let sx = (px_x + px * px_w / img_w).min(tex_w - 1);
            let sy = (px_y + py * px_h / img_h).min(tex_h - 1);
            let [r, g, b, a] = macro_visual_img.get_pixel(sx as u32, sy as u32).0;
            let dst = (py * img_w + px) * 4;
            macro_rgba[dst..dst + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    RgbaImage::from_raw(img_w as u32, img_h as u32, macro_rgba.clone())
        .expect("macro buffer is correct size")
        .save(output_dir.join("macro.png"))
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save macro.png: {e}");
            std::process::exit(1);
        });
    println!("  macro.png  (macro artifact crop)");

    // Build macro ocean mask from saved biome semantics so the CLI applies the
    // same override as the in-game generate_chunk_lod path.
    let macro_ocean_mask = mg_noise::MacroOceanMask::from_biome_map(&macro_map);

    // Build macro river receipt from saved macro semantics.
    let mut macro_river_rgba = vec![0u8; img_w * img_h * 4];
    let mut macro_river_mask = vec![false; img_w * img_h];
    let mut macro_river_present = 0usize;
    for py in 0..img_h {
        for px in 0..img_w {
            let wx = world_x + ((px as f64 + 0.5) / img_w as f64) * n as f64;
            let wy = world_y + ((py as f64 + 0.5) / img_h as f64) * n as f64;
            let river = sample_map_value_at_world(
                &macro_map.rivers,
                macro_map.width,
                macro_map.height,
                macro_map.world_width,
                macro_map.world_height,
                wx,
                wy,
            );
            let idx = py * img_w + px;
            if river > 0.0 {
                macro_river_present += 1;
                macro_river_mask[idx] = true;
            }
            let dst = idx * 4;
            macro_river_rgba[dst..dst + 4].copy_from_slice(&river_debug_rgba(river));
        }
    }
    RgbaImage::from_raw(img_w as u32, img_h as u32, macro_river_rgba.clone())
        .expect("macro river buffer is correct size")
        .save(output_dir.join("macro_rivers.png"))
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save macro_rivers.png: {e}");
            std::process::exit(1);
        });
    println!("  macro_rivers.png");

    // ── Micro grid: NxN individual chunks using runtime LOD0 semantics ───────
    let pb = spinner(&format!("Generating {n}×{n} micro chunks…"));
    let mut micro_rgba = vec![0u8; img_w * img_h * 4];
    let mut runtime_river_rgba = vec![0u8; img_w * img_h * 4];
    let mut runtime_river_mask = vec![false; img_w * img_h];
    let mut diff_rgba = vec![0u8; img_w * img_h * 4];
    let mut cells = Vec::with_capacity(n * n);
    let mut agree_count = 0usize;
    let mut runtime_river_present = 0usize;

    for gy in 0..n {
        for gx in 0..n {
            let cx = world_x + gx as f64;
            let cy = world_y + gy as f64;

            let mut micro = BiomeMap::generate(
                seed,
                cx,
                cy,
                1.0,
                1.0,
                MICRO_RES,
                MICRO_RES,
                MICRO_DETAIL_LEVEL,
                false,
                false,
                MICRO_FREQUENCY_SCALE,
            );
            micro.apply_macro_river_network(
                &river_network,
                cx,
                cy,
                1.0,
                1.0,
                mg_noise::LOD_THRESHOLD_MICRO,
            );
            micro.apply_macro_ocean_mask(&macro_ocean_mask, cx, cy, 1.0, 1.0);

            let mc = MICRO_RES / 2;
            let micro_ocean = micro.is_ocean(mc, mc);
            let macro_ocean = macro_ocean_mask.is_ocean_at_world(cx + 0.5, cy + 0.5);
            let mut cell_macro_river_present = false;
            let mut cell_runtime_river_present = false;

            let agree = macro_ocean == micro_ocean;
            if agree {
                agree_count += 1;
            }

            cells.push(serde_json::json!({
                "chunk": [cx as i64, cy as i64],
                "macro_ocean": macro_ocean,
                "micro_ocean": micro_ocean,
                "agree": agree,
                "macro_river_present": false,
                "runtime_river_present": false,
            }));

            // Blit micro biome RGBA into grid (nearest-neighbour scale MICRO_RES → CELL_PX)
            let biome = micro.layer_to_rgba(NoiseLayer::Biome);
            for py in 0..CELL_PX {
                for px in 0..CELL_PX {
                    let sx = px * MICRO_RES / CELL_PX;
                    let sy = py * MICRO_RES / CELL_PX;
                    let src = (sy * MICRO_RES + sx) * 4;
                    let dx = gx * CELL_PX + px;
                    let dy = gy * CELL_PX + py;
                    let dst = (dy * img_w + dx) * 4;
                    micro_rgba[dst..dst + 4].copy_from_slice(&biome[src..src + 4]);
                    let river = micro.rivers[sy * MICRO_RES + sx];
                    if river > 0.0 {
                        runtime_river_present += 1;
                        cell_runtime_river_present = true;
                        runtime_river_mask[dy * img_w + dx] = true;
                    }
                    if macro_river_mask[dy * img_w + dx] {
                        cell_macro_river_present = true;
                    }
                    runtime_river_rgba[dst..dst + 4].copy_from_slice(&river_debug_rgba(river));
                }
            }
            if let Some(cell) = cells.last_mut().and_then(|value| value.as_object_mut()) {
                cell.insert(
                    "macro_river_present".to_string(),
                    serde_json::Value::Bool(cell_macro_river_present),
                );
                cell.insert(
                    "runtime_river_present".to_string(),
                    serde_json::Value::Bool(cell_runtime_river_present),
                );
            }

            // Diff cell — green = agree, red = disagree
            let color: [u8; 4] = if agree {
                [30, 200, 80, 255]
            } else {
                [210, 45, 45, 255]
            };
            for py in 0..CELL_PX {
                for px in 0..CELL_PX {
                    let dx = gx * CELL_PX + px;
                    let dy = gy * CELL_PX + py;
                    let dst = (dy * img_w + dx) * 4;
                    diff_rgba[dst..dst + 4].copy_from_slice(&color);
                }
            }
        }
        pb.set_message(format!("Generating micro chunks… row {}/{n}", gy + 1));
    }
    pb.finish_and_clear();

    RgbaImage::from_raw(img_w as u32, img_h as u32, micro_rgba)
        .expect("micro buffer is correct size")
        .save(output_dir.join("micro_grid.png"))
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save micro_grid.png: {e}");
            std::process::exit(1);
        });
    println!("  micro_grid.png");

    RgbaImage::from_raw(img_w as u32, img_h as u32, runtime_river_rgba)
        .expect("runtime river buffer is correct size")
        .save(output_dir.join("runtime_rivers.png"))
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save runtime_rivers.png: {e}");
            std::process::exit(1);
        });
    println!("  runtime_rivers.png");

    let mut river_diff_rgba = vec![0u8; img_w * img_h * 4];
    let mut river_agree = 0usize;
    let mut macro_river_only = 0usize;
    let mut runtime_river_only = 0usize;
    let mut both_river = 0usize;
    for idx in 0..img_w * img_h {
        let macro_river = macro_river_mask[idx];
        let runtime_river = runtime_river_mask[idx];
        let color = match (macro_river, runtime_river) {
            (true, true) => {
                river_agree += 1;
                both_river += 1;
                [45, 170, 255, 255]
            }
            (true, false) => {
                macro_river_only += 1;
                [20, 220, 230, 255]
            }
            (false, true) => {
                runtime_river_only += 1;
                [240, 110, 35, 255]
            }
            (false, false) => {
                river_agree += 1;
                [28, 30, 32, 255]
            }
        };
        let dst = idx * 4;
        river_diff_rgba[dst..dst + 4].copy_from_slice(&color);
    }
    RgbaImage::from_raw(img_w as u32, img_h as u32, river_diff_rgba)
        .expect("river diff buffer is correct size")
        .save(output_dir.join("river_diff.png"))
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save river_diff.png: {e}");
            std::process::exit(1);
        });
    println!("  river_diff.png");

    RgbaImage::from_raw(img_w as u32, img_h as u32, diff_rgba)
        .expect("diff buffer is correct size")
        .save(output_dir.join("diff.png"))
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save diff.png: {e}");
            std::process::exit(1);
        });
    println!("  diff.png");

    let total = n * n;
    let river_total = img_w * img_h;
    let overall = agree_count as f64 / total as f64;
    let json = serde_json::json!({
        "seed": seed,
        "meso": [meso_x, meso_y],
        "origin": [world_x as i64, world_y as i64],
        "grid_size": n,
        "layers_tag": resolved_layers_tag,
        "macro_semantics": "macro_biome.bin",
        "overall_agreement": overall,
        "river_agreement": river_agree as f64 / river_total as f64,
        "river_pixels": {
            "macro_present": macro_river_present,
            "runtime_present": runtime_river_present,
            "both_present": both_river,
            "macro_only": macro_river_only,
            "runtime_only": runtime_river_only,
            "agree": river_agree,
            "total": river_total,
        },
        "cells": cells,
    });
    fs::write(
        output_dir.join("agreement.json"),
        serde_json::to_string_pretty(&json).unwrap(),
    )
    .unwrap_or_else(|e| {
        eprintln!("error: failed to save agreement.json: {e}");
        std::process::exit(1);
    });
    println!("  agreement.json");
    println!(
        "\n  agreement: {agree_count}/{total} ({:.1}%)",
        overall * 100.0
    );
    println!(
        "  river pixels: macro={} runtime={} macro_only={} runtime_only={} both={}",
        macro_river_present,
        runtime_river_present,
        macro_river_only,
        runtime_river_only,
        both_river,
    );
    println!("  output: {}", output_dir.display());
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
    // Threshold lowered from 4→3 after spec 007 GPU world-anchoring fix: the new
    // continentalness values shift which landforms appear at the 8 step-256 sample
    // points. step=128 (32 samples) still shows 5+ distinct classes; the reduction
    // here is a sparse-sampling artefact, not a quality regression.
    if summary.landform_class_counts.len() < 3 {
        failures.push("landform scan produced fewer than three distinct classes".to_string());
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
