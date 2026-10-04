//! LifeGen stage 3: factions.
//!
//! Ported from Randlebrot's `rb_world::lifegen::factions`. Capitals are placed
//! in the best provinces, then every faction grows outward over the province
//! adjacency graph until it runs out of budget or room. Provinces no faction
//! takes stay unclaimed; provinces too barren to hold stay uninhabited.

use crate::provinces::{Province, ProvinceMap};
use rand::{Rng, SeedableRng};
use rand_xorshift::XorShiftRng;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// A province must be at least this habitable to host a capital.
const CAPITAL_MIN_HABITABILITY: f32 = 0.35;
/// A province must be at least this habitable to be claimed at all.
const CLAIMABLE_MIN_HABITABILITY: f32 = 0.1;
/// Provinces above this habitability count towards the number of factions.
const COUNTED_MIN_HABITABILITY: f32 = 0.15;
const CAPITAL_RIVER_BONUS: f32 = 0.2;
const CAPITAL_COAST_BONUS: f32 = 0.1;
/// Capitals are at least this far apart (150 cells at Randlebrot's 8 per world unit).
const CAPITAL_MIN_SPACING_WU: f64 = 18.75;
const BASE_FACTION_COUNT: usize = 50;
/// One extra faction per this many habitable provinces...
const PROVINCES_PER_EXTRA_FACTION: usize = 80;
/// ...up to this many extra.
const MAX_EXTRA_FACTIONS: usize = 30;
const MAX_FACTION_PROVINCES: usize = 25;
/// Growth cost of taking a province: its terrain cost times this...
const TERRAIN_COST_WEIGHT: f64 = 5.0;
/// ...plus the distance between province sites times this.
const DISTANCE_COST_PER_WU: f64 = 0.4;
/// Each stage derives its own random stream from `civ_seed`.
const FACTION_SEED_OFFSET: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoliticalState {
    Claimed {
        faction_id: u16,
    },
    /// Habitable, but no faction holds it.
    Unclaimed,
    /// Too barren for any faction to hold.
    Uninhabited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Faction {
    /// 1-based.
    pub id: u16,
    pub capital_province: u16,
    pub province_count: u32,
    pub area_cells: u32,
}

/// Stage 3 output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactionMap {
    /// Indexed by `province id - 1`.
    pub political_states: Vec<PoliticalState>,
    /// Indexed by `faction id - 1`.
    pub factions: Vec<Faction>,
}

impl FactionMap {
    /// Faction holding a province, or 0 if none does (also for province id 0).
    pub fn faction_of_province(&self, province_id: u16) -> u16 {
        match province_id
            .checked_sub(1)
            .and_then(|index| self.political_states.get(index as usize))
        {
            Some(PoliticalState::Claimed { faction_id }) => *faction_id,
            _ => 0,
        }
    }
}

/// Place capitals and grow factions over the province graph. Deterministic for
/// a given province map and `civ_seed`.
pub fn generate_factions(
    province_map: &ProvinceMap,
    cells_per_world_unit: f64,
    civ_seed: u32,
) -> FactionMap {
    let provinces = &province_map.provinces;
    let capitals = place_capitals(provinces, cells_per_world_unit);
    let mut rng = XorShiftRng::seed_from_u64(civ_seed.wrapping_add(FACTION_SEED_OFFSET) as u64);
    let budgets: Vec<usize> = capitals
        .iter()
        .map(|&capital| province_budget(&provinces[(capital - 1) as usize], &mut rng))
        .collect();

    let mut owner = grow_factions(
        provinces,
        &province_map.adjacency,
        &capitals,
        &budgets,
        cells_per_world_unit,
    );
    absorb_enclosed_provinces(provinces, &province_map.adjacency, &mut owner);

    let political_states: Vec<PoliticalState> = provinces
        .iter()
        .map(|province| match owner[province.id as usize] {
            0 if province.habitability >= CLAIMABLE_MIN_HABITABILITY => PoliticalState::Unclaimed,
            0 => PoliticalState::Uninhabited,
            faction_id => PoliticalState::Claimed { faction_id },
        })
        .collect();

    let mut factions: Vec<Faction> = capitals
        .iter()
        .enumerate()
        .map(|(index, &capital_province)| Faction {
            id: (index + 1) as u16,
            capital_province,
            province_count: 0,
            area_cells: 0,
        })
        .collect();
    for province in provinces {
        let faction_id = owner[province.id as usize];
        if faction_id != 0 {
            let faction = &mut factions[(faction_id - 1) as usize];
            faction.province_count += 1;
            faction.area_cells += province.area_cells;
        }
    }

    FactionMap {
        political_states,
        factions,
    }
}

fn site_distance_wu(a: &Province, b: &Province, cells_per_world_unit: f64) -> f64 {
    let dx = a.site.0 as f64 - b.site.0 as f64;
    let dy = a.site.1 as f64 - b.site.1 as f64;
    (dx * dx + dy * dy).sqrt() / cells_per_world_unit
}

/// Choose capital provinces: best-scoring first, each at least the minimum
/// spacing from every capital already chosen. Returns province ids; the
/// faction id is the position in the list plus one.
fn place_capitals(provinces: &[Province], cells_per_world_unit: f64) -> Vec<u16> {
    let habitable_count = provinces
        .iter()
        .filter(|province| province.habitability > COUNTED_MIN_HABITABILITY)
        .count();
    let target = BASE_FACTION_COUNT
        + (habitable_count / PROVINCES_PER_EXTRA_FACTION).min(MAX_EXTRA_FACTIONS);

    let score = |province: &Province| {
        province.habitability
            + if province.is_river_junction {
                CAPITAL_RIVER_BONUS
            } else {
                0.0
            }
            + if province.is_coastal {
                CAPITAL_COAST_BONUS
            } else {
                0.0
            }
    };
    let mut candidates: Vec<&Province> = provinces
        .iter()
        .filter(|province| province.habitability > CAPITAL_MIN_HABITABILITY)
        .collect();
    candidates.sort_by(|a, b| score(b).total_cmp(&score(a)).then(a.id.cmp(&b.id)));

    let mut capitals: Vec<u16> = Vec::new();
    let select = |min_spacing_wu: f64, capitals: &mut Vec<u16>| {
        for candidate in &candidates {
            if capitals.len() >= target {
                break;
            }
            let too_close = capitals.iter().any(|&capital| {
                let capital = &provinces[(capital - 1) as usize];
                site_distance_wu(candidate, capital, cells_per_world_unit) < min_spacing_wu
            });
            if !too_close {
                capitals.push(candidate.id);
            }
        }
    };
    select(CAPITAL_MIN_SPACING_WU, &mut capitals);
    // Too few capitals fit at full spacing: try again at half spacing.
    if capitals.len() < BASE_FACTION_COUNT {
        select(CAPITAL_MIN_SPACING_WU / 2.0, &mut capitals);
    }
    capitals
}

/// How many provinces a faction may hold. Better capitals support larger
/// factions, within `MAX_FACTION_PROVINCES`.
fn province_budget(capital: &Province, rng: &mut XorShiftRng) -> usize {
    let mut score = capital.habitability;
    if capital.is_coastal {
        score += 0.1;
    }
    if capital.is_river_junction {
        score += 0.1;
    }
    let (low, high) = if score > 0.75 {
        (15, MAX_FACTION_PROVINCES)
    } else if score > 0.55 {
        (10, 18)
    } else {
        (5, 12)
    };
    rng.gen_range(low..=high)
}

/// Grow all factions at once from their capitals (Dijkstra over the province
/// graph). The faction that reaches a province most cheaply takes it, until
/// its budget is spent. Returns the owning faction per province id (0 = none).
fn grow_factions(
    provinces: &[Province],
    adjacency: &[Vec<u16>],
    capitals: &[u16],
    budgets: &[usize],
    cells_per_world_unit: f64,
) -> Vec<u16> {
    let mut owner = vec![0u16; provinces.len() + 1];
    let mut held = vec![0usize; capitals.len() + 1];
    let province = |id: u16| &provinces[(id - 1) as usize];

    // (cost so far, province id, faction id); Reverse makes the heap a min-heap.
    let mut frontier: BinaryHeap<(Reverse<u32>, u16, u16)> = BinaryHeap::new();
    let push_neighbours = |frontier: &mut BinaryHeap<(Reverse<u32>, u16, u16)>,
                               owner: &[u16],
                               from: u16,
                               cost: u32,
                               faction_id: u16| {
        for &neighbour in &adjacency[from as usize] {
            let target = province(neighbour);
            if owner[neighbour as usize] != 0 || target.habitability < CLAIMABLE_MIN_HABITABILITY {
                continue;
            }
            let step = target.terrain_cost as f64 * TERRAIN_COST_WEIGHT
                + site_distance_wu(province(from), target, cells_per_world_unit)
                    * DISTANCE_COST_PER_WU;
            frontier.push((
                Reverse(cost + (step * 1000.0) as u32),
                neighbour,
                faction_id,
            ));
        }
    };

    for (index, &capital) in capitals.iter().enumerate() {
        let faction_id = (index + 1) as u16;
        owner[capital as usize] = faction_id;
        held[faction_id as usize] = 1;
    }
    for (index, &capital) in capitals.iter().enumerate() {
        push_neighbours(&mut frontier, &owner, capital, 0, (index + 1) as u16);
    }

    while let Some((Reverse(cost), province_id, faction_id)) = frontier.pop() {
        let budget = budgets[(faction_id - 1) as usize];
        if owner[province_id as usize] != 0 || held[faction_id as usize] >= budget {
            continue;
        }
        owner[province_id as usize] = faction_id;
        held[faction_id as usize] += 1;
        if held[faction_id as usize] < budget {
            push_neighbours(&mut frontier, &owner, province_id, cost, faction_id);
        }
    }

    owner
}

/// Give leftover claimable provinces to a neighbouring faction that still has
/// room, smallest faction first, repeating until nothing changes. This closes
/// holes and gaps between factions.
fn absorb_enclosed_provinces(provinces: &[Province], adjacency: &[Vec<u16>], owner: &mut [u16]) {
    let faction_count = owner.iter().copied().max().unwrap_or(0) as usize;
    let mut sizes = vec![0usize; faction_count + 1];
    for &faction_id in owner.iter().filter(|&&faction_id| faction_id != 0) {
        sizes[faction_id as usize] += 1;
    }

    loop {
        let mut changed = false;
        for province in provinces {
            if owner[province.id as usize] != 0
                || province.habitability < CLAIMABLE_MIN_HABITABILITY
            {
                continue;
            }
            let smallest_neighbour = adjacency[province.id as usize]
                .iter()
                .map(|&neighbour| owner[neighbour as usize])
                .filter(|&faction_id| {
                    faction_id != 0 && sizes[faction_id as usize] < MAX_FACTION_PROVINCES
                })
                .min_by_key(|&faction_id| (sizes[faction_id as usize], faction_id));
            if let Some(faction_id) = smallest_neighbour {
                owner[province.id as usize] = faction_id;
                sizes[faction_id as usize] += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mg_core::TileType;

    fn province(id: u16, habitability: f32, site: (usize, usize)) -> Province {
        Province {
            id,
            site,
            biome: TileType::Plains,
            habitability,
            area_cells: 100,
            is_coastal: false,
            is_river_junction: false,
            elevation_mean: 0.1,
            terrain_cost: 0.5,
        }
    }

    /// Provinces in a row, each adjacent to the next.
    fn chain(habitabilities: &[f32], spacing: usize) -> ProvinceMap {
        let provinces: Vec<Province> = habitabilities
            .iter()
            .enumerate()
            .map(|(index, &habitability)| {
                province((index + 1) as u16, habitability, (index * spacing, 0))
            })
            .collect();
        let count = provinces.len() as u16;
        let adjacency = (0..=count)
            .map(|id| match id {
                0 => vec![],
                id => [id.wrapping_sub(1), id + 1]
                    .into_iter()
                    .filter(|&neighbour| neighbour >= 1 && neighbour <= count)
                    .collect(),
            })
            .collect();
        ProvinceMap {
            width: 0,
            height: 0,
            province_ids: vec![],
            provinces,
            adjacency,
        }
    }

    #[test]
    fn capitals_keep_their_minimum_spacing() {
        // Provinces 5 world units apart: capitals must be 18.75 apart, so at
        // most every fourth province can be one.
        let map = chain(&[0.9; 40], 5);
        let capitals = place_capitals(&map.provinces, 1.0);

        assert!(capitals.len() > 1);
        for (index, &a) in capitals.iter().enumerate() {
            for &b in &capitals[index + 1..] {
                let distance = site_distance_wu(
                    &map.provinces[(a - 1) as usize],
                    &map.provinces[(b - 1) as usize],
                    1.0,
                );
                // The fallback pass may halve the spacing, never less.
                assert!(distance >= CAPITAL_MIN_SPACING_WU / 2.0);
            }
        }
    }

    #[test]
    fn barren_provinces_get_no_capital() {
        let map = chain(&[0.2, 0.3], 100);

        assert!(place_capitals(&map.provinces, 1.0).is_empty());
    }

    #[test]
    fn a_faction_claims_what_it_can_reach_and_stops_at_barren_land() {
        // capital - habitable - barren - habitable
        // 5 world units apart: too close for a second capital.
        let map = chain(&[0.8, 0.6, 0.05, 0.3], 5);
        let factions = generate_factions(&map, 1.0, 7);

        assert_eq!(factions.factions.len(), 1);
        assert_eq!(
            factions.political_states[0],
            PoliticalState::Claimed { faction_id: 1 }
        );
        assert_eq!(
            factions.political_states[1],
            PoliticalState::Claimed { faction_id: 1 }
        );
        assert_eq!(factions.political_states[2], PoliticalState::Uninhabited);
        assert_eq!(factions.political_states[3], PoliticalState::Unclaimed);
        assert_eq!(factions.factions[0].province_count, 2);
        assert_eq!(factions.factions[0].area_cells, 200);
    }

    #[test]
    fn no_faction_exceeds_the_province_limit() {
        let map = chain(&[0.9; 200], 5);
        let factions = generate_factions(&map, 1.0, 7);

        for faction in &factions.factions {
            assert!(faction.province_count >= 1);
            assert!(faction.province_count as usize <= MAX_FACTION_PROVINCES);
        }
    }

    #[test]
    fn same_seed_gives_the_same_factions() {
        let map = chain(&[0.9; 200], 5);

        assert_eq!(
            generate_factions(&map, 1.0, 7),
            generate_factions(&map, 1.0, 7)
        );
    }

    #[test]
    fn faction_of_province_is_zero_for_unheld_and_unknown_provinces() {
        let map = chain(&[0.8, 0.05], 5);
        let factions = generate_factions(&map, 1.0, 7);

        assert_eq!(factions.faction_of_province(1), 1);
        assert_eq!(factions.faction_of_province(2), 0);
        assert_eq!(factions.faction_of_province(0), 0);
        assert_eq!(factions.faction_of_province(99), 0);
    }
}
