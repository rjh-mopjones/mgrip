//! LifeGen stage 3: factions.
//!
//! Based on Randlebrot's `rb_world::lifegen::factions`. Factions here are the
//! territorial states of the lore: they hold provinces. Authored states (the
//! named ones) pick their capital first, each in the province that best fits
//! its description. Generated minor states then fill the remaining good land.
//! Every faction grows outward over the province adjacency graph until it runs
//! out of budget or room. Provinces no faction takes stay unclaimed.

use crate::grid::Grid;
use crate::provinces::{Province, ProvinceMap};
use rand::{Rng, SeedableRng};
use rand_xorshift::XorShiftRng;
use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::ops::RangeInclusive;

/// A province must be at least this habitable to host a generated capital.
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
/// ...plus the distance between province sites times this...
const DISTANCE_COST_PER_WU: f64 = 0.4;
/// ...plus, for an authored state, how far the province's light level lies
/// outside the state's band times this. A tenth of the light range outside
/// costs about as much as an ordinary step.
const LIGHT_BAND_COST: f64 = 100.0;
// How much each preference adds to an authored state's capital score.
const PREFERENCE_COAST_BONUS: f32 = 0.3;
const PREFERENCE_RIVER_BONUS: f32 = 0.3;
const PREFERENCE_MOUNTAIN_WEIGHT: f32 = 1.0;
const PREFERENCE_RESOURCE_WEIGHT: f32 = 0.5;
const PREFERENCE_FERTILE_WEIGHT: f32 = 0.5;
/// Each stage derives its own random stream from `civ_seed`.
const FACTION_SEED_OFFSET: u32 = 3;

/// A named state from the lore, described well enough to place it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthoredState {
    pub name: String,
    /// Name of the capital city, where the lore gives one.
    pub capital: Option<String>,
    /// Light-level band (0.0 deep night, 1.0 sub-stellar) the state lives in.
    /// Its capital is placed inside the band and it grows along it.
    pub light: (f32, f32),
    pub size: StateSize,
    /// What the capital province should have, beyond being habitable.
    #[serde(default)]
    pub prefers: Vec<Preference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum StateSize {
    CityState,
    Small,
    Medium,
    Large,
}

impl StateSize {
    /// How many provinces a state of this size may hold.
    fn province_budget(self) -> RangeInclusive<usize> {
        match self {
            Self::CityState => 1..=3,
            Self::Small => 5..=9,
            Self::Medium => 11..=17,
            Self::Large => 20..=MAX_FACTION_PROVINCES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Preference {
    Coastal,
    MajorRiver,
    Mountains,
    Resources,
    Fertile,
}

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
    /// 1-based. Authored states come first, in the order they were given.
    pub id: u16,
    /// `None` for a generated minor state.
    pub name: Option<String>,
    pub capital_name: Option<String>,
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
    /// Names of authored states that found no province in their light band.
    pub unplaced_states: Vec<String>,
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

/// A faction before it has grown: where it starts and how it may grow.
struct Founding<'a> {
    capital_province: u16,
    authored: Option<&'a AuthoredState>,
    /// Provinces it may take while growing.
    budget: usize,
    /// Provinces it may hold after absorbing enclosed leftovers.
    limit: usize,
}

/// Place capitals and grow factions over the province graph. Deterministic for
/// a given province map, list of authored states and `civ_seed`.
pub fn generate_factions(
    province_map: &ProvinceMap,
    authored_states: &[AuthoredState],
    grid: Grid,
    civ_seed: u32,
) -> FactionMap {
    let provinces = &province_map.provinces;
    let mut rng = XorShiftRng::seed_from_u64(civ_seed.wrapping_add(FACTION_SEED_OFFSET) as u64);

    let mut foundings: Vec<Founding> = Vec::new();
    let mut unplaced_states = Vec::new();
    for state in authored_states {
        let taken: Vec<u16> = foundings.iter().map(|f| f.capital_province).collect();
        match authored_capital(provinces, state, &taken, grid) {
            Some(capital_province) => {
                let budget = rng.gen_range(state.size.province_budget());
                foundings.push(Founding {
                    capital_province,
                    authored: Some(state),
                    budget,
                    limit: budget,
                });
            }
            None => unplaced_states.push(state.name.clone()),
        }
    }
    let taken: Vec<u16> = foundings.iter().map(|f| f.capital_province).collect();
    for capital_province in generated_capitals(provinces, &taken, grid) {
        foundings.push(Founding {
            capital_province,
            authored: None,
            budget: generated_budget(&provinces[(capital_province - 1) as usize], &mut rng),
            limit: MAX_FACTION_PROVINCES,
        });
    }

    let mut owner = grow_factions(
        provinces,
        &province_map.adjacency,
        &foundings,
        grid,
    );
    absorb_enclosed_provinces(provinces, &province_map.adjacency, &foundings, &mut owner);

    let political_states: Vec<PoliticalState> = provinces
        .iter()
        .map(|province| match owner[province.id as usize] {
            0 if province.habitability >= CLAIMABLE_MIN_HABITABILITY => PoliticalState::Unclaimed,
            0 => PoliticalState::Uninhabited,
            faction_id => PoliticalState::Claimed { faction_id },
        })
        .collect();

    let mut factions: Vec<Faction> = foundings
        .iter()
        .enumerate()
        .map(|(index, founding)| Faction {
            id: (index + 1) as u16,
            name: founding.authored.map(|state| state.name.clone()),
            capital_name: founding.authored.and_then(|state| state.capital.clone()),
            capital_province: founding.capital_province,
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
        unplaced_states,
    }
}

fn site_distance_wu(a: &Province, b: &Province, grid: Grid) -> f64 {
    grid.distance(a.site, b.site) / grid.cells_per_world_unit
}

/// How far a light level lies outside a band; 0.0 inside it.
fn outside_band(light_level: f32, band: (f32, f32)) -> f32 {
    (band.0 - light_level).max(light_level - band.1).max(0.0)
}

/// True if `candidate` is at least `min_spacing_wu` from every capital in `taken`.
fn keeps_spacing(
    provinces: &[Province],
    candidate: &Province,
    taken: &[u16],
    min_spacing_wu: f64,
    grid: Grid,
) -> bool {
    taken.iter().all(|&capital| {
        let capital = &provinces[(capital - 1) as usize];
        site_distance_wu(candidate, capital, grid) >= min_spacing_wu
    })
}

/// How well a province suits an authored state's capital.
fn authored_score(province: &Province, state: &AuthoredState) -> f32 {
    let preference_score: f32 = state
        .prefers
        .iter()
        .map(|preference| match preference {
            Preference::Coastal if province.is_coastal => PREFERENCE_COAST_BONUS,
            Preference::MajorRiver if province.is_river_junction => PREFERENCE_RIVER_BONUS,
            Preference::Coastal | Preference::MajorRiver => 0.0,
            Preference::Mountains => province.elevation_mean * PREFERENCE_MOUNTAIN_WEIGHT,
            Preference::Resources => province.resources * PREFERENCE_RESOURCE_WEIGHT,
            Preference::Fertile => province.habitability * PREFERENCE_FERTILE_WEIGHT,
        })
        .sum();
    province.habitability + preference_score
}

/// The capital province for an authored state: the best-scoring province in
/// its light band that is not already a capital. Full spacing from other
/// capitals is tried first, then half, then none, so a state described by the
/// lore is only left out if its band holds no free province at all.
fn authored_capital(
    provinces: &[Province],
    state: &AuthoredState,
    taken: &[u16],
    grid: Grid,
) -> Option<u16> {
    let mut candidates: Vec<&Province> = provinces
        .iter()
        .filter(|province| {
            outside_band(province.light_level, state.light) == 0.0 && !taken.contains(&province.id)
        })
        .collect();
    candidates.sort_by(|a, b| {
        authored_score(b, state)
            .total_cmp(&authored_score(a, state))
            .then(a.id.cmp(&b.id))
    });
    [CAPITAL_MIN_SPACING_WU, CAPITAL_MIN_SPACING_WU / 2.0, 0.0]
        .into_iter()
        .find_map(|spacing| {
            candidates
                .iter()
                .find(|candidate| {
                    keeps_spacing(provinces, candidate, taken, spacing, grid)
                })
                .map(|candidate| candidate.id)
        })
}

/// Capitals for generated minor states: best-scoring provinces first, each
/// keeping the minimum spacing from every capital already chosen (including
/// `taken`). Fills up to the target number of factions.
fn generated_capitals(
    provinces: &[Province],
    taken: &[u16],
    grid: Grid,
) -> Vec<u16> {
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
        .filter(|province| {
            province.habitability > CAPITAL_MIN_HABITABILITY && !taken.contains(&province.id)
        })
        .collect();
    candidates.sort_by(|a, b| score(b).total_cmp(&score(a)).then(a.id.cmp(&b.id)));

    let mut all_capitals: Vec<u16> = taken.to_vec();
    let select = |min_spacing_wu: f64, all_capitals: &mut Vec<u16>| {
        for candidate in &candidates {
            if all_capitals.len() >= target {
                break;
            }
            if !all_capitals.contains(&candidate.id)
                && keeps_spacing(
                    provinces,
                    candidate,
                    all_capitals,
                    min_spacing_wu,
                    grid,
                )
            {
                all_capitals.push(candidate.id);
            }
        }
    };
    select(CAPITAL_MIN_SPACING_WU, &mut all_capitals);
    // Too few capitals fit at full spacing: try again at half spacing.
    if all_capitals.len() < BASE_FACTION_COUNT {
        select(CAPITAL_MIN_SPACING_WU / 2.0, &mut all_capitals);
    }
    all_capitals.split_off(taken.len())
}

/// How many provinces a generated state may take while growing. Better
/// capitals support larger states, within `MAX_FACTION_PROVINCES`.
fn generated_budget(capital: &Province, rng: &mut XorShiftRng) -> usize {
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
    foundings: &[Founding],
    grid: Grid,
) -> Vec<u16> {
    let mut owner = vec![0u16; provinces.len() + 1];
    let mut held = vec![0usize; foundings.len() + 1];
    let province = |id: u16| &provinces[(id - 1) as usize];
    let founding = |faction_id: u16| &foundings[(faction_id - 1) as usize];

    // (cost so far, province id, faction id); Reverse makes the heap a min-heap.
    type Frontier = BinaryHeap<(Reverse<u32>, u16, u16)>;
    let mut frontier: Frontier = BinaryHeap::new();
    let push_neighbours =
        |frontier: &mut Frontier, owner: &[u16], from: u16, cost: u32, faction_id: u16| {
            for &neighbour in &adjacency[from as usize] {
                let target = province(neighbour);
                if owner[neighbour as usize] != 0
                    || target.habitability < CLAIMABLE_MIN_HABITABILITY
                {
                    continue;
                }
                let outside = founding(faction_id)
                    .authored
                    .map_or(0.0, |state| outside_band(target.light_level, state.light));
                let step = target.terrain_cost as f64 * TERRAIN_COST_WEIGHT
                    + site_distance_wu(province(from), target, grid)
                        * DISTANCE_COST_PER_WU
                    + outside as f64 * LIGHT_BAND_COST;
                frontier.push((
                    Reverse(cost + (step * 1000.0) as u32),
                    neighbour,
                    faction_id,
                ));
            }
        };

    for (index, founding) in foundings.iter().enumerate() {
        owner[founding.capital_province as usize] = (index + 1) as u16;
        held[index + 1] = 1;
    }
    for (index, founding) in foundings.iter().enumerate() {
        if founding.budget > 1 {
            push_neighbours(
                &mut frontier,
                &owner,
                founding.capital_province,
                0,
                (index + 1) as u16,
            );
        }
    }

    while let Some((Reverse(cost), province_id, faction_id)) = frontier.pop() {
        let budget = founding(faction_id).budget;
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

/// Give leftover claimable provinces to a neighbouring faction that is still
/// under its limit, smallest faction first, repeating until nothing changes.
/// This closes holes and gaps between factions.
fn absorb_enclosed_provinces(
    provinces: &[Province],
    adjacency: &[Vec<u16>],
    foundings: &[Founding],
    owner: &mut [u16],
) {
    let mut sizes = vec![0usize; foundings.len() + 1];
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
                    faction_id != 0
                        && sizes[faction_id as usize] < foundings[(faction_id - 1) as usize].limit
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
            light_level: 0.4,
            resources: 0.3,
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

    fn state(
        name: &str,
        light: (f32, f32),
        size: StateSize,
        prefers: &[Preference],
    ) -> AuthoredState {
        AuthoredState {
            name: name.to_string(),
            capital: Some(format!("{name} City")),
            light,
            size,
            prefers: prefers.to_vec(),
        }
    }

    #[test]
    fn generated_capitals_keep_their_minimum_spacing() {
        // Provinces 5 world units apart: capitals must be 18.75 apart, so at
        // most every fourth province can be one.
        let map = chain(&[0.9; 40], 5);
        let capitals = generated_capitals(&map.provinces, &[], Grid::flat(1.0));

        assert!(capitals.len() > 1);
        for (index, &a) in capitals.iter().enumerate() {
            for &b in &capitals[index + 1..] {
                let distance = site_distance_wu(
                    &map.provinces[(a - 1) as usize],
                    &map.provinces[(b - 1) as usize],
                    Grid::flat(1.0),
                );
                // The fallback pass may halve the spacing, never less.
                assert!(distance >= CAPITAL_MIN_SPACING_WU / 2.0);
            }
        }
    }

    #[test]
    fn barren_provinces_get_no_generated_capital() {
        let map = chain(&[0.2, 0.3], 100);

        assert!(generated_capitals(&map.provinces, &[], Grid::flat(1.0)).is_empty());
    }

    #[test]
    fn a_faction_claims_what_it_can_reach_and_stops_at_barren_land() {
        // capital - habitable - barren - habitable, 5 world units apart: too
        // close for a second capital.
        let map = chain(&[0.8, 0.6, 0.05, 0.3], 5);
        let factions = generate_factions(&map, &[], Grid::flat(1.0), 7);

        assert_eq!(factions.factions.len(), 1);
        assert_eq!(factions.factions[0].name, None);
        assert_eq!(
            factions.political_states,
            vec![
                PoliticalState::Claimed { faction_id: 1 },
                PoliticalState::Claimed { faction_id: 1 },
                PoliticalState::Uninhabited,
                PoliticalState::Unclaimed,
            ]
        );
        assert_eq!(factions.factions[0].province_count, 2);
        assert_eq!(factions.factions[0].area_cells, 200);
    }

    #[test]
    fn no_faction_exceeds_the_province_limit() {
        let map = chain(&[0.9; 200], 5);
        let factions = generate_factions(&map, &[], Grid::flat(1.0), 7);

        for faction in &factions.factions {
            assert!(faction.province_count >= 1);
            assert!(faction.province_count as usize <= MAX_FACTION_PROVINCES);
        }
    }

    #[test]
    fn same_seed_gives_the_same_factions() {
        let map = chain(&[0.9; 200], 5);
        let states = [state("Corazon", (0.3, 0.5), StateSize::Large, &[])];

        assert_eq!(
            generate_factions(&map, &states, Grid::flat(1.0), 7),
            generate_factions(&map, &states, Grid::flat(1.0), 7)
        );
    }

    #[test]
    fn faction_of_province_is_zero_for_unheld_and_unknown_provinces() {
        let map = chain(&[0.8, 0.05], 5);
        let factions = generate_factions(&map, &[], Grid::flat(1.0), 7);

        assert_eq!(factions.faction_of_province(1), 1);
        assert_eq!(factions.faction_of_province(2), 0);
        assert_eq!(factions.faction_of_province(0), 0);
        assert_eq!(factions.faction_of_province(99), 0);
    }

    #[test]
    fn an_authored_state_takes_its_capital_inside_its_light_band() {
        // The most habitable province is in daylight; the state lives in the dark.
        let mut map = chain(&[0.9, 0.3, 0.2], 30);
        map.provinces[0].light_level = 0.8;
        map.provinces[1].light_level = 0.05;
        map.provinces[2].light_level = 0.02;
        let states = [state(
            "Umbral Sovereignty",
            (0.0, 0.1),
            StateSize::Small,
            &[],
        )];

        let factions = generate_factions(&map, &states, Grid::flat(1.0), 7);

        assert_eq!(
            factions.factions[0].name.as_deref(),
            Some("Umbral Sovereignty")
        );
        assert_eq!(
            factions.factions[0].capital_name.as_deref(),
            Some("Umbral Sovereignty City")
        );
        assert_eq!(factions.factions[0].capital_province, 2);
        assert!(factions.unplaced_states.is_empty());
    }

    #[test]
    fn a_preference_decides_between_otherwise_equal_provinces() {
        let mut map = chain(&[0.6, 0.6, 0.6], 30);
        map.provinces[2].is_coastal = true;
        let harbour = state(
            "Tidewall",
            (0.3, 0.5),
            StateSize::CityState,
            &[Preference::Coastal],
        );

        assert_eq!(
            authored_capital(&map.provinces, &harbour, &[], Grid::flat(1.0)),
            Some(3)
        );
    }

    #[test]
    fn authored_states_come_first_and_generated_states_follow() {
        let map = chain(&[0.9; 60], 10);
        let states = [
            state("Corazon", (0.3, 0.5), StateSize::Large, &[]),
            state("Furrow", (0.3, 0.5), StateSize::Medium, &[]),
        ];

        let factions = generate_factions(&map, &states, Grid::flat(1.0), 7);

        assert_eq!(factions.factions[0].name.as_deref(), Some("Corazon"));
        assert_eq!(factions.factions[1].name.as_deref(), Some("Furrow"));
        assert!(factions.factions.len() > 2);
        assert!(factions.factions[2..]
            .iter()
            .all(|faction| faction.name.is_none()));
        let capitals: std::collections::BTreeSet<u16> = factions
            .factions
            .iter()
            .map(|faction| faction.capital_province)
            .collect();
        assert_eq!(capitals.len(), factions.factions.len());
    }

    #[test]
    fn a_city_state_stays_small_even_with_free_land_around_it() {
        let map = chain(&[0.9; 12], 2);
        let states = [state("Vestara", (0.3, 0.5), StateSize::CityState, &[])];

        let factions = generate_factions(&map, &states, Grid::flat(1.0), 7);

        assert!(factions.factions[0].province_count <= 3);
    }

    #[test]
    fn a_state_grows_along_its_light_band_before_leaving_it() {
        // Capital in the middle. Provinces to the left stay in the band;
        // provinces to the right leave it.
        let mut map = chain(&[0.6; 9], 5);
        for (index, light_level) in [0.6, 0.6, 0.6, 0.6, 0.6, 0.2, 0.2, 0.2, 0.2]
            .iter()
            .enumerate()
        {
            map.provinces[index].light_level = *light_level;
        }
        map.provinces[4].habitability = 0.9;
        let strip = AuthoredState {
            name: "Cinderline".to_string(),
            capital: None,
            light: (0.55, 0.7),
            size: StateSize::CityState,
            prefers: vec![],
        };
        let foundings = [Founding {
            capital_province: 5,
            authored: Some(&strip),
            budget: 3,
            limit: 3,
        }];

        let owner = grow_factions(&map.provinces, &map.adjacency, &foundings, Grid::flat(1.0));

        assert_eq!(&owner[3..=5], &[1, 1, 1]);
        assert_eq!(&owner[6..=9], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_state_with_no_province_in_its_band_is_reported_unplaced() {
        let map = chain(&[0.9, 0.9], 30);
        let states = [state(
            "Frostdelve Communion",
            (0.0, 0.05),
            StateSize::Small,
            &[],
        )];

        let factions = generate_factions(&map, &states, Grid::flat(1.0), 7);

        assert_eq!(
            factions.unplaced_states,
            vec!["Frostdelve Communion".to_string()]
        );
        assert!(factions
            .factions
            .iter()
            .all(|faction| faction.name.is_none()));
    }
}
