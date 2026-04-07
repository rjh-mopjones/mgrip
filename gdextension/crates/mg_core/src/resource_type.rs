use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceType {
    Iron,
    Gold,
    Copper,
    Silver,
    Gems,
    Coal,
    Stone,
    Salt,
    Timber,
    Fish,
    FertileSoil,
    WildGame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainBias {
    Mountain,
    TectonicBoundary,
    Coastal,
    MultipleBiomes,
}

impl ResourceType {
    pub fn terrain_bias(&self) -> TerrainBias {
        match self {
            Self::Iron | Self::Copper | Self::Coal | Self::Stone => TerrainBias::Mountain,
            Self::Gold | Self::Silver | Self::Gems => TerrainBias::TectonicBoundary,
            Self::Salt | Self::Fish => TerrainBias::Coastal,
            Self::Timber | Self::FertileSoil | Self::WildGame => TerrainBias::MultipleBiomes,
        }
    }

    pub fn seed_offset(&self) -> u32 {
        match self {
            Self::Iron => 100,
            Self::Gold => 101,
            Self::Copper => 102,
            Self::Silver => 103,
            Self::Gems => 104,
            Self::Coal => 105,
            Self::Stone => 106,
            Self::Salt => 107,
            Self::Timber => 108,
            Self::Fish => 109,
            Self::FertileSoil => 110,
            Self::WildGame => 111,
        }
    }
}
