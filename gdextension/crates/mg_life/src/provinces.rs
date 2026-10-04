//! LifeGen stage 2: provinces.
//!
//! Ported from Randlebrot's `rb_world::lifegen::provinces`. Land is divided
//! into provinces: small and dense where habitability is high, large and
//! sparse where it is low. Ocean belongs to no province.

use crate::analysis::AnalysisGrids;
use crate::grid::Grid;
use mg_core::{TerrainQuery, TileType};
use rand::{Rng, SeedableRng};
use rand_xorshift::XorShiftRng;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, VecDeque};

// Seed spacing. Randlebrot used 0.5 and 10 world units with a hard cap of 1200
// provinces. On this world (89% land) that cap is reached long before habitable
// land fills up, which makes every province the same size. The radii are scaled
// by 3.8 so seeding saturates near the same province count on its own, keeping
// small provinces in habitable land and large ones in barren land.
/// Province seeds are at least this far apart in the most habitable land.
const MIN_SEED_RADIUS_WU: f64 = 2.0;
/// Province seeds are this far apart in barren land.
const MAX_SEED_RADIUS_WU: f64 = 38.0;
/// Barren land is treated as at least this habitable, so it still gets provinces.
const SEEDING_HABITABILITY_FLOOR: f64 = 0.02;
/// Safety bound only; seeding normally stops when no more seeds fit.
const MAX_PROVINCES: usize = 4000;
const MAX_SEED_ATTEMPTS: usize = 2_000_000;
/// This many land darts rejected in a row means no more seeds fit.
const SATURATION_REJECTIONS: usize = 20_000;
/// Land is never harder to cross than this when provinces grow over it.
const MIN_NAVIGATION_EASE: f32 = 0.05;
/// A province is coastal if any of its cells is this close to ocean.
const COAST_SEARCH_RADIUS_WU: f64 = 0.375;
/// A river of at least this size (share of the largest possible river, as
/// `TerrainQuery::river_at` reports it) is a major river. 0.4 is a river about
/// 0.8 world units wide.
const MAJOR_RIVER_SIZE: f64 = 0.4;
/// Each stage derives its own random stream from `civ_seed`.
const PROVINCE_SEED_OFFSET: u32 = 2;

const NEIGHBOURS_4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

#[derive(Debug, Clone, PartialEq)]
pub struct Province {
    /// 1-based. 0 means "no province" in `ProvinceMap::province_ids`.
    pub id: u16,
    /// Seed cell the province grew from.
    pub site: (usize, usize),
    /// Most common biome.
    pub biome: TileType,
    /// Mean habitability over the province.
    pub habitability: f32,
    pub area_cells: u32,
    pub is_coastal: bool,
    /// Contains a major river.
    pub is_river_junction: bool,
    pub elevation_mean: f32,
    /// Mean difficulty of travel: 0.0 trivial, 1.0 impassable.
    pub terrain_cost: f32,
    /// Mean light level: 0.0 deep night, 1.0 sub-stellar.
    pub light_level: f32,
    /// Mean resource desirability.
    pub resources: f32,
}

/// Stage 2 output.
pub struct ProvinceMap {
    pub width: usize,
    pub height: usize,
    /// Province id per cell, row-major. 0 = ocean.
    pub province_ids: Vec<u16>,
    /// Indexed by `id - 1`.
    pub provinces: Vec<Province>,
    /// Neighbouring province ids, indexed by province id (entry 0 is unused).
    pub adjacency: Vec<Vec<u16>>,
}

/// Divide land into provinces. Deterministic for a given terrain and `civ_seed`.
pub fn generate_provinces(
    terrain: &dyn TerrainQuery,
    analysis: &AnalysisGrids,
    grid: Grid,
    civ_seed: u32,
) -> ProvinceMap {
    let seeds = seed_provinces(
        terrain,
        &analysis.habitability,
        grid,
        civ_seed.wrapping_add(PROVINCE_SEED_OFFSET),
    );
    let (mut province_ids, adjacency) =
        tessellate_provinces(terrain, &seeds, &analysis.navigation_cost, grid);
    attach_unreached_land(terrain, &mut province_ids, grid);
    let provinces = bake_province_attributes(
        terrain,
        analysis,
        &province_ids,
        &seeds,
        grid,
    );
    ProvinceMap {
        width: terrain.width(),
        height: terrain.height(),
        province_ids,
        provinces,
        adjacency,
    }
}

/// Minimum distance, in cells, between a seed on land of this habitability and
/// any other seed.
fn seed_radius_cells(habitability: f32, grid: Grid) -> f64 {
    let habitability = (habitability as f64).max(SEEDING_HABITABILITY_FLOOR);
    let radius_wu =
        MAX_SEED_RADIUS_WU + (MIN_SEED_RADIUS_WU - MAX_SEED_RADIUS_WU) * habitability.powf(0.7);
    radius_wu * grid.cells_per_world_unit
}

/// Place province seeds by dart throwing: random land cells are accepted if no
/// existing seed lies within the larger of the two seeds' radii.
fn seed_provinces(
    terrain: &dyn TerrainQuery,
    habitability: &[f32],
    grid: Grid,
    seed: u32,
) -> Vec<(usize, usize)> {
    let (width, height) = (terrain.width(), terrain.height());

    // Spatial grid with at most one seed per bucket, so nearby seeds are found
    // without scanning them all.
    let bucket_size =
        (MIN_SEED_RADIUS_WU * grid.cells_per_world_unit / std::f64::consts::SQRT_2).max(1.0);
    let buckets_wide = (width as f64 / bucket_size).ceil() as usize;
    let buckets_high = (height as f64 / bucket_size).ceil() as usize;
    let mut buckets = vec![usize::MAX; buckets_wide * buckets_high];
    let search_buckets =
        (MAX_SEED_RADIUS_WU * grid.cells_per_world_unit / bucket_size).ceil() as usize + 1;

    let mut seeds: Vec<(usize, usize)> = Vec::with_capacity(MAX_PROVINCES);
    let mut rng = XorShiftRng::seed_from_u64(seed as u64);

    let mut rejections_in_a_row = 0;
    for _ in 0..MAX_SEED_ATTEMPTS {
        if seeds.len() >= MAX_PROVINCES || rejections_in_a_row >= SATURATION_REJECTIONS {
            break;
        }
        let x = rng.gen_range(0..width);
        let y = rng.gen_range(0..height);
        if terrain.is_ocean(x, y) {
            continue;
        }
        let radius = seed_radius_cells(habitability[y * width + x], grid);

        let bucket_x = (x as f64 / bucket_size) as usize;
        let bucket_y = (y as f64 / bucket_size) as usize;
        let x_buckets: Vec<usize> = (-(search_buckets as i32)..=search_buckets as i32)
            .filter_map(|offset| grid.step_x(bucket_x, offset, buckets_wide))
            .collect();
        let y_range = bucket_y.saturating_sub(search_buckets)
            ..=(bucket_y + search_buckets).min(buckets_high - 1);

        let too_close = y_range.into_iter().any(|by| {
            x_buckets.iter().any(|&bx| {
                let existing = buckets[by * buckets_wide + bx];
                if existing == usize::MAX {
                    return false;
                }
                let (sx, sy) = seeds[existing];
                let existing_radius =
                    seed_radius_cells(habitability[sy * width + sx], grid);
                let min_distance = radius.max(existing_radius);
                let (dx, dy) = (grid.dx(sx, x), y as f64 - sy as f64);
                dx * dx + dy * dy < min_distance * min_distance
            })
        });
        if too_close {
            rejections_in_a_row += 1;
            continue;
        }
        rejections_in_a_row = 0;

        buckets[bucket_y * buckets_wide + bucket_x] = seeds.len();
        seeds.push((x, y));
    }

    seeds
}

/// Grow every seed outward at once (Dijkstra). Each land cell joins the
/// province that reaches it most cheaply, where easy terrain is cheap to
/// cross. Ocean stays 0.
///
/// Returns province ids per cell and the adjacency list indexed by province id.
fn tessellate_provinces(
    terrain: &dyn TerrainQuery,
    seeds: &[(usize, usize)],
    navigation_cost: &[f32],
    grid: Grid,
) -> (Vec<u16>, Vec<Vec<u16>>) {
    let (width, height) = (terrain.width(), terrain.height());
    let mut province_ids = vec![0u16; width * height];
    let mut adjacency: Vec<BTreeSet<u16>> = vec![BTreeSet::new(); seeds.len() + 1];

    // (cost so far, cell index, province id); Reverse makes the heap a min-heap.
    let mut frontier: BinaryHeap<(Reverse<u32>, u32, u16)> = BinaryHeap::new();
    for (index, &(x, y)) in seeds.iter().enumerate() {
        frontier.push((Reverse(0), (y * width + x) as u32, (index + 1) as u16));
    }

    while let Some((Reverse(cost), cell, province_id)) = frontier.pop() {
        let cell = cell as usize;
        if province_ids[cell] != 0 {
            continue;
        }
        let (x, y) = (cell % width, cell / width);
        if terrain.is_ocean(x, y) {
            continue;
        }
        province_ids[cell] = province_id;

        for &(dx, dy) in &NEIGHBOURS_4 {
            let ny = y as i32 + dy;
            let Some(nx) = grid.step_x(x, dx, width) else {
                continue;
            };
            if ny < 0 || ny >= height as i32 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            let neighbour = ny * width + nx;

            let neighbour_id = province_ids[neighbour];
            if neighbour_id != 0 {
                if neighbour_id != province_id {
                    adjacency[province_id as usize].insert(neighbour_id);
                    adjacency[neighbour_id as usize].insert(province_id);
                }
                continue;
            }
            if terrain.is_ocean(nx, ny) {
                continue;
            }

            let ease = navigation_cost[neighbour].max(MIN_NAVIGATION_EASE);
            let step_cost = (1000.0 / ease) as u32;
            frontier.push((Reverse(cost + step_cost), neighbour as u32, province_id));
        }
    }

    let adjacency = adjacency
        .into_iter()
        .map(|neighbours| neighbours.into_iter().collect())
        .collect();
    (province_ids, adjacency)
}

/// Give land that no seed could reach (islets too small to be hit by seeding)
/// to the nearest province across the water. Breadth-first from all province
/// cells at once, through ocean as well as land.
fn attach_unreached_land(terrain: &dyn TerrainQuery, province_ids: &mut [u16], grid: Grid) {
    let (width, height) = (terrain.width(), terrain.height());
    let mut nearest: Vec<u16> = province_ids.to_vec();
    let mut frontier: VecDeque<usize> = (0..province_ids.len())
        .filter(|&cell| province_ids[cell] != 0)
        .collect();

    while let Some(cell) = frontier.pop_front() {
        let (x, y) = (cell % width, cell / width);
        for &(dx, dy) in &NEIGHBOURS_4 {
            let ny = y as i32 + dy;
            let Some(nx) = grid.step_x(x, dx, width) else {
                continue;
            };
            if ny < 0 || ny >= height as i32 {
                continue;
            }
            let neighbour = ny as usize * width + nx;
            if nearest[neighbour] == 0 {
                nearest[neighbour] = nearest[cell];
                frontier.push_back(neighbour);
            }
        }
    }

    for (cell, id) in province_ids.iter_mut().enumerate() {
        if *id == 0 && !terrain.is_ocean(cell % width, cell / width) {
            *id = nearest[cell];
        }
    }
}

/// Aggregate terrain and analysis values over each province's cells.
fn bake_province_attributes(
    terrain: &dyn TerrainQuery,
    analysis: &AnalysisGrids,
    province_ids: &[u16],
    seeds: &[(usize, usize)],
    grid: Grid,
) -> Vec<Province> {
    #[derive(Default, Clone)]
    struct Totals {
        area: u32,
        habitability: f64,
        elevation: f64,
        terrain_cost: f64,
        light_level: f64,
        resources: f64,
        biome_counts: Vec<(TileType, u32)>,
        is_coastal: bool,
        is_river_junction: bool,
    }

    let (width, height) = (terrain.width(), terrain.height());
    let coast_radius = ((COAST_SEARCH_RADIUS_WU * grid.cells_per_world_unit).ceil() as i32).max(1);
    let mut totals = vec![Totals::default(); seeds.len()];

    for (cell, &province_id) in province_ids.iter().enumerate() {
        if province_id == 0 {
            continue;
        }
        let province = &mut totals[(province_id - 1) as usize];
        let (x, y) = (cell % width, cell / width);

        province.area += 1;
        province.habitability += analysis.habitability[cell] as f64;
        province.elevation += terrain.heightmap_at(x, y);
        province.terrain_cost += (1.0 - analysis.navigation_cost[cell]) as f64;
        province.light_level += terrain.light_level_at(x, y);
        province.resources += analysis.resource_desirability[cell] as f64;

        let biome = terrain.biome_at(x, y);
        match province
            .biome_counts
            .iter_mut()
            .find(|(seen, _)| *seen == biome)
        {
            Some((_, count)) => *count += 1,
            None => province.biome_counts.push((biome, 1)),
        }

        if !province.is_coastal {
            province.is_coastal = (-coast_radius..=coast_radius).any(|dy| {
                (-coast_radius..=coast_radius).any(|dx| {
                    let ny = y as i32 + dy;
                    ny >= 0
                        && (ny as usize) < height
                        && grid
                            .step_x(x, dx, width)
                            .is_some_and(|nx| terrain.is_ocean(nx, ny as usize))
                })
            });
        }

        if terrain.river_at(x, y) >= MAJOR_RIVER_SIZE {
            province.is_river_junction = true;
        }
    }

    totals
        .into_iter()
        .zip(seeds)
        .enumerate()
        .map(|(index, (province, &site))| {
            let mean = |sum: f64| {
                if province.area > 0 {
                    (sum / province.area as f64) as f32
                } else {
                    0.0
                }
            };
            // First-seen biome wins ties, which keeps the result deterministic.
            let biome = province
                .biome_counts
                .iter()
                .rev()
                .max_by_key(|(_, count)| *count)
                .map(|&(biome, _)| biome)
                .unwrap_or_default();
            Province {
                id: (index + 1) as u16,
                site,
                biome,
                habitability: mean(province.habitability),
                area_cells: province.area,
                is_coastal: province.is_coastal,
                is_river_junction: province.is_river_junction,
                elevation_mean: mean(province.elevation),
                terrain_cost: mean(province.terrain_cost),
                light_level: mean(province.light_level),
                resources: mean(province.resources),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::compute_analysis_grids;
    use crate::test_support::MockTerrain;

    fn provinces_for(terrain: &MockTerrain, civ_seed: u32) -> ProvinceMap {
        let analysis = compute_analysis_grids(terrain, Grid::flat(1.0));
        generate_provinces(terrain, &analysis, Grid::flat(1.0), civ_seed)
    }

    #[test]
    fn every_land_cell_belongs_to_a_province_and_ocean_to_none() {
        let terrain = MockTerrain::with_ocean_column(64, 48, 30);
        let map = provinces_for(&terrain, 7);

        for y in 0..48 {
            for x in 0..64 {
                let id = map.province_ids[y * 64 + x];
                if x == 30 {
                    assert_eq!(id, 0, "ocean cell ({x}, {y}) has a province");
                } else {
                    assert!(id != 0, "land cell ({x}, {y}) has no province");
                    assert!(id as usize <= map.provinces.len());
                }
            }
        }
    }

    #[test]
    fn province_areas_add_up_to_the_land_area() {
        let terrain = MockTerrain::with_ocean_column(64, 48, 30);
        let map = provinces_for(&terrain, 7);

        let total: u32 = map
            .provinces
            .iter()
            .map(|province| province.area_cells)
            .sum();
        assert_eq!(total, 63 * 48);
    }

    #[test]
    fn provinces_do_not_cross_ocean() {
        let terrain = MockTerrain::with_ocean_column(64, 48, 30);
        let map = provinces_for(&terrain, 7);

        let west: BTreeSet<u16> = (0..48)
            .flat_map(|y| (0..30).map(move |x| (x, y)))
            .map(|(x, y)| map.province_ids[y * 64 + x])
            .collect();
        let east: BTreeSet<u16> = (0..48)
            .flat_map(|y| (31..64).map(move |x| (x, y)))
            .map(|(x, y)| map.province_ids[y * 64 + x])
            .collect();
        assert!(west.is_disjoint(&east));
    }

    #[test]
    fn provinces_beside_ocean_are_coastal() {
        let terrain = MockTerrain::with_ocean_column(64, 48, 30);
        let map = provinces_for(&terrain, 7);

        let beside_ocean = map.province_ids[10 * 64 + 29];
        assert!(map.provinces[(beside_ocean - 1) as usize].is_coastal);
    }

    #[test]
    fn same_seed_gives_the_same_provinces_and_a_different_seed_does_not() {
        let terrain = MockTerrain::flat(64, 48);

        let first = provinces_for(&terrain, 7);
        let again = provinces_for(&terrain, 7);
        let other = provinces_for(&terrain, 8);

        assert_eq!(first.province_ids, again.province_ids);
        assert_eq!(first.provinces, again.provinces);
        assert_eq!(first.adjacency, again.adjacency);
        assert_ne!(first.province_ids, other.province_ids);
    }

    #[test]
    fn land_no_seed_reached_joins_the_nearest_province_across_water() {
        // land | ocean | land, with only the first cell in a province
        let terrain = MockTerrain::with_ocean_column(3, 1, 1);
        let mut province_ids = vec![1, 0, 0];

        attach_unreached_land(&terrain, &mut province_ids, Grid::flat(1.0));

        assert_eq!(province_ids, vec![1, 0, 1]);
    }

    #[test]
    fn adjacency_is_symmetric() {
        let terrain = MockTerrain::flat(64, 48);
        let map = provinces_for(&terrain, 7);

        for (id, neighbours) in map.adjacency.iter().enumerate() {
            for &neighbour in neighbours {
                assert!(map.adjacency[neighbour as usize].contains(&(id as u16)));
            }
        }
    }
}
