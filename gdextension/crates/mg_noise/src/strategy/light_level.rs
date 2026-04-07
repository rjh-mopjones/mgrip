use noise::{NoiseFn, OpenSimplex};
use mg_core::NoiseStrategy;

pub struct LightLevelStrategy {
    noise: OpenSimplex,
    sub_stellar_x: f64,
    sub_stellar_y: f64,
    map_width: f64,
    map_height: f64,
}

impl LightLevelStrategy {
    pub fn new(seed: u32, sub_stellar_x: f64, sub_stellar_y: f64, map_width: f64, map_height: f64) -> Self {
        Self { noise: OpenSimplex::new(seed), sub_stellar_x, sub_stellar_y, map_width, map_height }
    }

    pub fn default_for_map(seed: u32) -> Self {
        Self::new(seed, 0.5, 1.0, 1024.0, 512.0)
    }

    fn scatter_noise(&self, x: f64, y: f64) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amp = 0.0;
        for _ in 0..3 {
            value += self.noise.get([x * 0.005 * freq, y * 0.005 * freq]) * amplitude;
            max_amp += amplitude;
            amplitude *= 0.5;
            freq *= 2.0;
        }
        (value / max_amp) * 0.05
    }
}

impl NoiseStrategy for LightLevelStrategy {
    fn generate(&self, x: f64, y: f64, _detail_level: u32) -> f64 {
        let nx = x / self.map_width;
        let ny = y / self.map_height;

        // Two-pass domain warping for irregular climate zone boundaries
        let warp1_x = self.noise.get([x * 0.0015, y * 0.0015 + 50.0]) * 0.12;
        let warp1_y = self.noise.get([x * 0.0015 + 150.0, y * 0.0015]) * 0.12;
        let warp2_x = self.noise.get([x * 0.005, y * 0.005 + 100.0]) * 0.06;
        let warp2_y = self.noise.get([x * 0.005 + 200.0, y * 0.005]) * 0.06;

        // Cylindrical wrapping: shortest horizontal path
        let raw_dx = nx - self.sub_stellar_x + warp1_x + warp2_x;
        let dx = crate::wrap::wrapped_dx_normalized(raw_dx);
        let dy = ny - self.sub_stellar_y + warp1_y + warp2_y;
        let dist = (dx * dx + dy * dy).sqrt().min(1.0);

        // Cosine falloff with extra darkening past dist=0.5
        let far_dist = ((dist - 0.5) / 0.5).max(0.0);
        let darkening = 1.0 + 1.5 * far_dist * far_dist;
        let base_light = (dist * std::f64::consts::FRAC_PI_2).cos().powf(darkening);

        (base_light + self.scatter_noise(x, y)).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "LightLevel"
    }
}
