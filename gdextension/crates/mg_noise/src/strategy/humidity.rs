use mg_core::NoiseStrategy;
use noise::{NoiseFn, OpenSimplex};

pub struct HumidityStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    persistence: f64,
    lacunarity: f64,
    world_width: f64,
}

impl HumidityStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 5,
            frequency: 1.0,
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

    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        for _ in 0..(self.octaves + detail_level) {
            let sample = if self.world_width > 0.0 {
                let [cx, cz, cy] =
                    crate::wrap::cylindrical_noise_coords(x, y, freq, 0.003, self.world_width);
                self.noise.get([cx, cz, cy])
            } else {
                self.noise.get([x * freq * 0.003, y * freq * 0.003])
            };
            value += sample * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }

    /// Terminator ring model — physics-motivated atmospheric circulation for a tidally locked planet.
    ///
    /// Gaussian humidity peak at the terminator (light ≈ 0.2), day-side drying,
    /// night-side cold trap, and continental moisture decay.
    pub fn generate_terminator_model(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        continentalness: f64,
        light_level: f64,
    ) -> f64 {
        let base_noise = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Gaussian peak at terminator (light ≈ 0.2, width 0.22)
        let terminator_peak = (-(light_level - 0.2).powi(2) / (2.0 * 0.22 * 0.22)).exp();

        // Day-side drying (quadratic for light > 0.4)
        let day_drying = if light_level > 0.4 {
            let t = (light_level - 0.4) / 0.6;
            1.0 - t * t * 0.8
        } else {
            1.0
        };

        // Night-side cold trap (light < 0.15)
        let night_trap = if light_level < 0.15 {
            0.15 + (light_level / 0.15) * 0.85
        } else {
            1.0
        };

        // Continental moisture decay
        let moisture_source = if continentalness < -0.01 {
            1.0
        } else if continentalness < 0.05 {
            1.0 - ((continentalness + 0.01) / 0.06) * 0.5
        } else if continentalness < 0.2 {
            0.5 - ((continentalness - 0.05) / 0.15) * 0.3
        } else {
            0.2 - ((continentalness - 0.2) / 0.3).min(1.0) * 0.1
        };

        let atmospheric = terminator_peak * day_drying * night_trap;
        let scaled_moisture = moisture_source * (0.3 + terminator_peak * 0.7);
        (base_noise * 0.2 + scaled_moisture * 0.3 + atmospheric * 0.5).clamp(0.0, 1.0)
    }
}

impl NoiseStrategy for HumidityStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        (self.fbm(x, y, detail_level) + 1.0) * 0.5
    }

    fn name(&self) -> &'static str {
        "Humidity"
    }
}
