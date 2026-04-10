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
//! │           ├── biome.png
//! │           └── ...
//! └── levels/
//!     └── <tag>/
//!         ├── manifest.ron
//!         └── micro_biome.bin
//! ```

mod error;
mod manifest;
mod store;

pub use error::ArtifactError;
pub use manifest::{LayerManifest, LevelManifest};
pub use store::{ArtifactKind, ArtifactStore};
