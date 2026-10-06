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
    /// How deep thick, fast ice digs its bed each unit of time. Ice follows
    /// its own surface, so it digs where it is, whatever the ground does
    /// further on: it can leave a basin deeper than its outlet, and cut
    /// below the sea.
    pub ice_cutting: f64,
    /// In desert a channel cuts only once it gathers the run-off of this
    /// many square world units of well-watered land: only floods do work
    /// there, and the ground between the few channels they cut stays whole.
    pub canyon_flood_area: f64,
    /// How many times harder a desert flood cuts than a steady river of the
    /// same flow: bare rock, no soil, all the water at once.
    pub canyon_power: f64,
    /// How far a desert canyon pulls the ground beside it down towards its
    /// floor each step, from 0 to 1: its walls break and fall back, so a
    /// canyon widens while the ground beyond its rim stays whole.
    pub scarp_retreat: f64,
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
            ice_cutting: 0.012,
            canyon_flood_area: 0.5,
            canyon_power: 4.0,
            scarp_retreat: 0.12,
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
/// Ice lies level across anything narrower than about this many world
/// units: it fills valleys and hollows up to the ground around them.
const ICE_SMOOTHING_WU: f64 = 6.0;
/// Snow that falls on a cell under an ice sheet, in the units of `rainfall`.
const SNOWFALL: f64 = 0.6;
/// Ice thinner than this digs nothing; water runs under and round it.
const ICE_MIN_THICKNESS: f64 = 0.004;
/// An ice surface falling this much height per world unit, or more, slides
/// at full speed.
const ICE_FULL_SLOPE: f64 = 0.03;
/// Ice digs no deeper than this (24 blocks below the sea): the depth of a
/// fjord.
const FJORD_FLOOR: f64 = -0.12;

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
    /// How far each cell is desert, from 0 (none) to 1.
    pub desert: &'a [f64],
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
    // Under ice it is the ice that flows, down its own surface: smoother than
    // the ground, level across valleys, and able to ride over a sill.
    let ice_thickness = ice_thickness(ground, land);
    let surface: Vec<f64> = ground
        .iter()
        .zip(&ice_thickness)
        .map(|(bed, ice)| bed + ice)
        .collect();
    // Little water runs off the night side, but none of its snow is lost:
    // it all leaves as ice, and as meltwater where the ice ends.
    let gathered: Vec<f64> = (0..ground.len())
        .map(|cell| {
            land.rainfall[cell] + (SNOWFALL - land.rainfall[cell]).max(0.0) * land.ice[cell]
        })
        .collect();
    let drainage = solve_drainage(
        &surface,
        land.is_base_level,
        &gathered,
        land.lake_evaporation,
        land.width,
        land.height,
        step,
    );

    let cells_per_wu = land.width as f64 / crate::biome_map::WORLD_WIDTH;
    let flood = params.canyon_flood_area * cells_per_wu * cells_per_wu;
    let full_ice_stream = ICE_STREAM_AREA_WU2 * cells_per_wu * cells_per_wu;
    let sea_level = crate::biome_map::SEA_LEVEL;

    // From the sea upwards, so each cell's receiver is already lowered.
    for &cell in &drainage.order {
        let cell = cell as usize;
        if land.is_base_level[cell] {
            continue;
        }
        // Well-watered land is kept down by its rivers and may rise to the
        // full limit. Dry and frozen land has only the ceiling to stop it.
        let wetness = (land.rainfall[cell] / CEILING_LIFTED_BY_RUNOFF).min(1.0);
        let by_uplift =
            (land.uplift_share[cell] / CEILING_FULL_AT_SHARE).clamp(CEILING_LEAST_SHARE, 1.0);
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
        let under_lake = drainage.lake_depth(&surface, cell) >= LAKE_MIN_DEPTH;
        if receiver == NO_RECEIVER || under_lake {
            ground[cell] = lifted;
            continue;
        }
        let receiver = receiver as usize;
        if ice_thickness[cell] >= ICE_MIN_THICKNESS {
            // Thick ice digs by how much of it there is and how fast it
            // slides, not towards the height of the ground downstream.
            let fall = (surface[cell] - surface[receiver])
                / step_distance(cell, receiver, land.width)
                * cells_per_wu;
            let dug = dt
                * params.ice_cutting
                * land.ice[cell]
                * (drainage.flow[cell] / full_ice_stream).sqrt().min(1.0)
                * (fall / ICE_FULL_SLOPE).clamp(0.0, 1.0);
            let lowered = (lifted - dug).max(FJORD_FLOOR.min(lifted));
            sediment[cell] += (ground[cell] - lowered).max(0.0);
            ground[cell] = lowered;
            continue;
        }
        // A river cuts down towards where it is going. At the sea that is
        // the sea's surface, not its bed; at a dried-out sea it is the bed.
        let towards = if land.is_base_level[receiver] {
            ground[receiver].max(sea_level) * (1.0 - land.desert[cell])
                + ground[receiver] * land.desert[cell]
        } else {
            ground[receiver]
        };
        // Water cannot cut towards ground that stands above it (as it may
        // where the way out was found over ice).
        if towards >= lifted {
            ground[cell] = lifted;
            continue;
        }
        // In desert only what a channel carries beyond a flood's worth
        // cuts, and cuts hard.
        let desert = land.desert[cell];
        let working_flow = (drainage.flow[cell] - flood * desert).max(0.0);
        let erodibility = params.erodibility
            * (1.5 - land.rock_hardness[cell])
            * (1.0 + (params.canyon_power - 1.0) * desert);
        let cutting = erodibility * dt * working_flow.powf(params.flow_exponent)
            / step_distance(cell, receiver, land.width);
        let lowered = (lifted + cutting * towards) / (1.0 + cutting);
        sediment[cell] += (ground[cell] - lowered).max(0.0);
        ground[cell] = lowered;
    }

    widen_valleys(ground, &drainage, land, params, flood);
    *ground = crept(
        ground,
        land.is_base_level,
        land.width,
        land.height,
        params.slope_creep,
    );
    drainage
}

/// How thick the ice lies on each cell: it fills everything below a
/// smoothed copy of the ground, so it is deep in valleys and hollows and
/// absent from ridges. Nothing where there is no ice.
fn ice_thickness(ground: &[f64], land: &Land) -> Vec<f64> {
    if land.ice.iter().all(|&ice| ice <= 0.0) {
        return vec![0.0; ground.len()];
    }
    let cells_per_wu = land.width as f64 / crate::biome_map::WORLD_WIDTH;
    let reach = (ICE_SMOOTHING_WU * cells_per_wu / 2.0).round().max(1.0) as i32;
    let smoothed = box_blurred(ground, land.width, land.height, reach);
    (0..ground.len())
        .map(|cell| {
            if land.is_base_level[cell] {
                0.0
            } else {
                (smoothed[cell] - ground[cell]).max(0.0) * land.ice[cell]
            }
        })
        .collect()
}

/// `field` averaged over `reach` cells each way, along rows and then along
/// columns. The map joins east to west; north and south edges repeat.
pub fn box_blurred(field: &[f64], width: usize, height: usize, reach: i32) -> Vec<f64> {
    let span = (2 * reach + 1) as f64;
    let rows: Vec<f64> = (0..width * height)
        .map(|cell| {
            let (x, y) = ((cell % width) as i32, cell / width);
            (-reach..=reach)
                .map(|dx| field[y * width + (x + dx).rem_euclid(width as i32) as usize])
                .sum::<f64>()
                / span
        })
        .collect();
    (0..width * height)
        .map(|cell| {
            let (x, y) = (cell % width, (cell / width) as i32);
            (-reach..=reach)
                .map(|dy| rows[(y + dy).clamp(0, height as i32 - 1) as usize * width + x])
                .sum::<f64>()
                / span
        })
        .collect()
}

/// Pull the ground beside a channel down towards the channel's level, where
/// something other than running water is at work on the valley's sides.
///
/// Under ice, the more ice gathers in a stream the harder it pulls: valley
/// floors widen into troughs with steep walls where the pull gives out. In
/// desert, the walls of a canyon in flood break and fall back.
fn widen_valleys(
    ground: &mut [f64],
    drainage: &Drainage,
    land: &Land,
    params: &ErosionParams,
    flood: f64,
) {
    if params.ice_widening <= 0.0 && params.scarp_retreat <= 0.0 {
        return;
    }
    let cells_per_wu = land.width as f64 / crate::biome_map::WORLD_WIDTH;
    let full_stream = ICE_STREAM_AREA_WU2 * cells_per_wu * cells_per_wu;
    // Highest first, so a trough is widened from its head down.
    for &cell in drainage.order.iter().rev() {
        let cell = cell as usize;
        let flow = drainage.flow[cell];
        let by_ice = params.ice_widening * land.ice[cell] * (flow / full_stream).sqrt().min(1.0);
        let in_flood = flow > flood * land.desert[cell];
        let by_scarps = if in_flood {
            params.scarp_retreat * land.desert[cell]
        } else {
            0.0
        };
        let pull = by_ice.max(by_scarps);
        if land.is_base_level[cell] || pull <= 0.0 {
            continue;
        }
        // Ice pulls only on ground that is itself under ice.
        let reaches = |beside: usize| {
            if by_ice >= by_scarps {
                land.ice[beside]
            } else {
                1.0
            }
        };
        let (x, y) = ((cell % land.width) as i32, (cell / land.width) as i32);
        for (dx, dy) in crate::rivers::D8_OFFSETS {
            let beside_y = y + dy;
            if beside_y < 0 || beside_y >= land.height as i32 {
                continue;
            }
            let beside_x = (x + dx).rem_euclid(land.width as i32) as usize;
            let beside = beside_y as usize * land.width + beside_x;
            if !land.is_base_level[beside] && ground[beside] > ground[cell] {
                ground[beside] -= pull * reaches(beside) * (ground[beside] - ground[cell]);
            }
        }
    }
}

/// The ground after one step of slope creep: each land cell moves `share` of
/// the way towards the mean of its four neighbours.
pub fn crept(
    ground: &[f64],
    is_base_level: &[bool],
    width: usize,
    height: usize,
    share: f64,
) -> Vec<f64> {
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
        erode_in(steps, ice_widening, 0.0)
    }

    /// As `erode_under`, on land that is `desert` (0 none to 1 all).
    fn erode_in(steps: u32, ice_widening: f64, desert: f64) -> (Vec<f64>, Drainage) {
        erode_with(steps, ice_widening, desert, 0.0, 0.0)
    }

    /// The grooved slope after `steps`, with everything set: how far the
    /// land is under `ice`, and how hard that ice widens and cuts.
    fn erode_with(
        steps: u32,
        ice_widening: f64,
        desert: f64,
        ice: f64,
        ice_cutting: f64,
    ) -> (Vec<f64>, Drainage) {
        let desert = [desert; 144];
        let ice = [ice; 144];
        let (mut ground, sea) = grooved_slope();
        let params = ErosionParams {
            erodibility: 0.04,
            uplift: 0.0,
            flow_exponent: 0.45,
            time_step: 1.2,
            slope_creep: 0.04,
            uplift_limit: 1.0,
            ice_widening,
            ice_cutting,
            // On this 16-cell-wide grid, the rain of three cells.
            canyon_flood_area: 3.0 * (1024.0 / 16.0) * (1024.0 / 16.0),
            canyon_power: 4.0,
            scarp_retreat: 0.0,
        };
        let land = Land {
            is_base_level: &sea,
            rock_hardness: &[0.5; 144],
            uplift_share: &[0.0; 144],
            rainfall: &[1.0; 144],
            ice: &ice,
            lake_evaporation: &[0.0; 144],
            desert: &desert,
            width: 16,
            height: 9,
        };
        let mut sediment = vec![0.0; 144];
        let mut drainage = None;
        for step in 0..steps {
            drainage = Some(erosion_step(
                &mut ground,
                &mut sediment,
                &land,
                &params,
                step,
            ));
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
        let (by_water, _) = erode_with(5, 0.0, 0.0, 1.0, 0.0);
        let (by_ice, _) = erode_with(5, 0.5, 0.0, 1.0, 0.0);
        // One row off the groove, in the middle of the slope: the valley side.
        let valley_side = 3 * 16 + 8;

        assert!(by_ice[valley_side] < by_water[valley_side]);
    }

    #[test]
    fn ice_fills_the_valley_and_digs_where_water_would_have_stopped() {
        let (ground, sea) = grooved_slope();
        let land = Land {
            is_base_level: &sea,
            rock_hardness: &[0.5; 144],
            uplift_share: &[0.0; 144],
            rainfall: &[1.0; 144],
            ice: &[1.0; 144],
            lake_evaporation: &[0.0; 144],
            desert: &[0.0; 144],
            width: 16,
            height: 9,
        };
        // Ice lies in the groove and not on the high ground beside it.
        let thickness = ice_thickness(&ground, &land);
        assert!(thickness[4 * 16 + 8] > thickness[16 + 8]);

        // Where the valley meets the sea, ice digs below sea level; water
        // only cuts down to its outlet.
        let coast = 4 * 16 + 1;
        let (by_water, _) = erode_with(40, 0.0, 0.0, 0.0, 0.0);
        // (Slopes on this 16-cell world are tiny per world unit, so the ice
        // is given a great deal of cutting power.)
        let (by_ice, _) = erode_with(40, 0.0, 0.0, 1.0, 2.0);
        assert!(by_water[coast] >= ground[4 * 16]);
        assert!(by_ice[coast] < ground[4 * 16]);
        assert!(by_ice[coast] >= FJORD_FLOOR);
    }

    #[test]
    fn in_desert_only_the_channel_is_cut() {
        let (before, _) = grooved_slope();
        let (after, _) = erode_in(5, 0.0, 1.0);
        // Low on the slope: in the groove, where the water has gathered, and
        // three rows off it, where each cell has only its own rain.
        let (in_channel, beside) = (4 * 16 + 3, 1 * 16 + 3);

        assert!(before[in_channel] - after[in_channel] > 0.0);
        // Off the channel nothing is cut; the ground only creeps.
        let (plain, _) = erode_in(5, 0.0, 0.0);
        assert!(before[beside] - after[beside] < before[beside] - plain[beside]);
    }

    #[test]
    fn the_sea_stays_where_it_is_and_the_result_drains_to_it() {
        let (before, _) = grooved_slope();
        let (after, drainage) = erode(5);

        for y in 0..9 {
            assert_eq!(after[y * 16], before[y * 16]);
        }
        let draining = drainage
            .receivers
            .iter()
            .filter(|&&r| r != NO_RECEIVER)
            .count();
        assert_eq!(draining, 144 - 9);
    }
}
