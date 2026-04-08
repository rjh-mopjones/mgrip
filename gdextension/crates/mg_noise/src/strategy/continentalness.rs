use mg_core::NoiseStrategy;
use noise::{NoiseFn, OpenSimplex};

pub struct ContinentalnessStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    lacunarity: f64,
    persistence: f64,
    world_width: f64,
}

impl ContinentalnessStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 16,
            frequency: 1.0,
            lacunarity: 2.0,
            persistence: 0.59,
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
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        for _ in 0..(self.octaves + detail_level) {
            let sample = if self.world_width > 0.0 {
                let [cx, cz, cy] =
                    crate::wrap::cylindrical_noise_coords(x, y, freq, 0.01, self.world_width);
                self.noise.get([cx, cz, cy])
            } else {
                self.noise.get([x * freq * 0.01, y * freq * 0.01])
            };
            value += sample * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }
}

impl NoiseStrategy for ContinentalnessStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        self.fbm(x, y, detail_level)
    }

    fn name(&self) -> &'static str {
        "Continentalness"
    }
}
