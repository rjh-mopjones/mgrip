use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerManifest {
    pub seed: u32,
    pub created: String,
    pub world_width: u32,
    pub world_height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub layer_images: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LevelManifest {
    pub parent_layers_tag: Option<String>,
    pub seed: u32,
    pub chunk_coord: (i32, i32),
    pub created: String,
}
