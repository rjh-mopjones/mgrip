use mg_core::{NoiseStrategy, Sphere};
use noise::{NoiseFn, OpenSimplex};

/// Light from the sub-stellar point, with noise warps so the zone edges are
/// ragged. The noise is sampled on the sphere, so it is seamless (spec 014).
/// The distance itself is still measured on the flat sheet; spec 014 stage 2
/// measures it on the sphere.
pub struct LightLevelStrategy {
    noise: OpenSimplex,
    sub_stellar_x: f64,
    sub_stellar_y: f64,
    map_width: f64,
    map_height: f64,
}

impl LightLevelStrategy {
    pub fn new(
        seed: u32,
        sub_stellar_x: f64,
        sub_stellar_y: f64,
        map_width: f64,
        map_height: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            sub_stellar_x,
            sub_stellar_y,
            map_width,
            map_height,
        }
    }

    pub fn default_for_map(seed: u32) -> Self {
        Self::new(seed, 0.5, 1.0, 1024.0, 512.0)
    }

    /// One noise sample at `frequency` cycles per world unit; `shift` picks
    /// an unrelated pattern.
    fn warp(&self, x: f64, y: f64, frequency: f64, shift: f64) -> f64 {
        let [px, py, pz] = Sphere::MARGIN.noise_point_at(x, y, frequency);
        self.noise.get([px + shift, py, pz])
    }

    fn scatter_noise(&self, x: f64, y: f64) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amp = 0.0;
        for _ in 0..3 {
            value += self.warp(x, y, 0.005 * freq, 0.0) * amplitude;
            max_amp += amplitude;
            amplitude *= 0.5;
            freq *= 2.0;
        }
        (value / max_amp) * 0.05
    }
}

impl NoiseStrategy for LightLevelStrategy {
    fn generate(&self, x: f64, y: f64, _detail_level: u32) -> f64 {
        let x = x.rem_euclid(self.map_width);
        let nx = x / self.map_width;
        let ny = y / self.map_height;

        // Two-pass domain warping for irregular climate zone boundaries
        let warp1_x = self.warp(x, y, 0.0015, 50.0) * 0.12;
        let warp1_y = self.warp(x, y, 0.0015, 150.0) * 0.12;
        let warp2_x = self.warp(x, y, 0.005, 100.0) * 0.06;
        let warp2_y = self.warp(x, y, 0.005, 200.0) * 0.06;

        // The short way round, east to west.
        let mut dx = nx - self.sub_stellar_x + warp1_x + warp2_x;
        if dx > 0.5 {
            dx -= 1.0;
        } else if dx < -0.5 {
            dx += 1.0;
        }
        let dy = ny - self.sub_stellar_y + warp1_y + warp2_y;
        let dist = (dx * dx + dy * dy).sqrt().min(1.0);

        // Cosine falloff with extra darkening past dist=0.5
        let far_dist = ((dist - 0.5) / 0.5).max(0.0);
        let darkening = 1.0 + 1.5 * far_dist * far_dist;
        let base_light = (dist * std::f64::consts::FRAC_PI_2).cos().powf(darkening);

        let scatter = self.scatter_noise(x, y);
        (base_light + scatter).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "LightLevel"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLD_WIDTH: f64 = 1024.0;

    fn strategy() -> LightLevelStrategy {
        LightLevelStrategy::new(42, 0.5, 1.0, WORLD_WIDTH, 512.0)
    }

    #[test]
    fn light_level_is_continuous_across_the_east_west_seam() {
        let light = strategy();
        for y in [10.0, 150.0, 240.0, 330.0, 500.0] {
            let east_edge = light.generate(WORLD_WIDTH - 0.001, y, 0);
            let west_edge = light.generate(0.0, y, 0);
            assert!(
                (east_edge - west_edge).abs() < 0.001,
                "seam at y={y}: {east_edge} vs {west_edge}"
            );
        }
    }

    #[test]
    fn a_position_one_lap_round_has_the_same_light_level() {
        let light = strategy();

        assert_eq!(
            light.generate(300.0, 200.0, 0),
            light.generate(300.0 + WORLD_WIDTH, 200.0, 0)
        );
        assert_eq!(
            light.generate(1000.0, 200.0, 0),
            light.generate(1000.0 - WORLD_WIDTH, 200.0, 0)
        );
    }
}
