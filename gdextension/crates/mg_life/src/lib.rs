//! LifeGen: civilisation layers derived from terrain (spec 011).
//!
//! Reads terrain only through `mg_core::TerrainQuery`. Terrain is immutable
//! input; nothing here writes back into it.

pub mod analysis;

pub use analysis::{compute_analysis_grids, AnalysisGrids};
