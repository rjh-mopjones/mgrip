use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use mg_noise::{BiomeMap, RiverNetwork};

use crate::error::ArtifactError;
use crate::manifest::{LayerManifest, LevelManifest};

const MANIFEST_FILE: &str = "manifest.ron";
const MACRO_BIOME_FILE: &str = "macro_biome.bin";
const RIVER_NETWORK_FILE: &str = "river_network.bin";
const MICRO_BIOME_FILE: &str = "micro_biome.bin";
const IMAGES_DIR: &str = "images";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Layers,
    Levels,
}

impl ArtifactKind {
    fn dir_name(&self) -> &'static str {
        match self { Self::Layers => "layers", Self::Levels => "levels" }
    }

    fn display_name(&self) -> &'static str {
        match self { Self::Layers => "layers", Self::Levels => "levels" }
    }
}

pub struct ArtifactStore {
    base_path: PathBuf,
}

impl ArtifactStore {
    pub fn new() -> Result<Self, ArtifactError> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| ArtifactError::NoHomeDirectory)?;
        Self::with_base_path(home.join(".margins_grip"))
    }

    pub fn with_base_path(base_path: PathBuf) -> Result<Self, ArtifactError> {
        for kind in &[ArtifactKind::Layers, ArtifactKind::Levels] {
            let dir = base_path.join(kind.dir_name());
            fs::create_dir_all(&dir).map_err(|e| ArtifactError::Io {
                context: format!("creating {}", dir.display()),
                source: e,
            })?;
        }
        Ok(Self { base_path })
    }

    pub fn base_path(&self) -> &Path { &self.base_path }

    // ── Layers ────────────────────────────────────────────────────────────────

    pub fn save_layers(
        &self,
        tag: &str,
        biome_map: &BiomeMap,
        river_network: &RiverNetwork,
        images: &HashMap<String, (u32, u32, Vec<u8>)>,
        manifest: &LayerManifest,
    ) -> Result<(), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Layers, tag);
        let images_dir = dir.join(IMAGES_DIR);
        fs::create_dir_all(&images_dir).map_err(|e| ArtifactError::Io {
            context: format!("creating {}", dir.display()), source: e,
        })?;

        write_bincode(&dir.join(MACRO_BIOME_FILE), biome_map, "macro BiomeMap")?;
        write_bincode(&dir.join(RIVER_NETWORK_FILE), river_network, "RiverNetwork")?;

        for (name, (width, height, rgba_data)) in images {
            let path = images_dir.join(name);
            let img = image::RgbaImage::from_raw(*width, *height, rgba_data.clone())
                .ok_or_else(|| ArtifactError::Io {
                    context: format!("image buffer mismatch for {name}"),
                    source: std::io::Error::new(std::io::ErrorKind::InvalidData, "bad buffer size"),
                })?;
            img.save(&path).map_err(|e| ArtifactError::Image {
                context: format!("saving {}", path.display()), source: e,
            })?;
        }

        write_ron(&dir.join(MANIFEST_FILE), manifest, "layer manifest")
    }

    pub fn load_layers_data(&self, tag: &str) -> Result<(BiomeMap, RiverNetwork), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Layers, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound { kind: "layers".into(), tag: tag.into() });
        }
        let biome_map = read_bincode(&dir.join(MACRO_BIOME_FILE), "macro BiomeMap")?;
        let river_network = read_bincode(&dir.join(RIVER_NETWORK_FILE), "RiverNetwork")?;
        Ok((biome_map, river_network))
    }

    pub fn load_layer_manifest(&self, tag: &str) -> Result<LayerManifest, ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Layers, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound { kind: "layers".into(), tag: tag.into() });
        }
        read_ron(&dir.join(MANIFEST_FILE), "layer manifest")
    }

    pub fn layer_image_path(&self, tag: &str, layer_name: &str) -> PathBuf {
        self.artifact_dir(ArtifactKind::Layers, tag).join(IMAGES_DIR).join(layer_name)
    }

    pub fn layer_images_dir(&self, tag: &str) -> PathBuf {
        self.artifact_dir(ArtifactKind::Layers, tag).join(IMAGES_DIR)
    }

    // ── Levels ────────────────────────────────────────────────────────────────

    pub fn save_level(
        &self,
        tag: &str,
        micro_biome: &BiomeMap,
        manifest: &LevelManifest,
    ) -> Result<(), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Levels, tag);
        fs::create_dir_all(&dir).map_err(|e| ArtifactError::Io {
            context: format!("creating {}", dir.display()), source: e,
        })?;
        write_bincode(&dir.join(MICRO_BIOME_FILE), micro_biome, "micro BiomeMap")?;
        write_ron(&dir.join(MANIFEST_FILE), manifest, "level manifest")
    }

    pub fn load_level(&self, tag: &str) -> Result<(BiomeMap, LevelManifest), ArtifactError> {
        validate_tag(tag)?;
        let dir = self.artifact_dir(ArtifactKind::Levels, tag);
        if !dir.exists() {
            return Err(ArtifactError::NotFound { kind: "levels".into(), tag: tag.into() });
        }
        let micro_biome = read_bincode(&dir.join(MICRO_BIOME_FILE), "micro BiomeMap")?;
        let manifest = read_ron(&dir.join(MANIFEST_FILE), "level manifest")?;
        Ok((micro_biome, manifest))
    }

    // ── Listing ───────────────────────────────────────────────────────────────

    pub fn list_layers(&self) -> Result<Vec<(String, LayerManifest)>, ArtifactError> {
        self.list_artifacts(ArtifactKind::Layers)
    }

    pub fn list_levels(&self) -> Result<Vec<(String, LevelManifest)>, ArtifactError> {
        self.list_artifacts(ArtifactKind::Levels)
    }

    pub fn exists(&self, kind: ArtifactKind, tag: &str) -> bool {
        validate_tag(tag).is_ok() && self.artifact_dir(kind, tag).exists()
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn artifact_dir(&self, kind: ArtifactKind, tag: &str) -> PathBuf {
        self.base_path.join(kind.dir_name()).join(tag)
    }

    fn list_artifacts<M: serde::de::DeserializeOwned>(
        &self,
        kind: ArtifactKind,
    ) -> Result<Vec<(String, M)>, ArtifactError> {
        let parent = self.base_path.join(kind.dir_name());
        let entries = fs::read_dir(&parent).map_err(|e| ArtifactError::Io {
            context: format!("reading {}", kind.display_name()), source: e,
        })?;
        let mut results = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let Some(tag) = path.file_name().and_then(|n| n.to_str()).map(String::from) else { continue; };
            let manifest_path = path.join(MANIFEST_FILE);
            if let Ok(manifest) = read_ron::<M>(&manifest_path, "manifest") {
                results.push((tag, manifest));
            }
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }
}

fn validate_tag(tag: &str) -> Result<(), ArtifactError> {
    if tag.is_empty() {
        return Err(ArtifactError::InvalidTag { tag: tag.into(), reason: "tag must not be empty" });
    }
    if !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ArtifactError::InvalidTag { tag: tag.into(), reason: "only alphanumeric, hyphens, underscores" });
    }
    Ok(())
}

fn write_bincode<T: serde::Serialize>(path: &Path, value: &T, ctx: &str) -> Result<(), ArtifactError> {
    let data = bincode::serialize(value).map_err(|e| ArtifactError::Bincode { context: ctx.into(), source: e })?;
    fs::write(path, &data).map_err(|e| ArtifactError::Io { context: format!("writing {ctx}"), source: e })
}

fn read_bincode<T: serde::de::DeserializeOwned>(path: &Path, ctx: &str) -> Result<T, ArtifactError> {
    let data = fs::read(path).map_err(|e| ArtifactError::Io { context: format!("reading {ctx}"), source: e })?;
    bincode::deserialize(&data).map_err(|e| ArtifactError::Bincode { context: format!("deserializing {ctx}"), source: e })
}

fn write_ron<T: serde::Serialize>(path: &Path, value: &T, ctx: &str) -> Result<(), ArtifactError> {
    let text = ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())
        .map_err(|e| ArtifactError::RonSerialize { context: ctx.into(), source: e })?;
    fs::write(path, text.as_bytes()).map_err(|e| ArtifactError::Io { context: format!("writing {ctx}"), source: e })
}

fn read_ron<T: serde::de::DeserializeOwned>(path: &Path, ctx: &str) -> Result<T, ArtifactError> {
    let text = fs::read_to_string(path)
        .map_err(|e| ArtifactError::Io { context: format!("reading {ctx}"), source: e })?;
    ron::de::from_str(&text)
        .map_err(|e| ArtifactError::RonDeserialize { context: format!("deserializing {ctx}"), source: e })
}
