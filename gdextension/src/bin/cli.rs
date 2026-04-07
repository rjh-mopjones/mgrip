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

use chrono::Utc;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use mg_artifacts::{ArtifactStore, LayerManifest, LevelManifest};
use mg_noise::{BiomeMap, NoiseLayer, RiverNetwork, rasterize_to_tile, LOD_THRESHOLD_MACRO};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

// ─── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "margins_grip", version, about = "Margin's Grip terrain generator")]
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

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Generate { kind } => match kind {
            GenerateKind::Layers { seed, tag } => run_generate_layers(seed, &tag),
            GenerateKind::Level { layers_tag, x, y, level_tag } => {
                run_generate_level(&layers_tag, x, y, &level_tag)
            }
        },
    }
}

// ─── generate layers ─────────────────────────────────────────────────────────

// World layout constants (matching biome_map.rs and spec)
const WORLD_WIDTH:  f64 = 1024.0;
const WORLD_HEIGHT: f64 = 512.0;
// Tile grid: 16×8 macro tiles of 64×64 world units each, rendered at 256px.
// Gives 4096×2048 final image at 0.25 world-units/pixel.
// 128 total dispatches vs 8192 for 32px tiles — critical for GPU efficiency.
const TILE_WORLD_SIZE: f64 = 64.0;
const TILE_PX: usize = 256;
const TILES_X: usize = (WORLD_WIDTH  / TILE_WORLD_SIZE) as usize;  // 16
const TILES_Y: usize = (WORLD_HEIGHT / TILE_WORLD_SIZE) as usize;  // 8
const FULL_W:  usize = TILES_X * TILE_PX;  // 4096
const FULL_H:  usize = TILES_Y * TILE_PX;  // 2048

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
        0.0, 0.0,
        WORLD_WIDTH, WORLD_HEIGHT,
        512, 256,
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
                wx, wy,
                TILE_WORLD_SIZE, TILE_WORLD_SIZE,
                TILE_PX, TILE_PX,
                1,
                false,
                false,
                1.0,
            );

            // Project the macro river network onto this tile's rivers layer.
            // Each macro pixel (2 world units wide) expands to an 8×8 meso pixel footprint.
            tile.rivers = rasterize_to_tile(
                &river_network,
                TILE_PX, TILE_PX,
                wx, wy,
                TILE_WORLD_SIZE, TILE_WORLD_SIZE,
                WORLD_WIDTH, WORLD_HEIGHT,
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
        images.insert(format!("{}.png", layer.name()), (FULL_W as u32, FULL_H as u32, buf));
    }

    let layer_images: Vec<String> = NoiseLayer::all()
        .iter()
        .map(|l| format!("{}.png", l.name()))
        .collect();

    let manifest = LayerManifest {
        seed,
        created: Utc::now().to_rfc3339(),
        world_width:  WORLD_WIDTH  as u32,
        world_height: WORLD_HEIGHT as u32,
        tile_width:   FULL_W as u32,
        tile_height:  FULL_H as u32,
        layer_images,
    };

    store
        .save_layers(tag, &macro_map, &river_network, &images, &manifest)
        .unwrap_or_else(|e| {
            eprintln!("error: failed to save layers: {e}");
            std::process::exit(1);
        });
    pb.finish_and_clear();

    println!(
        "Saved to {}/layers/{tag}/",
        store.base_path().display()
    );
    println!("  {FULL_W}×{FULL_H} PNGs, {} layers written", NoiseLayer::all().len());
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

    let map = BiomeMap::generate(
        seed,
        world_x, world_y,
        1.0, 1.0,
        512, 512,
        2,      // detail_level=2 — matches game config
        false,
        false,
        8.0,    // freq_scale — matches game config
    );

    pb.finish_and_clear();
    println!("  pipeline done in {:.1}s", t0.elapsed().as_secs_f64());

    // ── Diagnose heightmap ────────────────────────────────────────────────────
    {
        let h = &map.heightmap;
        let n = h.len() as f64;
        let min_h = h.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_h = h.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mean  = h.iter().sum::<f64>() / n;
        let land  = h.iter().filter(|&&v| v > -0.01).count();
        let std_dev = (h.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();

        // Biome distribution
        use mg_noise::NoiseLayer;
        let biome_rgba = map.layer_to_rgba(NoiseLayer::Biome);
        let mut biome_counts: std::collections::HashMap<[u8;3], (usize, String)> = std::collections::HashMap::new();
        for i in 0..h.len() {
            let r = biome_rgba[i*4]; let g = biome_rgba[i*4+1]; let b = biome_rgba[i*4+2];
            biome_counts.entry([r,g,b]).or_insert((0, format!("#{r:02x}{g:02x}{b:02x}"))).0 += 1;
        }
        let mut biome_vec: Vec<_> = biome_counts.values().collect();
        biome_vec.sort_by(|a, b| b.0.cmp(&a.0));

        println!("\n── Micro heightmap (detail_level=2, freq=8, world=({world_x},{world_y})) ──");
        println!("  range   : [{min_h:.4}, {max_h:.4}]   mean: {mean:.4}   σ: {std_dev:.4}");
        println!("  land    : {:.1}%  ocean: {:.1}%", 100.0*land as f64/n, 100.0*(h.len()-land) as f64/n);

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
        let light  = &map.light_level;
        let temp   = &map.temperature;
        let l_mean = light.iter().sum::<f64>() / n;
        let t_mean = temp.iter().sum::<f64>() / n;
        let above45 = temp.iter().filter(|&&t| t > 45.0).count();
        let l_min = light.iter().cloned().fold(f64::INFINITY, f64::min);
        let l_max = light.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let t_min = temp.iter().cloned().fold(f64::INFINITY, f64::min);
        let t_max = temp.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        println!("  light_level : [{l_min:.3}, {l_max:.3}]   mean: {l_mean:.3}");
        println!("  temperature : [{t_min:.1}°C, {t_max:.1}°C]   mean: {t_mean:.1}°C");
        println!("  above 45°C  : {:.1}%  (forced Arid)", 100.0 * above45 as f64 / n);
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
    let png_dir   = level_dir.join("images");
    std::fs::create_dir_all(&png_dir).ok();
    map.save_all_debug_pngs(&png_dir).unwrap_or_else(|e| eprintln!("warn: PNG save failed: {e}"));

    println!(
        "Saved to {}/levels/{level_tag}/",
        store.base_path().display()
    );
    println!("  PNGs: {}", png_dir.display());
    println!("  total: {:.1}s", t0.elapsed().as_secs_f64());
}

// ─── helpers ─────────────────────────────────────────────────────────────────

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
