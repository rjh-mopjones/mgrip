use crate::biome::TileType;

pub trait TerrainQuery: Send + Sync {
    fn width(&self) -> usize;
    fn height(&self) -> usize;

    fn heightmap_at(&self, x: usize, y: usize) -> f64;
    fn biome_at(&self, x: usize, y: usize) -> TileType;
    fn temperature_at(&self, x: usize, y: usize) -> f64;
    fn humidity_at(&self, x: usize, y: usize) -> f64;
    fn continentalness_at(&self, x: usize, y: usize) -> f64;
    fn erosion_at(&self, x: usize, y: usize) -> f64;
    fn light_level_at(&self, x: usize, y: usize) -> f64;
    fn rock_hardness_at(&self, x: usize, y: usize) -> f64;
    fn river_at(&self, x: usize, y: usize) -> f64;
    fn drainage_at(&self, x: usize, y: usize) -> f64;
    fn tectonic_at(&self, x: usize, y: usize) -> f64;
    fn peaks_valleys_at(&self, x: usize, y: usize) -> f64;
    fn aridity_at(&self, x: usize, y: usize) -> f64;
    fn slope_at(&self, x: usize, y: usize) -> f64;

    fn is_ocean(&self, x: usize, y: usize) -> bool;
    fn is_river(&self, x: usize, y: usize) -> bool;
}
