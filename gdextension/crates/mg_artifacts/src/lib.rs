//! Artifact storage at `~/.margins_grip/`.
//!
//! ```text
//! ~/.margins_grip/
//! ├── layers/
//! │   └── <tag>/
//! │       ├── manifest.ron
//! │       ├── macro_biome.bin
//! │       ├── river_network.bin
//! │       └── images/
//! │           ├── macromap.png
//! │           └── ...
//! └── levels/
//!     └── <tag>/
//!         ├── manifest.ron
//!         └── micro_biome.bin
//! ```

mod civ_pack;
mod error;
mod macro_pack;
mod manifest;
mod store;

pub use civ_pack::{CivFaction, CivPack, CivProvince, CivRoad, CivSettlement};
pub use error::ArtifactError;
pub use macro_pack::MacroPack;
pub use manifest::{LayerManifest, LevelManifest};
pub use store::{ArtifactKind, ArtifactStore};
