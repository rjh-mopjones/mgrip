use noise::{NoiseFn, OpenSimplex};
use mg_core::NoiseStrategy;

/// Ridged multifractal noise for mountain relief.
/// Amplitude is modulated by tectonic stress in derive_peaks_valleys — this
/// strategy generates the raw [-1, 1] base values only.
pub struct PeaksAndValleysStrategy {
    noise: OpenSimplex,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    world_width: f64,
}

impl PeaksAndValleysStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 6,
            persistence: 0.5,
            lacunarity: 2.0,
            world_width: 0.0,
        }
    }

    pub fn new_wrapping(seed: u32, world_width: f64) -> Self {
        let mut s = Self::new(seed);
        s.world_width = world_width;
        s
    }

    fn ridged_fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amplitude = 0.0;

        for _ in 0..(self.octaves + detail_level) {
            let sample = if self.world_width > 0.0 {
                let [cx, cz, cy] = crate::wrap::cylindrical_noise_coords(x, y, freq, 0.007, self.world_width);
                self.noise.get([cx, cz, cy])
            } else {
                self.noise.get([x * freq * 0.007, y * freq * 0.007])
            };
            // Ridged: fold negative values upward, then invert so ridges are positive
            let ridged = 1.0 - sample.abs();
            value += ridged * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        // Normalize and shift to [-1, 1]
        (value / max_amplitude) * 2.0 - 1.0
    }
}

impl NoiseStrategy for PeaksAndValleysStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        self.ridged_fbm(x, y, detail_level).clamp(-1.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "PeaksValleys"
    }
}
