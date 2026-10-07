//! The macro map: the whole world generated on the cubed sphere (spec 015).
//!
//! Everything that is solved over the world as a whole (the landscape, the
//! climate, the rivers, the sand) runs on `CubeGrid`, where every cell is
//! about the same size and shape and no cell is a pole. The flat map the
//! game and the site use (`BiomeMap`, one cell per chunk on a 1024 by 512
//! equirectangular raster) is read off the cube at the end, by
//! `MacroMap::to_biome_map`. The land's fine heights stay on the cube and
//! are sampled from it wherever they are needed.

use std::sync::Arc;

use mg_core::{CubeGrid, NoiseStrategy, Sphere, TileType};
use rayon::prelude::*;

use crate::biome_map::{
    sea_margin_drift, tile_has_fluid_surface, tile_is_ice, BiomeMap, MACRO_MAP_HEIGHT,
    MACRO_MAP_WIDTH, SEA_LEVEL, WORLD_HEIGHT, WORLD_WIDTH,
};
use crate::biome_splines::BiomeSplines;
use crate::derived;
use crate::landscape::{
    flatness_on_cube, grow_landscape, refine_landscape, FineHeights, Landscape, LandscapeInputs,
    SeedGround,
};
use crate::seed_land::SeedLand;
use crate::rivers::RiverNetwork;
use crate::strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy, PeaksAndValleysStrategy,
    RockHardnessStrategy, TectonicPlatesStrategy,
};

/// Cells a side of each cube face. Six faces of this many squared is
/// 525,696 cells: one per chunk, as the flat map has.
pub const MACRO_CUBE_N: usize = 296;
/// The probe (`generate_macro_probe`) is the same pipeline on a cube this
/// small, cheap enough to run at every start.
const PROBE_CUBE_N: usize = 16;

/// Seed offsets per base layer (additive from world_seed). The same as the
/// flat tiles use, so a chunk's own noise agrees with the macro map's.
pub(crate) const SEED_CONTINENTALNESS: u32 = 0;
pub(crate) const SEED_TECTONIC: u32 = 1;
pub(crate) const SEED_HUMIDITY: u32 = 2;
pub(crate) const SEED_ROCK_HARDNESS: u32 = 3;
pub(crate) const SEED_LIGHT_LEVEL: u32 = 4;
pub(crate) const SEED_PEAKS_VALLEYS: u32 = 7;

/// A cell with at least this much sand on it (see `MacroMap::sand`) is a
/// sand sea.
pub const SAND_SEA_FROM: f64 = 0.45;

/// A seeded world (spec 017) holds the seed's land fully up to this
/// latitude and not at all from `SEED_FADES_OUT_DEGREES`, where the
/// sphere's own layers stand.
pub const SEED_HOLDS_TO_DEGREES: f64 = 70.0;
pub const SEED_FADES_OUT_DEGREES: f64 = 85.0;

/// Depths a strait's route may dredge sea to, tried shallowest first: just
/// past the shallows a hot sea dries out from, and past a hot basin's.
const STRAIT_SHALLOWS: [f64; 2] = [0.09, 0.17];

/// The seed's heights and how far they hold, one each per cube cell.
struct Seeding {
    heights: Vec<f64>,
    hold: Vec<f64>,
}

/// The world on the cube, every layer one value per cell.
///
/// Stored in a layers artifact without its river network and fine heights,
/// which the flat map beside it carries.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MacroMap {
    #[serde(with = "crate::landscape::cube_grid_by_size")]
    pub grid: CubeGrid,

    // Base layers
    pub continentalness: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub tectonic_plate_ids: Vec<f64>,
    pub humidity: Vec<f64>,
    pub rock_hardness: Vec<f64>,
    pub light_level: Vec<f64>,
    pub peaks_valleys: Vec<f64>,

    // The land and its climate
    pub heightmap: Vec<f64>,
    /// Level of standing water over each cell: a lake's surface, or
    /// negative infinity where there is none.
    pub water_level: Vec<f32>,
    pub temperature: Vec<f64>,
    /// How flat the ground is, 0 steep to 1 level.
    pub erosion: Vec<f64>,
    pub rivers: Vec<f64>,
    pub aridity: Vec<f64>,
    pub precipitation_type: Vec<f64>,
    pub water_table: Vec<f64>,
    pub resource_richness: Vec<f64>,
    pub snowpack: Vec<f64>,
    pub drainage_area: Vec<u32>,
    pub sediment: Vec<f64>,
    /// Wind-blown sand on each cell, 0 bare to 1 a sand sea (`wind.rs`).
    pub sand: Vec<f64>,

    pub biomes: Vec<TileType>,
    pub vegetation_density: Vec<f64>,
    pub soil_type: Vec<f64>,

    #[serde(skip)]
    pub river_network: Arc<RiverNetwork>,
    /// The land on a cube four times finer, which tiles and chunks take
    /// their ground from.
    #[serde(skip, default = "FineHeights::empty")]
    pub fine_heights: FineHeights,
}

/// The layers that come straight from noise at a position, before any
/// solve over the world.
struct Base {
    grid: CubeGrid,
    continentalness: Vec<f64>,
    tectonic: Vec<f64>,
    tectonic_plate_ids: Vec<f64>,
    humidity: Vec<f64>,
    rock_hardness: Vec<f64>,
    light_level: Vec<f64>,
    peaks_valleys: Vec<f64>,
}

impl Base {
    fn sample(seed: u32, n: usize) -> Self {
        let grid = CubeGrid::margin(n);
        let cont_strat = ContinentalnessStrategy::new(seed.wrapping_add(SEED_CONTINENTALNESS));
        let tect_strat = TectonicPlatesStrategy::new(seed.wrapping_add(SEED_TECTONIC));
        let humid_strat = HumidityStrategy::new(seed.wrapping_add(SEED_HUMIDITY));
        let rock_strat = RockHardnessStrategy::new(seed.wrapping_add(SEED_ROCK_HARDNESS));
        let light_strat = LightLevelStrategy::new(seed.wrapping_add(SEED_LIGHT_LEVEL));
        let pv_strat = PeaksAndValleysStrategy::new(seed.wrapping_add(SEED_PEAKS_VALLEYS));

        let positions: Vec<(f64, f64)> = (0..grid.cell_count())
            .map(|cell| grid.world_position(cell))
            .collect();
        // (continentalness, tectonic, plate id, humidity, rock, light, peaks)
        let layers: Vec<[f64; 7]> = positions
            .par_iter()
            .map(|&(wx, wy)| {
                let plates = tect_strat.generate_full(wx, wy);
                let (tect, plate_id) = (plates.boundary_distance, plates.plate_id);
                let cont = cont_strat.generate(wx, wy, 0);
                let light = light_strat.generate(wx, wy, 0);
                let rock = rock_strat.generate(wx, wy, 0);
                let humid = humid_strat.generate_terminator_model(wx, wy, 0, cont, light);
                let peaks = derived::derive_peaks_valleys(pv_strat.generate(wx, wy, 0), tect, rock);
                [cont, tect, plate_id, humid, rock, light, peaks]
            })
            .collect();
        let layer = |which: usize| -> Vec<f64> { layers.iter().map(|cell| cell[which]).collect() };
        Self {
            continentalness: layer(0),
            tectonic: layer(1),
            tectonic_plate_ids: layer(2),
            humidity: layer(3),
            rock_hardness: layer(4),
            light_level: layer(5),
            peaks_valleys: layer(6),
            grid,
        }
    }

    /// The cube's own layers with the flat map's laid over them where the
    /// seed holds (spec 017): the plates and the land as the flat map had
    /// them, the sphere's own towards the poles.
    fn sample_seeded(seed: u32, n: usize, land: &SeedLand) -> (Self, Seeding) {
        let mut base = Self::sample(seed, n);
        let grid = &base.grid;
        let (holds_to, fades_out) = (
            SEED_HOLDS_TO_DEGREES.to_radians(),
            SEED_FADES_OUT_DEGREES.to_radians(),
        );
        let mut seeding = Seeding {
            heights: Vec::with_capacity(grid.cell_count()),
            hold: Vec::with_capacity(grid.cell_count()),
        };
        for cell in 0..grid.cell_count() {
            let latitude = grid.point(cell)[2].clamp(-1.0, 1.0).asin().abs();
            let fade = ((latitude - holds_to) / (fades_out - holds_to)).clamp(0.0, 1.0);
            let hold = 1.0 - fade * fade * (3.0 - 2.0 * fade);
            let (wx, wy) = grid.world_position(cell);
            let lay = |own: &mut f64, field: &[f64]| {
                *own += (land.sample(field, wx, wy) - *own) * hold;
            };
            lay(&mut base.continentalness[cell], &land.continentalness);
            lay(&mut base.tectonic[cell], &land.tectonic);
            lay(&mut base.rock_hardness[cell], &land.rock_hardness);
            lay(&mut base.peaks_valleys[cell], &land.peaks_valleys);
            // The flat map's light is what put its terminus where it is;
            // the sphere's own sun takes over only towards the poles, where
            // the flat map's light would meet itself.
            lay(&mut base.light_level[cell], &land.light_level);
            if hold > 0.5 {
                base.tectonic_plate_ids[cell] = land.nearest(&land.tectonic_plate_ids, wx, wy);
            }
            seeding.heights.push(land.sample(&land.heightmap, wx, wy));
            seeding.hold.push(hold);
        }
        (base, seeding)
    }

    /// Join the terminus seas into one that can be sailed right round the
    /// world, before the land is grown, so the straits get coasts like any
    /// other.
    ///
    /// The route is found on a flat projection of the cube (the rim sea is
    /// a ring round the map), and the straits it cuts are written back to
    /// the cube's cells.
    fn open_rim_sea(&mut self) {
        let grid = &self.grid;
        // Finer than the smallest cube cell (at a corner, about three
        // quarters of the mean), so every cell under a strait is cut.
        let (width, height) = (6 * grid.n, 3 * grid.n);
        let wu_per_cell = WORLD_WIDTH / width as f64;
        let position = |cell: usize| {
            (
                ((cell % width) as f64 + 0.5) * wu_per_cell,
                ((cell / width) as f64 + 0.5) * wu_per_cell,
            )
        };
        let points: Vec<_> = (0..width * height)
            .map(|cell| {
                let (wx, wy) = position(cell);
                grid.sphere.point_at(wx, wy)
            })
            .collect();
        let before: Vec<f64> = points
            .iter()
            .map(|&point| grid.sample(&self.continentalness, point))
            .collect();
        let splines = BiomeSplines::new(SEA_LEVEL);
        // Whether sea `depth` below sea level at a cell stays liquid. Judged
        // as a strait would be, whatever is there now, in both the driest
        // and the most humid air the climate pass may leave over it: humid
        // air reads warmer, and a sea must neither freeze in the one nor
        // dry in the other.
        let liquid_at = |cell: usize, depth: f64| {
            let (wx, wy) = position(cell);
            let light = grid.sample(&self.light_level, points[cell]);
            let tectonic = grid.sample(&self.tectonic, points[cell]);
            let drift = sea_margin_drift(wx, wy);
            [0.0, 1.0].into_iter().all(|humidity| {
                let at_sea = derived::derive_temperature(light, SEA_LEVEL, humidity, SEA_LEVEL);
                splines.sea_is_liquid(SEA_LEVEL - depth, at_sea, tectonic, light, drift)
            })
        };
        let stays_liquid: Vec<bool> = (0..width * height)
            .map(|cell| liquid_at(cell, crate::rim_sea::STRAIT_DEPTH))
            .collect();
        // The least depth at which sea stays liquid at each cell: what the
        // route must dredge shallows to, and no more.
        let needed_depth: Vec<f64> = (0..width * height)
            .map(|cell| {
                STRAIT_SHALLOWS
                    .into_iter()
                    .find(|&depth| liquid_at(cell, depth))
                    .unwrap_or(crate::rim_sea::STRAIT_DEPTH)
            })
            .collect();
        let mut after = before.clone();
        crate::rim_sea::open_rim_sea(
            &mut after,
            &stays_liquid,
            &needed_depth,
            width,
            height,
            SEA_LEVEL,
        );

        // Every cell under a cut is lowered to the deepest cut over it. A
        // strait shallows to its shores, and a cell that took the cut over
        // its centre alone could miss the channel and leave the strait a
        // chain of pools.
        for flat in 0..width * height {
            if after[flat] < before[flat] {
                let cell = grid.cell_of(points[flat]);
                self.continentalness[cell] = self.continentalness[cell].min(after[flat]);
            }
        }
        for cell in 0..grid.cell_count() {
            let (wx, wy) = grid.world_position(cell);
            let x = ((wx / wu_per_cell).floor() as usize).min(width - 1);
            let y = ((wy / wu_per_cell).floor() as usize).min(height - 1);
            let flat = y * width + x;
            if after[flat] < before[flat] {
                self.continentalness[cell] = self.continentalness[cell].min(after[flat]);
            }
        }
    }

    fn landscape_inputs(&self) -> LandscapeInputs<'_> {
        LandscapeInputs {
            continentalness: &self.continentalness,
            peaks_valleys: &self.peaks_valleys,
            tectonic: &self.tectonic,
            rock_hardness: &self.rock_hardness,
            light_level: &self.light_level,
            humidity: &self.humidity,
            grid: &self.grid,
            seed: None,
        }
    }

    fn temperature(&self, heightmap: &[f64]) -> Vec<f64> {
        (0..self.grid.cell_count())
            .map(|cell| {
                derived::derive_temperature(
                    self.light_level[cell],
                    heightmap[cell],
                    self.humidity[cell],
                    self.continentalness[cell],
                )
            })
            .collect()
    }

    /// The land grown from flat, or from the seed where one is given, with
    /// the straits of the rim sea opened first.
    fn grown(mut self, seeding: Option<&Seeding>) -> (Self, Landscape) {
        self.open_rim_sea();
        let inputs = LandscapeInputs {
            seed: seeding.map(|seeding| SeedGround {
                heights: &seeding.heights,
                hold: &seeding.hold,
            }),
            ..self.landscape_inputs()
        };
        let landscape = grow_landscape(&inputs);
        (self, landscape)
    }
}

/// A coarse heightmap of the whole world, cheap to generate. Saved macro
/// data stores it; if this build produces a different probe for the same
/// seed, the generator has changed and the saved data is stale.
pub fn generate_macro_probe(seed: u32) -> Vec<f32> {
    let (_, landscape) = Base::sample(seed, PROBE_CUBE_N).grown(None);
    landscape
        .heightmap
        .iter()
        .map(|&height| height as f32)
        .collect()
}

impl MacroMap {
    /// The world for `seed`, grown from flat. Takes about half a minute.
    pub fn generate(seed: u32) -> Self {
        Self::generate_with(seed, None)
    }

    /// The world for `seed`, grown from the flat map's land (spec 017).
    pub fn generate_seeded(seed: u32, land: &SeedLand) -> Self {
        Self::generate_with(seed, Some(land))
    }

    fn generate_with(seed: u32, land: Option<&SeedLand>) -> Self {
        let (mut base, landscape) = match land {
            None => Base::sample(seed, MACRO_CUBE_N).grown(None),
            Some(land) => {
                let (base, seeding) = Base::sample_seeded(seed, MACRO_CUBE_N, land);
                base.grown(Some(&seeding))
            }
        };
        let grid = base.grid.clone();
        let total = grid.cell_count();
        let heightmap = landscape.heightmap;

        // ── Climate from the land ─────────────────────────────────────
        // With the mountains grown, the air can be followed over them:
        // temperature falls with height, and moisture off the liquid sea is
        // carried by the wind and rained out on windward slopes. The
        // humidity noise is replaced by that rain, and everything after
        // (the rivers' run-off, aridity, biomes, LifeGen) reads it.
        let temperature = base.temperature(&heightmap);
        let splines = BiomeSplines::new(SEA_LEVEL);
        let is_liquid_sea: Vec<bool> = (0..total)
            .map(|cell| {
                let (wx, wy) = grid.world_position(cell);
                heightmap[cell] < SEA_LEVEL
                    && splines.sea_is_liquid(
                        heightmap[cell],
                        temperature[cell],
                        base.tectonic[cell],
                        base.light_level[cell],
                        sea_margin_drift(wx, wy),
                    )
            })
            .collect();
        let wind = crate::wind::surface_wind(&base.light_level, &heightmap, &grid);
        let rain = crate::climate::rainfall(
            &wind,
            &crate::climate::Land {
                heightmap: &heightmap,
                temperature: &temperature,
                is_liquid_sea: &is_liquid_sea,
                grid: &grid,
            },
        );
        base.humidity = (0..total)
            .map(|cell| crate::climate::humidity_from_rain(rain[cell], base.light_level[cell]))
            .collect();
        // Humid air reads warmer: the temperature with the rain in it.
        let temperature = base.temperature(&heightmap);

        // ── Rivers ────────────────────────────────────────────────────
        // Read from a finer copy of the land, so they follow valleys
        // narrower than a macro cell. Its run-off is the rain.
        let fine = refine_landscape(&base.landscape_inputs(), &heightmap, &temperature);
        let river_network = RiverNetwork::generate(
            &fine.drainage,
            &fine.tectonic,
            &fine.ground,
            &fine.light_level,
            &fine.humidity,
            &fine.temperature,
            &fine.heights.grid,
            SEA_LEVEL,
        );
        let rivers = river_network.to_flow_grid(&grid);

        // ── Derived layers ────────────────────────────────────────────
        let erosion = flatness_on_cube(&heightmap, &grid);
        let mut map = Self {
            aridity: vec![0.0; total],
            precipitation_type: vec![0.0; total],
            water_table: vec![0.0; total],
            resource_richness: vec![0.0; total],
            snowpack: vec![0.0; total],
            biomes: vec![TileType::Sea; total],
            vegetation_density: vec![0.0; total],
            soil_type: vec![0.0; total],
            drainage_area: landscape
                .drainage
                .flow
                .iter()
                .map(|&flow| flow.round() as u32)
                .collect(),
            sediment: landscape.sediment,
            sand: vec![0.0; total],
            water_level: landscape.water_level,
            river_network: Arc::new(river_network),
            fine_heights: fine.heights,
            continentalness: base.continentalness,
            tectonic: base.tectonic,
            tectonic_plate_ids: base.tectonic_plate_ids,
            humidity: base.humidity,
            rock_hardness: base.rock_hardness,
            light_level: base.light_level,
            peaks_valleys: base.peaks_valleys,
            heightmap,
            temperature,
            erosion,
            rivers,
            grid,
        };
        for cell in 0..total {
            let (temp, humid) = (map.temperature[cell], map.humidity[cell]);
            let h = map.heightmap[cell];
            map.aridity[cell] = derived::derive_aridity(temp, humid);
            map.precipitation_type[cell] = derived::derive_precipitation_type(temp, humid, h);
            map.snowpack[cell] = derived::derive_snowpack(
                map.precipitation_type[cell],
                temp,
                h,
                map.light_level[cell],
            );
            map.water_table[cell] = derived::derive_water_table(
                map.rivers[cell],
                humid,
                h,
                map.precipitation_type[cell],
                map.continentalness[cell],
            );
            map.resource_richness[cell] = derived::derive_resource_richness(
                map.tectonic[cell],
                map.rock_hardness[cell],
                map.erosion[cell],
            );
        }

        // ── Biomes ────────────────────────────────────────────────────
        for cell in 0..total {
            let (wx, wy) = map.grid.world_position(cell);
            let drift = sea_margin_drift(wx, wy);
            // Biomes read the land and its climate, not the noise layers:
            // the peaks-and-valleys layer takes no part.
            let biome = splines.evaluate_with_light(
                map.heightmap[cell],
                map.temperature[cell],
                map.tectonic[cell],
                map.erosion[cell],
                0.0,
                map.humidity[cell],
                map.aridity[cell],
                map.rock_hardness[cell],
                map.light_level[cell],
                drift,
            );
            // A hollow holding water is a lake: open water where the sea
            // would be liquid, ice where it would be frozen, a salt flat
            // where it would have dried out.
            let lake_level = map.water_level[cell] as f64;
            let biome = if map.heightmap[cell] >= SEA_LEVEL && map.heightmap[cell] < lake_level {
                splines.lake_biome(
                    lake_level - map.heightmap[cell],
                    map.temperature[cell],
                    map.light_level[cell],
                    drift,
                )
            } else {
                biome
            };
            map.biomes[cell] = biome;
        }

        // The wind gathers sand where it slackens; where the sand lies thick
        // is a sand sea, whatever lay there before.
        let is_water: Vec<bool> = map
            .biomes
            .iter()
            .map(|&biome| tile_has_fluid_surface(biome) || tile_is_ice(biome))
            .collect();
        map.sand = crate::wind::drifted_sand(
            &wind,
            &map.heightmap,
            &map.sediment,
            &map.light_level,
            &is_water,
            &map.grid,
        );
        for cell in 0..total {
            if map.sand[cell] >= SAND_SEA_FROM && !is_water[cell] {
                map.biomes[cell] = TileType::Erg;
            } else if map.biomes[cell] == TileType::Erg {
                // Sand seas are where the wind leaves sand, nowhere else.
                map.biomes[cell] = TileType::Desert;
            }
        }

        crate::biome_map::apply_polar_ice_cap(
            &mut map.biomes,
            &map.light_level,
            &map.continentalness,
            &map.peaks_valleys,
            &map.rock_hardness,
            &map.temperature,
            SEA_LEVEL,
        );
        for cell in 0..total {
            map.vegetation_density[cell] =
                derived::derive_vegetation_density(map.biomes[cell], map.water_table[cell]);
            map.soil_type[cell] = derived::derive_soil_type(
                map.biomes[cell],
                map.erosion[cell],
                map.rock_hardness[cell],
            );
        }
        map
    }

    /// The world as a flat map, one cell per chunk (`MACRO_MAP_WIDTH` by
    /// `MACRO_MAP_HEIGHT`): every layer read off the cube at each raster
    /// cell's world position, continuous layers between cells and biomes
    /// from the nearest. The rivers are drawn on the raster from their
    /// courses, as every other scale draws them.
    pub fn to_biome_map(&self) -> BiomeMap {
        let (width, height) = (MACRO_MAP_WIDTH, MACRO_MAP_HEIGHT);
        let mut map = BiomeMap::empty(width, height, WORLD_WIDTH, WORLD_HEIGHT);
        // The raster's cell (x, y) holds the value at world position (x, y):
        // what `BiomeMap::sample_field_at` reads it as.
        let points: Vec<_> = (0..width * height)
            .map(|cell| {
                Sphere::MARGIN.point_at(
                    (cell % width) as f64 * (WORLD_WIDTH / width as f64),
                    (cell / width) as f64 * (WORLD_HEIGHT / height as f64),
                )
            })
            .collect();
        let nearest: Vec<usize> = points
            .iter()
            .map(|&point| self.grid.cell_of(point))
            .collect();
        let sampled = |field: &[f64]| -> Vec<f64> {
            points
                .par_iter()
                .map(|&point| self.grid.sample(field, point))
                .collect()
        };

        map.continentalness = sampled(&self.continentalness);
        map.tectonic = sampled(&self.tectonic);
        map.tectonic_plate_ids = nearest
            .iter()
            .map(|&cell| self.tectonic_plate_ids[cell])
            .collect();
        map.humidity = sampled(&self.humidity);
        map.rock_hardness = sampled(&self.rock_hardness);
        map.light_level = sampled(&self.light_level);
        map.peaks_valleys = sampled(&self.peaks_valleys);
        map.heightmap = sampled(&self.heightmap);
        map.temperature = sampled(&self.temperature);
        map.erosion = sampled(&self.erosion);
        map.rivers = self.river_network.to_flow_raster(width, height);
        map.aridity = sampled(&self.aridity);
        map.precipitation_type = sampled(&self.precipitation_type);
        map.water_table = sampled(&self.water_table);
        map.resource_richness = sampled(&self.resource_richness);
        map.snowpack = sampled(&self.snowpack);
        map.biomes = nearest.iter().map(|&cell| self.biomes[cell]).collect();
        map.vegetation_density = sampled(&self.vegetation_density);
        map.soil_type = sampled(&self.soil_type);
        map.drainage_area = nearest
            .iter()
            .map(|&cell| self.drainage_area[cell])
            .collect();
        map.sediment = sampled(&self.sediment);
        map.sand = sampled(&self.sand);
        map.river_network = Some(Arc::clone(&self.river_network));
        map.fine_heights = Some(self.fine_heights.clone());
        map
    }
}

/// The macro world map for `seed` as a flat map: the whole world at one cell
/// per chunk, with the land, the climate and the global river network. This
/// is the single definition of the map that layers artifacts, macro packs
/// and the runtime all use. Takes about half a minute.
pub fn generate_macro_map(seed: u32) -> BiomeMap {
    MacroMap::generate(seed).to_biome_map()
}

/// As `generate_macro_map`, grown from the flat map's land (spec 017).
pub fn generate_macro_map_seeded(seed: u32, land: &SeedLand) -> BiomeMap {
    MacroMap::generate_seeded(seed, land).to_biome_map()
}
