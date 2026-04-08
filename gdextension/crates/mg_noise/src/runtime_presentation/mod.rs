mod atmosphere;
mod landform;
mod surface_palette;
mod water_state;
mod zone;

use crate::{tile_has_fluid_surface, BiomeMap, SEA_LEVEL};
use mg_core::TileType;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

pub use atmosphere::AtmosphereClass;
pub use landform::LandformClass;
pub use surface_palette::SurfacePaletteClass;
pub use water_state::SurfaceWaterState;
pub use zone::PlanetZone;

const TARGET_AXIS_SAMPLES: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeChunkPresentation {
    pub planet_zone: PlanetZone,
    pub atmosphere_class: AtmosphereClass,
    pub water_state: SurfaceWaterState,
    pub landform_class: LandformClass,
    pub surface_palette_class: SurfacePaletteClass,
    pub interestingness_score: f32,
    pub average_light_level: f32,
    pub average_temperature: f32,
    pub average_humidity: f32,
    pub average_aridity: f32,
    pub average_snowpack: f32,
    pub average_water_table: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeReducedGrid<T> {
    pub width: usize,
    pub height: usize,
    pub values: Vec<T>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeChunkPresentationGrids {
    pub water_state_grid: RuntimeReducedGrid<SurfaceWaterState>,
    pub landform_grid: RuntimeReducedGrid<LandformClass>,
    pub surface_palette_grid: RuntimeReducedGrid<SurfacePaletteClass>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeChunkPresentationBundle {
    pub summary: RuntimeChunkPresentation,
    pub reduced_grids: RuntimeChunkPresentationGrids,
}

#[derive(Clone, Copy)]
pub(super) struct RuntimePresentationSample {
    pub biome: TileType,
    pub continentalness: f64,
    pub heightmap: f64,
    pub temperature: f64,
    pub humidity: f64,
    pub light_level: f64,
    pub rivers: f64,
    pub aridity: f64,
    pub snowpack: f64,
    pub water_table: f64,
    pub vegetation_density: f64,
    pub soil_type: f64,
    pub rock_hardness: f64,
    pub tectonic: f64,
    pub erosion: f64,
    pub peaks_valleys: f64,
    pub slope: f64,
    pub local_relief: f64,
    pub curvature: f64,
    pub is_ocean: bool,
}

impl BiomeMap {
    pub fn build_runtime_chunk_presentation(&self) -> RuntimeChunkPresentation {
        build_runtime_chunk_presentation_bundle(self).summary
    }

    pub fn build_runtime_chunk_presentation_bundle(&self) -> RuntimeChunkPresentationBundle {
        build_runtime_chunk_presentation_bundle(self)
    }
}

pub fn build_runtime_chunk_presentation(map: &BiomeMap) -> RuntimeChunkPresentation {
    build_runtime_chunk_presentation_bundle(map).summary
}

pub fn build_runtime_chunk_presentation_bundle(map: &BiomeMap) -> RuntimeChunkPresentationBundle {
    let (grid_width, grid_height) = reduced_grid_dimensions(map);
    if grid_width == 0 || grid_height == 0 {
        return RuntimeChunkPresentationBundle {
            summary: RuntimeChunkPresentation {
                planet_zone: PlanetZone::InnerTerminus,
                atmosphere_class: AtmosphereClass::TemperateTwilight,
                water_state: SurfaceWaterState::None,
                landform_class: LandformClass::FlatPlain,
                surface_palette_class: SurfacePaletteClass::ExposedStone,
                interestingness_score: 0.0,
                average_light_level: 0.0,
                average_temperature: 0.0,
                average_humidity: 0.0,
                average_aridity: 0.0,
                average_snowpack: 0.0,
                average_water_table: 0.0,
            },
            reduced_grids: RuntimeChunkPresentationGrids {
                water_state_grid: RuntimeReducedGrid {
                    width: 0,
                    height: 0,
                    values: Vec::new(),
                },
                landform_grid: RuntimeReducedGrid {
                    width: 0,
                    height: 0,
                    values: Vec::new(),
                },
                surface_palette_grid: RuntimeReducedGrid {
                    width: 0,
                    height: 0,
                    values: Vec::new(),
                },
            },
        };
    }

    let mut zone_counts = [0_u32; PlanetZone::COUNT];
    let mut atmosphere_counts = [0_u32; AtmosphereClass::COUNT];
    let mut water_counts = [0_u32; SurfaceWaterState::COUNT];
    let mut landform_counts = [0_u32; LandformClass::COUNT];
    let mut surface_palette_counts = [0_u32; SurfacePaletteClass::COUNT];
    let mut water_state_grid = Vec::with_capacity(grid_width * grid_height);
    let mut landform_grid = Vec::with_capacity(grid_width * grid_height);
    let mut surface_palette_grid = Vec::with_capacity(grid_width * grid_height);

    let mut light_sum = 0.0_f64;
    let mut temp_sum = 0.0_f64;
    let mut humidity_sum = 0.0_f64;
    let mut aridity_sum = 0.0_f64;
    let mut snowpack_sum = 0.0_f64;
    let mut water_table_sum = 0.0_f64;
    let mut relief_sum = 0.0_f64;
    let mut slope_sum = 0.0_f64;
    let mut slope_sq_sum = 0.0_f64;
    let mut max_coastalness = 0.0_f64;
    let mut max_river = 0.0_f64;

    for grid_y in 0..grid_height {
        for grid_x in 0..grid_width {
            let sample_x = reduced_grid_coord(grid_x, grid_width, map.width);
            let sample_y = reduced_grid_coord(grid_y, grid_height, map.height);
            let sample = sample_from_coords(map, sample_x, sample_y);
            light_sum += sample.light_level;
            temp_sum += sample.temperature;
            humidity_sum += sample.humidity;
            aridity_sum += sample.aridity;
            snowpack_sum += sample.snowpack;
            water_table_sum += sample.water_table;
            relief_sum += sample.local_relief;
            slope_sum += sample.slope;
            slope_sq_sum += sample.slope * sample.slope;
            max_coastalness = max_coastalness.max(coastalness(sample.continentalness));
            max_river = max_river.max(sample.rivers);

            let zone = PlanetZone::classify(&sample);
            let atmosphere = AtmosphereClass::classify(&sample, zone);
            let water_state = SurfaceWaterState::classify(&sample, zone);
            let landform = LandformClass::classify(&sample, zone, water_state);
            let surface_palette = SurfacePaletteClass::classify(&sample, zone, water_state);

            zone_counts[zone.as_index()] += 1;
            atmosphere_counts[atmosphere.as_index()] += 1;
            water_counts[water_state.as_index()] += 1;
            landform_counts[landform.as_index()] += 1;
            surface_palette_counts[surface_palette.as_index()] += 1;
            water_state_grid.push(water_state);
            landform_grid.push(landform);
            surface_palette_grid.push(surface_palette);
        }
    }

    let count = (grid_width * grid_height).max(1) as f64;
    let average_relief = relief_sum / count;
    let average_slope = slope_sum / count;
    let slope_variance = ((slope_sq_sum / count) - average_slope * average_slope).max(0.0);

    let dominant_zone = dominant_planet_zone(&zone_counts);
    let dominant_water_state = dominant_water_state(&water_counts);

    RuntimeChunkPresentationBundle {
        summary: RuntimeChunkPresentation {
            planet_zone: dominant_zone,
            atmosphere_class: dominant_atmosphere_class(&atmosphere_counts),
            water_state: dominant_water_state,
            landform_class: dominant_landform_class(
                &landform_counts,
                dominant_water_state,
                dominant_zone,
            ),
            surface_palette_class: dominant_surface_palette_class(&surface_palette_counts),
            interestingness_score: interestingness_score(
                average_relief,
                slope_variance,
                max_coastalness,
                max_river,
                &zone_counts,
                &water_counts,
                &landform_counts,
            ) as f32,
            average_light_level: (light_sum / count) as f32,
            average_temperature: (temp_sum / count) as f32,
            average_humidity: (humidity_sum / count) as f32,
            average_aridity: (aridity_sum / count) as f32,
            average_snowpack: (snowpack_sum / count) as f32,
            average_water_table: (water_table_sum / count) as f32,
        },
        reduced_grids: RuntimeChunkPresentationGrids {
            water_state_grid: RuntimeReducedGrid {
                width: grid_width,
                height: grid_height,
                values: water_state_grid,
            },
            landform_grid: RuntimeReducedGrid {
                width: grid_width,
                height: grid_height,
                values: landform_grid,
            },
            surface_palette_grid: RuntimeReducedGrid {
                width: grid_width,
                height: grid_height,
                values: surface_palette_grid,
            },
        },
    }
}

impl RuntimeChunkPresentationGrids {
    pub fn water_state_ids(&self) -> Vec<u8> {
        self.water_state_grid
            .values
            .iter()
            .map(|value| *value as u8)
            .collect()
    }

    pub fn landform_ids(&self) -> Vec<u8> {
        self.landform_grid
            .values
            .iter()
            .map(|value| *value as u8)
            .collect()
    }

    pub fn surface_palette_ids(&self) -> Vec<u8> {
        self.surface_palette_grid
            .values
            .iter()
            .map(|value| *value as u8)
            .collect()
    }

    pub fn water_state_digest(&self) -> String {
        stable_grid_digest(&self.water_state_ids())
    }

    pub fn landform_digest(&self) -> String {
        stable_grid_digest(&self.landform_ids())
    }

    pub fn surface_palette_digest(&self) -> String {
        stable_grid_digest(&self.surface_palette_ids())
    }
}

fn reduced_grid_dimensions(map: &BiomeMap) -> (usize, usize) {
    (
        map.width.min(TARGET_AXIS_SAMPLES),
        map.height.min(TARGET_AXIS_SAMPLES),
    )
}

fn reduced_grid_coord(grid_index: usize, grid_size: usize, sample_size: usize) -> usize {
    if grid_size <= 1 || sample_size <= 1 {
        return 0;
    }
    ((grid_index * sample_size.saturating_sub(1)) / (grid_size - 1)).min(sample_size - 1)
}

fn sample_from_coords(map: &BiomeMap, x: usize, y: usize) -> RuntimePresentationSample {
    let idx = y.saturating_mul(map.width).saturating_add(x);
    let biome = map.biomes.get(idx).copied().unwrap_or_default();
    let heightmap = *map.heightmap.get(idx).unwrap_or(&0.0);
    RuntimePresentationSample {
        biome,
        continentalness: *map.continentalness.get(idx).unwrap_or(&0.0),
        heightmap,
        temperature: *map.temperature.get(idx).unwrap_or(&0.0),
        humidity: *map.humidity.get(idx).unwrap_or(&0.0),
        light_level: *map.light_level.get(idx).unwrap_or(&0.0),
        rivers: *map.rivers.get(idx).unwrap_or(&0.0),
        aridity: *map.aridity.get(idx).unwrap_or(&0.0),
        snowpack: *map.snowpack.get(idx).unwrap_or(&0.0),
        water_table: *map.water_table.get(idx).unwrap_or(&0.0),
        vegetation_density: *map.vegetation_density.get(idx).unwrap_or(&0.0),
        soil_type: *map.soil_type.get(idx).unwrap_or(&0.0),
        rock_hardness: *map.rock_hardness.get(idx).unwrap_or(&0.0),
        tectonic: *map.tectonic.get(idx).unwrap_or(&0.0),
        erosion: *map.erosion.get(idx).unwrap_or(&0.0),
        peaks_valleys: *map.peaks_valleys.get(idx).unwrap_or(&0.0),
        slope: terrain_slope(map, x_from_index(map, idx), y_from_index(map, idx)),
        local_relief: terrain_local_relief(map, x_from_index(map, idx), y_from_index(map, idx)),
        curvature: terrain_curvature(map, x_from_index(map, idx), y_from_index(map, idx)),
        is_ocean: tile_has_fluid_surface(biome),
    }
}

fn stable_grid_digest(ids: &[u8]) -> String {
    let mut hash = 1469598103934665603_u64;
    for &value in ids {
        hash ^= u64::from(value);
        hash = hash.wrapping_mul(1099511628211_u64);
    }
    let mut digest = String::with_capacity(16);
    let _ = write!(&mut digest, "{hash:016x}");
    digest
}

fn dominant_planet_zone(counts: &[u32; PlanetZone::COUNT]) -> PlanetZone {
    let mut winner = PlanetZone::InnerTerminus;
    let mut best = 0_u32;
    for zone in PlanetZone::ALL {
        let count = counts[zone.as_index()];
        if count > best {
            best = count;
            winner = zone;
        }
    }
    winner
}

fn dominant_atmosphere_class(counts: &[u32; AtmosphereClass::COUNT]) -> AtmosphereClass {
    let mut winner = AtmosphereClass::TemperateTwilight;
    let mut best = 0_u32;
    for atmosphere in AtmosphereClass::ALL {
        let count = counts[atmosphere.as_index()];
        if count > best {
            best = count;
            winner = atmosphere;
        }
    }
    winner
}

fn dominant_water_state(counts: &[u32; SurfaceWaterState::COUNT]) -> SurfaceWaterState {
    let mut winner = SurfaceWaterState::None;
    let mut best = 0_u32;
    for water_state in SurfaceWaterState::ALL {
        let count = counts[water_state.as_index()];
        if count > best {
            best = count;
            winner = water_state;
        }
    }
    winner
}

fn dominant_landform_class(
    counts: &[u32; LandformClass::COUNT],
    water_state: SurfaceWaterState,
    zone: PlanetZone,
) -> LandformClass {
    let mut winner = LandformClass::FlatPlain;
    let mut best = 0_u32;
    for landform in LandformClass::ALL {
        let count = counts[landform.as_index()];
        if count > best {
            best = count;
            winner = landform;
        }
    }
    if winner == LandformClass::FlatPlain {
        winner = match water_state {
            SurfaceWaterState::EvaporiteBasin | SurfaceWaterState::BrineFlat => {
                LandformClass::Basin
            }
            SurfaceWaterState::LiquidRiver
            | SurfaceWaterState::FrozenRiver
            | SurfaceWaterState::MeltwaterChannel => LandformClass::RiverCutLowland,
            SurfaceWaterState::FrozenSea | SurfaceWaterState::IceSheet if zone.is_nightside() => {
                LandformClass::FrozenShelf
            }
            _ => winner,
        };
    }
    winner
}

fn dominant_surface_palette_class(
    counts: &[u32; SurfacePaletteClass::COUNT],
) -> SurfacePaletteClass {
    let mut winner = SurfacePaletteClass::ExposedStone;
    let mut best = 0_u32;
    for palette in SurfacePaletteClass::ALL {
        let count = counts[palette.as_index()];
        if count > best {
            best = count;
            winner = palette;
        }
    }
    winner
}

pub(super) fn score_high(value: f64, start: f64, end: f64) -> f64 {
    if end <= start {
        return f64::from(value >= end);
    }
    ((value - start) / (end - start)).clamp(0.0, 1.0)
}

pub(super) fn score_low(value: f64, start: f64, end: f64) -> f64 {
    1.0 - score_high(value, start, end)
}

pub(super) fn band_score(value: f64, min: f64, full_min: f64, full_max: f64, max: f64) -> f64 {
    if value <= min || value >= max {
        return 0.0;
    }
    if value >= full_min && value <= full_max {
        return 1.0;
    }
    if value < full_min {
        return ((value - min) / (full_min - min)).clamp(0.0, 1.0);
    }
    ((max - value) / (max - full_max)).clamp(0.0, 1.0)
}

pub(super) fn coastalness(continentalness: f64) -> f64 {
    let distance = (continentalness - SEA_LEVEL).abs();
    (1.0 - distance / 0.12).clamp(0.0, 1.0)
}

fn x_from_index(map: &BiomeMap, idx: usize) -> usize {
    idx % map.width
}

fn y_from_index(map: &BiomeMap, idx: usize) -> usize {
    idx / map.width
}

fn terrain_height(map: &BiomeMap, x: usize, y: usize) -> f64 {
    *map.heightmap
        .get(y.saturating_mul(map.width).saturating_add(x))
        .unwrap_or(&0.0)
}

fn clamped_coord(value: isize, upper_bound: usize) -> usize {
    value.clamp(0, upper_bound.saturating_sub(1) as isize) as usize
}

fn terrain_slope(map: &BiomeMap, x: usize, y: usize) -> f64 {
    let left = terrain_height(map, clamped_coord(x as isize - 1, map.width), y);
    let right = terrain_height(map, clamped_coord(x as isize + 1, map.width), y);
    let back = terrain_height(map, x, clamped_coord(y as isize - 1, map.height));
    let forward = terrain_height(map, x, clamped_coord(y as isize + 1, map.height));
    let dx = (right - left) * 0.5;
    let dy = (forward - back) * 0.5;
    (dx * dx + dy * dy).sqrt()
}

fn terrain_local_relief(map: &BiomeMap, x: usize, y: usize) -> f64 {
    let mut min_height = f64::INFINITY;
    let mut max_height = f64::NEG_INFINITY;
    for sample_y in -2..=2 {
        for sample_x in -2..=2 {
            let height = terrain_height(
                map,
                clamped_coord(x as isize + sample_x, map.width),
                clamped_coord(y as isize + sample_y, map.height),
            );
            min_height = min_height.min(height);
            max_height = max_height.max(height);
        }
    }
    if min_height.is_finite() && max_height.is_finite() {
        max_height - min_height
    } else {
        0.0
    }
}

fn terrain_curvature(map: &BiomeMap, x: usize, y: usize) -> f64 {
    let center = terrain_height(map, x, y);
    let left = terrain_height(map, clamped_coord(x as isize - 1, map.width), y);
    let right = terrain_height(map, clamped_coord(x as isize + 1, map.width), y);
    let back = terrain_height(map, x, clamped_coord(y as isize - 1, map.height));
    let forward = terrain_height(map, x, clamped_coord(y as isize + 1, map.height));
    ((left + right + back + forward) * 0.25) - center
}

fn count_nonzero(counts: &[u32]) -> usize {
    counts.iter().filter(|&&count| count > 0).count()
}

fn interestingness_score(
    average_relief: f64,
    slope_variance: f64,
    max_coastalness: f64,
    max_river: f64,
    zone_counts: &[u32; PlanetZone::COUNT],
    water_counts: &[u32; SurfaceWaterState::COUNT],
    landform_counts: &[u32; LandformClass::COUNT],
) -> f64 {
    let zone_diversity = ((count_nonzero(zone_counts).saturating_sub(1)).min(3) as f64) / 3.0;
    let water_diversity = ((count_nonzero(water_counts).saturating_sub(1)).min(4) as f64) / 4.0;
    let landform_diversity =
        ((count_nonzero(landform_counts).saturating_sub(1)).min(5) as f64) / 5.0;

    (score_high(average_relief, 0.08, 0.26) * 0.26
        + score_high(slope_variance, 0.0008, 0.0080) * 0.14
        + score_high(max_river, 0.05, 0.18) * 0.12
        + score_high(max_coastalness, 0.24, 0.74) * 0.10
        + landform_diversity * 0.18
        + water_diversity * 0.10
        + zone_diversity * 0.10)
        .clamp(0.0, 1.0)
}

pub(super) fn is_frozen_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::White
            | TileType::Glacier
            | TileType::Snow
            | TileType::IceSheet
            | TileType::FrozenBog
            | TileType::Tundra
            | TileType::Taiga
            | TileType::AlpineMeadow
    )
}

pub(super) fn is_volcanic_biome(biome: TileType) -> bool {
    matches!(
        biome,
        TileType::Volcanic | TileType::LavaField | TileType::MoltenWaste
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AtmosphereClass, LandformClass, PlanetZone, RuntimeChunkPresentationBundle,
        SurfacePaletteClass, SurfaceWaterState,
    };
    use crate::BiomeMap;

    const TEST_SEED: u32 = 42;
    const MICRO_CHUNK_WORLD_SIZE: f64 = 1.0;
    const MICRO_TILE_RESOLUTION: usize = 512;
    const MICRO_DETAIL_LEVEL: u32 = 2;
    const MICRO_FREQUENCY_SCALE: f64 = 8.0;

    fn build_reference_summary(world_x: f64, world_y: f64) -> super::RuntimeChunkPresentation {
        build_reference_bundle(world_x, world_y).summary
    }

    fn build_reference_bundle(world_x: f64, world_y: f64) -> RuntimeChunkPresentationBundle {
        BiomeMap::generate(
            TEST_SEED,
            world_x,
            world_y,
            MICRO_CHUNK_WORLD_SIZE,
            MICRO_CHUNK_WORLD_SIZE,
            MICRO_TILE_RESOLUTION,
            MICRO_TILE_RESOLUTION,
            MICRO_DETAIL_LEVEL,
            false,
            false,
            MICRO_FREQUENCY_SCALE,
        )
        .build_runtime_chunk_presentation_bundle()
    }

    #[test]
    fn classifies_reference_chunks_for_dayside_terminus_and_nightside() {
        let nightside = build_reference_summary(256.0, 0.0);
        assert_eq!(nightside.planet_zone, PlanetZone::DeepNightIce);
        assert_eq!(nightside.atmosphere_class, AtmosphereClass::BlackIceDark);
        assert_eq!(nightside.water_state, SurfaceWaterState::None);
        assert_eq!(nightside.landform_class, LandformClass::FlatPlain);
        assert_eq!(
            nightside.surface_palette_class,
            SurfacePaletteClass::BlackIceRock
        );
        assert!(nightside.interestingness_score <= 0.01);
        assert!(nightside.average_light_level < 0.01);
        assert!(nightside.average_temperature < -40.0);

        let dayside_margin = build_reference_summary(400.0, 250.0);
        assert_eq!(dayside_margin.planet_zone, PlanetZone::DryDaysideMargin);
        assert_eq!(
            dayside_margin.atmosphere_class,
            AtmosphereClass::HarshAmberHaze
        );
        assert_eq!(
            dayside_margin.water_state,
            SurfaceWaterState::EvaporiteBasin
        );
        assert_eq!(dayside_margin.landform_class, LandformClass::Basin);
        assert_eq!(
            dayside_margin.surface_palette_class,
            SurfacePaletteClass::SaltCrust
        );
        assert!(dayside_margin.interestingness_score > 0.13);
        assert!(dayside_margin.average_light_level > 0.6);
        assert!(dayside_margin.average_aridity > 0.8);

        let inferno = build_reference_summary(500.0, 450.0);
        assert_eq!(inferno.planet_zone, PlanetZone::SubstellarInferno);
        assert_eq!(inferno.atmosphere_class, AtmosphereClass::BlastedRadiance);
        assert_eq!(inferno.water_state, SurfaceWaterState::None);
        assert_eq!(inferno.landform_class, LandformClass::DuneWaste);
        assert_eq!(
            inferno.surface_palette_class,
            SurfacePaletteClass::ScorchedStone
        );
        assert!(inferno.interestingness_score > 0.09);
        assert!(dayside_margin.interestingness_score > nightside.interestingness_score + 0.10);
        assert!(inferno.average_light_level > 0.9);
        assert!(inferno.average_temperature > 100.0);
    }

    #[test]
    fn builds_reduced_grids_for_micro_chunks() {
        let bundle = build_reference_bundle(400.0, 250.0);
        assert_eq!(bundle.reduced_grids.water_state_grid.width, 64);
        assert_eq!(bundle.reduced_grids.water_state_grid.height, 64);
        assert_eq!(bundle.reduced_grids.water_state_grid.values.len(), 64 * 64);
        assert_eq!(bundle.reduced_grids.landform_grid.values.len(), 64 * 64);
        assert_eq!(
            bundle.reduced_grids.surface_palette_grid.values.len(),
            64 * 64
        );
        assert!(!bundle.reduced_grids.water_state_digest().is_empty());
        assert!(!bundle.reduced_grids.landform_digest().is_empty());
        assert!(!bundle.reduced_grids.surface_palette_digest().is_empty());
    }
}
