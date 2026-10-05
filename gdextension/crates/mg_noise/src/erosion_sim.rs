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

use crate::drainage::{solve_drainage, step_distance, Drainage, LAKE_MIN_DEPTH, NO_RECEIVER};

pub struct ErosionParams {
    /// How easily average rock is cut (K). Soft rock erodes faster.
    pub erodibility: f64,
    /// Uplift per unit time where `Land::uplift_share` is 1.
    pub uplift: f64,
    /// How much more a big river cuts than a small one (m).
    pub flow_exponent: f64,
    pub time_step: f64,
    /// Share of the way each cell moves towards the mean of its four
    /// neighbours each step: soil creep, which rounds ridges and valley sides.
    pub slope_creep: f64,
    /// Height at which uplift stops where the crust is pushed up hardest:
    /// the higher the land stands, the less it is lifted, as the crust sags
    /// under its load. Land pushed up less hard stops lower (see
    /// `CEILING_FULL_AT_SHARE`). This is what bounds land where little water
    /// runs to cut it down, and there it leaves the pattern of the uplift
    /// standing: ranges where the push is strong, low ground where it is not.
    pub uplift_limit: f64,
    /// How far an ice stream pulls the ground beside it down towards its own
    /// level each step, from 0 (not at all) to 1 (all the way). Ice does not
    /// keep to a channel as water does: it widens its valley into a trough.
    pub ice_widening: f64,
}

impl Default for ErosionParams {
    /// Chosen by eye in the erosion sandbox (2026-10-05).
    fn default() -> Self {
        Self {
            erodibility: 0.021,
            uplift: 0.008,
            flow_exponent: 0.46,
            time_step: 2.0,
            slope_creep: 0.08,
            uplift_limit: 1.0,
            ice_widening: 0.25,
        }
    }
}

/// Land lifted at this share of the uplift rate, or more, can rise to the
/// full `uplift_limit`; land lifted less stops proportionally lower, down to
/// `CEILING_LEAST_SHARE` of it.
const CEILING_FULL_AT_SHARE: f64 = 1.2;
const CEILING_LEAST_SHARE: f64 = 0.15;
/// Sinking ground stops sinking at this height (30 blocks below the sea),
/// slowing over the last `RIFT_FLOOR_EASE` above it.
const RIFT_FLOOR: f64 = -0.15;
const RIFT_FLOOR_EASE: f64 = 0.1;
/// Land with at least this much run-off has no ceiling but the full limit.
const CEILING_LIFTED_BY_RUNOFF: f64 = 0.8;
/// Run-off gathered from this many square world units of well-watered land
/// makes a full-strength ice stream.
const ICE_STREAM_AREA_WU2: f64 = 12.0;

/// The land being eroded: what does not change from step to step.
pub struct Land<'a> {
    /// The sea: fixed, and everything drains to it.
    pub is_base_level: &'a [bool],
    pub rock_hardness: &'a [f64],
    /// How fast each cell is lifted, as a share of `ErosionParams::uplift`.
    pub uplift_share: &'a [f64],
    /// Run-off each cell adds.
    pub rainfall: &'a [f64],
    /// How far each cell is under ice, from 0 (none) to 1 (ice sheet).
    pub ice: &'a [f64],
    /// What a cell of open lake loses to the air each step.
    pub lake_evaporation: &'a [f64],
    pub width: usize,
    pub height: usize,
}

/// One step of uplift and erosion on `ground`. Adds the rock removed to
/// `sediment`. Returns the drainage the step was cut with. `step` is the
/// number of the step, so each routes its water a little differently.
pub fn erosion_step(
    ground: &mut Vec<f64>,
    sediment: &mut [f64],
    land: &Land,
    params: &ErosionParams,
    step: u32,
) -> Drainage {
    let dt = params.time_step;
    let drainage = solve_drainage(
        ground,
        land.is_base_level,
        land.rainfall,
        land.lake_evaporation,
        land.width,
        land.height,
        step,
    );

    // From the sea upwards, so each cell's receiver is already lowered.
    for &cell in &drainage.order {
        let cell = cell as usize;
        if land.is_base_level[cell] {
            continue;
        }
        // Well-watered land is kept down by its rivers and may rise to the
        // full limit. Dry and frozen land has only the ceiling to stop it.
        let wetness = (land.rainfall[cell] / CEILING_LIFTED_BY_RUNOFF).min(1.0);
        let by_uplift = (land.uplift_share[cell] / CEILING_FULL_AT_SHARE).clamp(CEILING_LEAST_SHARE, 1.0);
        let ceiling = params.uplift_limit * (by_uplift + (1.0 - by_uplift) * wetness);
        let share = land.uplift_share[cell];
        // Sinking ground (a negative share) sinks whatever its height, until
        // it nears the floor of the rift.
        let room_to_rise = if share > 0.0 {
            (1.0 - ground[cell] / ceiling).clamp(0.0, 1.0)
        } else {
            ((ground[cell] - RIFT_FLOOR) / RIFT_FLOOR_EASE).clamp(0.0, 1.0)
        };
        let lifted = ground[cell] + dt * params.uplift * share * room_to_rise;
        let receiver = drainage.receivers[cell];
        // Standing water cuts nothing: a lake bed only rises or sinks.
        let under_lake = drainage.lake_depth(ground, cell) >= LAKE_MIN_DEPTH;
        if receiver == NO_RECEIVER || under_lake {
            ground[cell] = lifted;
            continue;
        }
        let receiver = receiver as usize;
        let erodibility = params.erodibility * (1.5 - land.rock_hardness[cell]);
        let cutting = erodibility * dt * drainage.flow[cell].powf(params.flow_exponent)
            / step_distance(cell, receiver, land.width);
        let lowered = (lifted + cutting * ground[receiver]) / (1.0 + cutting);
        sediment[cell] += (ground[cell] - lowered).max(0.0);
        ground[cell] = lowered;
    }

    widen_ice_valleys(ground, &drainage, land, params.ice_widening);
    *ground = crept(ground, land.is_base_level, land.width, land.height, params.slope_creep);
    drainage
}

/// Under ice, pull the ground beside each ice stream down towards the
/// stream's level: the more ice gathers in it, the harder. Valley floors
/// widen into troughs with steep walls where the pull gives out.
fn widen_ice_valleys(ground: &mut [f64], drainage: &Drainage, land: &Land, widening: f64) {
    if widening <= 0.0 {
        return;
    }
    let cells_per_wu = land.width as f64 / crate::biome_map::WORLD_WIDTH;
    let full_stream = ICE_STREAM_AREA_WU2 * cells_per_wu * cells_per_wu;
    // Highest first, so a trough is widened from its head down.
    for &cell in drainage.order.iter().rev() {
        let cell = cell as usize;
        let pull = widening * land.ice[cell] * (drainage.flow[cell] / full_stream).sqrt().min(1.0);
        if land.is_base_level[cell] || pull <= 0.0 {
            continue;
        }
        let (x, y) = ((cell % land.width) as i32, (cell / land.width) as i32);
        for (dx, dy) in crate::rivers::D8_OFFSETS {
            let beside_y = y + dy;
            if beside_y < 0 || beside_y >= land.height as i32 {
                continue;
            }
            let beside_x = crate::wrap::wrap_grid_x(x + dx, land.width) as usize;
            let beside = beside_y as usize * land.width + beside_x;
            if !land.is_base_level[beside] && ground[beside] > ground[cell] {
                ground[beside] -= pull * land.ice[beside] * (ground[beside] - ground[cell]);
            }
        }
    }
}

/// The ground after one step of slope creep: each land cell moves `share` of
/// the way towards the mean of its four neighbours.
pub fn crept(ground: &[f64], is_base_level: &[bool], width: usize, height: usize, share: f64) -> Vec<f64> {
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

    /// The slope after `steps` of erosion with no uplift, and its drainage.
    fn erode(steps: u32) -> (Vec<f64>, Drainage) {
        erode_under(steps, 0.0)
    }

    /// As `erode`, under ice that widens its valleys by `ice_widening`.
    fn erode_under(steps: u32, ice_widening: f64) -> (Vec<f64>, Drainage) {
        let (mut ground, sea) = grooved_slope();
        let params = ErosionParams {
            erodibility: 0.04,
            uplift: 0.0,
            flow_exponent: 0.45,
            time_step: 1.2,
            slope_creep: 0.04,
            uplift_limit: 1.0,
            ice_widening,
        };
        let land = Land {
            is_base_level: &sea,
            rock_hardness: &[0.5; 144],
            uplift_share: &[0.0; 144],
            rainfall: &[1.0; 144],
            ice: &[1.0; 144],
            lake_evaporation: &[0.0; 144],
            width: 16,
            height: 9,
        };
        let mut sediment = vec![0.0; 144];
        let mut drainage = None;
        for step in 0..steps {
            drainage = Some(erosion_step(&mut ground, &mut sediment, &land, &params, step));
        }
        (ground, drainage.expect("at least one step"))
    }

    #[test]
    fn a_river_cuts_a_valley_deeper_than_the_ground_beside_it() {
        let (before, _) = grooved_slope();
        let (after, _) = erode(5);
        // Middle of the slope: in the groove, and three rows off it.
        let (in_valley, beside) = (4 * 16 + 8, 1 * 16 + 8);

        let cut_in_valley = before[in_valley] - after[in_valley];
        let cut_beside = before[beside] - after[beside];
        assert!(cut_in_valley > cut_beside);
        assert!(after[in_valley] < after[beside]);
    }

    #[test]
    fn ice_cuts_a_wider_valley_than_water() {
        let (by_water, _) = erode_under(5, 0.0);
        let (by_ice, _) = erode_under(5, 0.5);
        // One row off the groove, in the middle of the slope: the valley side.
        let valley_side = 3 * 16 + 8;

        assert!(by_ice[valley_side] < by_water[valley_side]);
    }

    #[test]
    fn the_sea_stays_where_it_is_and_the_result_drains_to_it() {
        let (before, _) = grooved_slope();
        let (after, drainage) = erode(5);

        for y in 0..9 {
            assert_eq!(after[y * 16], before[y * 16]);
        }
        let draining = drainage.receivers.iter().filter(|&&r| r != NO_RECEIVER).count();
        assert_eq!(draining, 144 - 9);
    }
}
