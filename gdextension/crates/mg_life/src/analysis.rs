//! LifeGen stage 1: analysis grids.
//!
//! Ported from Randlebrot's `rb_world::lifegen::analysis`. Each grid holds one
//! `f32` per terrain cell and is a pure function of terrain.

use mg_core::{TerrainQuery, TileType};
use rayon::prelude::*;
use std::collections::VecDeque;

/// Randlebrot ran LifeGen on a grid of 8 cells per world unit. Distances and
/// slopes below were tuned at that resolution; they are kept in world units
/// and rescaled to whatever grid the terrain is queried on.
const REFERENCE_CELLS_PER_WORLD_UNIT: f64 = 8.0;
/// Rivers add to water availability within this distance.
const RIVER_WATER_BONUS_RADIUS_WU: f64 = 12.0 / REFERENCE_CELLS_PER_WORLD_UNIT;
/// Travel is easier within this distance of a river (valley roads).
const VALLEY_ROAD_BONUS_RADIUS_WU: f64 = 20.0 / REFERENCE_CELLS_PER_WORLD_UNIT;

/// Stage 1 output. Every grid is `width * height`, row-major.
pub struct AnalysisGrids {
    pub width: usize,
    pub height: usize,
    /// Distance to the nearest river cell, in cells. `f32::MAX` if the map has no rivers.
    pub river_distance: Vec<f32>,
    /// How well a cell supports settlement, 0.0 to 1.0. Ocean is 0.0.
    pub habitability: Vec<f32>,
    /// Ease of travel: 0.0 impassable, 1.0 trivial. Ocean is 0.0.
    pub navigation_cost: Vec<f32>,
    /// Geological resource potential, 0.0 to 1.0.
    pub resource_desirability: Vec<f32>,
}

/// Compute all stage 1 grids. `cells_per_world_unit` is the resolution of the
/// grid behind `terrain` (1.0 for the macro map: one cell per chunk).
pub fn compute_analysis_grids(
    terrain: &dyn TerrainQuery,
    cells_per_world_unit: f64,
) -> AnalysisGrids {
    let river_distance = compute_river_distance_field(terrain);
    AnalysisGrids {
        width: terrain.width(),
        height: terrain.height(),
        habitability: compute_habitability(terrain, &river_distance, cells_per_world_unit),
        navigation_cost: compute_navigation_cost(terrain, &river_distance, cells_per_world_unit),
        resource_desirability: compute_resource_desirability(terrain),
        river_distance,
    }
}

/// Slope as Randlebrot's formulas expect it: height change per reference cell.
/// A coarser grid spans more ground per cell, so its raw slope is larger.
fn reference_slope(
    terrain: &dyn TerrainQuery,
    x: usize,
    y: usize,
    cells_per_world_unit: f64,
) -> f64 {
    terrain.slope_at(x, y) * cells_per_world_unit / REFERENCE_CELLS_PER_WORLD_UNIT
}

/// Distance from every cell to the nearest river cell, in cells.
///
/// 4-connected BFS from all river cells, then one pass that tightens the
/// estimate using diagonal steps. River cells are 0.0.
pub fn compute_river_distance_field(terrain: &dyn TerrainQuery) -> Vec<f32> {
    let w = terrain.width();
    let h = terrain.height();
    let mut dist = vec![f32::MAX; w * h];
    let mut queue = VecDeque::new();

    for y in 0..h {
        for x in 0..w {
            if terrain.is_river(x, y) {
                dist[y * w + x] = 0.0;
                queue.push_back((x, y));
            }
        }
    }

    while let Some((x, y)) = queue.pop_front() {
        let current_dist = dist[y * w + x];
        let offsets: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        for &(dx, dy) in &offsets {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let (nux, nuy) = (nx as usize, ny as usize);
            let ni = nuy * w + nux;
            let new_dist = current_dist + 1.0;
            if new_dist < dist[ni] {
                dist[ni] = new_dist;
                queue.push_back((nux, nuy));
            }
        }
    }

    let snapshot = dist.clone();
    dist.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let neighbours: [(i32, i32, f32); 8] = [
                (-1, -1, 1.414),
                (0, -1, 1.0),
                (1, -1, 1.414),
                (-1, 0, 1.0),
                (1, 0, 1.0),
                (-1, 1, 1.414),
                (0, 1, 1.0),
                (1, 1, 1.414),
            ];
            for &(dx, dy, cost) in &neighbours {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let candidate = snapshot[ny as usize * w + nx as usize] + cost;
                if candidate < row[x] {
                    row[x] = candidate;
                }
            }
        }
    });

    dist
}

/// Habitability, 0.0 to 1.0, from four weighted factors:
/// - Temperature comfort (35%): peaks at 15 C, zero at -20 C and 50 C
/// - Water availability (30%): humidity, river proximity, drainage
/// - Elevation comfort (20%): best between sea level and 0.3
/// - Terrain stability (15%): low slope, low tectonic stress
///
/// Ocean cells are 0.0.
pub fn compute_habitability(
    terrain: &dyn TerrainQuery,
    river_distance: &[f32],
    cells_per_world_unit: f64,
) -> Vec<f32> {
    let w = terrain.width();
    let mut grid = vec![0.0f32; w * terrain.height()];

    grid.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            if terrain.is_ocean(x, y) {
                continue;
            }

            let temperature = terrain.temperature_at(x, y);
            let temperature_score = (1.0 - ((temperature - 15.0) / 35.0).powi(2)).clamp(0.0, 1.0);

            let river_cells = river_distance[y * w + x] as f64;
            let river_wu = river_cells / cells_per_world_unit;
            let river_bonus = if river_cells < 1.0 {
                0.4
            } else if river_wu < RIVER_WATER_BONUS_RADIUS_WU {
                0.3 * (1.0 - river_wu / RIVER_WATER_BONUS_RADIUS_WU)
            } else {
                0.0
            };
            let drainage_score = (terrain.drainage_at(x, y) / 1000.0).min(1.0) * 0.1;
            let water_score =
                (terrain.humidity_at(x, y) * 0.5 + river_bonus + drainage_score).min(1.0);

            let elevation = terrain.heightmap_at(x, y);
            let elevation_score = if elevation < 0.0 {
                0.2
            } else if elevation <= 0.3 {
                1.0
            } else if elevation <= 0.6 {
                1.0 - (elevation - 0.3) / 0.3 * 0.5
            } else {
                0.3 * (1.0 - elevation)
            };

            let slope_penalty =
                (reference_slope(terrain, x, y, cells_per_world_unit) * 5.0).min(1.0);
            let tectonic_penalty = (terrain.tectonic_at(x, y) * 0.5).min(0.5);
            let stability_score = (1.0 - slope_penalty - tectonic_penalty).max(0.0);

            let composite = temperature_score * 0.35
                + water_score * 0.30
                + elevation_score * 0.20
                + stability_score * 0.15;
            row[x] = composite.clamp(0.0, 1.0) as f32;
        }
    });

    grid
}

/// Ease of travel: 0.0 impassable, 1.0 trivial. (The name follows Randlebrot;
/// higher is easier.)
///
/// Biome traversability minus slope and high-elevation penalties. Crossing a
/// river is expensive; travelling near one is slightly easier. Ocean is 0.0.
pub fn compute_navigation_cost(
    terrain: &dyn TerrainQuery,
    river_distance: &[f32],
    cells_per_world_unit: f64,
) -> Vec<f32> {
    let w = terrain.width();
    let mut grid = vec![0.0f32; w * terrain.height()];

    grid.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            if terrain.is_ocean(x, y) {
                continue;
            }

            let biome_ease = biome_traversability(terrain.biome_at(x, y)) as f64;
            let slope_penalty =
                (reference_slope(terrain, x, y, cells_per_world_unit) * 3.0).min(0.7);
            let elevation = terrain.heightmap_at(x, y);
            let elevation_penalty = if elevation > 0.7 {
                (elevation - 0.7) * 2.0
            } else {
                0.0
            };
            let mut ease = (biome_ease - slope_penalty - elevation_penalty).clamp(0.0, 1.0);

            let river_cells = river_distance[y * w + x] as f64;
            let river_wu = river_cells / cells_per_world_unit;
            if river_cells < 1.0 {
                ease *= 0.15;
            } else if river_wu < VALLEY_ROAD_BONUS_RADIUS_WU {
                let bonus = 0.15 * (1.0 - river_wu / VALLEY_ROAD_BONUS_RADIUS_WU);
                ease = (ease + bonus).min(1.0);
            }

            row[x] = ease as f32;
        }
    });

    grid
}

/// Geological resource potential, 0.0 to 1.0:
/// - Minerals (40%): tectonic activity
/// - Rock (25%): rock hardness
/// - Exposure (20%): erosion exposes deposits
/// - Fertility (15%): humidity, reduced by erosion
pub fn compute_resource_desirability(terrain: &dyn TerrainQuery) -> Vec<f32> {
    let w = terrain.width();
    let mut grid = vec![0.0f32; w * terrain.height()];

    grid.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let erosion = terrain.erosion_at(x, y);
            let mineral_score = (terrain.tectonic_at(x, y) * 1.5).min(1.0);
            let rock_score = terrain.rock_hardness_at(x, y);
            let exposure_score = (erosion * 2.0).min(1.0);
            let fertility_score =
                (terrain.humidity_at(x, y) * (1.0 - erosion * 0.5)).clamp(0.0, 1.0);

            let composite = mineral_score * 0.40
                + rock_score * 0.25
                + exposure_score * 0.20
                + fertility_score * 0.15;
            row[x] = composite.clamp(0.0, 1.0) as f32;
        }
    });

    grid
}

/// How easy a biome is to cross on foot: 0.0 impassable, 1.0 trivial.
fn biome_traversability(biome: TileType) -> f32 {
    match biome {
        TileType::Plains | TileType::Meadow | TileType::Oasis => 1.0,

        TileType::Beach | TileType::Steppe | TileType::Savanna | TileType::AlpineMeadow => 0.85,

        TileType::Forest
        | TileType::Woodland
        | TileType::DeciduousForest
        | TileType::TemperateRainforest
        | TileType::Taiga
        | TileType::SubtropicalForest
        | TileType::CloudForest => 0.7,

        TileType::River => 0.6,

        TileType::Mangrove
        | TileType::RockyCoast
        | TileType::SeaCliff
        | TileType::Scrubland
        | TileType::Badlands
        | TileType::Hamada => 0.5,

        TileType::Desert
        | TileType::Sahara
        | TileType::Jungle
        | TileType::Marsh
        | TileType::FrozenBog
        | TileType::DryWoodland
        | TileType::Thornland
        | TileType::HighlandSavanna => 0.4,

        TileType::Snow
        | TileType::Tundra
        | TileType::IceSheet
        | TileType::Glacier
        | TileType::Erg
        | TileType::SaltFlat => 0.3,

        TileType::Mountain | TileType::Plateau => 0.2,

        TileType::Volcanic
        | TileType::LavaField
        | TileType::MoltenWaste
        | TileType::ScorchedRock => 0.1,

        // Deep-night ice: half frozen sea, half frozen land. Crossable, but
        // harder than ordinary snow and ice.
        TileType::White => 0.2,

        TileType::Sea
        | TileType::ShallowSea
        | TileType::ContinentalShelf
        | TileType::DeepOcean
        | TileType::OceanTrench
        | TileType::OceanRidge => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat, temperate plains with rivers wherever `rivers` is true.
    struct MockTerrain {
        width: usize,
        height: usize,
        rivers: Vec<bool>,
    }

    impl MockTerrain {
        fn with_river_along_row(width: usize, height: usize, river_row: usize) -> Self {
            let mut rivers = vec![false; width * height];
            for x in 0..width {
                rivers[river_row * width + x] = true;
            }
            Self {
                width,
                height,
                rivers,
            }
        }
    }

    impl TerrainQuery for MockTerrain {
        fn width(&self) -> usize {
            self.width
        }
        fn height(&self) -> usize {
            self.height
        }
        fn heightmap_at(&self, _x: usize, _y: usize) -> f64 {
            0.1
        }
        fn biome_at(&self, _x: usize, _y: usize) -> TileType {
            TileType::Plains
        }
        fn temperature_at(&self, _x: usize, _y: usize) -> f64 {
            15.0
        }
        fn humidity_at(&self, _x: usize, _y: usize) -> f64 {
            0.5
        }
        fn continentalness_at(&self, _x: usize, _y: usize) -> f64 {
            0.5
        }
        fn erosion_at(&self, _x: usize, _y: usize) -> f64 {
            0.1
        }
        fn light_level_at(&self, _x: usize, _y: usize) -> f64 {
            0.5
        }
        fn rock_hardness_at(&self, _x: usize, _y: usize) -> f64 {
            0.5
        }
        fn river_at(&self, x: usize, y: usize) -> f64 {
            if self.is_river(x, y) {
                1.0
            } else {
                0.0
            }
        }
        fn drainage_at(&self, _x: usize, _y: usize) -> f64 {
            0.0
        }
        fn tectonic_at(&self, _x: usize, _y: usize) -> f64 {
            0.0
        }
        fn peaks_valleys_at(&self, _x: usize, _y: usize) -> f64 {
            0.0
        }
        fn aridity_at(&self, _x: usize, _y: usize) -> f64 {
            0.0
        }
        fn slope_at(&self, _x: usize, _y: usize) -> f64 {
            0.0
        }
        fn is_ocean(&self, _x: usize, _y: usize) -> bool {
            false
        }
        fn is_river(&self, x: usize, y: usize) -> bool {
            x < self.width && y < self.height && self.rivers[y * self.width + x]
        }
    }

    #[test]
    fn river_distance_grows_away_from_the_river() {
        let terrain = MockTerrain::with_river_along_row(100, 100, 50);
        let dist = compute_river_distance_field(&terrain);

        for x in 0..100 {
            assert_eq!(dist[50 * 100 + x], 0.0);
            assert!(dist[49 * 100 + x] <= 1.01);
        }
        for x in [10, 50, 90] {
            for y in 0..49 {
                assert!(dist[y * 100 + x] >= dist[(y + 1) * 100 + x]);
            }
        }
    }

    #[test]
    fn habitability_is_higher_beside_a_river_than_far_from_it() {
        let terrain = MockTerrain::with_river_along_row(40, 40, 20);
        let grids = compute_analysis_grids(&terrain, 1.0);

        let beside_river = grids.habitability[19 * 40 + 20];
        let far_from_river = grids.habitability[2 * 40 + 20];
        assert!(beside_river > far_from_river);
    }

    #[test]
    fn river_bonus_reach_is_the_same_distance_on_a_finer_grid() {
        // 1.5 world units is the water bonus radius: 1 cell away is inside it
        // at 1 cell per world unit, 11 cells away is inside it at 8.
        let coarse = compute_analysis_grids(&MockTerrain::with_river_along_row(40, 40, 20), 1.0);
        let fine = compute_analysis_grids(&MockTerrain::with_river_along_row(40, 40, 20), 8.0);
        let baseline = coarse.habitability[2 * 40 + 20];

        assert!(coarse.habitability[19 * 40 + 20] > baseline);
        assert_eq!(coarse.habitability[18 * 40 + 20], baseline);
        assert!(fine.habitability[9 * 40 + 20] > baseline);
    }

    #[test]
    fn crossing_a_river_is_harder_than_walking_beside_it() {
        let terrain = MockTerrain::with_river_along_row(40, 40, 20);
        let grids = compute_analysis_grids(&terrain, 1.0);

        assert!(grids.navigation_cost[20 * 40 + 20] < grids.navigation_cost[19 * 40 + 20]);
    }

    #[test]
    fn open_water_is_impassable_and_plains_are_easiest() {
        assert_eq!(biome_traversability(TileType::Sea), 0.0);
        assert_eq!(biome_traversability(TileType::DeepOcean), 0.0);
        assert_eq!(biome_traversability(TileType::Plains), 1.0);
    }
}
