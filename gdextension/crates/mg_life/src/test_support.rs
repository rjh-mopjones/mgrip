//! Terrain stub for unit tests: flat, temperate plains, with optional rivers
//! and ocean.

use mg_core::{TerrainQuery, TileType};

pub(crate) struct MockTerrain {
    pub width: usize,
    pub height: usize,
    pub rivers: Vec<bool>,
    pub ocean: Vec<bool>,
}

impl MockTerrain {
    pub fn flat(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            rivers: vec![false; width * height],
            ocean: vec![false; width * height],
        }
    }

    pub fn with_river_along_row(width: usize, height: usize, river_row: usize) -> Self {
        let mut terrain = Self::flat(width, height);
        for x in 0..width {
            terrain.rivers[river_row * width + x] = true;
        }
        terrain
    }

    /// Ocean in every cell of column `ocean_column`, splitting the land in two.
    pub fn with_ocean_column(width: usize, height: usize, ocean_column: usize) -> Self {
        let mut terrain = Self::flat(width, height);
        for y in 0..height {
            terrain.ocean[y * width + ocean_column] = true;
        }
        terrain
    }

    fn in_bounds(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height
    }
}

impl TerrainQuery for MockTerrain {
    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }
    fn heightmap_at(&self, _x: usize, _y: usize) -> f64 {
        0.1
    }
    fn biome_at(&self, x: usize, y: usize) -> TileType {
        if self.is_ocean(x, y) {
            TileType::Sea
        } else {
            TileType::Plains
        }
    }
    fn temperature_at(&self, _x: usize, _y: usize) -> f64 {
        15.0
    }
    fn humidity_at(&self, _x: usize, _y: usize) -> f64 {
        0.5
    }
    fn continentalness_at(&self, _x: usize, _y: usize) -> f64 {
        0.5
    }
    fn erosion_at(&self, _x: usize, _y: usize) -> f64 {
        0.1
    }
    fn light_level_at(&self, _x: usize, _y: usize) -> f64 {
        0.5
    }
    fn rock_hardness_at(&self, _x: usize, _y: usize) -> f64 {
        0.5
    }
    fn river_at(&self, x: usize, y: usize) -> f64 {
        if self.is_river(x, y) {
            1.0
        } else {
            0.0
        }
    }
    fn drainage_at(&self, _x: usize, _y: usize) -> f64 {
        0.0
    }
    fn tectonic_at(&self, _x: usize, _y: usize) -> f64 {
        0.0
    }
    fn peaks_valleys_at(&self, _x: usize, _y: usize) -> f64 {
        0.0
    }
    fn aridity_at(&self, _x: usize, _y: usize) -> f64 {
        0.0
    }
    fn slope_at(&self, _x: usize, _y: usize) -> f64 {
        0.0
    }
    fn is_ocean(&self, x: usize, y: usize) -> bool {
        self.in_bounds(x, y) && self.ocean[y * self.width + x]
    }
    fn is_river(&self, x: usize, y: usize) -> bool {
        self.in_bounds(x, y) && self.rivers[y * self.width + x]
    }
}
