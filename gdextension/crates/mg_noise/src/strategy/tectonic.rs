use noise::{NoiseFn, OpenSimplex};
use mg_core::NoiseStrategy;
use std::collections::HashMap;

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
    pub boundary_tangent: (f64, f64),
}

pub struct Plate {
    pub center: (f64, f64),
    pub velocity: (f64, f64),
    pub density: f64,
    pub age: f64,
}

pub struct Hotspot {
    pub pos: (f64, f64),
    pub intensity: f64,
    pub radius: f64,
}

pub struct PlateRegistry {
    pub plates: Vec<Plate>,
    pub hotspots: Vec<Hotspot>,
    cell_to_plate: HashMap<(i32, i32), usize>,
}

impl PlateRegistry {
    pub fn from_seed(seed: u32, plate_scale: f64) -> Self {
        let mut rng_state = seed as u64 ^ 0xDEADBEEF_CAFEBABE;
        let mut next_f64 = move || -> f64 {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            (rng_state & 0xFFFFFFFF) as f64 / 0xFFFFFFFF_u64 as f64
        };

        let world_width = 1024.0;
        let world_height = 512.0;
        let cell_range_x = (world_width * plate_scale).ceil() as i32 + 4;
        let cell_range_y = (world_height * plate_scale).ceil() as i32 + 4;

        let hash = |ix: i32, iy: i32, s: u32| -> (f64, f64) {
            let n = (ix.wrapping_mul(374761393) as u32)
                .wrapping_add((iy.wrapping_mul(668265263)) as u32)
                .wrapping_add(s);
            let n1 = n.wrapping_mul(1103515245).wrapping_add(12345);
            let n2 = n1.wrapping_mul(1103515245).wrapping_add(12345);
            ((n1 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64,
             (n2 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64)
        };

        let mut all_cells: Vec<(i32, i32, f64, f64)> = Vec::new();
        for iy in -2..cell_range_y + 2 {
            for ix in -2..cell_range_x + 2 {
                let (ox, oy) = hash(ix, iy, seed.wrapping_add(2));
                all_cells.push((ix, iy, ix as f64 + ox, iy as f64 + oy));
            }
        }

        let target_count = 25 + (next_f64() * 10.0) as usize;
        let min_dist_sq = { let d = 1.0 / (target_count as f64).sqrt() * 0.5; d * d };

        let mut plates = Vec::new();
        let mut selected_centers: Vec<(f64, f64)> = Vec::new();

        let mut indices: Vec<usize> = (0..all_cells.len()).collect();
        for i in (1..indices.len()).rev() {
            let j = (next_f64() * (i + 1) as f64) as usize % (i + 1);
            indices.swap(i, j);
        }

        for &cell_idx in &indices {
            if plates.len() >= target_count { break; }
            let (_ix, _iy, cx, cy) = all_cells[cell_idx];
            let too_close = selected_centers.iter().any(|&(sx, sy)| {
                let dx = cx - sx; let dy = cy - sy;
                dx * dx + dy * dy < min_dist_sq
            });
            if too_close { continue; }

            let vel_angle = next_f64() * std::f64::consts::TAU;
            let vel_mag = next_f64() * 0.8 + 0.2;
            plates.push(Plate {
                center: (cx, cy),
                velocity: (vel_angle.cos() * vel_mag, vel_angle.sin() * vel_mag),
                density: next_f64(),
                age: next_f64(),
            });
            selected_centers.push((cx, cy));
        }

        let mut cell_to_plate = HashMap::new();
        for &(ix, iy, cx, cy) in &all_cells {
            let mut best_plate = 0usize;
            let mut best_dist = f64::MAX;
            for (pi, plate) in plates.iter().enumerate() {
                let dx = cx - plate.center.0;
                let dy = cy - plate.center.1;
                let d = dx * dx + dy * dy;
                if d < best_dist { best_dist = d; best_plate = pi; }
            }
            cell_to_plate.insert((ix, iy), best_plate);
        }

        let hotspot_count = 1 + (next_f64() * 3.0) as usize;
        let hotspots = (0..hotspot_count).map(|_| Hotspot {
            pos: (next_f64() * world_width, next_f64() * world_height),
            intensity: 0.4 + next_f64() * 0.3,
            radius: 10.0 + next_f64() * 20.0,
        }).collect();

        Self { plates, hotspots, cell_to_plate }
    }

    fn plate_for_cell(&self, ix: i32, iy: i32) -> usize {
        if let Some(&idx) = self.cell_to_plate.get(&(ix, iy)) {
            return idx;
        }
        let cx = ix as f64 + 0.5;
        let cy = iy as f64 + 0.5;
        self.plates.iter().enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (cx - a.center.0).powi(2) + (cy - a.center.1).powi(2);
                let db = (cx - b.center.0).powi(2) + (cy - b.center.1).powi(2);
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

pub struct TectonicPlatesStrategy {
    seed: u32,
    warp1_x: OpenSimplex,
    warp1_y: OpenSimplex,
    warp2_x: OpenSimplex,
    warp2_y: OpenSimplex,
    boundary_perturb: OpenSimplex,
    interior_noise: OpenSimplex,
    plate_scale: f64,
    registry: PlateRegistry,
    world_width: f64,
    cell_period: i32,
}

impl TectonicPlatesStrategy {
    pub fn new(seed: u32) -> Self {
        let plate_scale = 0.004;
        Self {
            seed,
            warp1_x: OpenSimplex::new(seed.wrapping_add(100)),
            warp1_y: OpenSimplex::new(seed.wrapping_add(101)),
            warp2_x: OpenSimplex::new(seed.wrapping_add(200)),
            warp2_y: OpenSimplex::new(seed.wrapping_add(201)),
            boundary_perturb: OpenSimplex::new(seed.wrapping_add(300)),
            interior_noise: OpenSimplex::new(seed.wrapping_add(400)),
            plate_scale,
            registry: PlateRegistry::from_seed(seed, plate_scale),
            world_width: 0.0,
            cell_period: 0,
        }
    }

    pub fn new_wrapping(seed: u32, world_width: f64) -> Self {
        let mut s = Self::new(seed);
        s.world_width = world_width;
        s.cell_period = (world_width * s.plate_scale).round() as i32;
        s
    }

    fn wrap_cell_ix(&self, ix: i32) -> i32 {
        if self.cell_period > 0 {
            ((ix % self.cell_period) + self.cell_period) % self.cell_period
        } else {
            ix
        }
    }

    fn hash(&self, ix: i32, iy: i32) -> (f64, f64) {
        let ix = self.wrap_cell_ix(ix);
        let n = (ix.wrapping_mul(374761393) as u32)
            .wrapping_add((iy.wrapping_mul(668265263)) as u32)
            .wrapping_add(self.seed);
        let n1 = n.wrapping_mul(1103515245).wrapping_add(12345);
        let n2 = n1.wrapping_mul(1103515245).wrapping_add(12345);
        ((n1 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64,
         (n2 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64)
    }

    fn plate_id_hash(&self, ix: i32, iy: i32) -> f64 {
        let ix = self.wrap_cell_ix(ix);
        let n = (ix.wrapping_mul(127) as u32)
            .wrapping_add((iy.wrapping_mul(311)) as u32)
            .wrapping_add(self.seed);
        let n = n.wrapping_mul(1103515245).wrapping_add(12345);
        (n & 0xFF) as f64 / 255.0
    }

    fn warp_coordinates(&self, x: f64, y: f64) -> (f64, f64) {
        let wx = x
            + self.warp1_x.get([x * 0.002, y * 0.002]) * 120.0
            + self.warp2_x.get([x * 0.008, y * 0.008]) * 40.0;
        let wy = y
            + self.warp1_y.get([x * 0.002 + 43.7, y * 0.002 + 17.3]) * 120.0
            + self.warp2_y.get([x * 0.008 + 91.2, y * 0.008 + 55.8]) * 40.0;
        (wx * self.plate_scale, wy * self.plate_scale)
    }

    fn classify_boundary(a: &Plate, b: &Plate, normal: (f64, f64)) -> BoundaryType {
        let rel_vel = (a.velocity.0 - b.velocity.0, a.velocity.1 - b.velocity.1);
        let dot = rel_vel.0 * normal.0 + rel_vel.1 * normal.1;
        if dot > 0.1 {
            match (a.density > 0.5, b.density > 0.5) {
                (true, true) => BoundaryType::Convergent,
                (false, false) => BoundaryType::OceanicSubduction,
                _ => BoundaryType::Subduction,
            }
        } else if dot < -0.1 {
            BoundaryType::Divergent
        } else {
            BoundaryType::Transform
        }
    }

    pub fn generate_full(&self, x: f64, y: f64) -> TectonicSample {
        let (mut sx, sy) = self.warp_coordinates(x, y);

        let cp_f = if self.cell_period > 0 {
            let cp = self.world_width * self.plate_scale;
            sx = ((sx % cp) + cp) % cp;
            cp
        } else {
            0.0
        };

        let ix = sx.floor() as i32;
        let iy = sy.floor() as i32;

        let mut min_dist = f64::MAX;
        let mut second_dist = f64::MAX;
        let mut nearest_cell = (0i32, 0i32);
        let mut second_cell = (0i32, 0i32);
        let mut nearest_center = (0.0f64, 0.0f64);
        let mut second_center = (0.0f64, 0.0f64);

        for dx in -2..=2 {
            for dy in -2..=2 {
                let cell_x = ix + dx;
                let cell_y = iy + dy;
                let (ox, oy) = self.hash(cell_x, cell_y);
                let cx = cell_x as f64 + ox;
                let cy = cell_y as f64 + oy;

                let mut ddx = sx - cx;
                if cp_f > 0.0 {
                    if ddx > cp_f * 0.5 { ddx -= cp_f; }
                    if ddx < -cp_f * 0.5 { ddx += cp_f; }
                }
                let dist = (ddx.powi(2) + (sy - cy).powi(2)).sqrt();

                if dist < min_dist {
                    second_dist = min_dist; second_cell = nearest_cell; second_center = nearest_center;
                    min_dist = dist; nearest_cell = (cell_x, cell_y); nearest_center = (cx, cy);
                } else if dist < second_dist {
                    second_dist = dist; second_cell = (cell_x, cell_y); second_center = (cx, cy);
                }
            }
        }

        let plate_a_idx = self.registry.plate_for_cell(self.wrap_cell_ix(nearest_cell.0), nearest_cell.1);
        let plate_b_idx = self.registry.plate_for_cell(self.wrap_cell_ix(second_cell.0), second_cell.1);
        let plate_id = self.plate_id_hash(nearest_cell.0, nearest_cell.1);

        let f2_minus_f1 = second_dist - min_dist;
        let perturb = self.boundary_perturb.get([x * 0.015, y * 0.015]) * 0.15;
        let perturbed_dist = f2_minus_f1 + perturb;

        let (boundary_type, boundary_tangent) = if plate_a_idx == plate_b_idx {
            (BoundaryType::None, (1.0, 0.0))
        } else {
            let pa = &self.registry.plates[plate_a_idx];
            let pb = &self.registry.plates[plate_b_idx];
            let ndx = pb.center.0 - pa.center.0;
            let ndy = pb.center.1 - pa.center.1;
            let len = (ndx * ndx + ndy * ndy).sqrt().max(0.001);
            let normal = (ndx / len, ndy / len);
            let btype = Self::classify_boundary(pa, pb, normal);
            (btype, (-normal.1, normal.0))
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
        let interior = self.interior_noise.get([sx * 1.5, sy * 1.5]).abs() * 0.25;
        let age_damping = 1.0 - self.registry.plates.get(plate_a_idx).map(|p| p.age).unwrap_or(0.5) * 0.7;
        let stress = (boundary_stress + interior * age_damping).clamp(0.0, 1.0);

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
