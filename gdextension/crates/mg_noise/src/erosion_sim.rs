//! Stream power erosion simulation — implicit scheme, unconditionally stable.
//!
//! h_new = (h_old + dt*U + F*h_new_receiver) / (1 + F)
//! where F = dt * K * A^m / dx

use crate::rivers::{D8_DISTANCES, D8_OFFSETS, NO_FLOW};

pub struct ErosionParams {
    pub k_base: f64,
    pub u_base: f64,
    pub m: f64,
    pub dt: f64,
    pub iterations: u32,
    pub dx: f64,
    pub talus_angle: f64,
    pub thermal_rate: f64,
    pub sea_level: f64,
}

impl Default for ErosionParams {
    fn default() -> Self {
        Self {
            k_base: 0.04,
            u_base: 0.015,
            m: 0.45,
            dt: 1.0,
            iterations: 120,
            dx: 1.0,
            talus_angle: 0.5,
            thermal_rate: 0.3,
            sea_level: -0.01,
        }
    }
}

pub struct ErosionResult {
    pub heightmap: Vec<f64>,
    pub drainage_area: Vec<u32>,
    pub sediment: Vec<f64>,
}

fn compute_d8_flow(elevation: &[f64], width: usize, height: usize) -> Vec<u8> {
    let total = width * height;
    let mut flow_dir = vec![NO_FLOW; total];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let h = elevation[idx];
            let mut best_slope = 0.0;
            let mut best_dir = NO_FLOW;

            for (d, &(dx, dy)) in D8_OFFSETS.iter().enumerate() {
                let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width);
                let ny = y as i32 + dy;
                if ny < 0 || ny >= height as i32 {
                    continue;
                }
                let nidx = ny as usize * width + nx as usize;
                let slope = (h - elevation[nidx]) / D8_DISTANCES[d];
                if slope > best_slope {
                    best_slope = slope;
                    best_dir = d as u8;
                }
            }
            flow_dir[idx] = best_dir;
        }
    }
    flow_dir
}

fn receiver_index(idx: usize, dir: u8, width: usize, height: usize) -> Option<usize> {
    if dir == NO_FLOW {
        return None;
    }
    let (dx, dy) = D8_OFFSETS[dir as usize];
    let x = (idx % width) as i32 + dx;
    let y = (idx / width) as i32 + dy;
    if y < 0 || y >= height as i32 {
        return None;
    }
    Some(y as usize * width + crate::wrap::wrap_grid_x(x, width) as usize)
}

fn compute_flow_accumulation(
    flow_dir: &[u8],
    elevation: &[f64],
    width: usize,
    height: usize,
) -> Vec<u32> {
    let total = width * height;
    let mut acc = vec![1u32; total];

    let mut order: Vec<usize> = (0..total).collect();
    order.sort_unstable_by(|&a, &b| {
        elevation[b]
            .partial_cmp(&elevation[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for &idx in &order {
        let dir = flow_dir[idx];
        if dir == NO_FLOW {
            continue;
        }
        if let Some(recv) = receiver_index(idx, dir, width, height) {
            acc[recv] += acc[idx];
        }
    }
    acc
}

fn fill_depressions(
    elevation: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
    _extra: Option<()>,
) -> Vec<f64> {
    crate::rivers::fill_depressions(elevation, width, height, sea_level, None)
}

pub fn simulate_erosion(
    heightmap: &[f64],
    rock_hardness: &[f64],
    tectonic_stress: &[f64],
    continentalness: &[f64],
    width: usize,
    height: usize,
    params: &ErosionParams,
) -> ErosionResult {
    let total = width * height;
    let mut h = heightmap.to_vec();
    let mut sediment = vec![0.0f64; total];

    let erodibility: Vec<f64> = rock_hardness
        .iter()
        .map(|&r| params.k_base * (1.5 - r))
        .collect();
    let uplift: Vec<f64> = tectonic_stress
        .iter()
        .map(|&s| params.u_base * s * s)
        .collect();

    let mut flow_dir;
    let mut accumulation;

    for iter in 0..params.iterations {
        if iter % 5 == 0 {
            h = fill_depressions(&h, width, height, params.sea_level, None);
        }

        flow_dir = compute_d8_flow(&h, width, height);
        accumulation = compute_flow_accumulation(&flow_dir, &h, width, height);

        let mut sorted: Vec<usize> = (0..total).collect();
        sorted.sort_by(|&a, &b| h[a].partial_cmp(&h[b]).unwrap_or(std::cmp::Ordering::Equal));

        for &idx in &sorted {
            if continentalness[idx] < params.sea_level {
                continue;
            }

            let dir = flow_dir[idx];
            if dir == NO_FLOW {
                h[idx] += params.dt * uplift[idx];
                continue;
            }

            let Some(recv_idx) = receiver_index(idx, dir, width, height) else {
                h[idx] += params.dt * uplift[idx];
                continue;
            };

            let k = erodibility[idx];
            let a = accumulation[idx] as f64;
            let f = params.dt * k * a.powf(params.m) / params.dx;

            let h_old = h[idx];
            let h_recv = h[recv_idx];
            let h_new = (h_old + params.dt * uplift[idx] + f * h_recv) / (1.0 + f);

            sediment[idx] += (h_old - h_new).max(0.0);
            h[idx] = h_new;
        }

        // Thermal erosion every 3 iterations
        if iter % 3 == 0 {
            let mut thermal_sorted: Vec<usize> = (0..total)
                .filter(|&i| continentalness[i] >= params.sea_level)
                .collect();
            thermal_sorted
                .sort_by(|&a, &b| h[b].partial_cmp(&h[a]).unwrap_or(std::cmp::Ordering::Equal));

            for &idx in &thermal_sorted {
                let x = idx % width;
                let y = idx / width;
                for (d, &(dx, dy)) in D8_OFFSETS.iter().enumerate() {
                    let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width);
                    let ny = y as i32 + dy;
                    if ny < 0 || ny >= height as i32 {
                        continue;
                    }
                    let nidx = ny as usize * width + nx as usize;
                    let slope = (h[idx] - h[nidx]) / (D8_DISTANCES[d] * params.dx);
                    if slope > params.talus_angle {
                        let transfer = (slope - params.talus_angle)
                            * D8_DISTANCES[d]
                            * params.dx
                            * params.thermal_rate
                            * 0.5;
                        h[idx] -= transfer;
                        h[nidx] += transfer;
                        sediment[nidx] += transfer;
                    }
                }
            }
        }
    }

    for v in h.iter_mut() {
        *v = v.clamp(-1.0, 1.0);
    }

    h = fill_depressions(&h, width, height, params.sea_level, None);
    flow_dir = compute_d8_flow(&h, width, height);
    accumulation = compute_flow_accumulation(&flow_dir, &h, width, height);

    ErosionResult {
        heightmap: h,
        drainage_area: accumulation,
        sediment,
    }
}
