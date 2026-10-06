use mg_core::{NoiseStrategy, Sphere};
use noise::{NoiseFn, OpenSimplex};

/// Ridged multifractal noise for mountain relief.
/// Amplitude is modulated by tectonic stress in derive_peaks_valleys — this
/// strategy generates the raw [-1, 1] base values only. Sampled on the
/// sphere, so it is seamless everywhere (spec 014).
pub struct PeaksAndValleysStrategy {
    noise: OpenSimplex,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
}

impl PeaksAndValleysStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 6,
            persistence: 0.5,
            lacunarity: 2.0,
        }
    }

    fn ridged_fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amplitude = 0.0;

        for _ in 0..(self.octaves + detail_level) {
            let sample = self
                .noise
                .get(Sphere::MARGIN.noise_point_at(x, y, freq * 0.007));
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
