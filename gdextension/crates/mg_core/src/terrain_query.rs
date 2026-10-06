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
    /// Size of the river here as a share of the largest possible river:
    /// 0.0 where there is none, 1.0 for one two world units wide.
    fn river_at(&self, x: usize, y: usize) -> f64;
    fn drainage_at(&self, x: usize, y: usize) -> f64;
    /// Tectonic stress: 1.0 at a plate boundary, 0.0 in a quiet plate interior.
    fn tectonic_at(&self, x: usize, y: usize) -> f64;
    fn peaks_valleys_at(&self, x: usize, y: usize) -> f64;
    fn aridity_at(&self, x: usize, y: usize) -> f64;
    fn slope_at(&self, x: usize, y: usize) -> f64;
    /// Depth of rock that erosion has carried away from here, as a share of
    /// the most anywhere: 0.0 untouched, 1.0 the most worn.
    fn sediment_at(&self, x: usize, y: usize) -> f64;
    /// Wind-blown sand lying here: 0.0 bare, 1.0 a sand sea.
    fn sand_at(&self, x: usize, y: usize) -> f64;

    /// True where the surface is liquid water.
    fn is_ocean(&self, x: usize, y: usize) -> bool;
    fn is_river(&self, x: usize, y: usize) -> bool;
}
