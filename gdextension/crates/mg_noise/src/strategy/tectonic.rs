//! Tectonic plates on the sphere (spec 014, stage 4).
//!
//! The plates are a Voronoi of seeds on the sphere, nearest by angle. Each
//! plate rotates about its own axis (an Euler pole, which is how plates
//! move), so the relative motion at a boundary, and whether the two sides
//! converge, pull apart or slide past, follows from the two rotations.
//! Stress falls off with the distance from the nearest boundary. The sample
//! point is first warped along the ground by two octaves of noise, so
//! boundaries wander, and the boundary distance is roughened by a third.
//!
//! Distances are in the lattice unit of the flat model this replaces, so
//! its falloffs and thresholds hold.

use mg_core::{sphere::Point, NoiseStrategy, Sphere};
use noise::{NoiseFn, OpenSimplex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryType {
    None,
    Convergent,
    Subduction,
    OceanicSubduction,
    Divergent,
    Transform,
}

#[derive(Clone, Copy, Debug)]
pub struct TectonicSample {
    pub plate_id: f64,
    pub boundary_distance: f64,
    pub stress: f64,
    pub boundary_type: BoundaryType,
    pub volcanism: f64,
    /// Along the boundary, as east and south components on the map.
    pub boundary_tangent: (f64, f64),
}

pub struct Plate {
    /// Where the plate is seeded: the point every cell nearest it belongs to.
    pub seed: Point,
    /// The plate's rotation: its axis scaled by its rate. Its velocity at a
    /// point `p` is this crossed with `p`.
    pub rotation: Point,
    pub density: f64,
    pub age: f64,
}

/// How many plates the sphere is cut into. The flat model had one Voronoi
/// cell per `LATTICE_WU` squared, about eight over the sphere's area; a
/// few more, so that no plate spans a quarter of the world.
const PLATE_COUNT_LEAST: usize = 10;
const PLATE_COUNT_SPREAD: usize = 4;
/// Seeds are at least this far apart, in radians.
const PLATE_LEAST_SEPARATION: f64 = 0.45;
/// The flat model's lattice unit, in world units. Boundary distances are
/// measured in these.
const LATTICE_WU: f64 = 1.0 / 0.0049;
/// The warps that move a sample along the ground before the plates are
/// looked up: (frequency per world unit, reach in world units).
const WARP_BROAD: (f64, f64) = (0.002, 120.0);
const WARP_FINE: (f64, f64) = (0.008, 40.0);
/// The boundary distance is roughened by noise of this frequency and size.
const BOUNDARY_PERTURB: (f64, f64) = (0.015, 0.15);
/// Stress inside a plate, away from its edges, from noise.
const INTERIOR_FREQUENCY: f64 = 1.5 * 0.0049;
const INTERIOR_STRESS: f64 = 0.25;

pub struct TectonicPlatesStrategy {
    seed: u32,
    warp1_x: OpenSimplex,
    warp1_y: OpenSimplex,
    warp2_x: OpenSimplex,
    warp2_y: OpenSimplex,
    boundary_perturb: OpenSimplex,
    interior_noise: OpenSimplex,
    pub plates: Vec<Plate>,
}

impl TectonicPlatesStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            warp1_x: OpenSimplex::new(seed.wrapping_add(100)),
            warp1_y: OpenSimplex::new(seed.wrapping_add(101)),
            warp2_x: OpenSimplex::new(seed.wrapping_add(200)),
            warp2_y: OpenSimplex::new(seed.wrapping_add(201)),
            boundary_perturb: OpenSimplex::new(seed.wrapping_add(300)),
            interior_noise: OpenSimplex::new(seed.wrapping_add(400)),
            plates: seed_plates(seed),
        }
    }

    /// The sample point moved along the ground by the warps.
    fn warped(&self, x: f64, y: f64) -> Point {
        let sphere = Sphere::MARGIN;
        let point = sphere.point_at(x, y);
        let sample = |noise: &OpenSimplex, (frequency, reach): (f64, f64), shift: f64| {
            let [px, py, pz] = sphere.noise_point_at(x, y, frequency);
            noise.get([px + shift, py + shift, pz]) * reach
        };
        let east_wu =
            sample(&self.warp1_x, WARP_BROAD, 0.0) + sample(&self.warp2_x, WARP_FINE, 0.0);
        let south_wu =
            sample(&self.warp1_y, WARP_BROAD, 43.7) + sample(&self.warp2_y, WARP_FINE, 91.2);
        let (east, south) = tangents(point);
        let radius = sphere.radius();
        normalised(add(
            point,
            add(
                scaled(east, east_wu / radius),
                scaled(south, south_wu / radius),
            ),
        ))
    }

    /// The nearest plate and the next nearest, with the angle to each.
    fn nearest_two(&self, point: Point) -> ((usize, f64), (usize, f64)) {
        let (mut first, mut second) = ((0, f64::MAX), (0, f64::MAX));
        for (index, plate) in self.plates.iter().enumerate() {
            let angle = angle_between(point, plate.seed);
            if angle < first.1 {
                second = first;
                first = (index, angle);
            } else if angle < second.1 {
                second = (index, angle);
            }
        }
        (first, second)
    }

    fn classify_boundary(a: &Plate, b: &Plate, at: Point, normal: Point) -> BoundaryType {
        let relative = cross(sub(a.rotation, b.rotation), at);
        let closing = dot(relative, normal);
        if closing > 0.1 {
            match (a.density > 0.5, b.density > 0.5) {
                (true, true) => BoundaryType::Convergent,
                (false, false) => BoundaryType::OceanicSubduction,
                _ => BoundaryType::Subduction,
            }
        } else if closing < -0.1 {
            BoundaryType::Divergent
        } else {
            BoundaryType::Transform
        }
    }

    pub fn generate_full(&self, x: f64, y: f64) -> TectonicSample {
        let sphere = Sphere::MARGIN;
        let lattice = LATTICE_WU / sphere.radius();
        let at = self.warped(x, y);
        let ((nearest, first), (next, second)) = self.nearest_two(at);
        let plate_id = ((nearest * 7919 + self.seed as usize) % 251) as f64 / 251.0;

        let perturb = self
            .boundary_perturb
            .get(sphere.noise_point_at(x, y, BOUNDARY_PERTURB.0))
            * BOUNDARY_PERTURB.1;
        let perturbed_dist = (second - first) / lattice + perturb;

        let (boundary_type, boundary_tangent) = if self.plates.len() < 2 {
            (BoundaryType::None, (1.0, 0.0))
        } else {
            let (a, b) = (&self.plates[nearest], &self.plates[next]);
            // The boundary's normal: along the ground towards the other plate.
            let towards = sub(b.seed, scaled(at, dot(at, b.seed)));
            let normal = normalised(towards);
            let btype = Self::classify_boundary(a, b, at, normal);
            let tangent = cross(at, normal);
            let (east, south) = tangents(at);
            (btype, (dot(tangent, east), dot(tangent, south)))
        };

        let (intensity, falloff) = match boundary_type {
            BoundaryType::Convergent => (1.0, 5.0),
            BoundaryType::Subduction => (0.8, 4.5),
            BoundaryType::OceanicSubduction => (0.7, 4.0),
            BoundaryType::Divergent => (0.4, 7.0),
            BoundaryType::Transform => (0.25, 9.0),
            BoundaryType::None => (0.0, 5.0),
        };

        let boundary_stress = intensity * (-perturbed_dist.abs() * falloff).exp();
        let interior = self
            .interior_noise
            .get(sphere.noise_point_at(x, y, INTERIOR_FREQUENCY))
            .abs()
            * INTERIOR_STRESS;
        let age_damping = 1.0 - self.plates[nearest].age * 0.7;
        let raw_stress = (boundary_stress + interior * age_damping).clamp(0.0, 1.0);
        // Compress mid-strength tectonic influence so only sharper boundary zones
        // stay dominant in downstream relief and biome classification.
        let stress = raw_stress.powf(1.35);

        TectonicSample {
            plate_id,
            boundary_distance: 1.0 - stress,
            stress,
            boundary_type,
            volcanism: 0.0,
            boundary_tangent,
        }
    }
}

impl NoiseStrategy for TectonicPlatesStrategy {
    fn generate(&self, x: f64, y: f64, _detail_level: u32) -> f64 {
        self.generate_full(x, y).boundary_distance
    }

    fn name(&self) -> &'static str {
        "Tectonic"
    }
}

/// The plates for a seed: seeds spread over the sphere, no two closer than
/// `PLATE_LEAST_SEPARATION`, each with its own rotation, density and age.
fn seed_plates(seed: u32) -> Vec<Plate> {
    let mut state = seed as u64 ^ 0xDEADBEEF_CAFEBABE;
    let mut next = move || -> f64 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state & 0xFFFFFFFF) as f64 / 0xFFFFFFFF_u64 as f64
    };
    // A point spread evenly over the sphere: any height, any longitude.
    let mut random_point = |next: &mut dyn FnMut() -> f64| -> Point {
        let z = next() * 2.0 - 1.0;
        let longitude = next() * std::f64::consts::TAU;
        let flat = (1.0 - z * z).sqrt();
        [flat * longitude.cos(), flat * longitude.sin(), z]
    };
    let count = PLATE_COUNT_LEAST + (next() * PLATE_COUNT_SPREAD as f64) as usize;
    let mut plates: Vec<Plate> = Vec::with_capacity(count);
    let mut attempts = 0;
    while plates.len() < count && attempts < 10_000 {
        attempts += 1;
        let candidate = random_point(&mut next);
        let too_close = plates
            .iter()
            .any(|plate| angle_between(candidate, plate.seed) < PLATE_LEAST_SEPARATION);
        if too_close {
            continue;
        }
        let axis = random_point(&mut next);
        let rate = next() * 0.8 + 0.2;
        plates.push(Plate {
            seed: candidate,
            rotation: scaled(axis, rate),
            density: next(),
            age: next(),
        });
    }
    plates
}

/// The directions east and south along the ground at a point. At a pole,
/// where there is no east, the x axis stands in.
fn tangents(point: Point) -> (Point, Point) {
    let flat = (point[0] * point[0] + point[1] * point[1]).sqrt();
    let east = if flat < 1e-9 {
        [1.0, 0.0, 0.0]
    } else {
        [-point[1] / flat, point[0] / flat, 0.0]
    };
    let north = cross(point, east);
    (east, scaled(north, -1.0))
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scaled(a: Point, by: f64) -> Point {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalised(a: Point) -> Point {
    let length = dot(a, a).sqrt();
    if length < 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        scaled(a, 1.0 / length)
    }
}

fn angle_between(a: Point, b: Point) -> f64 {
    let c = cross(a, b);
    dot(c, c).sqrt().atan2(dot(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plates_are_seeded_apart_and_rotate() {
        let plates = seed_plates(42);
        assert!(plates.len() >= PLATE_COUNT_LEAST);
        for (i, a) in plates.iter().enumerate() {
            for b in &plates[i + 1..] {
                assert!(angle_between(a.seed, b.seed) >= PLATE_LEAST_SEPARATION);
            }
            assert!(dot(a.rotation, a.rotation).sqrt() >= 0.2);
        }
    }

    #[test]
    fn the_field_is_seamless_and_one_at_each_pole() {
        let tectonic = TectonicPlatesStrategy::new(42);
        for y in [20.0, 256.0, 480.0] {
            let east = tectonic.generate_full(1023.999, y);
            let west = tectonic.generate_full(0.0, y);
            assert!((east.boundary_distance - west.boundary_distance).abs() < 1e-3);
            assert_eq!(east.plate_id, west.plate_id);
        }
        // Every column meets at the pole, so the pole is one place.
        let at_pole: Vec<f64> = [0.0, 300.0, 700.0]
            .iter()
            .map(|&x| tectonic.generate_full(x, 0.0).stress)
            .collect();
        assert!((at_pole[0] - at_pole[1]).abs() < 1e-9 && (at_pole[1] - at_pole[2]).abs() < 1e-9);
    }

    #[test]
    fn there_are_boundaries_and_quiet_interiors() {
        let tectonic = TectonicPlatesStrategy::new(42);
        let stresses: Vec<f64> = (0..64)
            .flat_map(|i| (0..32).map(move |j| (i as f64 * 16.0, j as f64 * 16.0)))
            .map(|(x, y)| tectonic.generate_full(x, y).stress)
            .collect();
        let highest = stresses.iter().copied().fold(0.0, f64::max);
        let lowest = stresses.iter().copied().fold(1.0, f64::min);
        assert!(highest > 0.5, "highest stress {highest}");
        assert!(lowest < 0.1, "lowest stress {lowest}");
    }
}
