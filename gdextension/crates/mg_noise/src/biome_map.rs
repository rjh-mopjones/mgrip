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
use crate::rivers::{
    generate_river_network, rasterize_from_network, RiverNetwork, LOD_THRESHOLD_MACRO,
};
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

/// Identifies ocean pixels from legacy biome.png RGB values.
/// Used by macro-ocean-mask loading and older comparison paths that still rely
/// on biome image artifacts rather than semantic chunk data.
pub fn pixel_is_ocean_rgb(r: u8, g: u8, b: u8) -> bool {
    let rf = r as f64 / 255.0;
    let bf = b as f64 / 255.0;
    // Blue-dominant biomes: Sea, ShallowSea, ContinentalShelf, DeepOcean, OceanTrench
    if bf - rf > 0.25 && bf > 0.35 {
        return true;
    }
    // OceanRidge: rgb(120, 80, 60)
    r == 120 && g == 80 && b == 60
}

/// Ocean mask derived from biome.png — authoritative ocean/land from the macro pipeline.
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
    /// Load from a biome.png file produced by `BiomeMap::save_layer_png(NoiseLayer::Biome, ...)`.
    pub fn load(path: &std::path::Path, world_width: f64, world_height: f64) -> Result<Self, String> {
        let img = image::open(path)
            .map_err(|e| format!("failed to load {}: {e}", path.display()))?
            .into_rgb8();
        let width = img.width() as usize;
        let height = img.height() as usize;
        let pixels = img.pixels().map(|p| pixel_is_ocean_rgb(p[0], p[1], p[2])).collect();
        Ok(Self { pixels, width, height, world_width, world_height })
    }

    /// Returns true if the world position maps to an ocean pixel in biome.png.
    pub fn is_ocean_at_world(&self, wx: f64, wy: f64) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        let px = ((wx / self.world_width * self.width as f64) as usize).min(self.width - 1);
        let py = ((wy / self.world_height * self.height as f64) as usize).min(self.height - 1);
        self.pixels.get(py * self.width + px).copied().unwrap_or(false)
    }
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
        //     (ocean/land boundary, zone, palette) is therefore identical to biome.png for
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
            let network = generate_river_network(
                &map.heightmap,
                tile_w,
                tile_h,
                &map.light_level,
                &map.humidity,
                &map.temperature,
                LOD_THRESHOLD_MACRO,
            );
            let river_grid = rasterize_from_network(&network, tile_w, tile_h, LOD_THRESHOLD_MACRO);
            map.rivers = river_grid;
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

    /// Override biome classification for cells where the macro biome.png says ocean.
    ///
    /// The macro world map is authoritative for ocean placement. Where biome.png shows
    /// ocean but the noise pipeline classified as land (e.g. dayside cells demoted to
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
        // Sample biome.png at the centre of each 1-world-unit cell, not at every
        // runtime pixel. biome.png is only 4 px/wu — sampling at individual runtime
        // pixel coords near a coastline can straddle two biome.png pixels and produce
        // a jagged split-ocean-land result within a single chunk. Snapping to
        // (floor + 0.5) gives the same result as the compare tool, which also samples
        // the centre of each 1×1 chunk cell.
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

    /// Quick accessor — returns heightmap value at pixel (x, y).
    pub fn heightmap_at(&self, x: usize, y: usize) -> f64 {
        self.heightmap[y * self.width + x]
    }

    pub fn biome_at(&self, x: usize, y: usize) -> TileType {
        self.biomes[y * self.width + x]
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
