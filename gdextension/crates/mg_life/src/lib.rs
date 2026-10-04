//! LifeGen: civilisation layers derived from terrain (spec 011).
//!
//! Reads terrain only through `mg_core::TerrainQuery`. Terrain is immutable
//! input; nothing here writes back into it.

pub mod analysis;
pub mod factions;
pub mod provinces;
pub mod settlements;
#[cfg(test)]
mod test_support;

pub use analysis::{compute_analysis_grids, AnalysisGrids};
pub use factions::{generate_factions, Faction, FactionMap, PoliticalState};
pub use provinces::{generate_provinces, Province, ProvinceMap};
pub use settlements::{place_settlements, Settlement, SizeClass};
