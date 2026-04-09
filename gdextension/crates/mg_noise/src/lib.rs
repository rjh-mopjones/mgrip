pub mod biome_map;
pub mod biome_splines;
pub mod derived;
pub mod erosion_sim;
pub mod gpu;
pub mod rivers;
pub mod runtime_presentation;
pub mod strategy;
pub mod visualization;
pub mod wrap;

pub use biome_map::{tile_has_fluid_surface, pixel_is_ocean_rgb, BiomeMap, MacroOceanMask, SEA_LEVEL};
pub use biome_splines::BiomeSplines;
pub use derived::{
    derive_aridity, derive_erosion, derive_heightmap, derive_micro_heightmap, derive_peaks_valleys,
    derive_precipitation_type, derive_resource_richness, derive_snowpack, derive_soil_type,
    derive_temperature, derive_vegetation_density, derive_water_table,
};
pub use erosion_sim::{simulate_erosion, ErosionParams, ErosionResult};
pub use rivers::{
    rasterize_to_tile, RiverCharacter, RiverNetwork, RiverSegment, LOD_THRESHOLD_MACRO,
    LOD_THRESHOLD_MESO, LOD_THRESHOLD_MICRO,
};
pub use runtime_presentation::{
    AtmosphereClass, LandformClass, PlanetZone, RuntimeChunkPresentation,
    RuntimeChunkPresentationBundle, RuntimeChunkPresentationGrids, RuntimeReducedGrid,
    SurfacePaletteClass, SurfaceWaterState,
};
pub use strategy::{
    ContinentalnessStrategy, HumidityStrategy, LightLevelStrategy, PeaksAndValleysStrategy,
    RockHardnessStrategy, TectonicPlatesStrategy,
};
pub use visualization::NoiseLayer;
