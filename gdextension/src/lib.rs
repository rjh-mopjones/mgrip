//! GDExtension entry point — exposes Rust terrain generation to Godot 4.x GDScript.
//!
//! Exposed classes:
//! - `MgBiomeMap`     — result of terrain generation, queryable per pixel
//! - `MgTerrainGen`   — entry point to generate MacroMap and chunk maps

use godot::prelude::*;
use mg_noise::{BiomeMap, SEA_LEVEL};
use std::sync::Arc;

struct MarginsGripExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MarginsGripExtension {}

// ─── MgBiomeMap ──────────────────────────────────────────────────────────────

/// A generated terrain tile. Wraps `mg_noise::BiomeMap`.
/// All pixel queries use (x, y) in [0, width) × [0, height).
#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct MgBiomeMap {
    inner: Option<Arc<BiomeMap>>,
    base: Base<RefCounted>,
}

#[godot_api]
impl IRefCounted for MgBiomeMap {
    fn init(base: Base<RefCounted>) -> Self {
        Self { inner: None, base }
    }
}

#[godot_api]
impl MgBiomeMap {
    #[func]
    pub fn width(&self) -> i64 {
        self.inner.as_ref().map(|m| m.width as i64).unwrap_or(0)
    }

    #[func]
    pub fn height(&self) -> i64 {
        self.inner.as_ref().map(|m| m.height as i64).unwrap_or(0)
    }

    #[func]
    pub fn heightmap_at(&self, x: i64, y: i64) -> f64 {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.heightmap_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn biome_index_at(&self, x: i64, y: i64) -> i64 {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.biome_at(x as usize, y as usize) as i64)
            .unwrap_or(0)
    }

    #[func]
    pub fn temperature_at(&self, x: i64, y: i64) -> f64 {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.temperature_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn humidity_at(&self, x: i64, y: i64) -> f64 {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.humidity_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn light_level_at(&self, x: i64, y: i64) -> f64 {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.light_level_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn river_at(&self, x: i64, y: i64) -> f64 {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.river_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn is_ocean(&self, x: i64, y: i64) -> bool {
        self.inner.as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.is_ocean(x as usize, y as usize))
            .unwrap_or(true)
    }

    /// Export a layer as a PackedByteArray of RGBA bytes (width * height * 4 bytes).
    /// Layer names: "biome", "Heightmap", "temperature", "humidity", "light_level",
    ///              "rivers", "tectonic", "continentalness", "erosion", "aridity" …
    #[func]
    pub fn export_layer_rgba(&self, layer_name: GString) -> PackedByteArray {
        use mg_noise::NoiseLayer;
        let Some(map) = &self.inner else { return PackedByteArray::new(); };

        let layer = match layer_name.to_string().as_str() {
            "biome" => NoiseLayer::Biome,
            "Heightmap" => NoiseLayer::Heightmap,
            "temperature" => NoiseLayer::Temperature,
            "humidity" => NoiseLayer::Humidity,
            "continentalness" => NoiseLayer::Continentalness,
            "tectonic" => NoiseLayer::Tectonic,
            "rock_hardness" => NoiseLayer::RockHardness,
            "light_level" => NoiseLayer::LightLevel,
            "peaks_valleys" => NoiseLayer::PeaksValleys,
            "erosion" => NoiseLayer::Erosion,
            "rivers" => NoiseLayer::Rivers,
            "aridity" => NoiseLayer::Aridity,
            "precipitation_type" => NoiseLayer::PrecipitationType,
            "snowpack" => NoiseLayer::Snowpack,
            "water_table" => NoiseLayer::WaterTable,
            "vegetation_density" => NoiseLayer::VegetationDensity,
            "soil_type" => NoiseLayer::SoilType,
            "resource_richness" => NoiseLayer::ResourceRichness,
            _ => {
                godot_warn!("MgBiomeMap: unknown layer '{layer_name}', defaulting to biome");
                NoiseLayer::Biome
            }
        };

        let rgba = map.layer_to_rgba(layer);
        PackedByteArray::from(rgba.as_slice())
    }

    /// Ocean mask — 1 byte per pixel, 1 = ocean, 0 = land.
    /// Derived from the final heightmap (after micro-detail) so the mask agrees
    /// with rendered geometry even when detail_level=2 perturbs coastal heights.
    #[func]
    pub fn is_ocean_grid(&self) -> PackedByteArray {
        let Some(map) = &self.inner else { return PackedByteArray::new(); };
        let data: Vec<u8> = map.heightmap.iter()
            .map(|&h| if h < SEA_LEVEL { 1 } else { 0 })
            .collect();
        PackedByteArray::from(data.as_slice())
    }

    /// Block heights array for 3D level rendering.
    /// Returns PackedInt32Array of length (width * height).
    /// Each value = floor(heightmap * HEIGHT_SCALE).
    #[func]
    pub fn block_heights(&self, height_scale: f64) -> PackedInt32Array {
        let Some(map) = &self.inner else { return PackedInt32Array::new(); };
        let data: Vec<i32> = map.heightmap.iter()
            .map(|&h| (h * height_scale).floor() as i32)
            .collect();
        PackedInt32Array::from(data.as_slice())
    }
}

// ─── MgTerrainGen ─────────────────────────────────────────────────────────────

/// Entry point for terrain generation callable from GDScript.
#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct MgTerrainGen {
    base: Base<RefCounted>,
}

#[godot_api]
impl IRefCounted for MgTerrainGen {
    fn init(base: Base<RefCounted>) -> Self {
        Self { base }
    }
}

#[godot_api]
impl MgTerrainGen {
    /// Generate a 512×512 macro-level map (full pipeline: erosion + rivers).
    ///
    /// `seed`: world seed (u32)
    /// Returns an `MgBiomeMap` resource.
    #[func]
    pub fn generate_macro(&self, seed: i64) -> Gd<MgBiomeMap> {
        let map = BiomeMap::generate(
            seed as u32,
            0.0, 0.0,       // origin
            1024.0, 512.0,  // world extent
            512, 512,       // pixel resolution
            0,              // detail_level = Macro
            true,           // run_erosion
            true,           // run_rivers
            1.0,            // freq_scale — world scale
        );
        let mut gd = Gd::<MgBiomeMap>::from_init_fn(|base| MgBiomeMap { inner: None, base });
        gd.bind_mut().inner = Some(Arc::new(map));
        gd
    }

    /// Generate a 512×512 meso tile at a given world chunk coordinate.
    ///
    /// `chunk_x`, `chunk_y`: position in the 16×8 macro grid.
    #[func]
    pub fn generate_meso_tile(&self, seed: i64, chunk_x: i64, chunk_y: i64) -> Gd<MgBiomeMap> {
        // Each meso tile is 64 world units wide × 64 world units tall
        let origin_x = chunk_x as f64 * 64.0;
        let origin_y = chunk_y as f64 * 64.0;
        let map = BiomeMap::generate(
            seed as u32,
            origin_x, origin_y,
            64.0, 64.0,
            512, 512,
            1,      // detail_level = Meso
            false,
            false,
            1.0,    // freq_scale — world scale
        );
        let mut gd = Gd::<MgBiomeMap>::from_init_fn(|base| MgBiomeMap { inner: None, base });
        gd.bind_mut().inner = Some(Arc::new(map));
        gd
    }

    /// Generate a 512×512 micro chunk (1 world unit) for 3D level rendering.
    #[func]
    pub fn generate_chunk(&self, seed: i64, world_x: f64, world_y: f64) -> Gd<MgBiomeMap> {
        let map = BiomeMap::generate(
            seed as u32,
            world_x, world_y,
            1.0, 1.0,   // 1 world unit = 1 chunk = 512 blocks
            512, 512,
            2,          // detail_level=2 enables derive_micro_heightmap
            false,
            false,
            8.0,        // freq_scale — ~64px wavelength terrain features
        );
        let mut gd = Gd::<MgBiomeMap>::from_init_fn(|base| MgBiomeMap { inner: None, base });
        gd.bind_mut().inner = Some(Arc::new(map));
        gd
    }

    /// Sea level constant (heightmap threshold for ocean vs land).
    #[func]
    pub fn sea_level() -> f64 {
        SEA_LEVEL
    }
}
