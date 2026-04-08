//! GPU-accelerated noise generation via wgpu compute shaders.
//!
//! Generates the 5 independent base layers (continentalness, peaks_valleys,
//! humidity, light_level, rock_hardness) on the GPU. Tectonic (Voronoi) and
//! all derived layers remain on CPU.
//!
//! Falls back transparently to the CPU rayon path if no GPU is available.

mod context;
mod perm_table;
mod pipelines;

pub use context::GpuNoiseContext;
pub use perm_table::{generate_permutation_table, permutation_table_to_u32};
pub use pipelines::NoisePipelines;

/// All 5 GPU-generated base layers for a single tile.
pub struct GpuNoiseResult {
    pub continentalness: Vec<f32>,
    pub peaks_valleys: Vec<f32>,
    pub humidity: Vec<f32>,
    pub light_level: Vec<f32>,
    pub rock_hardness: Vec<f32>,
}

impl GpuNoiseResult {
    pub fn into_f64(self) -> GpuNoiseResultF64 {
        GpuNoiseResultF64 {
            continentalness: self.continentalness.into_iter().map(|v| v as f64).collect(),
            peaks_valleys: self.peaks_valleys.into_iter().map(|v| v as f64).collect(),
            humidity: self.humidity.into_iter().map(|v| v as f64).collect(),
            light_level: self.light_level.into_iter().map(|v| v as f64).collect(),
            rock_hardness: self.rock_hardness.into_iter().map(|v| v as f64).collect(),
        }
    }
}

pub struct GpuNoiseResultF64 {
    pub continentalness: Vec<f64>,
    pub peaks_valleys: Vec<f64>,
    pub humidity: Vec<f64>,
    pub light_level: Vec<f64>,
    pub rock_hardness: Vec<f64>,
}
