use mg_core::NoiseStrategy;
use noise::{NoiseFn, OpenSimplex};

/// The warp and scatter noise below is sampled on a flat plane, so on its own
/// it does not join up where the world wraps east to west. Over this many
/// world units before the east edge it is crossfaded into the noise from just
/// beyond the west edge, which makes it continuous across the seam and leaves
/// the rest of the world untouched. The GPU shader does the same
/// (`LIGHT_LEVEL_FUNCS` in `gpu/pipelines.rs`).
pub const SEAM_BLEND_WU: f64 = 64.0;

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

    /// `sample(x)` made continuous across the east-west seam. `x` must already
    /// be wrapped into `0..map_width`.
    fn seamless(&self, x: f64, sample: impl Fn(f64) -> f64) -> f64 {
        let blend_start = self.map_width - SEAM_BLEND_WU;
        if x <= blend_start {
            return sample(x);
        }
        let t = (x - blend_start) / SEAM_BLEND_WU;
        let weight = t * t * (3.0 - 2.0 * t);
        // The same place, counted one lap to the west.
        sample(x) * (1.0 - weight) + sample(x - self.map_width) * weight
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
        let x = crate::wrap::wrap_x(x, self.map_width);
        let nx = x / self.map_width;
        let ny = y / self.map_height;

        // Two-pass domain warping for irregular climate zone boundaries
        let noise = &self.noise;
        let warp1_x = self.seamless(x, |x| noise.get([x * 0.0015, y * 0.0015 + 50.0])) * 0.12;
        let warp1_y = self.seamless(x, |x| noise.get([x * 0.0015 + 150.0, y * 0.0015])) * 0.12;
        let warp2_x = self.seamless(x, |x| noise.get([x * 0.005, y * 0.005 + 100.0])) * 0.06;
        let warp2_y = self.seamless(x, |x| noise.get([x * 0.005 + 200.0, y * 0.005])) * 0.06;

        // Cylindrical wrapping: shortest horizontal path
        let raw_dx = nx - self.sub_stellar_x + warp1_x + warp2_x;
        let dx = crate::wrap::wrapped_dx_normalized(raw_dx);
        let dy = ny - self.sub_stellar_y + warp1_y + warp2_y;
        let dist = (dx * dx + dy * dy).sqrt().min(1.0);

        // Cosine falloff with extra darkening past dist=0.5
        let far_dist = ((dist - 0.5) / 0.5).max(0.0);
        let darkening = 1.0 + 1.5 * far_dist * far_dist;
        let base_light = (dist * std::f64::consts::FRAC_PI_2).cos().powf(darkening);

        let scatter = self.seamless(x, |x| self.scatter_noise(x, y));
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

        assert_eq!(light.generate(300.0, 200.0, 0), light.generate(300.0 + WORLD_WIDTH, 200.0, 0));
        assert_eq!(light.generate(1000.0, 200.0, 0), light.generate(1000.0 - WORLD_WIDTH, 200.0, 0));
    }

    #[test]
    fn noise_is_only_blended_close_to_the_east_edge() {
        let light = strategy();
        let identity = |x: f64| x;

        // Untouched up to the start of the blend band.
        assert_eq!(light.seamless(500.0, identity), 500.0);
        assert_eq!(light.seamless(WORLD_WIDTH - SEAM_BLEND_WU, identity), WORLD_WIDTH - SEAM_BLEND_WU);
        // Halfway through the band: an even mix of here and one lap west.
        let halfway = WORLD_WIDTH - SEAM_BLEND_WU / 2.0;
        assert_eq!(light.seamless(halfway, identity), (halfway + (halfway - WORLD_WIDTH)) / 2.0);
    }
}
