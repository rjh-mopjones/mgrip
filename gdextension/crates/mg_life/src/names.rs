//! LifeGen stage 7: names.
//!
//! Every settlement, province and state gets a name. Authored states and
//! their capitals keep the names the lore gives them. Everything else is
//! generated from word lists (`data/lifegen_names.ron`): a first half chosen
//! by how much light the place gets and a second half chosen by its ground,
//! joined into one word, as the lore's own names are (Tidewall, Frostdelve).
//!
//! Randlebrot had no name generation; this stage is new.

use std::collections::HashSet;

use serde::Deserialize;

use crate::factions::FactionMap;
use crate::provinces::{Province, ProvinceMap};
use crate::settlements::{Settlement, SizeClass};

/// Provinces darker than this take their first halves from the night list.
const NIGHT_MAX_LIGHT: f32 = 0.25;
/// Provinces brighter than this take their first halves from the day list.
const DAY_MIN_LIGHT: f32 = 0.55;
/// The highest share of provinces by mean elevation count as highland.
const HIGHLAND_SHARE: f64 = 0.25;
/// A state with at most this many provinces is named as a city-state.
const CITY_STATE_MAX_PROVINCES: u32 = 3;
const NAME_SEED_OFFSET: u32 = 7;

/// Word lists that names are built from. See `data/lifegen_names.ron`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NameParts {
    // First halves, by light level.
    pub night: Vec<String>,
    pub terminus: Vec<String>,
    pub day: Vec<String>,
    pub anywhere: Vec<String>,
    // Second halves, by ground.
    pub coastal: Vec<String>,
    pub river: Vec<String>,
    pub highland: Vec<String>,
    pub lowland: Vec<String>,
    pub any_ground: Vec<String>,
    /// Words that follow the main settlement's name to name an outpost.
    pub outposts: Vec<String>,
    /// Forms for a generated state's name; `{}` stands for its capital's name.
    pub city_state_forms: Vec<String>,
    pub state_forms: Vec<String>,
}

/// Stage 7 output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// Indexed by `settlement id - 1`.
    pub settlements: Vec<String>,
    /// Indexed by `province id - 1`. A province is named for its main settlement.
    pub provinces: Vec<String>,
    /// Indexed by `faction id - 1`.
    pub factions: Vec<String>,
}

/// Name everything. Deterministic for a given seed, and no two settlements
/// share a name.
pub fn generate_names(
    province_map: &ProvinceMap,
    faction_map: &FactionMap,
    settlements: &[Settlement],
    parts: &NameParts,
    seed: u32,
) -> Names {
    let seed = seed.wrapping_add(NAME_SEED_OFFSET) as u64;
    let highland_from = highland_min_elevation(&province_map.provinces);
    let province = |id: u16| &province_map.provinces[(id - 1) as usize];
    let compound = |province: &Province, salt: u64, used: &HashSet<String>| {
        compound_name(parts, province, highland_from, mix(seed, salt), used)
    };

    // Names the lore has already taken.
    let mut used: HashSet<String> = faction_map
        .factions
        .iter()
        .flat_map(|faction| [faction.name.clone(), faction.capital_name.clone()])
        .flatten()
        .collect();

    // A province's settlements are listed main settlement first.
    let mut province_names: Vec<Option<String>> = vec![None; province_map.provinces.len()];
    let mut settlement_names = Vec::with_capacity(settlements.len());
    for settlement in settlements {
        let home = province(settlement.province_id);
        let main_name = &mut province_names[(settlement.province_id - 1) as usize];
        let authored_capital = faction_map
            .factions
            .iter()
            .find(|faction| faction.capital_province == settlement.province_id)
            .and_then(|faction| faction.capital_name.clone())
            .filter(|_| settlement.size_class == SizeClass::Metropolis);

        let name = match (authored_capital, main_name.as_deref()) {
            (Some(capital), _) => capital,
            (None, Some(main)) if settlement.size_class == SizeClass::Outpost => {
                outpost_name(parts, main, mix(seed, settlement.id as u64), &used)
            }
            _ => compound(home, settlement.id as u64, &used),
        };
        used.insert(name.clone());
        main_name.get_or_insert_with(|| name.clone());
        settlement_names.push(name);
    }

    // A province with no settlement still gets a name.
    let provinces = province_map
        .provinces
        .iter()
        .zip(province_names)
        .map(|(province, name)| {
            let name = name.unwrap_or_else(|| compound(province, u64::MAX - province.id as u64, &used));
            used.insert(name.clone());
            name
        })
        .collect::<Vec<_>>();

    let factions = faction_map
        .factions
        .iter()
        .map(|faction| {
            faction.name.clone().unwrap_or_else(|| {
                let capital = &provinces[(faction.capital_province - 1) as usize];
                let forms = if faction.province_count <= CITY_STATE_MAX_PROVINCES {
                    &parts.city_state_forms
                } else {
                    &parts.state_forms
                };
                state_name(forms, capital, mix(seed, faction.id as u64))
            })
        })
        .collect();

    Names {
        settlements: settlement_names,
        provinces,
        factions,
    }
}

/// Mean elevation at or above which a province counts as highland.
fn highland_min_elevation(provinces: &[Province]) -> f32 {
    let mut elevations: Vec<f32> = provinces.iter().map(|p| p.elevation_mean).collect();
    elevations.sort_by(f32::total_cmp);
    let index = ((elevations.len() as f64) * (1.0 - HIGHLAND_SHARE)) as usize;
    elevations.get(index).copied().unwrap_or(f32::INFINITY)
}

/// A first half and a second half joined into one word, fitting the province
/// and not yet in `used`. `hash` decides where in the lists the search starts.
fn compound_name(
    parts: &NameParts,
    province: &Province,
    highland_from: f32,
    hash: u64,
    used: &HashSet<String>,
) -> String {
    let by_light = if province.light_level < NIGHT_MAX_LIGHT {
        &parts.night
    } else if province.light_level > DAY_MIN_LIGHT {
        &parts.day
    } else {
        &parts.terminus
    };
    let by_ground = if province.is_coastal {
        &parts.coastal
    } else if province.is_river_junction {
        &parts.river
    } else if province.elevation_mean >= highland_from {
        &parts.highland
    } else {
        &parts.lowland
    };
    // Halves that fit the province first. If every such pair is taken, any
    // ground will do, and then any light.
    let every_ground = || {
        [&parts.coastal, &parts.river, &parts.highland, &parts.lowland, &parts.any_ground]
            .into_iter()
            .flatten()
    };
    let fitting_firsts: Vec<&String> = by_light.iter().chain(&parts.anywhere).collect();
    let fitting_seconds: Vec<&String> = by_ground.iter().chain(&parts.any_ground).collect();
    let every_first: Vec<&String> = [&parts.night, &parts.terminus, &parts.day, &parts.anywhere]
        .into_iter()
        .flatten()
        .collect();
    let tiers = [
        (&fitting_firsts, fitting_seconds),
        (&fitting_firsts, every_ground().collect()),
        (&every_first, every_ground().collect()),
    ];

    tiers
        .iter()
        .flat_map(|(firsts, seconds)| {
            let pairs = (firsts.len() * seconds.len()) as u64;
            (0..pairs)
                .map(move |attempt| ((hash % pairs + attempt) % pairs) as usize)
                .map(move |pair| (firsts[pair / seconds.len()], seconds[pair % seconds.len()]))
        })
        .filter(|(first, second)| joins_cleanly(first, second))
        .map(|(first, second)| format!("{first}{second}"))
        .find(|name| !used.contains(name))
        // Every pair of every list taken: only with very short word lists.
        .unwrap_or_else(|| format!("{}{} {hash}", every_first[0], parts.any_ground[0]))
}

/// Whether two halves read well as one word: not the same word twice
/// (Hollowhollow) and no doubled letter at the join (Lastton).
fn joins_cleanly(first: &str, second: &str) -> bool {
    let last = first.chars().last().map(|c| c.to_ascii_lowercase());
    let next = second.chars().next().map(|c| c.to_ascii_lowercase());
    !first.eq_ignore_ascii_case(second) && last != next
}

/// "Ashford Relay": an outpost named for its province's main settlement.
fn outpost_name(parts: &NameParts, main: &str, hash: u64, used: &HashSet<String>) -> String {
    let words = parts.outposts.len() as u64;
    (0..words)
        .map(|attempt| &parts.outposts[((hash % words + attempt) % words) as usize])
        // Not "Ashgate Gate".
        .filter(|word| !main.to_lowercase().ends_with(&word.to_lowercase()))
        .map(|word| format!("{main} {word}"))
        .find(|name| !used.contains(name))
        .unwrap_or_else(|| format!("{main} {hash}"))
}

/// "Rustweir March": a state named for its capital, in one of `forms`.
fn state_name(forms: &[String], capital: &str, hash: u64) -> String {
    let count = forms.len() as u64;
    (0..count)
        .map(|attempt| &forms[((hash % count + attempt) % count) as usize])
        // Not "Saltreach Reach".
        .find(|form| {
            let word = form.replace("{}", "").trim().to_lowercase();
            word.is_empty() || !capital.to_lowercase().ends_with(&word)
        })
        .unwrap_or(&forms[0])
        .replace("{}", capital)
}

/// Mix a seed and a salt into a well-spread number (SplitMix64).
fn mix(seed: u64, salt: u64) -> u64 {
    let mut z = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factions::{Faction, PoliticalState};
    use mg_core::TileType;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| word.to_string()).collect()
    }

    fn parts() -> NameParts {
        NameParts {
            night: words(&["Frost", "Rime", "Gloom"]),
            terminus: words(&["Tide", "Ash", "Quiet"]),
            day: words(&["Ember", "Cinder", "Sear"]),
            anywhere: words(&["Old"]),
            coastal: words(&["haven", "port", "wall"]),
            river: words(&["ford", "run"]),
            highland: words(&["delve", "crag"]),
            lowland: words(&["line", "stead", "ton"]),
            any_ground: words(&["gate"]),
            outposts: words(&["Watch", "Gate", "Relay", "Cairn", "Post"]),
            city_state_forms: words(&["Free City of {}"]),
            state_forms: words(&["{} League"]),
        }
    }

    fn province(id: u16, light_level: f32, is_coastal: bool) -> Province {
        Province {
            id,
            site: (0, 0),
            biome: TileType::Plains,
            habitability: 0.6,
            area_cells: 100,
            is_coastal,
            is_river_junction: false,
            // Higher ids stand higher, so province 1 is lowland.
            elevation_mean: id as f32 * 0.1,
            terrain_cost: 0.5,
            light_level,
            resources: 0.3,
        }
    }

    fn settlement(id: u32, province_id: u16, size_class: SizeClass) -> Settlement {
        Settlement {
            id,
            position: (0, 0),
            province_id,
            size_class,
        }
    }

    /// Two provinces: 1 is the capital of authored Corazon, 2 of a generated
    /// state of four provinces.
    fn world() -> (ProvinceMap, FactionMap, Vec<Settlement>) {
        let province_map = ProvinceMap {
            width: 0,
            height: 0,
            province_ids: vec![],
            provinces: vec![province(1, 0.4, false), province(2, 0.8, true)],
            adjacency: vec![vec![], vec![], vec![]],
        };
        let faction = |id, name: Option<&str>, capital_name: Option<&str>, province_count| Faction {
            id,
            name: name.map(String::from),
            capital_name: capital_name.map(String::from),
            capital_province: id,
            province_count,
            area_cells: 100,
        };
        let faction_map = FactionMap {
            political_states: vec![
                PoliticalState::Claimed { faction_id: 1 },
                PoliticalState::Claimed { faction_id: 2 },
            ],
            factions: vec![
                faction(1, Some("Corazon"), Some("Violetta"), 22),
                faction(2, None, None, 4),
            ],
            unplaced_states: vec![],
        };
        let settlements = vec![
            settlement(1, 1, SizeClass::Metropolis),
            settlement(2, 1, SizeClass::Village),
            settlement(3, 1, SizeClass::Outpost),
            settlement(4, 2, SizeClass::Metropolis),
            settlement(5, 2, SizeClass::Outpost),
            settlement(6, 2, SizeClass::Outpost),
        ];
        (province_map, faction_map, settlements)
    }

    fn names(seed: u32) -> Names {
        let (province_map, faction_map, settlements) = world();
        generate_names(&province_map, &faction_map, &settlements, &parts(), seed)
    }

    #[test]
    fn authored_states_and_their_capitals_keep_their_names() {
        let names = names(1);
        assert_eq!(names.factions[0], "Corazon");
        assert_eq!(names.settlements[0], "Violetta");
        assert_eq!(names.provinces[0], "Violetta");
    }

    #[test]
    fn a_generated_state_is_named_for_its_capital() {
        let names = names(1);
        assert_eq!(names.factions[1], format!("{} League", names.settlements[3]));
    }

    #[test]
    fn outposts_are_named_for_the_main_settlement_of_their_province() {
        let names = names(1);
        for outpost in [&names.settlements[4], &names.settlements[5]] {
            assert!(outpost.starts_with(&format!("{} ", names.settlements[3])));
        }
        assert_ne!(names.settlements[4], names.settlements[5]);
    }

    #[test]
    fn names_fit_the_light_and_ground_of_their_province() {
        let names = names(1);
        // Province 1: terminus light, inland lowland.
        let village = &names.settlements[1];
        assert!(["Tide", "Ash", "Quiet", "Old"].iter().any(|first| village.starts_with(first)));
        assert!(["line", "stead", "ton", "gate"].iter().any(|second| village.ends_with(second)));
        // Province 2: dayside coast.
        let capital = &names.settlements[3];
        assert!(["Ember", "Cinder", "Sear", "Old"].iter().any(|first| capital.starts_with(first)));
        assert!(["haven", "port", "wall", "gate"].iter().any(|second| capital.ends_with(second)));
    }

    #[test]
    fn no_two_settlements_share_a_name_and_names_repeat_for_a_seed() {
        let names = names(1);
        let distinct: HashSet<&String> = names.settlements.iter().collect();
        assert_eq!(distinct.len(), names.settlements.len());
        assert_eq!(names, self::names(1));
        assert_ne!(names.settlements, self::names(2).settlements);
    }

    #[test]
    fn when_fitting_names_run_out_other_ground_words_are_used() {
        let (province_map, _, _) = world();
        let inland = &province_map.provinces[0];
        // Take every name that fits an inland terminus lowland.
        let mut used = HashSet::new();
        for first in ["Tide", "Ash", "Quiet", "Old"] {
            for second in ["line", "stead", "ton", "gate"] {
                used.insert(format!("{first}{second}"));
            }
        }

        let name = compound_name(&parts(), inland, f32::INFINITY, 1, &used);

        assert!(!used.contains(&name));
        assert!(["Tide", "Ash", "Quiet", "Old"].iter().any(|first| name.starts_with(first)));
        assert!(!name.contains(' '));
    }

    #[test]
    fn halves_that_join_badly_are_not_used() {
        assert!(joins_cleanly("Ash", "ford"));
        assert!(!joins_cleanly("Last", "ton"));
        assert!(!joins_cleanly("Hollow", "hollow"));
        assert_eq!(outpost_name(&parts(), "Ashgate", 1, &HashSet::new()), "Ashgate Relay");
        let forms = words(&["{} Reach", "{} League"]);
        assert_eq!(state_name(&forms, "Saltreach", 0), "Saltreach League");
    }
}
