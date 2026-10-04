//! LifeGen: civilisation layers derived from terrain (spec 011).
//!
//! Reads terrain only through `mg_core::TerrainQuery`. Terrain is immutable
//! input; nothing here writes back into it.

pub mod analysis;
pub mod factions;
pub mod grid;
pub mod provinces;
pub mod roads;
pub mod settlements;
pub mod trade;
#[cfg(test)]
mod test_support;

pub use analysis::{compute_analysis_grids, AnalysisGrids};
pub use factions::{
    generate_factions, AuthoredState, Faction, FactionMap, PoliticalState, Preference, StateSize,
};
pub use grid::Grid;
pub use provinces::{generate_provinces, Province, ProvinceMap};
pub use roads::{build_roads, Road, RoadKind};
pub use settlements::{place_settlements, Settlement, SizeClass};
pub use trade::{build_trade_flows, TradeFlow};
