//! `TerrainQuery` over a `BiomeMap`, so consumers such as LifeGen can read
//! terrain without depending on `BiomeMap` itself. Cells are map pixels.

use crate::biome_map::{tile_has_fluid_surface, BiomeMap};
use mg_core::{TerrainQuery, TileType};

/// Value of `layer` at a cell; 0.0 outside the map.
fn cell(map: &BiomeMap, layer: &[f64], x: usize, y: usize) -> f64 {
    if x >= map.width || y >= map.height {
        return 0.0;
    }
    layer.get(y * map.width + x).copied().unwrap_or(0.0)
}

impl TerrainQuery for BiomeMap {
    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }

    fn heightmap_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.heightmap, x, y)
    }
    fn biome_at(&self, x: usize, y: usize) -> TileType {
        if x >= self.width || y >= self.height {
            return TileType::default();
        }
        self.biomes
            .get(y * self.width + x)
            .copied()
            .unwrap_or_default()
    }
    fn temperature_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.temperature, x, y)
    }
    fn humidity_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.humidity, x, y)
    }
    fn continentalness_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.continentalness, x, y)
    }
    fn erosion_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.erosion, x, y)
    }
    fn light_level_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.light_level, x, y)
    }
    fn rock_hardness_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.rock_hardness, x, y)
    }
    fn river_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.rivers, x, y)
    }
    fn drainage_at(&self, x: usize, y: usize) -> f64 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        self.drainage_area
            .get(y * self.width + x)
            .map_or(0.0, |&area| area as f64)
    }
    /// The tectonic layer stores distance from the nearest plate boundary
    /// (1.0 = quiet interior), so stress is its complement.
    fn tectonic_at(&self, x: usize, y: usize) -> f64 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        1.0 - cell(self, &self.tectonic, x, y)
    }
    fn peaks_valleys_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.peaks_valleys, x, y)
    }
    fn aridity_at(&self, x: usize, y: usize) -> f64 {
        cell(self, &self.aridity, x, y)
    }

    /// Height difference across the cell's neighbours (central difference,
    /// clamped at the map edge). Not normalised by cell size.
    fn slope_at(&self, x: usize, y: usize) -> f64 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let x_lo = x.saturating_sub(1);
        let x_hi = (x + 1).min(self.width - 1);
        let y_lo = y.saturating_sub(1);
        let y_hi = (y + 1).min(self.height - 1);
        let dx = self.heightmap_at(x_hi, y) - self.heightmap_at(x_lo, y);
        let dy = self.heightmap_at(x, y_hi) - self.heightmap_at(x, y_lo);
        (dx * dx + dy * dy).sqrt()
    }

    /// Liquid surface water, by the same rule as `MacroOceanMask`. Dried
    /// dayside basins and frozen nightside seas lie below sea level but are
    /// not ocean.
    fn is_ocean(&self, x: usize, y: usize) -> bool {
        tile_has_fluid_surface(self.biome_at(x, y))
    }
    fn is_river(&self, x: usize, y: usize) -> bool {
        self.river_at(x, y) > 0.0
    }
}
