//! GDExtension entry point — exposes Rust terrain generation to Godot 4.x GDScript.
//!
//! Exposed classes:
//! - `MgBiomeMap`     — result of terrain generation, queryable per pixel
//! - `MgTerrainGen`   — entry point to generate MacroMap and chunk maps

mod mesh;

use godot::prelude::*;
use mg_noise::{
    AtmosphereClass, BiomeMap, LandformClass, MacroOceanMask, PlanetZone, RuntimeChunkPresentation,
    RuntimeChunkPresentationBundle, RuntimeChunkPresentationGrids, SurfacePaletteClass,
    SurfaceWaterState, SEA_LEVEL,
};
use rayon::spawn;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::Instant;

static MACRO_OCEAN_MASK: OnceLock<Option<MacroOceanMask>> = OnceLock::new();

fn get_macro_ocean_mask() -> Option<&'static MacroOceanMask> {
    MACRO_OCEAN_MASK
        .get_or_init(|| {
            let store = mg_artifacts::ArtifactStore::new().ok()?;
            let macro_map = newest_macro_biome(&store)?;
            Some(MacroOceanMask::from_biome_map(&macro_map))
        })
        .as_ref()
}

fn newest_macro_biome(store: &mg_artifacts::ArtifactStore) -> Option<BiomeMap> {
    let layers = store.list_layers().ok()?;
    let mut best: Option<(String, std::time::SystemTime)> = None;
    for (tag, _manifest) in &layers {
        let path = store.layer_image_path(tag, "macromap.png");
        if !path.exists() {
            continue;
        }
        if let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            if best.as_ref().map_or(true, |(_, t)| mtime > *t) {
                best = Some((tag.clone(), mtime));
            }
        }
    }
    let (tag, _) = best?;
    store.load_layers_data(&tag).ok().map(|(map, _)| map)
}

struct MarginsGripExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MarginsGripExtension {}

fn biome_map_from_arc(map: Arc<BiomeMap>) -> Gd<MgBiomeMap> {
    let mut gd = Gd::<MgBiomeMap>::from_init_fn(|base| MgBiomeMap { inner: None, base });
    gd.bind_mut().inner = Some(map);
    gd
}

fn named_enum_value(id: i32, name: &str) -> Dictionary {
    let mut value = Dictionary::new();
    value.set("id", id as i64);
    value.set("name", GString::from(name));
    value
}

fn named_enum_legend(values: &[(i32, &'static str)]) -> Array<Dictionary> {
    let mut legend = Array::new();
    for &(id, name) in values {
        legend.push(&named_enum_value(id, name));
    }
    legend
}

fn reduced_grid_dictionary(
    width: usize,
    height: usize,
    ids: &[u8],
    digest: &str,
    legend: Array<Dictionary>,
) -> Dictionary {
    let mut result = Dictionary::new();
    result.set("width", width as i64);
    result.set("height", height as i64);
    result.set("ids", PackedByteArray::from(ids));
    result.set("digest", GString::from(digest));
    result.set("legend", legend);
    result
}

fn runtime_chunk_summary_dictionary(summary: &RuntimeChunkPresentation) -> Dictionary {
    let mut result = Dictionary::new();
    result.set(
        "planet_zone",
        named_enum_value(summary.planet_zone as i32, summary.planet_zone.as_str()),
    );
    result.set(
        "atmosphere_class",
        named_enum_value(
            summary.atmosphere_class as i32,
            summary.atmosphere_class.as_str(),
        ),
    );
    result.set(
        "water_state",
        named_enum_value(summary.water_state as i32, summary.water_state.as_str()),
    );
    result.set(
        "landform_class",
        named_enum_value(
            summary.landform_class as i32,
            summary.landform_class.as_str(),
        ),
    );
    result.set(
        "surface_palette_class",
        named_enum_value(
            summary.surface_palette_class as i32,
            summary.surface_palette_class.as_str(),
        ),
    );
    result.set(
        "interestingness_score",
        summary.interestingness_score as f64,
    );
    result.set("average_light_level", summary.average_light_level as f64);
    result.set("average_temperature", summary.average_temperature as f64);
    result.set("average_humidity", summary.average_humidity as f64);
    result.set("average_aridity", summary.average_aridity as f64);
    result.set("average_snowpack", summary.average_snowpack as f64);
    result.set("average_water_table", summary.average_water_table as f64);
    result
}

fn runtime_chunk_reduced_grids_dictionary(grids: &RuntimeChunkPresentationGrids) -> Dictionary {
    let water_ids = grids.water_state_ids();
    let landform_ids = grids.landform_ids();
    let surface_palette_ids = grids.surface_palette_ids();

    let mut result = Dictionary::new();
    result.set(
        "water_state_grid",
        reduced_grid_dictionary(
            grids.water_state_grid.width,
            grids.water_state_grid.height,
            &water_ids,
            &grids.water_state_digest(),
            named_enum_legend(&SurfaceWaterState::ALL.map(|value| (value as i32, value.as_str()))),
        ),
    );
    result.set(
        "landform_grid",
        reduced_grid_dictionary(
            grids.landform_grid.width,
            grids.landform_grid.height,
            &landform_ids,
            &grids.landform_digest(),
            named_enum_legend(&LandformClass::ALL.map(|value| (value as i32, value.as_str()))),
        ),
    );
    result.set(
        "surface_palette_grid",
        reduced_grid_dictionary(
            grids.surface_palette_grid.width,
            grids.surface_palette_grid.height,
            &surface_palette_ids,
            &grids.surface_palette_digest(),
            named_enum_legend(
                &SurfacePaletteClass::ALL.map(|value| (value as i32, value.as_str())),
            ),
        ),
    );
    result
}

fn runtime_chunk_presentation_data_dictionary(
    bundle: &RuntimeChunkPresentationBundle,
) -> Dictionary {
    let mut result = Dictionary::new();
    result.set("summary", runtime_chunk_summary_dictionary(&bundle.summary));
    result.set(
        "reduced_grids",
        runtime_chunk_reduced_grids_dictionary(&bundle.reduced_grids),
    );
    result
}

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
        self.inner
            .as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.heightmap_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn biome_index_at(&self, x: i64, y: i64) -> i64 {
        self.inner
            .as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.biome_at(x as usize, y as usize) as i64)
            .unwrap_or(0)
    }

    #[func]
    pub fn temperature_at(&self, x: i64, y: i64) -> f64 {
        self.inner
            .as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.temperature_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn humidity_at(&self, x: i64, y: i64) -> f64 {
        self.inner
            .as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.humidity_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn light_level_at(&self, x: i64, y: i64) -> f64 {
        self.inner
            .as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.light_level_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn river_at(&self, x: i64, y: i64) -> f64 {
        self.inner
            .as_ref()
            .filter(|m| x >= 0 && y >= 0 && (x as usize) < m.width && (y as usize) < m.height)
            .map(|m| m.river_at(x as usize, y as usize))
            .unwrap_or(0.0)
    }

    #[func]
    pub fn is_ocean(&self, x: i64, y: i64) -> bool {
        self.inner
            .as_ref()
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
        let Some(map) = &self.inner else {
            return PackedByteArray::new();
        };

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

    /// Legacy compatibility helper.
    /// Returns a semantic fluid-surface grid, not a raw "below sea level" mask.
    /// 1 = flat fluid surface should render here, 0 = terrain should stand here.
    #[func]
    pub fn is_ocean_grid(&self) -> PackedByteArray {
        let Some(map) = &self.inner else {
            return PackedByteArray::new();
        };
        let data: Vec<u8> = map
            .biomes
            .iter()
            .map(|&biome| u8::from(mg_noise::tile_has_fluid_surface(biome)))
            .collect();
        PackedByteArray::from(data.as_slice())
    }

    /// Block heights array for 3D level rendering.
    /// Returns PackedInt32Array of length (width * height).
    /// Each value = floor(heightmap * HEIGHT_SCALE).
    #[func]
    pub fn block_heights(&self, height_scale: f64) -> PackedInt32Array {
        let Some(map) = &self.inner else {
            return PackedInt32Array::new();
        };
        let data: Vec<i32> = map
            .heightmap
            .iter()
            .map(|&h| (h * height_scale).floor() as i32)
            .collect();
        PackedInt32Array::from(data.as_slice())
    }

    #[func]
    pub fn build_chunk_mesh_data(
        &self,
        height_scale: f64,
        sub_size: i64,
        use_edge_skirts: bool,
    ) -> Dictionary {
        let Some(map) = &self.inner else {
            return Dictionary::new();
        };
        mesh::build_chunk_mesh_data(map.as_ref(), height_scale, sub_size, use_edge_skirts)
    }

    #[func]
    pub fn build_runtime_chunk_summary(&self) -> Dictionary {
        let Some(map) = &self.inner else {
            return Dictionary::new();
        };
        let summary = map.build_runtime_chunk_presentation();
        runtime_chunk_summary_dictionary(&summary)
    }

    #[func]
    pub fn build_runtime_chunk_presentation_data(&self) -> Dictionary {
        let Some(map) = &self.inner else {
            return Dictionary::new();
        };
        let bundle = map.build_runtime_chunk_presentation_bundle();
        runtime_chunk_presentation_data_dictionary(&bundle)
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
            0.0,
            0.0, // origin
            1024.0,
            512.0, // world extent
            512,
            512,  // pixel resolution
            0,    // detail_level = Macro
            true, // run_erosion
            true, // run_rivers
            1.0,  // freq_scale — world scale
        );
        biome_map_from_arc(Arc::new(map))
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
            origin_x,
            origin_y,
            64.0,
            64.0,
            512,
            512,
            1, // detail_level = Meso
            false,
            false,
            1.0, // freq_scale — world scale
        );
        biome_map_from_arc(Arc::new(map))
    }

    /// Generate a 512×512 micro chunk (1 world unit) for 3D level rendering.
    #[func]
    pub fn generate_chunk(&self, seed: i64, world_x: f64, world_y: f64) -> Gd<MgBiomeMap> {
        self.generate_chunk_lod(seed, world_x, world_y, 512, 2, 8.0)
    }

    /// Generate a chunk with an explicit sample resolution and detail level.
    #[func]
    pub fn generate_chunk_lod(
        &self,
        seed: i64,
        world_x: f64,
        world_y: f64,
        resolution: i64,
        detail_level: i64,
        freq_scale: f64,
    ) -> Gd<MgBiomeMap> {
        let mut map = BiomeMap::generate(
            seed as u32,
            world_x,
            world_y,
            1.0,
            1.0, // 1 world unit = 1 chunk = 512 blocks
            resolution.max(2) as usize,
            resolution.max(2) as usize,
            detail_level.max(0) as u32,
            false,
            false,
            freq_scale.max(0.1),
        );
        if let Some(mask) = get_macro_ocean_mask() {
            map.apply_macro_ocean_mask(mask, world_x, world_y, 1.0, 1.0);
        }
        biome_map_from_arc(Arc::new(map))
    }

    /// Generate a multi-chunk region in a single BiomeMap.
    /// Covers `world_w` × `world_h` world units at resolution `res_w` × `res_h` pixels.
    /// Use `freq_scale=1.0` for macro (world-anchored) classification.
    #[func]
    pub fn generate_region(
        &self,
        seed: i64,
        world_x: f64,
        world_y: f64,
        world_w: f64,
        world_h: f64,
        res_w: i64,
        res_h: i64,
        detail_level: i64,
        freq_scale: f64,
    ) -> Gd<MgBiomeMap> {
        let map = BiomeMap::generate(
            seed as u32,
            world_x,
            world_y,
            world_w.max(0.1),
            world_h.max(0.1),
            res_w.max(2) as usize,
            res_h.max(2) as usize,
            detail_level.max(0) as u32,
            false,
            false,
            freq_scale.max(0.1),
        );
        biome_map_from_arc(Arc::new(map))
    }

    /// Sea level constant (heightmap threshold for ocean vs land).
    #[func]
    pub fn sea_level() -> f64 {
        SEA_LEVEL
    }
}

struct ChunkBuildNativeResult {
    biome_map: Arc<BiomeMap>,
    mesh_buffers: mesh::ChunkMeshBuffers,
    generation_ms: f64,
    mesh_prep_ms: f64,
}

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct MgChunkBuildJob {
    result: Arc<Mutex<Option<ChunkBuildNativeResult>>>,
    running: Arc<AtomicBool>,
    base: Base<RefCounted>,
}

#[godot_api]
impl IRefCounted for MgChunkBuildJob {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            result: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            base,
        }
    }
}

#[godot_api]
impl MgChunkBuildJob {
    #[func]
    pub fn start_chunk_build(
        &mut self,
        seed: i64,
        world_x: f64,
        world_y: f64,
        resolution: i64,
        detail_level: i64,
        freq_scale: f64,
        height_scale: f64,
        sub_size: i64,
        use_edge_skirts: bool,
    ) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        *self.result.lock().unwrap() = None;

        let result_slot = Arc::clone(&self.result);
        let running_flag = Arc::clone(&self.running);

        spawn(move || {
            let generation_start = Instant::now();
            let mut map = BiomeMap::generate(
                seed as u32,
                world_x,
                world_y,
                1.0,
                1.0,
                resolution.max(2) as usize,
                resolution.max(2) as usize,
                detail_level.max(0) as u32,
                false,
                false,
                freq_scale.max(0.1),
            );
            if let Some(mask) = get_macro_ocean_mask() {
                map.apply_macro_ocean_mask(mask, world_x, world_y, 1.0, 1.0);
            }
            let map = Arc::new(map);
            let generation_ms = generation_start.elapsed().as_secs_f64() * 1000.0;

            let mesh_start = Instant::now();
            let mesh_buffers = mesh::build_chunk_mesh_buffers(
                map.as_ref(),
                height_scale,
                sub_size,
                use_edge_skirts,
            );
            let mesh_prep_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;

            *result_slot.lock().unwrap() = Some(ChunkBuildNativeResult {
                biome_map: map,
                mesh_buffers,
                generation_ms,
                mesh_prep_ms,
            });
            running_flag.store(false, Ordering::SeqCst);
        });

        true
    }

    #[func]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    #[func]
    pub fn is_ready(&self) -> bool {
        !self.is_running() && self.result.lock().unwrap().is_some()
    }

    #[func]
    pub fn take_result(&mut self) -> Dictionary {
        let Some(output) = self.result.lock().unwrap().take() else {
            return Dictionary::new();
        };

        let mut result = Dictionary::new();
        result.set("biome_map", biome_map_from_arc(output.biome_map));
        result.set(
            "mesh_data",
            mesh::chunk_mesh_buffers_into_dictionary(output.mesh_buffers),
        );
        result.set("generation_ms", output.generation_ms);
        result.set("mesh_prep_ms", output.mesh_prep_ms);
        result
    }
}
