//! BiomeMap: complete terrain snapshot for a given LOD tile.
//! Holds all base and derived noise layers plus the computed biome grid.

use crate::gpu::GpuNoiseContext;
use mg_core::{NoiseStrategy, TileType};
use noise::OpenSimplex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::biome_splines::BiomeSplines;
use crate::derived;
use crate::erosion_sim::{simulate_erosion, ErosionParams};
use crate::rivers::{rasterize_to_tile, RiverNetwork, LOD_THRESHOLD_MACRO};
use crate::strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy, PeaksAndValleysStrategy,
    RockHardnessStrategy, TectonicPlatesStrategy,
};
use crate::visualization::NoiseLayer;

pub const SEA_LEVEL: f64 = -0.01;

pub fn tile_has_fluid_surface(tile: TileType) -> bool {
    matches!(
        tile,
        TileType::Sea
            | TileType::ShallowSea
            | TileType::ContinentalShelf
            | TileType::DeepOcean
            | TileType::OceanTrench
            | TileType::OceanRidge
    )
}

/// Ocean mask derived from the macro biome artifact — authoritative ocean/land
/// from the macro pipeline.
///
/// The macro pipeline runs 120-iteration erosion and classifies biomes at world scale.
/// Where its classification differs from the runtime micro pipeline (e.g. dayside cells
/// that the splines demote to SaltFlat despite negative continentalness), this mask
/// overrides the runtime result.
pub struct MacroOceanMask {
    pixels: Vec<bool>,
    width: usize,
    height: usize,
    world_width: f64,
    world_height: f64,
}

impl MacroOceanMask {
    /// Build from saved macro biome semantics (`macro_biome.bin`).
    pub fn from_biome_map(map: &BiomeMap) -> Self {
        let pixels = map
            .biomes
            .iter()
            .copied()
            .map(tile_has_fluid_surface)
            .collect();
        Self {
            pixels,
            width: map.width,
            height: map.height,
            world_width: map.world_width,
            world_height: map.world_height,
        }
    }

    /// Returns true if the world position maps to an ocean cell in the macro mask.
    pub fn is_ocean_at_world(&self, wx: f64, wy: f64) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        let px = ((wx / self.world_width * self.width as f64) as usize).min(self.width - 1);
        let py = ((wy / self.world_height * self.height as f64) as usize).min(self.height - 1);
        self.pixels.get(py * self.width + px).copied().unwrap_or(false)
    }
}

/// Bilinear sample a per-pixel field slice at world coordinates.
///
/// The x-axis wraps (cylindrical world); the y-axis is clamped. Used to smoothly
/// interpolate macro artifact fields when projecting them into finer runtime tiles —
/// nearest-neighbor sampling would produce hard seams at runtime chunk boundaries.
pub fn sample_field_bilinear(
    field: &[f64],
    wx: f64,
    wy: f64,
    world_width: f64,
    world_height: f64,
    width: usize,
    height: usize,
) -> f64 {
    if field.is_empty() || width == 0 || height == 0 {
        return 0.0;
    }
    debug_assert_eq!(field.len(), width * height);

    let wrapped_x = crate::wrap::wrap_x(wx, world_width);
    let fx = wrapped_x * width as f64 / world_width;
    let clamped_y = wy.clamp(0.0, world_height);
    let fy = (clamped_y * height as f64 / world_height).min((height - 1) as f64);

    let x0f = fx.floor();
    let y0f = fy.floor();
    let tx = fx - x0f;
    let ty = fy - y0f;

    let x0 = crate::wrap::wrap_grid_x(x0f as i32, width) as usize;
    let x1 = crate::wrap::wrap_grid_x(x0f as i32 + 1, width) as usize;
    let y0_i = (y0f as i32).max(0);
    let y1_i = (y0_i + 1).min(height as i32 - 1);
    let y0 = y0_i as usize;
    let y1 = y1_i as usize;

    let v00 = field[y0 * width + x0];
    let v10 = field[y0 * width + x1];
    let v01 = field[y1 * width + x0];
    let v11 = field[y1 * width + x1];

    let top = v00 * (1.0 - tx) + v10 * tx;
    let bot = v01 * (1.0 - tx) + v11 * tx;
    top * (1.0 - ty) + bot * ty
}

/// Seed offsets per base layer (additive from world_seed).
const SEED_CONTINENTALNESS: u32 = 0;
const SEED_TECTONIC: u32 = 1;
const SEED_HUMIDITY: u32 = 2;
const SEED_ROCK_HARDNESS: u32 = 3;
const SEED_LIGHT_LEVEL: u32 = 4;
const SEED_PEAKS_VALLEYS: u32 = 7;
const SEED_MICRO_DETAIL: u32 = 50;

#[derive(Serialize, Deserialize)]
pub struct BiomeMap {
    pub width: usize,
    pub height: usize,

    // Base layers
    pub continentalness: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub tectonic_plate_ids: Vec<f64>,
    pub humidity: Vec<f64>,
    pub rock_hardness: Vec<f64>,
    pub light_level: Vec<f64>,

    // Derived layers
    pub peaks_valleys: Vec<f64>,
    pub volcanism: Vec<f64>,
    pub heightmap: Vec<f64>,
    pub temperature: Vec<f64>,
    pub erosion: Vec<f64>,
    pub rivers: Vec<f64>,
    pub aridity: Vec<f64>,
    pub precipitation_type: Vec<f64>,
    pub water_table: Vec<f64>,
    pub wind_speed: Vec<f64>,
    pub resource_richness: Vec<f64>,
    pub snowpack: Vec<f64>,

    pub biomes: Vec<TileType>,
    pub vegetation_density: Vec<f64>,
    pub soil_type: Vec<f64>,

    pub drainage_area: Vec<u32>,
    pub sediment: Vec<f64>,

    #[serde(skip)]
    pub river_network: Option<Arc<RiverNetwork>>,

    pub world_width: f64,
    pub world_height: f64,
}

impl BiomeMap {
    fn empty(width: usize, height: usize, world_width: f64, world_height: f64) -> Self {
        let n = width * height;
        Self {
            width,
            height,
            continentalness: vec![0.0; n],
            tectonic: vec![0.0; n],
            tectonic_plate_ids: vec![0.0; n],
            humidity: vec![0.0; n],
            rock_hardness: vec![0.0; n],
            light_level: vec![0.0; n],
            peaks_valleys: vec![0.0; n],
            volcanism: vec![0.0; n],
            heightmap: vec![0.0; n],
            temperature: vec![0.0; n],
            erosion: vec![0.0; n],
            rivers: vec![0.0; n],
            aridity: vec![0.0; n],
            precipitation_type: vec![0.0; n],
            water_table: vec![0.0; n],
            wind_speed: vec![0.0; n],
            resource_richness: vec![0.0; n],
            snowpack: vec![0.0; n],
            biomes: vec![TileType::Sea; n],
            vegetation_density: vec![0.0; n],
            soil_type: vec![0.0; n],
            drainage_area: vec![0; n],
            sediment: vec![0.0; n],
            river_network: None,
            world_width,
            world_height,
        }
    }

    /// Generate a complete BiomeMap for a region.
    ///
    /// - `seed`: world seed
    /// - `origin_x/y`: world-space top-left corner of this tile (true world coords)
    /// - `world_size_x/y`: world-space extent of this tile
    /// - `tile_w/h`: pixel resolution
    /// - `detail_level`: 0=Macro, 1=Meso (unused for micro — freq_scale handles detail)
    /// - `run_erosion`: run 120-iteration erosion sim (macro only)
    /// - `run_rivers`: compute global river network (macro only)
    /// - `freq_scale`: multiply noise coordinates by this factor before sampling fBm layers.
    ///   Use 1.0 for macro/meso. For a playable micro level (1×1 world unit, 512×512 blocks)
    ///   use ~100.0 so the noise has full continent-scale variation within the tile.
    ///   Light level always uses true world coords regardless of this value.
    pub fn generate(
        seed: u32,
        origin_x: f64,
        origin_y: f64,
        world_size_x: f64,
        world_size_y: f64,
        tile_w: usize,
        tile_h: usize,
        detail_level: u32,
        run_erosion: bool,
        run_rivers: bool,
        freq_scale: f64,
    ) -> Self {
        let world_width = 1024.0;
        let world_height = 512.0;
        let mut map = Self::empty(tile_w, tile_h, world_width, world_height);

        // Tier 1 (identity) layers — continentalness and tectonic — always use raw world
        // coordinates and always wrap. They define the identity of a place and must be
        // stable regardless of freq_scale or LOD.
        // Tier 2 (detail) layers use scaled coordinates and only wrap at macro scale.
        let detail_wrap = freq_scale == 1.0;
        let cont_strat = ContinentalnessStrategy::new_wrapping(
            seed.wrapping_add(SEED_CONTINENTALNESS),
            world_width,
        );
        let tect_strat =
            TectonicPlatesStrategy::new_wrapping(seed.wrapping_add(SEED_TECTONIC), world_width);
        let humid_strat = if detail_wrap {
            HumidityStrategy::new_wrapping(seed.wrapping_add(SEED_HUMIDITY), world_width)
        } else {
            HumidityStrategy::new(seed.wrapping_add(SEED_HUMIDITY))
        };
        let rock_strat = if detail_wrap {
            RockHardnessStrategy::new_wrapping(seed.wrapping_add(SEED_ROCK_HARDNESS), world_width)
        } else {
            RockHardnessStrategy::new(seed.wrapping_add(SEED_ROCK_HARDNESS))
        };
        let light_strat = LightLevelStrategy::new(
            seed.wrapping_add(SEED_LIGHT_LEVEL),
            0.5,
            1.0,
            world_width,
            world_height,
        );
        let pv_strat = if detail_wrap {
            PeaksAndValleysStrategy::new_wrapping(
                seed.wrapping_add(SEED_PEAKS_VALLEYS),
                world_width,
            )
        } else {
            PeaksAndValleysStrategy::new(seed.wrapping_add(SEED_PEAKS_VALLEYS))
        };
        let detail_noise = OpenSimplex::new(seed.wrapping_add(SEED_MICRO_DETAIL));

        // Pixel → world coordinate mapping
        let px_to_wx = |px: usize| sample_world_coord(origin_x, world_size_x, tile_w, px);
        let py_to_wy = |py: usize| sample_world_coord(origin_y, world_size_y, tile_h, py);

        // ── Phase 1: Generate all base layers ─────────────────────────────────
        // pixels stores (idx, true_wx, true_wy) — true world coords.
        // fBm strategies receive (true_wx * freq_scale, true_wy * freq_scale).
        // LightLevelStrategy always gets true coords (it normalises by map_width).
        let pixels: Vec<(usize, f64, f64)> = (0..tile_h)
            .flat_map(|py| {
                (0..tile_w).map(move |px| {
                    let wx = sample_world_coord(origin_x, world_size_x, tile_w, px);
                    let wy = sample_world_coord(origin_y, world_size_y, tile_h, py);
                    (py * tile_w + px, wx, wy)
                })
            })
            .collect();

        // Tectonic (Voronoi plates) is always CPU — no GPU equivalent.
        // Uses raw world coordinates (Tier 1 — world-anchored).
        let tect_data: Vec<(f64, f64)> = pixels
            .par_iter()
            .map(|&(_, wx, wy)| {
                let s = tect_strat.generate_full(wx, wy);
                (s.boundary_distance, s.plate_id)
            })
            .collect();

        // Two-way dispatch:
        //   GPU (available) — all layers from GPU at true world coords. Biome classification
        //     (ocean/land boundary, zone, palette) is therefore stable macro truth for
        //     any freq_scale. Micro-scale terrain variety comes from derive_micro_heightmap
        //     at detail_level >= 2, not from scaled noise layers.
        //   CPU fallback (no GPU) — all layers from CPU at true world coords (wx, wy).
        //     freq_scale is intentionally ignored here; the scaled coords were causing
        //     coast_perturb to flip shallow-ocean cells to land.
        let scale = sample_world_step(world_size_x, tile_w);
        let gpu_layers = GpuNoiseContext::global().map(|gpu| {
            gpu.generate_layers(
                seed,
                tile_w,
                tile_h,
                origin_x,
                origin_y,
                scale,
                world_height,
                detail_level,
            )
            .into_f64()
        });

        match gpu_layers {
            Some(gpu) => {
                // GPU path — all layers at world scale regardless of freq_scale.
                for (i, &(tect, plate_id)) in tect_data.iter().enumerate() {
                    map.continentalness[i] = gpu.continentalness[i];
                    map.tectonic[i] = tect;
                    map.light_level[i] = gpu.light_level[i];
                    map.rock_hardness[i] = gpu.rock_hardness[i];
                    map.humidity[i] = gpu.humidity[i];
                    map.peaks_valleys[i] = derived::derive_peaks_valleys(
                        gpu.peaks_valleys[i],
                        tect,
                        gpu.rock_hardness[i],
                    );
                    map.tectonic_plate_ids[i] = plate_id;
                }
            }
            None => {
                // CPU fallback — all layers at true world coords (wx, wy).
                let base_data: Vec<(f64, f64, f64, f64, f64)> = pixels
                    .par_iter()
                    .map(|&(_, wx, wy)| {
                        let cont = cont_strat.generate(wx, wy, detail_level);
                        let light = light_strat.generate(wx, wy, detail_level);
                        let rock = rock_strat.generate(wx, wy, detail_level);
                        let humid = humid_strat.generate_terminator_model(
                            wx,
                            wy,
                            detail_level,
                            cont,
                            light,
                        );
                        let pv_base = pv_strat.generate(wx, wy, detail_level);
                        (cont, light, rock, humid, pv_base)
                    })
                    .collect();
                for (i, (&(cont, light, rock, humid, pv_base), &(tect, plate_id))) in
                    base_data.iter().zip(tect_data.iter()).enumerate()
                {
                    map.continentalness[i] = cont;
                    map.tectonic[i] = tect;
                    map.light_level[i] = light;
                    map.rock_hardness[i] = rock;
                    map.humidity[i] = humid;
                    map.peaks_valleys[i] = derived::derive_peaks_valleys(pv_base, tect, rock);
                    map.tectonic_plate_ids[i] = plate_id;
                }
            }
        }

        // ── Phase 2: Derived layers (depend on base layers) ───────────────────
        // Temperature (needs light, heightmap placeholder, humidity, continentalness)
        // We need heightmap first, so compute it from current peaks_valleys
        for i in 0..tile_w * tile_h {
            let h = derived::derive_heightmap(
                map.continentalness[i],
                map.tectonic[i],
                map.peaks_valleys[i],
            );
            map.heightmap[i] = h;
        }

        // Temperature
        for i in 0..tile_w * tile_h {
            map.temperature[i] = derived::derive_temperature(
                map.light_level[i],
                map.heightmap[i],
                map.humidity[i],
                map.continentalness[i],
            );
        }

        // ── Phase 3: Erosion simulation (macro only) ──────────────────────────
        if run_erosion {
            let tectonic_stress: Vec<f64> = map.tectonic.iter().map(|&t| 1.0 - t).collect();
            let erosion_result = simulate_erosion(
                &map.heightmap,
                &map.rock_hardness,
                &tectonic_stress,
                &map.continentalness,
                tile_w,
                tile_h,
                &ErosionParams::default(),
            );
            map.heightmap = erosion_result.heightmap;
            map.drainage_area = erosion_result.drainage_area;
            map.sediment = erosion_result.sediment;

            // Recompute temperature with eroded heightmap
            for i in 0..tile_w * tile_h {
                map.temperature[i] = derived::derive_temperature(
                    map.light_level[i],
                    map.heightmap[i],
                    map.humidity[i],
                    map.continentalness[i],
                );
            }
        }

        // ── Phase 4: River network (macro only) ───────────────────────────────
        if run_rivers {
            let network = RiverNetwork::generate(
                &map.heightmap,
                &map.rock_hardness,
                &map.tectonic,
                &map.continentalness,
                &map.light_level,
                &map.humidity,
                &map.temperature,
                tile_w,
                tile_h,
                SEA_LEVEL,
            );
            map.rivers = network.to_flow_grid(tile_w, tile_h);
            map.river_network = Some(Arc::new(network));
        }

        // ── Phase 5: Remaining derived layers ─────────────────────────────────
        for i in 0..tile_w * tile_h {
            let h = map.heightmap[i];
            let cont = map.continentalness[i];
            let temp = map.temperature[i];
            let humid = map.humidity[i];
            let rock = map.rock_hardness[i];
            let tect = map.tectonic[i];
            let river = map.rivers[i];
            let light = map.light_level[i];

            map.erosion[i] = derived::derive_erosion(h, rock, humid);
            map.aridity[i] = derived::derive_aridity(temp, humid);
            map.precipitation_type[i] = derived::derive_precipitation_type(temp, humid, h);
            map.snowpack[i] = derived::derive_snowpack(map.precipitation_type[i], temp, h, light);
            map.water_table[i] =
                derived::derive_water_table(river, humid, h, map.precipitation_type[i], cont);
            map.resource_richness[i] =
                derived::derive_resource_richness(tect, rock, map.erosion[i]);
        }

        // Apply micro detail if detail_level == 2
        if detail_level >= 2 {
            for i in 0..tile_w * tile_h {
                let px = i % tile_w;
                let py = i / tile_w;
                let wx = px_to_wx(px);
                let wy = py_to_wy(py);
                map.heightmap[i] =
                    derived::derive_micro_heightmap(map.heightmap[i], wx, wy, &detail_noise);
            }
        }

        // ── Phase 6: Biome classification ─────────────────────────────────────
        let splines = BiomeSplines::new(SEA_LEVEL);

        for i in 0..tile_w * tile_h {
            let biome = splines.evaluate_with_light(
                map.continentalness[i],
                map.temperature[i],
                map.tectonic[i],
                map.erosion[i],
                map.peaks_valleys[i],
                map.humidity[i],
                map.aridity[i],
                map.rock_hardness[i],
                map.light_level[i],
            );
            map.biomes[i] = biome;
            map.vegetation_density[i] =
                derived::derive_vegetation_density(biome, map.water_table[i]);
            map.soil_type[i] =
                derived::derive_soil_type(biome, map.erosion[i], map.rock_hardness[i]);
        }

        // Apply polar ice cap override
        apply_polar_ice_cap(
            &mut map.biomes,
            &map.light_level,
            &map.continentalness,
            &map.peaks_valleys,
            &map.rock_hardness,
            &map.temperature,
            SEA_LEVEL,
        );

        map
    }

    /// Override biome classification for cells where the macro biome artifact says ocean.
    ///
    /// The macro world map is authoritative for ocean placement. Where the saved macro
    /// biome semantics show ocean but the noise pipeline classified as land (e.g. dayside cells demoted to
    /// SaltFlat by the temperature gate in `below_sea_biome`), this restores the correct
    /// ocean biome. Call after `generate()`.
    pub fn apply_macro_ocean_mask(
        &mut self,
        mask: &MacroOceanMask,
        origin_x: f64,
        origin_y: f64,
        world_size_x: f64,
        world_size_y: f64,
    ) {
        let tile_w = self.width;
        let tile_h = self.height;
        // Sample macro truth at the centre of each 1-world-unit cell, not at every
        // runtime pixel. This keeps coastline overrides chunk-stable and matches the
        // compare tool, which also samples the centre of each 1×1 chunk cell.
        for i in 0..tile_w * tile_h {
            let px = i % tile_w;
            let py = i / tile_w;
            let wx = sample_world_coord(origin_x, world_size_x, tile_w, px);
            let wy = sample_world_coord(origin_y, world_size_y, tile_h, py);
            let wx_center = wx.floor() + 0.5;
            let wy_center = wy.floor() + 0.5;
            if mask.is_ocean_at_world(wx_center, wy_center) && !tile_has_fluid_surface(self.biomes[i]) {
                let depth = SEA_LEVEL - self.continentalness[i];
                self.biomes[i] = if depth > 0.25 {
                    TileType::DeepOcean
                } else if depth > 0.10 {
                    TileType::Sea
                } else {
                    TileType::ShallowSea
                };
                self.vegetation_density[i] = 0.0;
            }
        }
    }

    /// Project the saved macro river network into this runtime tile.
    ///
    /// Runtime chunks are generated without the full global river solve. This keeps chunk
    /// generation cheap, then reuses the saved macro network so local maps and level chunks
    /// can still expose the same drainage paths as `macromap.png`.
    pub fn apply_macro_river_network(
        &mut self,
        network: &RiverNetwork,
        origin_x: f64,
        origin_y: f64,
        world_size_x: f64,
        world_size_y: f64,
        threshold: u32,
    ) {
        let tile_w = self.width;
        let tile_h = self.height;
        self.rivers = rasterize_to_tile(
            network,
            tile_w,
            tile_h,
            origin_x,
            origin_y,
            world_size_x,
            world_size_y,
            self.world_width,
            self.world_height,
            threshold as f64,
        );

        for i in 0..tile_w * tile_h {
            if self.rivers[i] <= 0.0 {
                continue;
            }
            self.water_table[i] = derived::derive_water_table(
                self.rivers[i],
                self.humidity[i],
                self.heightmap[i],
                self.precipitation_type[i],
                self.continentalness[i],
            );
            if self.rivers[i] > 0.1
                && !tile_has_fluid_surface(self.biomes[i])
                && self.aridity[i] < 0.7
            {
                self.biomes[i] = TileType::River;
            }
            self.vegetation_density[i] =
                derived::derive_vegetation_density(self.biomes[i], self.water_table[i]);
            self.soil_type[i] =
                derived::derive_soil_type(self.biomes[i], self.erosion[i], self.rock_hardness[i]);
        }
    }

    /// Anchor this tile's derived layers and biomes to a macro `BiomeMap` plus the global
    /// `RiverNetwork`.
    ///
    /// Heightmap, temperature, erosion, aridity, precipitation, snowpack, resource_richness,
    /// biomes, rivers, water_table, vegetation_density and soil_type are all rewritten so the
    /// tile trends toward the values the macro artifact already produced. Base identity layers
    /// (continentalness, tectonic, rock_hardness, humidity, light_level, peaks_valleys) are
    /// left untouched — they define the identity of a place and must come from this tile's
    /// own coordinate space.
    ///
    /// This is the single source of truth for "look like the macromap" semantics. Both the CLI
    /// meso tile render and runtime micro chunks route through it so they can never drift.
    ///
    /// Parameters:
    /// - `mountain_detail_gain`: weight applied to local `peaks_valleys` as relief detail on
    ///   top of the macro heightmap. `0.2` matches the existing meso pipeline.
    /// - `apply_micro_detail`: when `true`, fold sub-pixel `derive_micro_heightmap` noise onto
    ///   the anchored heightmap. Use for runtime micro chunks; leave `false` for meso tiles.
    pub fn anchor_to_macro(
        &mut self,
        macro_map: &BiomeMap,
        river_network: &RiverNetwork,
        seed: u32,
        origin_x: f64,
        origin_y: f64,
        world_size_x: f64,
        world_size_y: f64,
        river_threshold: u32,
        mountain_detail_gain: f64,
        apply_micro_detail: bool,
    ) {
        let tile_w = self.width;
        let tile_h = self.height;
        if tile_w == 0 || tile_h == 0 || macro_map.heightmap.is_empty() {
            return;
        }

        let detail_noise = OpenSimplex::new(seed.wrapping_add(SEED_MICRO_DETAIL));
        let splines = BiomeSplines::new(SEA_LEVEL);

        for py in 0..tile_h {
            for px in 0..tile_w {
                let idx = py * tile_w + px;
                let wx = origin_x + (px as f64 + 0.5) * world_size_x / tile_w as f64;
                let wy = origin_y + (py as f64 + 0.5) * world_size_y / tile_h as f64;

                // Anchor every base layer that feeds the biome spline from the macro
                // artifact. Macro and runtime sample noise at different `freq_scale`
                // values (1.0 vs 8.0), so the same world coord lands on different
                // continentalness / humidity / rock / tectonic / peaks values in each
                // pass. The spline is sensitive to all of these for both ocean/land
                // and land-biome classification — runtime drift is dominated by the
                // freq_scale mismatch, not by genuine local detail. Anchoring the
                // full base set forces spline inputs to match macro inputs at every
                // pixel.
                //
                // Sub-pixel intra-chunk variation still comes from two sources:
                //   - the dithered spline's coordinate-hash perturbation in
                //     `evaluate_dithered_with_light`, which jitters cont/temp/humid
                //     locally
                //   - `derive_micro_heightmap`, which adds high-frequency noise on
                //     top of the macro-anchored heightmap
                self.continentalness[idx] =
                    macro_map.sample_field_at(&macro_map.continentalness, wx, wy);
                self.tectonic[idx] =
                    macro_map.sample_field_at(&macro_map.tectonic, wx, wy);
                self.humidity[idx] =
                    macro_map.sample_field_at(&macro_map.humidity, wx, wy);
                self.rock_hardness[idx] =
                    macro_map.sample_field_at(&macro_map.rock_hardness, wx, wy);
                self.peaks_valleys[idx] =
                    macro_map.sample_field_at(&macro_map.peaks_valleys, wx, wy);

                // Sample the macro heightmap. This is the value the macro pass
                // used for ALL its derivations and biome classification. Runtime
                // must use the same value as the spline input to match macro.
                let macro_hm = macro_map.sample_field_at(&macro_map.heightmap, wx, wy);

                // Pull anchored base identity layers.
                let cont = self.continentalness[idx];
                let humid = self.humidity[idx];
                let rock = self.rock_hardness[idx];
                let tect = self.tectonic[idx];
                let light = self.light_level[idx];
                let peaks = self.peaks_valleys[idx];

                // Derive climate from MACRO heightmap (matches macro Phase 5
                // derivations exactly, since macro derives from its own hm).
                let temp = derived::derive_temperature(light, macro_hm, humid, cont);
                let eros = derived::derive_erosion(macro_hm, rock, humid);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, macro_hm);
                let snow = derived::derive_snowpack(precip, temp, macro_hm, light);

                self.temperature[idx] = temp;
                self.erosion[idx] = eros;
                self.aridity[idx] = arid;
                self.precipitation_type[idx] = precip;
                self.snowpack[idx] = snow;
                self.resource_richness[idx] =
                    derived::derive_resource_richness(tect, rock, eros);

                // Classify biome with the SAME spline call the macro pass uses
                // (`biome_map.rs:483`, non-dithered). With every spline input
                // matching macro, the biome enum must match macro.
                self.biomes[idx] = splines.evaluate_with_light(
                    cont, temp, tect, eros, peaks, humid, arid, rock, light,
                );

                // Write the rendered heightmap with mesh detail on top of the
                // macro-anchored value. This drives mesh generation and visual
                // hillshade — biome classification already happened above using
                // raw macro_hm so the detail doesn't perturb biome boundaries.
                let stress = 1.0 - tect;
                let above_sea = (macro_hm - SEA_LEVEL).max(0.0);
                let mountain_intensity = (stress * above_sea * 3.0).min(1.0);
                let mountain_detail = peaks * mountain_intensity * mountain_detail_gain;
                let mut hm = (macro_hm + mountain_detail).clamp(-1.0, 1.0);
                if apply_micro_detail {
                    hm = derived::derive_micro_heightmap(hm, wx, wy, &detail_noise);
                }
                self.heightmap[idx] = hm;
            }
        }

        // Project the global river network into this tile at the requested LOD threshold.
        self.rivers = rasterize_to_tile(
            river_network,
            tile_w,
            tile_h,
            origin_x,
            origin_y,
            world_size_x,
            world_size_y,
            self.world_width,
            self.world_height,
            river_threshold as f64,
        );

        // Secondary derives that depend on rivers.
        for i in 0..tile_w * tile_h {
            self.water_table[i] = derived::derive_water_table(
                self.rivers[i],
                self.humidity[i],
                self.heightmap[i],
                self.precipitation_type[i],
                self.continentalness[i],
            );
            if self.rivers[i] > 0.1
                && !tile_has_fluid_surface(self.biomes[i])
                && self.aridity[i] < 0.7
            {
                self.biomes[i] = TileType::River;
            }
            self.vegetation_density[i] =
                derived::derive_vegetation_density(self.biomes[i], self.water_table[i]);
            self.soil_type[i] =
                derived::derive_soil_type(self.biomes[i], self.erosion[i], self.rock_hardness[i]);
        }

        // Polar ice cap override — idempotent, matches the tail of `generate()`.
        apply_polar_ice_cap(
            &mut self.biomes,
            &self.light_level,
            &self.continentalness,
            &self.peaks_valleys,
            &self.rock_hardness,
            &self.temperature,
            SEA_LEVEL,
        );
    }

    /// Quick accessor — returns heightmap value at pixel (x, y).
    pub fn heightmap_at(&self, x: usize, y: usize) -> f64 {
        self.heightmap[y * self.width + x]
    }

    pub fn biome_at(&self, x: usize, y: usize) -> TileType {
        self.biomes[y * self.width + x]
    }

    /// Sample the heightmap at world coordinates using wrapped nearest-neighbor.
    /// This preserves sharp macro erosion features when meso tiles inherit macro height.
    pub fn sample_heightmap_at(&self, wx: f64, wy: f64) -> f64 {
        if self.heightmap.is_empty() {
            return 0.0;
        }
        let wrapped_x = crate::wrap::wrap_x(wx, self.world_width);
        let x = (wrapped_x.round() as usize).min(self.width - 1);
        let y = (wy.clamp(0.0, self.world_height - 1.0).round() as usize).min(self.height - 1);
        self.heightmap[y * self.width + x]
    }

    /// Bilinear sample any `f64` field slice sized `width * height` at world coord.
    ///
    /// Used when anchoring a finer tile to this map's fields — interpolates between
    /// macro pixels so downstream tiles don't show hard seams at macro pixel boundaries.
    pub fn sample_field_at(&self, field: &[f64], wx: f64, wy: f64) -> f64 {
        sample_field_bilinear(
            field,
            wx,
            wy,
            self.world_width,
            self.world_height,
            self.width,
            self.height,
        )
    }

    /// Nearest-neighbor discrete biome sample at world coordinates.
    ///
    /// Biomes are `TileType` enums — bilinear is not meaningful. Mirrors the
    /// convention used by `MacroOceanMask::is_ocean_at_world`.
    pub fn sample_biome_at_world(&self, wx: f64, wy: f64) -> TileType {
        if self.biomes.is_empty() {
            return TileType::Sea;
        }
        let wrapped_x = crate::wrap::wrap_x(wx, self.world_width);
        let px = ((wrapped_x / self.world_width * self.width as f64) as usize).min(self.width - 1);
        let py = ((wy.clamp(0.0, self.world_height) / self.world_height * self.height as f64)
            as usize)
            .min(self.height - 1);
        self.biomes[py * self.width + px]
    }

    pub fn temperature_at(&self, x: usize, y: usize) -> f64 {
        self.temperature[y * self.width + x]
    }

    pub fn humidity_at(&self, x: usize, y: usize) -> f64 {
        self.humidity[y * self.width + x]
    }

    pub fn light_level_at(&self, x: usize, y: usize) -> f64 {
        self.light_level[y * self.width + x]
    }

    pub fn river_at(&self, x: usize, y: usize) -> f64 {
        self.rivers[y * self.width + x]
    }

    pub fn is_ocean(&self, x: usize, y: usize) -> bool {
        tile_has_fluid_surface(self.biomes[y * self.width + x])
    }

    pub fn has_surface_fluid(&self, x: usize, y: usize) -> bool {
        tile_has_fluid_surface(self.biomes[y * self.width + x])
    }

    /// Export a debug PNG for the given layer. Returns RGBA bytes.
    pub fn layer_to_rgba(&self, layer: NoiseLayer) -> Vec<u8> {
        use crate::visualization::*;
        let n = self.width * self.height;
        let mut out = Vec::with_capacity(n * 4);

        for i in 0..n {
            let rgba = match layer {
                NoiseLayer::Biome => {
                    let [r, g, b, a] = self.biomes[i].color();
                    [r, g, b, a]
                }
                NoiseLayer::Heightmap => heightmap_to_rgba(self.heightmap[i]),
                NoiseLayer::Temperature => temperature_to_rgba(self.temperature[i]),
                NoiseLayer::Humidity => humidity_to_rgba(self.humidity[i]),
                NoiseLayer::Continentalness => {
                    let v = (self.continentalness[i] + 1.0) * 0.5;
                    grayscale_to_rgba(v)
                }
                NoiseLayer::Tectonic => tectonic_to_rgba(self.tectonic[i]),
                NoiseLayer::RockHardness => rock_hardness_to_rgba(self.rock_hardness[i]),
                NoiseLayer::LightLevel => light_level_to_rgba(self.light_level[i]),
                NoiseLayer::PeaksValleys => peaks_to_rgba(self.peaks_valleys[i]),
                NoiseLayer::Erosion => erosion_to_rgba(self.erosion[i]),
                NoiseLayer::Rivers => river_to_rgba(self.rivers[i]),
                NoiseLayer::Aridity => aridity_to_rgba(self.aridity[i]),
                NoiseLayer::PrecipitationType => {
                    precipitation_type_to_rgba(self.precipitation_type[i])
                }
                NoiseLayer::Snowpack => snowpack_to_rgba(self.snowpack[i]),
                NoiseLayer::WaterTable => water_table_to_rgba(self.water_table[i]),
                NoiseLayer::VegetationDensity => vegetation_to_rgba(self.vegetation_density[i]),
                NoiseLayer::SoilType => soil_type_to_rgba(self.soil_type[i]),
                NoiseLayer::ResourceRichness => resources_to_rgba(self.resource_richness[i]),
                NoiseLayer::WindSpeed => wind_speed_to_rgba(self.wind_speed[i]),
                NoiseLayer::Volcanism => volcanism_to_rgba(self.volcanism[i]),
            };
            out.extend_from_slice(&rgba);
        }
        out
    }

    /// Save a single layer as PNG to the given path.
    pub fn save_layer_png(
        &self,
        layer: NoiseLayer,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let rgba = self.layer_to_rgba(layer);
        let img = image::RgbaImage::from_raw(self.width as u32, self.height as u32, rgba)
            .ok_or("Failed to create image from buffer")?;
        img.save(path)?;
        Ok(())
    }

    /// Save all debug PNGs to the given directory.
    pub fn save_all_debug_pngs(
        &self,
        dir: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(dir)?;
        for &layer in NoiseLayer::all() {
            let path = dir.join(format!("{}.png", layer.name()));
            self.save_layer_png(layer, &path)?;
        }
        Ok(())
    }
}

fn sample_world_coord(origin: f64, world_size: f64, sample_count: usize, index: usize) -> f64 {
    if sample_count <= 1 {
        return origin;
    }
    origin + (index as f64 / (sample_count - 1) as f64) * world_size
}

fn sample_world_step(world_size: f64, sample_count: usize) -> f64 {
    if sample_count <= 1 {
        return world_size;
    }
    world_size / (sample_count - 1) as f64
}

/// Compute slope grid from heightmap using 3x3 finite differences.
pub fn compute_slope_grid(heightmap: &[f64], width: usize, height: usize) -> Vec<f64> {
    let total = width * height;
    let mut slope = vec![0.0f64; total];
    for y in 1..(height - 1) {
        for x in 1..(width - 1) {
            let idx = y * width + x;
            let dzdx = (heightmap[idx + 1] - heightmap[idx - 1]) * 0.5;
            let dzdy = (heightmap[idx + width] - heightmap[idx - width]) * 0.5;
            slope[idx] = (dzdx * dzdx + dzdy * dzdy).sqrt();
        }
    }
    for x in 0..width {
        slope[x] = slope[width + x.clamp(1, width - 2)];
        slope[(height - 1) * width + x] = slope[(height - 2) * width + x.clamp(1, width - 2)];
    }
    for y in 0..height {
        slope[y * width] = slope[y * width + 1];
        slope[y * width + width - 1] = slope[y * width + width - 2];
    }
    slope
}

fn apply_polar_ice_cap(
    biomes: &mut [TileType],
    light_level: &[f64],
    continentalness: &[f64],
    peaks_valleys: &[f64],
    rock_hardness: &[f64],
    _temperature: &[f64],
    sea_level: f64,
) {
    for idx in 0..biomes.len() {
        let light = light_level[idx];
        let cont = continentalness[idx];
        let rock = rock_hardness[idx];
        let pv = peaks_valleys[idx];

        if light < 0.05 {
            biomes[idx] = TileType::White;
            continue;
        }

        let land_bonus = if cont >= sea_level { 0.02 } else { -0.02 };
        let light_perturb = pv * 0.06 + (rock - 0.5) * 0.06 + land_bonus;
        let threshold = 0.12 + light_perturb;

        if light < threshold {
            biomes[idx] = if cont < sea_level {
                TileType::White
            } else {
                TileType::IceSheet
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sample_world_coord, sample_world_step, tile_has_fluid_surface};
    use mg_core::TileType;

    #[test]
    fn adjacent_tiles_share_the_same_border_samples() {
        let sample_count = 512;
        let left_edge = sample_world_coord(440.0, 1.0, sample_count, sample_count - 1);
        let right_edge = sample_world_coord(441.0, 1.0, sample_count, 0);

        assert!((left_edge - right_edge).abs() < f64::EPSILON);
    }

    #[test]
    fn sample_step_reaches_tile_extent_inclusively() {
        let sample_count = 512;
        let step = sample_world_step(1.0, sample_count);
        let last_sample = step * (sample_count - 1) as f64;

        assert!((last_sample - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn fluid_surface_tiles_are_semantic_not_elevation_based() {
        assert!(tile_has_fluid_surface(TileType::Sea));
        assert!(tile_has_fluid_surface(TileType::DeepOcean));
        assert!(!tile_has_fluid_surface(TileType::IceSheet));
        assert!(!tile_has_fluid_surface(TileType::White));
        assert!(!tile_has_fluid_surface(TileType::SaltFlat));
        assert!(!tile_has_fluid_surface(TileType::ScorchedRock));
    }
}
