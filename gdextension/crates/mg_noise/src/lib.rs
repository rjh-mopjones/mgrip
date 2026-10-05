pub mod biome_map;
pub mod biome_splines;
pub mod derived;
pub mod drainage;
pub mod landscape;
pub mod rim_sea;
pub mod erosion_sim;
pub mod rivers;
pub mod runtime_presentation;
pub mod strategy;
pub mod terrain_query;
pub mod terrain_render;
pub mod visualization;
pub mod wrap;

pub use biome_map::{
    generate_map_tile,
    generate_macro_map, generate_macro_probe, sample_field_bilinear, tile_has_fluid_surface,
    BiomeMap, MacroOceanMask, MACRO_MAP_HEIGHT, MACRO_MAP_WIDTH, SEA_LEVEL, WORLD_HEIGHT,
    WORLD_WIDTH,
};
pub use biome_splines::BiomeSplines;
pub use derived::{
    derive_aridity, derive_heightmap, derive_micro_heightmap, derive_peaks_valleys,
    derive_precipitation_type, derive_resource_richness, derive_snowpack, derive_soil_type,
    derive_temperature, derive_vegetation_density, derive_water_table,
};
pub use erosion_sim::{erosion_step, ErosionParams, Land};
pub use rivers::{
    rasterize_from_network, rasterize_to_tile, RiverCharacter, RiverConstraint, RiverNetwork,
    RiverSegment, LOD_THRESHOLD_MACRO, LOD_THRESHOLD_MESO, LOD_THRESHOLD_MICRO, rasterize_courses, RiverCourse,
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
pub use terrain_render::{render_terrain, NormalizationHints};
pub use visualization::NoiseLayer;
