pub mod biome_map;
pub mod gpu;
pub mod biome_splines;
pub mod derived;
pub mod erosion_sim;
pub mod rivers;
pub mod strategy;
pub mod visualization;
pub mod wrap;

pub use biome_map::{BiomeMap, SEA_LEVEL};
pub use biome_splines::BiomeSplines;
pub use erosion_sim::{ErosionParams, ErosionResult, simulate_erosion};
pub use rivers::{
    RiverCharacter, RiverNetwork, RiverSegment,
    rasterize_to_tile,
    LOD_THRESHOLD_MACRO, LOD_THRESHOLD_MESO, LOD_THRESHOLD_MICRO,
};
pub use strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy,
    PeaksAndValleysStrategy, RockHardnessStrategy, TectonicPlatesStrategy,
};
pub use derived::{
    derive_aridity, derive_erosion, derive_heightmap, derive_micro_heightmap,
    derive_peaks_valleys, derive_precipitation_type, derive_resource_richness,
    derive_snowpack, derive_soil_type, derive_temperature, derive_vegetation_density,
    derive_water_table,
};
pub use visualization::NoiseLayer;
