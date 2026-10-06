use mg_core::{sphere::Point, NoiseStrategy, Sphere};
use noise::{NoiseFn, OpenSimplex};

/// Light from the sub-stellar point: a function of the angle from it on the
/// sphere, warped by noise so the zone edges are ragged (spec 014). The
/// sub-stellar point sits at the bottom centre of the map, 45° up from the
/// south pole (spec 015), so the terminator is a great circle that arcs
/// across the flat map: north of the equator in the middle, south of it at
/// the edges. Night is deepest at the map's top edges, day hottest at its
/// bottom centre.
pub struct LightLevelStrategy {
    noise: OpenSimplex,
}

/// Where the sun stands: its latitude, and its longitude as a world
/// position (the middle of the map). −90° would be spec 014's pole sun,
/// whose light runs in bands across the map.
pub const SUN_LATITUDE_DEGREES: f64 = -45.0;
pub const SUN_LONGITUDE_WU: f64 = mg_core::sphere::WORLD_WIDTH / 2.0;
/// The warps move a place along the ground before its angle from the sun
/// is measured, so the zones' edges are ragged: a broad swell and a finer
/// one, each (frequency per world unit, reach in world units).
const WARP_BROAD: (f64, f64) = (0.0015, 61.0);
const WARP_FINE: (f64, f64) = (0.005, 31.0);
/// The cosine of the angle from the sun is raised to this power (its sign
/// kept): below 1 the light changes fastest at the terminator, so the
/// terminus (the zones between light 0.2 and 0.6) is a narrow band.
const TERMINATOR_STEEPNESS: f64 = 0.5;

/// The sub-stellar point on the unit sphere.
pub fn sun() -> Point {
    let sphere = Sphere::MARGIN;
    let south_of_pole = (90.0 - SUN_LATITUDE_DEGREES) / 180.0 * sphere.height();
    sphere.point_at(SUN_LONGITUDE_WU, south_of_pole)
}

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
        let east = self.warp(x, y, WARP_BROAD.0, 50.0) * WARP_BROAD.1
            + self.warp(x, y, WARP_FINE.0, 100.0) * WARP_FINE.1;
        let south = self.warp(x, y, WARP_BROAD.0, 150.0) * WARP_BROAD.1
            + self.warp(x, y, WARP_FINE.0, 200.0) * WARP_FINE.1;
        let warped = sphere.moved(sphere.point_at(x, y), east, south);
        let from_sun = sphere.angle(warped, sun());

        // Full under the sun, half at the terminator, none at the
        // anti-stellar point, symmetric about the terminator and steep
        // across it.
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
    fn the_sun_stands_over_the_bottom_centre_and_night_lies_at_the_top_edges() {
        let light = strategy();
        // Under the sun: longitude 512, 45° south.
        assert!(light.generate(512.0, 384.0, 0) > 0.85);
        // Opposite it: the map's edge, 45° north.
        assert!(light.generate(0.0, 128.0, 0) < 0.15);
        // Both poles are ordinary places now: the north one dim, the south
        // one bright, neither extreme.
        assert!(light.generate(300.0, 0.0, 0) < 0.3);
        assert!(light.generate(300.0, 512.0, 0) > 0.7);
    }

    #[test]
    fn the_terminus_arcs_across_the_map() {
        let light = strategy();
        // Where light crosses one half, going down a column: north of the
        // equator in the middle of the map, south of it at the edges.
        let terminator_row = |x: f64| {
            (0..512)
                .map(|row| row as f64)
                .find(|&y| light.generate(x, y, 0) >= 0.5)
                .expect("light reaches a half")
        };
        let middle = terminator_row(512.0);
        let edge = terminator_row(0.0);
        assert!(
            middle < 256.0 && edge > 256.0,
            "middle {middle}, edge {edge}"
        );
        // Not stripes: the two differ by a good part of the map's height.
        assert!(edge - middle > 150.0, "middle {middle}, edge {edge}");
    }
}
