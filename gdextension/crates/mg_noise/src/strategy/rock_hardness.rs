use noise::{NoiseFn, OpenSimplex};
use mg_core::NoiseStrategy;

pub struct RockHardnessStrategy {
    noise: OpenSimplex,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    world_width: f64,
}

impl RockHardnessStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 3,
            persistence: 0.6,
            lacunarity: 2.0,
            world_width: 0.0,
        }
    }

    pub fn new_wrapping(seed: u32, world_width: f64) -> Self {
        let mut s = Self::new(seed);
        s.world_width = world_width;
        s
    }

    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amplitude = 0.0;

        for _ in 0..(self.octaves + detail_level) {
            let sample = if self.world_width > 0.0 {
                let [cx, cz, cy] = crate::wrap::cylindrical_noise_coords(x, y, freq, 0.0125, self.world_width);
                self.noise.get([cx, cz, cy])
            } else {
                self.noise.get([x * freq * 0.0125, y * freq * 0.0125])
            };
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
