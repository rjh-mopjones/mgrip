use mg_core::{sphere::Point, NoiseStrategy, Sphere};
use noise::{NoiseFn, OpenSimplex};

/// Light from the sub-stellar point: a function of the angle from it on the
/// sphere, warped by noise so the zone edges are ragged (spec 014). The
/// sub-stellar point is the south pole and the anti-stellar point the
/// north, so on the flat map the light runs in bands across it and the
/// terminus is the equator ring.
pub struct LightLevelStrategy {
    noise: OpenSimplex,
}

const SOUTH_POLE: Point = [0.0, 0.0, -1.0];
/// The warps push a place along its meridian, towards or away from the
/// sun, by up to this share of the way from pole to pole: a broad swell and
/// a finer one, each (frequency per world unit, amplitude).
const WARP_BROAD: (f64, f64) = (0.0015, 0.12);
const WARP_FINE: (f64, f64) = (0.005, 0.06);
/// The cosine of the angle from the sun is raised to this power (its sign
/// kept): below 1 the light changes fastest at the terminator. 0.5 gives,
/// on seed 42 by area, about half the world day side, a quarter terminus
/// and a fifth night.
const TERMINATOR_STEEPNESS: f64 = 0.5;

impl LightLevelStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
        }
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
        let sphere = Sphere::MARGIN;
        // The warps move the place along its meridian before its angle from
        // the sun is measured; a push east or west would change nothing.
        let warp = self.warp(x, y, WARP_BROAD.0, 150.0) * WARP_BROAD.1
            + self.warp(x, y, WARP_FINE.0, 200.0) * WARP_FINE.1;
        let warped_y = (y + warp * sphere.height()).clamp(0.0, sphere.height());
        let from_sun = sphere.angle(sphere.point_at(x, warped_y), SOUTH_POLE);

        // Full under the sun, half at the terminator (the equator), none at
        // the anti-stellar point, symmetric about the terminator and steep
        // across it, so the terminus (the zones between light 0.2 and 0.6)
        // is a narrow band round the equator and the night a hemisphere.
        // (The flat model's curve had 0.71 at the terminator, which put the
        // terminus well north of it on the sphere; a plain cosine spread it
        // over 40% of the world.)
        let towards_sun = from_sun.cos();
        let base_light =
            0.5 + 0.5 * towards_sun.signum() * towards_sun.abs().powf(TERMINATOR_STEEPNESS);

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
        LightLevelStrategy::new(42)
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

    #[test]
    fn the_sun_is_overhead_at_the_south_pole_and_the_north_pole_is_dark() {
        let light = strategy();
        for x in [0.0, 256.0, 700.0] {
            assert!(light.generate(x, 512.0, 0) > 0.85, "south pole at x={x}");
            assert!(light.generate(x, 0.0, 0) < 0.1, "north pole at x={x}");
        }
    }

    #[test]
    fn light_falls_from_south_to_north_round_the_whole_world() {
        let light = strategy();
        let mean_at = |y: f64| {
            (0..64)
                .map(|i| light.generate(i as f64 * 16.0, y, 0))
                .sum::<f64>()
                / 64.0
        };
        assert!(mean_at(450.0) > mean_at(300.0));
        assert!(mean_at(300.0) > mean_at(150.0));
        assert!(mean_at(150.0) > mean_at(30.0));
        // The terminus is a band round the equator: half light there.
        let equator = mean_at(256.0);
        assert!(equator > 0.4 && equator < 0.6, "equator {equator}");
    }
}
