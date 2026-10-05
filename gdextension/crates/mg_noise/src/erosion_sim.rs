//! Erosion: uplift raises the land and rivers cut it down.
//!
//! Each step lifts the land, solves drainage over it (`drainage.rs`), then
//! lowers every cell towards the cell it drains to, faster the more water
//! runs through it (the stream power law), and finally lets slopes creep.
//! Valleys, ridges and the river network come out of this together.
//!
//! The lowering is solved implicitly, cell by cell from the sea upwards, so
//! each cell uses the already-lowered height of the cell below it. That is
//! what keeps large time steps stable (Braun and Willett 2013):
//!
//!   h = (h + dt*U + f*h_receiver) / (1 + f),   f = K * dt * flow^m / distance

use crate::drainage::{solve_drainage, step_distance, Drainage, NO_RECEIVER};

pub struct ErosionParams {
    /// How easily average rock is cut (K). Soft rock erodes faster.
    pub erodibility: f64,
    /// Uplift per unit time where tectonic stress is highest.
    pub uplift: f64,
    /// How much more a big river cuts than a small one (m).
    pub flow_exponent: f64,
    pub time_step: f64,
    pub steps: u32,
    /// Share of the way each cell moves towards the mean of its four
    /// neighbours each step: soil creep, which rounds ridges and valley sides.
    pub slope_creep: f64,
}

impl Default for ErosionParams {
    fn default() -> Self {
        Self {
            erodibility: 0.04,
            uplift: 0.015,
            flow_exponent: 0.45,
            time_step: 1.2,
            steps: 100,
            slope_creep: 0.04,
        }
    }
}

pub struct ErosionResult {
    /// The eroded ground, with any remaining hollow filled level.
    pub heightmap: Vec<f64>,
    /// Drainage over the eroded ground: the rivers that cut it.
    pub drainage: Drainage,
    /// Depth of rock removed from each cell.
    pub sediment: Vec<f64>,
}

/// Erode `heightmap`. `is_base_level` cells (the sea) stay fixed and
/// everything drains to them. `rainfall` is the run-off each cell adds.
pub fn simulate_erosion(
    heightmap: &[f64],
    rock_hardness: &[f64],
    tectonic_stress: &[f64],
    is_base_level: &[bool],
    rainfall: &[f64],
    width: usize,
    height: usize,
    params: &ErosionParams,
) -> ErosionResult {
    let total = width * height;
    let mut ground = heightmap.to_vec();
    let mut sediment = vec![0.0f64; total];
    let erodibility: Vec<f64> = rock_hardness
        .iter()
        .map(|&hardness| params.erodibility * (1.5 - hardness))
        .collect();
    let uplift: Vec<f64> = tectonic_stress
        .iter()
        .map(|&stress| params.uplift * stress * stress)
        .collect();
    let dt = params.time_step;

    for _ in 0..params.steps {
        let drainage = solve_drainage(&ground, is_base_level, rainfall, width, height);

        // From the sea upwards, so each cell's receiver is already lowered.
        for &cell in &drainage.order {
            let cell = cell as usize;
            if is_base_level[cell] {
                continue;
            }
            let lifted = ground[cell] + dt * uplift[cell];
            let receiver = drainage.receivers[cell];
            if receiver == NO_RECEIVER {
                ground[cell] = lifted;
                continue;
            }
            let receiver = receiver as usize;
            let cutting = erodibility[cell] * dt * drainage.flow[cell].powf(params.flow_exponent)
                / step_distance(cell, receiver, width);
            let lowered = (lifted + cutting * ground[receiver]) / (1.0 + cutting);
            sediment[cell] += (ground[cell] - lowered).max(0.0);
            ground[cell] = lowered;
        }

        ground = crept(&ground, is_base_level, width, height, params.slope_creep);
    }

    for cell in ground.iter_mut() {
        *cell = cell.clamp(-1.0, 1.0);
    }
    let drainage = solve_drainage(&ground, is_base_level, rainfall, width, height);
    ErosionResult {
        heightmap: drainage.filled.clone(),
        drainage,
        sediment,
    }
}

/// The ground after one step of slope creep: each land cell moves `share` of
/// the way towards the mean of its four neighbours.
fn crept(ground: &[f64], is_base_level: &[bool], width: usize, height: usize, share: f64) -> Vec<f64> {
    (0..width * height)
        .map(|cell| {
            if is_base_level[cell] {
                return ground[cell];
            }
            let (x, y) = (cell % width, cell / width);
            // The map joins east to west; north and south edges repeat.
            let west = ground[y * width + (x + width - 1) % width];
            let east = ground[y * width + (x + 1) % width];
            let north = ground[y.saturating_sub(1) * width + x];
            let south = ground[(y + 1).min(height - 1) * width + x];
            ground[cell] + share * ((west + east + north + south) / 4.0 - ground[cell])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16 by 9 slope rising eastwards from a sea in column 0, with a
    /// groove along the middle row steep enough to gather the water.
    fn grooved_slope() -> (Vec<f64>, Vec<bool>) {
        let (width, height) = (16, 9);
        let mut ground = vec![0.0; width * height];
        let mut sea = vec![false; width * height];
        for y in 0..height {
            for x in 0..width {
                let off_groove = (y as f64 - 4.0).abs() * 0.1;
                ground[y * width + x] = x as f64 * 0.05 + off_groove;
                sea[y * width + x] = x == 0;
            }
        }
        (ground, sea)
    }

    fn erode(steps: u32) -> ErosionResult {
        let (ground, sea) = grooved_slope();
        let params = ErosionParams { steps, uplift: 0.0, ..ErosionParams::default() };
        simulate_erosion(&ground, &[0.5; 144], &[0.0; 144], &sea, &[1.0; 144], 16, 9, &params)
    }

    #[test]
    fn a_river_cuts_a_valley_deeper_than_the_ground_beside_it() {
        let (before, _) = grooved_slope();
        let after = erode(5).heightmap;
        // Middle of the slope: in the groove, and three rows off it.
        let (in_valley, beside) = (4 * 16 + 8, 1 * 16 + 8);

        let cut_in_valley = before[in_valley] - after[in_valley];
        let cut_beside = before[beside] - after[beside];
        assert!(cut_in_valley > cut_beside);
        assert!(after[in_valley] < after[beside]);
    }

    #[test]
    fn the_sea_stays_where_it_is_and_the_result_drains_to_it() {
        let (before, _) = grooved_slope();
        let result = erode(5);

        for y in 0..9 {
            assert_eq!(result.heightmap[y * 16], before[y * 16]);
        }
        let draining = result.drainage.receivers.iter().filter(|&&r| r != NO_RECEIVER).count();
        assert_eq!(draining, 144 - 9);
    }
}
