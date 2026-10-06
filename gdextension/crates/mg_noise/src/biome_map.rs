//! BiomeMap: complete terrain snapshot for a given LOD tile.
//! Holds all base and derived noise layers plus the computed biome grid.

use mg_core::{NoiseStrategy, TileType};
use noise::OpenSimplex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::biome_splines::BiomeSplines;
use crate::derived;
use crate::landscape::{
    flatness, flatness_of_slope, grow_landscape, refine_landscape, refined_field, FineHeights,
    LandscapeInputs, REFINED_CELLS_PER_MACRO_CELL,
};
use crate::rivers::{
    carve_depths, rasterize_courses, rasterize_to_tile, sea_bodies, RiverCourse, RiverNetwork,
    LOD_THRESHOLD_MACRO,
};

/// Cutting a river's bed never takes the ground closer to sea level than this.
const RIVER_BED_ABOVE_SEA: f64 = 0.0005;
use crate::strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy, PeaksAndValleysStrategy,
    RockHardnessStrategy, TectonicPlatesStrategy,
};
use crate::visualization::NoiseLayer;

pub const SEA_LEVEL: f64 = -0.01;

/// A cell with at least this much sand on it (see `BiomeMap::sand`) is a
/// sand sea.
const SAND_SEA_FROM: f64 = 0.45;

/// Whether a tile is ice lying on water or land.
pub fn tile_is_ice(tile: TileType) -> bool {
    matches!(
        tile,
        TileType::White | TileType::IceSheet | TileType::Glacier
    )
}

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

/// World size in world units (one world unit is one chunk).
pub const WORLD_WIDTH: f64 = 1024.0;
pub const WORLD_HEIGHT: f64 = 512.0;
/// The macro map has one cell per chunk. The D8 river flow solve needs this
/// resolution: at 2 cells per world unit segments halved and rivers fragmented.
pub const MACRO_MAP_WIDTH: usize = 1024;
pub const MACRO_MAP_HEIGHT: usize = 512;
/// Size of the low-resolution terrain sample used to tell whether saved macro
/// data still matches this generator.
const MACRO_PROBE_WIDTH: usize = 64;
const MACRO_PROBE_HEIGHT: usize = 32;

/// The macro world map for `seed`: the whole world at one cell per chunk, with
/// erosion and the global river network. This is the single definition of the
/// map that layers artifacts, macro packs and the runtime all use. Takes
/// several seconds.
pub fn generate_macro_map(seed: u32) -> BiomeMap {
    BiomeMap::generate(
        seed,
        0.0,
        0.0,
        WORLD_WIDTH,
        WORLD_HEIGHT,
        MACRO_MAP_WIDTH,
        MACRO_MAP_HEIGHT,
        0,
        true,
        true,
        1.0,
    )
}

/// The sea's freezing and drying lines wander by up to this much light level
/// either way: about ten world units on the ground.
const SEA_MARGIN_DRIFT: f64 = 0.06;
/// The broadest bends in those lines are about this many world units long.
const SEA_MARGIN_BEND_WU: f64 = 45.0;
const SEA_MARGIN_OCTAVES: usize = 4;

/// How far the sea's freezing and drying lines are displaced at a world
/// position, as a change in light level. Where the sea turns to ice or dries
/// out depends on light, which alone would draw both edges as clean arcs
/// across the world. Pack ice and a retreating shoreline are ragged: tongues,
/// bays and outliers. Continuous across the east-west seam.
pub fn sea_margin_drift(wx: f64, wy: f64) -> f64 {
    use noise::NoiseFn;
    static DRIFT: std::sync::OnceLock<OpenSimplex> = std::sync::OnceLock::new();
    let noise = DRIFT.get_or_init(|| OpenSimplex::new(0x5EA_1CEu32));
    let (mut sum, mut weight, mut total) = (0.0, 1.0, 0.0);
    for octave in 0..SEA_MARGIN_OCTAVES {
        let frequency = 2f64.powi(octave as i32) / SEA_MARGIN_BEND_WU;
        let [cx, cz, cy] =
            crate::wrap::cylindrical_noise_coords(wx, wy, frequency, 1.0, WORLD_WIDTH);
        sum += weight * noise.get([cx, cz, cy]);
        total += weight;
        weight *= 0.55;
    }
    sum / total * SEA_MARGIN_DRIFT * 2.0
}

/// Light level at a point of the world: 0 in deep night, 1 under the sun.
/// A function of position and seed alone, so anything can ask for it without
/// a generated map.
pub fn light_level_at(seed: u32, wx: f64, wy: f64) -> f64 {
    LightLevelStrategy::new(
        seed.wrapping_add(SEED_LIGHT_LEVEL),
        0.5,
        1.0,
        WORLD_WIDTH,
        WORLD_HEIGHT,
    )
    .generate(wx, wy, 0)
}

// Map tiles show the shape of the land, not the roughness of the ground
// underfoot: they are generated without the fine noise the game's chunks add.
const MAP_TILE_DETAIL_LEVEL: u32 = 1;
const MAP_TILE_MOUNTAIN_DETAIL_GAIN: f64 = 0.2;

/// Terrain for a rectangle of the world at any resolution, agreeing with the
/// macro map: the tiles of `macromap.png` and the tiles the website's map
/// renders as it zooms in are both made by this.
pub fn generate_map_tile(
    macro_map: &BiomeMap,
    river_courses: &[RiverCourse],
    seed: u32,
    origin_x: f64,
    origin_y: f64,
    world_size_x: f64,
    world_size_y: f64,
    samples_x: usize,
    samples_y: usize,
) -> BiomeMap {
    let mut tile = BiomeMap::generate(
        seed,
        origin_x,
        origin_y,
        world_size_x,
        world_size_y,
        samples_x,
        samples_y,
        MAP_TILE_DETAIL_LEVEL,
        false,
        false,
        1.0,
    );
    tile.anchor_to_macro(
        macro_map,
        river_courses,
        seed,
        origin_x,
        origin_y,
        world_size_x,
        world_size_y,
        crate::rivers::LOD_THRESHOLD_MACRO,
        MAP_TILE_MOUNTAIN_DETAIL_GAIN,
        false,
    );
    tile
}

/// A coarse heightmap of the whole world, cheap to generate. Saved macro data
/// stores it; if this build produces a different probe for the same seed, the
/// generator has changed and the saved data is stale.
pub fn generate_macro_probe(seed: u32) -> Vec<f32> {
    BiomeMap::generate(
        seed,
        0.0,
        0.0,
        WORLD_WIDTH,
        WORLD_HEIGHT,
        MACRO_PROBE_WIDTH,
        MACRO_PROBE_HEIGHT,
        0,
        true,
        false,
        1.0,
    )
    .heightmap
    .iter()
    .map(|&height| height as f32)
    .collect()
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
        self.pixels
            .get(py * self.width + px)
            .copied()
            .unwrap_or(false)
    }
}

/// Bilinear sample a per-pixel field slice at world coordinates.
///
/// The x-axis wraps (cylindrical world); the y-axis is clamped. Used to smoothly
/// interpolate macro artifact fields when projecting them into finer runtime tiles —
/// nearest-neighbor sampling would produce hard seams at runtime chunk boundaries.
/// Sample a field at a world coordinate with a smooth curve (a cubic
/// B-spline) over its cells, on the same lattice as `sample_field_bilinear`.
/// Bilinear sampling is continuous but creased along every cell edge, which
/// shows as square facets when the result is shaded as relief; this has no
/// creases. It rounds the field off slightly rather than passing exactly
/// through the cell values, so use it for shading, not for terrain.
pub fn sample_field_smooth(
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
    let fx = crate::wrap::wrap_x(wx, world_width) * width as f64 / world_width;
    let fy = (wy.clamp(0.0, world_height) * height as f64 / world_height).min((height - 1) as f64);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    // Cubic B-spline weights for the four cells around a position.
    let weights = |t: f64| {
        let u = 1.0 - t;
        [
            u * u * u / 6.0,
            (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0,
            (3.0 * u * u * u - 6.0 * u * u + 4.0) / 6.0,
            t * t * t / 6.0,
        ]
    };
    let (weights_x, weights_y) = (weights(tx), weights(ty));
    let mut value = 0.0;
    for (row, weight_y) in weights_y.iter().enumerate() {
        // North and south edges are clamped; the map joins east to west.
        let y = (y0 as i32 + row as i32 - 1).clamp(0, height as i32 - 1) as usize;
        for (column, weight_x) in weights_x.iter().enumerate() {
            let x = crate::wrap::wrap_grid_x(x0 as i32 + column as i32 - 1, width) as usize;
            value += field[y * width + x] * weight_x * weight_y;
        }
    }
    value
}

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
    /// How much wind-blown sand lies on each cell, from 0 (bare) to 1 (a
    /// sand sea). See `wind.rs`.
    pub sand: Vec<f64>,

    #[serde(skip)]
    pub river_network: Option<Arc<RiverNetwork>>,

    /// The macro map only: the land's height on a finer grid, which tiles
    /// and chunks anchored to this map take their ground from.
    pub fine_heights: Option<FineHeights>,

    pub world_width: f64,
    pub world_height: f64,
}

impl BiomeMap {
    /// A map of the given size with every layer zeroed.
    pub fn empty(width: usize, height: usize, world_width: f64, world_height: f64) -> Self {
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
            sand: vec![0.0; n],
            river_network: None,
            fine_heights: None,
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

        // All base layers are sampled at true world coords (wx, wy). freq_scale is
        // intentionally ignored here; scaled coords were causing coast_perturb to
        // flip shallow-ocean cells to land. Micro-scale terrain variety comes from
        // derive_micro_heightmap at detail_level >= 2, not from scaled noise layers.
        let base_data: Vec<(f64, f64, f64, f64, f64)> = pixels
            .par_iter()
            .map(|&(_, wx, wy)| {
                let cont = cont_strat.generate(wx, wy, detail_level);
                let light = light_strat.generate(wx, wy, detail_level);
                let rock = rock_strat.generate(wx, wy, detail_level);
                let humid =
                    humid_strat.generate_terminator_model(wx, wy, detail_level, cont, light);
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

        // ── Phase 2.5: The rim sea (macro only) ──────────────────────────────
        // Join the terminus seas into one that can be sailed right round
        // the world, before the land is grown, so the straits get coasts
        // like any other.
        if run_erosion {
            let splines = BiomeSplines::new(SEA_LEVEL);
            let stays_liquid: Vec<bool> = (0..tile_w * tile_h)
                .map(|i| {
                    let (wx, wy) = (px_to_wx(i % tile_w), py_to_wy(i / tile_w));
                    // Judged as a strait would be, whatever is there now, in
                    // both the driest and the most humid air the climate
                    // pass may leave over it: humid air reads warmer, and a
                    // sea must neither freeze in the one nor dry in the other.
                    let drift = sea_margin_drift(wx, wy);
                    [0.0, 1.0].into_iter().all(|humidity| {
                        let at_sea = derived::derive_temperature(
                            map.light_level[i],
                            SEA_LEVEL,
                            humidity,
                            SEA_LEVEL,
                        );
                        splines.sea_is_liquid(
                            SEA_LEVEL - crate::rim_sea::STRAIT_DEPTH,
                            at_sea,
                            map.tectonic[i],
                            map.light_level[i],
                            drift,
                        )
                    })
                })
                .collect();
            crate::rim_sea::open_rim_sea(
                &mut map.continentalness,
                &stays_liquid,
                tile_w,
                tile_h,
                SEA_LEVEL,
            );
        }

        // ── Phase 3: Erosion (macro only) ─────────────────────────────────────
        // Erosion and the rivers share one drainage: water runs to the sea
        // (not to ponds), fed by run-off that is fullest in the terminus.
        let mut drainage = None;
        let mut fine_land = None;
        // Level of standing water over each macro cell, where there is a lake.
        let mut macro_water_level: Vec<f32> = Vec::new();
        let drains = |ground: &[f64], map: &BiomeMap| {
            let is_sea = sea_bodies(&map.continentalness, tile_w, tile_h, SEA_LEVEL);
            let rainfall: Vec<f64> = (0..tile_w * tile_h)
                .map(|i| crate::drainage::rainfall(map.light_level[i], map.humidity[i]))
                .collect();
            (ground.to_vec(), is_sea, rainfall)
        };
        if run_erosion {
            // The land is grown from uplift and erosion; the noise heightmap
            // above only stands in for tiles that are anchored to the macro
            // map later.
            let inputs = LandscapeInputs {
                continentalness: &map.continentalness,
                peaks_valleys: &map.peaks_valleys,
                tectonic: &map.tectonic,
                rock_hardness: &map.rock_hardness,
                light_level: &map.light_level,
                humidity: &map.humidity,
                width: tile_w,
                height: tile_h,
            };
            let landscape = grow_landscape(&inputs);
            map.heightmap = landscape.heightmap;

            // ── Climate from the land ─────────────────────────────────────
            // With the mountains grown, the air can be followed over them:
            // temperature falls with height, and moisture off the liquid sea
            // is carried by the wind and rained out on windward slopes. The
            // humidity noise is replaced by that rain, and everything after
            // (the rivers' run-off, aridity, biomes, LifeGen) reads it.
            for i in 0..tile_w * tile_h {
                map.temperature[i] = derived::derive_temperature(
                    map.light_level[i],
                    map.heightmap[i],
                    map.humidity[i],
                    map.continentalness[i],
                );
            }
            let splines = BiomeSplines::new(SEA_LEVEL);
            let is_liquid_sea: Vec<bool> = (0..tile_w * tile_h)
                .map(|i| {
                    map.heightmap[i] < SEA_LEVEL
                        && splines.sea_is_liquid(
                            map.heightmap[i],
                            map.temperature[i],
                            map.tectonic[i],
                            map.light_level[i],
                            sea_margin_drift(px_to_wx(i % tile_w), py_to_wy(i / tile_w)),
                        )
                })
                .collect();
            let wind = crate::wind::surface_wind(&map.light_level, &map.heightmap, tile_w, tile_h);
            let rain = crate::climate::rainfall(
                &wind,
                &crate::climate::Land {
                    heightmap: &map.heightmap,
                    temperature: &map.temperature,
                    is_liquid_sea: &is_liquid_sea,
                    width: tile_w,
                    height: tile_h,
                },
            );
            map.humidity = (0..tile_w * tile_h)
                .map(|i| crate::climate::humidity_from_rain(rain[i], map.light_level[i]))
                .collect();

            // Rivers are read from a finer copy of the land, so they follow
            // valleys narrower than a macro cell. Its run-off is the rain.
            if run_rivers {
                let inputs = LandscapeInputs {
                    continentalness: &map.continentalness,
                    peaks_valleys: &map.peaks_valleys,
                    tectonic: &map.tectonic,
                    rock_hardness: &map.rock_hardness,
                    light_level: &map.light_level,
                    humidity: &map.humidity,
                    width: tile_w,
                    height: tile_h,
                };
                fine_land = Some(refine_landscape(&inputs, &map.heightmap));
            }
            macro_water_level = landscape.water_level;
            map.drainage_area = landscape
                .drainage
                .flow
                .iter()
                .map(|&flow| flow.round() as u32)
                .collect();
            map.sediment = landscape.sediment;
            drainage = Some(landscape.drainage);

            // Recompute temperature with the grown land
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
            let network = if let Some(fine) = fine_land {
                let scale = REFINED_CELLS_PER_MACRO_CELL;
                let temperature = refined_field(&map.temperature, tile_w, tile_h, scale);
                // On the fine grid the ground itself says where the sea is.
                let network = RiverNetwork::generate(
                    &fine.drainage,
                    &fine.tectonic,
                    &fine.ground,
                    &fine.light_level,
                    &fine.humidity,
                    &temperature,
                    fine.heights.width,
                    fine.heights.height,
                    SEA_LEVEL,
                );
                map.fine_heights = Some(fine.heights);
                network
            } else {
                // Without erosion there is no drainage yet: solve it on the
                // ground as it stands.
                let drainage = drainage.unwrap_or_else(|| {
                    let (ground, is_sea, rainfall) = drains(&map.heightmap, &map);
                    let evaporation: Vec<f64> = map
                        .light_level
                        .iter()
                        .map(|&light| crate::drainage::lake_evaporation(light))
                        .collect();
                    crate::drainage::solve_drainage(
                        &ground,
                        &is_sea,
                        &rainfall,
                        &evaporation,
                        tile_w,
                        tile_h,
                        0,
                    )
                });
                RiverNetwork::generate(
                    &drainage,
                    &map.tectonic,
                    &map.continentalness,
                    &map.light_level,
                    &map.humidity,
                    &map.temperature,
                    tile_w,
                    tile_h,
                    SEA_LEVEL,
                )
            };
            map.rivers = network.to_flow_grid(tile_w, tile_h);
            map.river_network = Some(Arc::new(network));
        }

        // ── Phase 5: Remaining derived layers ─────────────────────────────────
        // The erosion layer holds how flat the ground is (0 steep, 1 level).
        let flat = flatness(&map.heightmap, tile_w, tile_h, world_size_x / tile_w as f64);
        for i in 0..tile_w * tile_h {
            let h = map.heightmap[i];
            let cont = map.continentalness[i];
            let temp = map.temperature[i];
            let humid = map.humidity[i];
            let rock = map.rock_hardness[i];
            let tect = map.tectonic[i];
            let river = map.rivers[i];
            let light = map.light_level[i];

            map.erosion[i] = flat[i];
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
            let drift = sea_margin_drift(px_to_wx(i % tile_w), py_to_wy(i / tile_w));
            // Biomes read the land and its climate, not the noise layers: the
            // peaks-and-valleys layer takes no part.
            let biome = splines.evaluate_with_light(
                map.heightmap[i],
                map.temperature[i],
                map.tectonic[i],
                map.erosion[i],
                0.0,
                map.humidity[i],
                map.aridity[i],
                map.rock_hardness[i],
                map.light_level[i],
                drift,
            );
            // A hollow holding water is a lake: open water where the sea
            // would be liquid, ice where it would be frozen, a salt flat
            // where it would have dried out.
            let lake_level = macro_water_level
                .get(i)
                .copied()
                .unwrap_or(f32::NEG_INFINITY) as f64;
            let biome = if map.heightmap[i] >= SEA_LEVEL && map.heightmap[i] < lake_level {
                splines.lake_biome(
                    lake_level - map.heightmap[i],
                    map.temperature[i],
                    map.light_level[i],
                    drift,
                )
            } else {
                biome
            };
            map.biomes[i] = biome;
            map.vegetation_density[i] =
                derived::derive_vegetation_density(biome, map.water_table[i]);
            map.soil_type[i] =
                derived::derive_soil_type(biome, map.erosion[i], map.rock_hardness[i]);
        }

        // The wind gathers sand where it slackens; where the sand lies thick
        // is a sand sea, whatever lay there before (macro only).
        if run_erosion {
            let is_water: Vec<bool> = map
                .biomes
                .iter()
                .map(|&biome| tile_has_fluid_surface(biome) || tile_is_ice(biome))
                .collect();
            let wind = crate::wind::surface_wind(&map.light_level, &map.heightmap, tile_w, tile_h);
            map.sand = crate::wind::drifted_sand(
                &wind,
                &map.heightmap,
                &map.sediment,
                &map.light_level,
                &is_water,
                tile_w,
                tile_h,
            );
            for i in 0..tile_w * tile_h {
                if map.sand[i] >= SAND_SEA_FROM && !is_water[i] {
                    map.biomes[i] = TileType::Erg;
                } else if map.biomes[i] == TileType::Erg {
                    // Sand seas are where the wind leaves sand, nowhere else.
                    map.biomes[i] = TileType::Desert;
                }
            }
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
            if mask.is_ocean_at_world(wx_center, wy_center)
                && !tile_has_fluid_surface(self.biomes[i])
            {
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
        river_courses: &[RiverCourse],
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
                // Same sample positions as `generate`: the first and last sample
                // of a tile lie on its edges, so a tile's last column and its
                // neighbour's first column are the same world positions and must
                // come out identical. Sampling cell centres here instead put a
                // step in the terrain along one chunk border in eight.
                let wx = sample_world_coord(origin_x, world_size_x, tile_w, px);
                let wy = sample_world_coord(origin_y, world_size_y, tile_h, py);

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
                self.tectonic[idx] = macro_map.sample_field_at(&macro_map.tectonic, wx, wy);
                self.humidity[idx] = macro_map.sample_field_at(&macro_map.humidity, wx, wy);
                self.rock_hardness[idx] =
                    macro_map.sample_field_at(&macro_map.rock_hardness, wx, wy);
                self.peaks_valleys[idx] =
                    macro_map.sample_field_at(&macro_map.peaks_valleys, wx, wy);

                // Sample the macro heightmap. This is the value the macro pass
                // used for ALL its derivations and biome classification. Runtime
                // must use the same value as the spline input to match macro.
                // Where the macro map carries a finer copy of its land, the
                // ground comes from that.
                let macro_hm = match &macro_map.fine_heights {
                    Some(fine) => fine.sample(wx, wy),
                    None => macro_map.sample_field_at(&macro_map.heightmap, wx, wy),
                };

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
                // How flat the macro land is here, from its slope over a chunk.
                let rise = |dx: f64, dy: f64| {
                    macro_map.sample_field_at(&macro_map.heightmap, wx + dx, wy + dy)
                        - macro_map.sample_field_at(&macro_map.heightmap, wx - dx, wy - dy)
                };
                let slope = (rise(0.5, 0.0).powi(2) + rise(0.0, 0.5).powi(2)).sqrt();
                let eros = flatness_of_slope(slope);
                let arid = derived::derive_aridity(temp, humid);
                let precip = derived::derive_precipitation_type(temp, humid, macro_hm);
                let snow = derived::derive_snowpack(precip, temp, macro_hm, light);

                self.temperature[idx] = temp;
                self.erosion[idx] = eros;
                self.aridity[idx] = arid;
                self.precipitation_type[idx] = precip;
                self.snowpack[idx] = snow;
                self.resource_richness[idx] = derived::derive_resource_richness(tect, rock, eros);

                // Classify biome with the SAME spline call the macro pass uses
                // (`biome_map.rs:483`, non-dithered). With every spline input
                // matching macro, the biome enum must match macro.
                let lake_level = macro_map
                    .fine_heights
                    .as_ref()
                    .and_then(|fine| fine.lake_level(wx, wy))
                    .filter(|_| macro_hm >= SEA_LEVEL);
                self.biomes[idx] = if let Some(level) = lake_level {
                    splines.lake_biome(level - macro_hm, temp, light, sea_margin_drift(wx, wy))
                } else {
                    splines.evaluate_with_light(
                        macro_hm,
                        temp,
                        tect,
                        eros,
                        0.0,
                        humid,
                        arid,
                        rock,
                        light,
                        sea_margin_drift(wx, wy),
                    )
                };

                // Write the rendered heightmap with mesh detail on top of the
                // macro-anchored value. This drives mesh generation and visual
                // hillshade — biome classification already happened above using
                // raw macro_hm so the detail doesn't perturb biome boundaries.
                let stress = 1.0 - tect;
                let above_sea = (macro_hm - SEA_LEVEL).max(0.0);
                let mountain_intensity = (stress * above_sea * 3.0).min(1.0);
                // Fine land has its own relief; the peaks layer adds bumps
                // only where there is none.
                let mountain_detail = if macro_map.fine_heights.is_some() {
                    0.0
                } else {
                    peaks * mountain_intensity * mountain_detail_gain
                };
                let mut hm = (macro_hm + mountain_detail).clamp(-1.0, 1.0);
                // Where the wind has left a sand sea, the ground is sand and
                // stands in dunes.
                let sand = if macro_map.sand.is_empty() {
                    0.0
                } else {
                    macro_map.sample_field_at(&macro_map.sand, wx, wy)
                };
                let dry_land =
                    !tile_has_fluid_surface(self.biomes[idx]) && !tile_is_ice(self.biomes[idx]);
                // (The bed of a dried sea is dry land too, and holds the most.)
                if dry_land {
                    if sand >= SAND_SEA_FROM {
                        self.biomes[idx] = TileType::Erg;
                    } else if self.biomes[idx] == TileType::Erg {
                        self.biomes[idx] = TileType::Desert;
                    }
                    hm +=
                        crate::wind::dune_height(sand, wx, wy, world_size_x / (tile_w - 1) as f64);
                }
                if apply_micro_detail {
                    hm = derived::derive_micro_heightmap(hm, wx, wy, &detail_noise);
                }
                self.heightmap[idx] = hm;
            }
        }

        // Rivers are drawn from the world's river courses at this tile's own
        // resolution. The same geometry is drawn at every scale, so a river
        // here is the river on the macro map, and neighbouring tiles agree
        // along their shared border.
        self.rivers = rasterize_courses(
            river_courses,
            origin_x,
            origin_y,
            world_size_x,
            world_size_y,
            tile_w,
            tile_h,
        );

        // Cut each river's bed and valley floor into the ground. The land
        // is never taken below the sea by this, and the sea is left alone.
        let carved = carve_depths(
            river_courses,
            origin_x,
            origin_y,
            world_size_x,
            world_size_y,
            tile_w,
            tile_h,
        );
        for i in 0..tile_w * tile_h {
            if self.heightmap[i] > SEA_LEVEL {
                self.heightmap[i] =
                    (self.heightmap[i] - carved[i]).max(SEA_LEVEL + RIVER_BED_ABOVE_SEA);
            }
        }

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

    /// As `sample_field_at`, without creases along cell edges. See
    /// `sample_field_smooth`.
    pub fn sample_field_smooth_at(&self, field: &[f64], wx: f64, wy: f64) -> f64 {
        sample_field_smooth(
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
    use super::{sample_world_coord, sample_world_step, tile_has_fluid_surface, BiomeMap};
    use crate::rivers::LOD_THRESHOLD_MICRO;
    use mg_core::TileType;

    const SEED: u32 = 42;

    /// A small chunk anchored to `macro_map`, the way the runtime builds one.
    fn anchored_chunk(macro_map: &BiomeMap, x: f64, y: f64) -> BiomeMap {
        let mut chunk = BiomeMap::generate(SEED, x, y, 1.0, 1.0, 32, 32, 2, false, false, 8.0);
        chunk.anchor_to_macro(
            macro_map,
            &[],
            SEED,
            x,
            y,
            1.0,
            1.0,
            LOD_THRESHOLD_MICRO,
            0.2,
            true,
        );
        chunk
    }

    #[test]
    fn smooth_sampling_follows_the_field_and_wraps_east_to_west() {
        // A 4 by 2 field over a 4 by 2 world: one cell per world unit.
        let field = [0.0, 1.0, 4.0, 9.0, 0.0, 1.0, 4.0, 9.0];
        let sample = |wx: f64, wy: f64| super::sample_field_smooth(&field, wx, wy, 4.0, 2.0, 4, 2);

        // Between cells it lies between its neighbours.
        assert!(sample(1.5, 0.0) > 1.0 && sample(1.5, 0.0) < 4.0);
        // Just west of the seam is just east of the last column.
        assert_eq!(sample(-0.5, 0.0), sample(3.5, 0.0));
        // A flat field stays flat.
        let flat = [0.3; 8];
        let level = super::sample_field_smooth(&flat, 1.3, 0.6, 4.0, 2.0, 4, 2);
        assert!((level - 0.3).abs() < 1e-12);
    }

    #[test]
    fn anchored_chunks_have_identical_heights_along_their_shared_border() {
        // A coarse macro map without the river pass, which assumes the full-size grid.
        let macro_map = BiomeMap::generate(
            SEED, 0.0, 0.0, 1024.0, 512.0, 256, 128, 0, false, false, 1.0,
        );
        let here = anchored_chunk(&macro_map, 440.0, 220.0);
        let east = anchored_chunk(&macro_map, 441.0, 220.0);
        let south = anchored_chunk(&macro_map, 440.0, 221.0);
        let (w, h) = (here.width, here.height);

        for row in 0..h {
            assert_eq!(
                here.heightmap[row * w + (w - 1)],
                east.heightmap[row * w],
                "east border, row {row}"
            );
        }
        for column in 0..w {
            assert_eq!(
                here.heightmap[(h - 1) * w + column],
                south.heightmap[column],
                "south border, column {column}"
            );
        }
    }

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
