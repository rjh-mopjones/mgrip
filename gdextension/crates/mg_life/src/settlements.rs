//! LifeGen stage 4: settlements.
//!
//! Based on Randlebrot's `rb_world::lifegen::settlements`. Every province
//! gets a few settlements on its most habitable cells: one main settlement
//! sized by the province's habitability, and smaller ones around it.
//! Habitability decides how big they are, not whether they exist: even
//! desolate provinces hold outposts. Nothing here is random.

use crate::analysis::AnalysisGrids;
use crate::factions::{FactionMap, PoliticalState};
use crate::grid::Grid;
use crate::provinces::{Province, ProvinceMap};

/// Settlements in the same province are at least this far apart
/// (60 cells at Randlebrot's 8 per world unit).
const SETTLEMENT_MIN_SPACING_WU: f64 = 7.5;
/// Provinces smaller than this (in square world units) hold one settlement.
const TINY_PROVINCE_AREA_WU2: f64 = 3.125;
// Province habitability needed for its main settlement to reach each size.
// Calibrated on seed 42 (`inspect layer-stats`): province mean habitability
// there has median 0.53, upper quartile 0.74 and 90th percentile 0.78.
const CITY_MIN_HABITABILITY: f32 = 0.78;
/// A province on a major river needs less to hold a city.
const RIVER_CITY_MIN_HABITABILITY: f32 = 0.70;
const TOWN_MIN_HABITABILITY: f32 = 0.55;
const VILLAGE_MIN_HABITABILITY: f32 = 0.30;
/// Cells at or below this habitability never hold a settlement.
const MIN_SITE_HABITABILITY: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeClass {
    /// A faction's capital.
    Metropolis,
    City,
    Town,
    Village,
    Outpost,
    Ruins,
}

impl SizeClass {
    pub const ALL: [Self; 6] = [
        Self::Metropolis,
        Self::City,
        Self::Town,
        Self::Village,
        Self::Outpost,
        Self::Ruins,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    /// 1-based.
    pub id: u32,
    /// Cell the settlement stands on.
    pub position: (usize, usize),
    pub province_id: u16,
    pub size_class: SizeClass,
}

/// Place settlements in every province. Deterministic; uses no seed.
pub fn place_settlements(
    province_map: &ProvinceMap,
    faction_map: &FactionMap,
    analysis: &AnalysisGrids,
    grid: Grid,
) -> Vec<Settlement> {
    let width = province_map.width;
    let min_spacing_cells = SETTLEMENT_MIN_SPACING_WU * grid.cells_per_world_unit;

    // Candidate cells per province, best habitability first. The sort is
    // stable, so equally habitable cells keep scan order.
    let mut candidates: Vec<Vec<(f32, usize)>> = vec![Vec::new(); province_map.provinces.len()];
    for (cell, &province_id) in province_map.province_ids.iter().enumerate() {
        let habitability = analysis.habitability[cell];
        if province_id != 0 && habitability > MIN_SITE_HABITABILITY {
            candidates[(province_id - 1) as usize].push((habitability, cell));
        }
    }
    for cells in &mut candidates {
        cells.sort_by(|a, b| b.0.total_cmp(&a.0));
    }

    let mut settlements = Vec::new();
    for province in &province_map.provinces {
        let state = faction_map.political_states[(province.id - 1) as usize];
        let is_capital_province = faction_map
            .factions
            .iter()
            .any(|faction| faction.capital_province == province.id);
        let sites = best_sites(
            &candidates[(province.id - 1) as usize],
            settlement_count(province, grid),
            width,
            min_spacing_cells,
            grid,
        );

        for (slot, position) in sites.into_iter().enumerate() {
            let main_size = main_settlement_size(province, is_capital_province);
            let size_class = if slot == 0 {
                main_size
            } else {
                secondary_settlement_size(main_size)
            };
            let size_class = match state {
                PoliticalState::Claimed { .. } => size_class,
                PoliticalState::Unclaimed | PoliticalState::Uninhabited => {
                    without_faction(size_class)
                }
            };
            settlements.push(Settlement {
                id: settlements.len() as u32 + 1,
                position,
                province_id: province.id,
                size_class,
            });
        }
    }
    settlements
}

/// How many settlements a province gets: three, more if it is very habitable,
/// one if it is tiny.
fn settlement_count(province: &Province, grid: Grid) -> usize {
    let area_wu2 = province.area_cells as f64 / (grid.cells_per_world_unit * grid.cells_per_world_unit);
    if area_wu2 < TINY_PROVINCE_AREA_WU2 {
        1
    } else if province.habitability > 0.7 {
        5
    } else if province.habitability > 0.5 {
        4
    } else {
        3
    }
}

/// The `count` most habitable cells that keep the minimum spacing from each
/// other. `candidates` is `(habitability, cell)` sorted best first. Fewer are
/// returned if the province has no room for more.
fn best_sites(
    candidates: &[(f32, usize)],
    count: usize,
    width: usize,
    min_spacing_cells: f64,
    grid: Grid,
) -> Vec<(usize, usize)> {
    let mut sites: Vec<(usize, usize)> = Vec::new();
    for &(_, cell) in candidates {
        if sites.len() >= count {
            break;
        }
        let (x, y) = (cell % width, cell / width);
        let too_close = sites.iter().any(|&(sx, sy)| {
            let (dx, dy) = (grid.dx(sx, x), y as f64 - sy as f64);
            dx * dx + dy * dy < min_spacing_cells * min_spacing_cells
        });
        if !too_close {
            sites.push((x, y));
        }
    }
    sites
}

/// Size of a province's main settlement, the one on its best site. A faction's
/// capital is always a metropolis. Otherwise it follows the province's mean
/// habitability, which on this world runs from about 0.15 to 0.8: the top
/// tenth of provinces hold a city, the upper half a town.
fn main_settlement_size(province: &Province, is_capital: bool) -> SizeClass {
    let habitability = province.habitability;
    if is_capital {
        SizeClass::Metropolis
    } else if habitability >= CITY_MIN_HABITABILITY
        || (habitability >= RIVER_CITY_MIN_HABITABILITY && province.is_river_junction)
    {
        SizeClass::City
    } else if habitability >= TOWN_MIN_HABITABILITY {
        SizeClass::Town
    } else if habitability >= VILLAGE_MIN_HABITABILITY {
        SizeClass::Village
    } else {
        SizeClass::Outpost
    }
}

/// Size of a province's other settlements: villages around a town or
/// larger, outposts otherwise.
fn secondary_settlement_size(main_size: SizeClass) -> SizeClass {
    match main_size {
        SizeClass::Metropolis | SizeClass::City | SizeClass::Town => SizeClass::Village,
        SizeClass::Village | SizeClass::Outpost | SizeClass::Ruins => SizeClass::Outpost,
    }
}

/// Land no faction holds has nothing larger than a village.
fn without_faction(size_class: SizeClass) -> SizeClass {
    match size_class {
        SizeClass::Metropolis | SizeClass::City | SizeClass::Town => SizeClass::Village,
        SizeClass::Village => SizeClass::Outpost,
        smaller => smaller,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::compute_analysis_grids;
    use crate::factions::{generate_factions, Faction};
    use crate::provinces::generate_provinces;
    use crate::test_support::MockTerrain;
    use mg_core::TileType;

    fn province(habitability: f32, area_cells: u32) -> Province {
        Province {
            id: 1,
            site: (0, 0),
            biome: TileType::Plains,
            habitability,
            area_cells,
            is_coastal: false,
            is_river_junction: false,
            elevation_mean: 0.1,
            terrain_cost: 0.5,
            light_level: 0.4,
            resources: 0.3,
        }
    }

    #[test]
    fn more_habitable_provinces_get_more_settlements() {
        assert_eq!(settlement_count(&province(0.8, 500), Grid::flat(1.0)), 5);
        assert_eq!(settlement_count(&province(0.6, 500), Grid::flat(1.0)), 4);
        assert_eq!(settlement_count(&province(0.2, 500), Grid::flat(1.0)), 3);
        assert_eq!(settlement_count(&province(0.01, 500), Grid::flat(1.0)), 3);
    }

    #[test]
    fn a_tiny_province_gets_one_settlement() {
        assert_eq!(settlement_count(&province(0.8, 3), Grid::flat(1.0)), 1);
        // The same ground on a grid of 8 cells per world unit is 192 cells.
        assert_eq!(settlement_count(&province(0.8, 192), Grid::flat(8.0)), 1);
    }

    #[test]
    fn a_province_main_settlement_follows_habitability_and_capitals_are_metropolises() {
        let main = |habitability: f32, is_capital: bool| {
            main_settlement_size(&province(habitability, 500), is_capital)
        };
        assert_eq!(main(0.5, true), SizeClass::Metropolis);
        assert_eq!(main(0.8, false), SizeClass::City);
        assert_eq!(main(0.72, false), SizeClass::Town);
        assert_eq!(main(0.4, false), SizeClass::Village);
        assert_eq!(main(0.2, false), SizeClass::Outpost);

        let mut on_major_river = province(0.72, 500);
        on_major_river.is_river_junction = true;
        assert_eq!(main_settlement_size(&on_major_river, false), SizeClass::City);
    }

    #[test]
    fn other_settlements_are_smaller_than_the_main_one() {
        assert_eq!(secondary_settlement_size(SizeClass::Metropolis), SizeClass::Village);
        assert_eq!(secondary_settlement_size(SizeClass::Town), SizeClass::Village);
        assert_eq!(secondary_settlement_size(SizeClass::Village), SizeClass::Outpost);
        assert_eq!(secondary_settlement_size(SizeClass::Outpost), SizeClass::Outpost);
    }

    #[test]
    fn land_without_a_faction_has_nothing_larger_than_a_village() {
        assert_eq!(without_faction(SizeClass::City), SizeClass::Village);
        assert_eq!(without_faction(SizeClass::Town), SizeClass::Village);
        assert_eq!(without_faction(SizeClass::Village), SizeClass::Outpost);
        assert_eq!(without_faction(SizeClass::Ruins), SizeClass::Ruins);
    }

    #[test]
    fn sites_take_the_best_cells_that_keep_their_spacing() {
        // Cells along one row of a 200-wide map: x = 0, 10 and 100.
        let candidates = [(0.9, 0), (0.85, 10), (0.8, 100)];

        assert_eq!(
            best_sites(&candidates, 3, 200, 60.0, Grid::flat(1.0)),
            vec![(0, 0), (100, 0)]
        );
    }

    #[test]
    fn settlements_stand_inside_their_own_province_and_each_faction_has_one_metropolis() {
        let terrain = MockTerrain::flat(96, 64);
        let analysis = compute_analysis_grids(&terrain, Grid::flat(1.0));
        let provinces = generate_provinces(&terrain, &analysis, Grid::flat(1.0), 7);
        let factions = generate_factions(&provinces, &[], Grid::flat(1.0), 7);

        let settlements = place_settlements(&provinces, &factions, &analysis, Grid::flat(1.0));

        assert!(!settlements.is_empty());
        for settlement in &settlements {
            let (x, y) = settlement.position;
            assert_eq!(provinces.province_ids[y * 96 + x], settlement.province_id);
        }
        for Faction {
            capital_province, ..
        } in &factions.factions
        {
            let metropolises = settlements
                .iter()
                .filter(|s| {
                    s.province_id == *capital_province && s.size_class == SizeClass::Metropolis
                })
                .count();
            assert_eq!(metropolises, 1);
        }
    }
}
