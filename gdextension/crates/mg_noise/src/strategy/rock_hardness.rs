use mg_core::{NoiseStrategy, Sphere};
use noise::{NoiseFn, OpenSimplex};

/// Sampled on the sphere, so it is seamless everywhere (spec 014).
pub struct RockHardnessStrategy {
    noise: OpenSimplex,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
}

impl RockHardnessStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 3,
            persistence: 0.6,
            lacunarity: 2.0,
        }
    }

    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amplitude = 0.0;

        for _ in 0..(self.octaves + detail_level) {
            let sample = self
                .noise
                .get(Sphere::MARGIN.noise_point_at(x, y, freq * 0.0125));
            value += sample * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }
}

impl NoiseStrategy for RockHardnessStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        ((self.fbm(x, y, detail_level) + 1.0) * 0.5).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "RockHardness"
    }
}
