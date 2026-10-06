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
//!
//! The land is the cubed sphere (spec 015): distances between cells are
//! the true ones, neighbours run across face edges, and a blur reaches
//! across them too.

use mg_core::CubeGrid;

use crate::drainage::{solve_drainage, Drainage, LAKE_MIN_DEPTH, NO_RECEIVER};

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
    pub grid: &'a CubeGrid,
}

/// Distance between a cell and the one it drains to, in mean cells.
pub fn step_to(grid: &CubeGrid, from: usize, to: usize) -> f64 {
    grid.steps(from)
        .find(|&(cell, _)| cell == to)
        .map_or_else(|| grid.distance_cells(from, to), |(_, distance)| distance)
}

/// A reach in world units as a count of mean cells, at least one.
pub fn reach_cells(grid: &CubeGrid, reach_wu: f64) -> usize {
    (reach_wu * grid.cells_per_world_unit() / 2.0)
        .round()
        .max(1.0) as usize
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
    let grid = land.grid;
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
        grid,
        step,
    );

    let cells_per_wu = grid.cells_per_world_unit();
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
        // In mean cells, so per world unit is this times cells per unit.
        let distance = step_to(grid, cell, receiver);
        if ice_thickness[cell] >= ICE_MIN_THICKNESS {
            // Thick ice digs by how much of it there is and how fast it
            // slides, not towards the height of the ground downstream.
            let fall = (surface[cell] - surface[receiver]) / distance * cells_per_wu;
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
        let cutting = erodibility * dt * working_flow.powf(params.flow_exponent) / distance;
        let lowered = (lifted + cutting * towards) / (1.0 + cutting);
        sediment[cell] += (ground[cell] - lowered).max(0.0);
        ground[cell] = lowered;
    }

    widen_valleys(ground, &drainage, land, params, flood);
    *ground = crept(ground, land.is_base_level, grid, params.slope_creep);
    drainage
}

/// How thick the ice lies on each cell: it fills everything below a
/// smoothed copy of the ground, so it is deep in valleys and hollows and
/// absent from ridges. Nothing where there is no ice.
fn ice_thickness(ground: &[f64], land: &Land) -> Vec<f64> {
    if land.ice.iter().all(|&ice| ice <= 0.0) {
        return vec![0.0; ground.len()];
    }
    let smoothed = land
        .grid
        .blur(ground, reach_cells(land.grid, ICE_SMOOTHING_WU));
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
    let cells_per_wu = land.grid.cells_per_world_unit();
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
        for (beside, _) in land.grid.steps(cell) {
            if !land.is_base_level[beside] && ground[beside] > ground[cell] {
                ground[beside] -= pull * reaches(beside) * (ground[beside] - ground[cell]);
            }
        }
    }
}

/// The ground after one step of slope creep: each land cell moves `share` of
/// the way towards the mean of its four nearest neighbours.
pub fn crept(ground: &[f64], is_base_level: &[bool], grid: &CubeGrid, share: f64) -> Vec<f64> {
    (0..grid.cell_count())
        .map(|cell| {
            if is_base_level[cell] {
                return ground[cell];
            }
            let beside = grid.neighbours(cell);
            // The four across an edge: north, east, south, west.
            let (mut sum, mut count) = (0.0, 0.0);
            for direction in [0, 2, 4, 6] {
                if let Some(to) = beside[direction] {
                    sum += ground[to];
                    count += 1.0;
                }
            }
            ground[cell] + share * (sum / count - ground[cell])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mg_core::CubeGrid;

    const N: usize = 16;

    /// Land rising northwards from a southern sea, with a groove along the
    /// meridian of longitude 0° (where the sphere's y is 0): a V two cells
    /// wide each side, its sides steeper than the fall to the sea so the
    /// water it gathers cannot wander out of it, and flat ground beyond.
    /// The coast is at sea level: the land stands above it, the sea floor
    /// below.
    fn grooved_world(grid: &CubeGrid) -> (Vec<f64>, Vec<bool>) {
        let mut ground = Vec::with_capacity(grid.cell_count());
        let mut sea = Vec::with_capacity(grid.cell_count());
        for cell in 0..grid.cell_count() {
            let [_, y, z] = grid.point(cell);
            let off_groove = (y.abs() * 1.5).min(0.3);
            if z < -0.6 {
                ground.push(-0.3);
                sea.push(true);
            } else {
                ground.push(z + 0.6 + off_groove);
                sea.push(false);
            }
        }
        (ground, sea)
    }

    /// A cell of face 0, `rows` down from its top in `column`. Longitude 0°
    /// runs between the face's two middle columns.
    fn on_face_0(grid: &CubeGrid, rows: usize, column: usize) -> usize {
        grid.index(0, column, rows)
    }

    /// The groove lies between two columns, and the channel the water cuts
    /// runs in one or the other: the column it runs in on `row`, by flow.
    fn channel_column(grid: &CubeGrid, drainage: &Drainage, row: usize) -> usize {
        [N / 2 - 1, N / 2]
            .into_iter()
            .max_by(|&a, &b| {
                drainage.flow[on_face_0(grid, row, a)]
                    .total_cmp(&drainage.flow[on_face_0(grid, row, b)])
            })
            .unwrap()
    }

    /// The world after `steps` of erosion with no uplift, and its drainage.
    fn erode(steps: u32) -> (CubeGrid, Vec<f64>, Drainage) {
        erode_with(steps, 0.0, 0.0, 0.0, 0.0)
    }

    fn erode_with(
        steps: u32,
        ice_widening: f64,
        desert: f64,
        ice: f64,
        ice_cutting: f64,
    ) -> (CubeGrid, Vec<f64>, Drainage) {
        let grid = CubeGrid::margin(N);
        let total = grid.cell_count();
        let desert = vec![desert; total];
        let ice = vec![ice; total];
        let (mut ground, sea) = grooved_world(&grid);
        let params = ErosionParams {
            // Gentle enough that a cell is cut by its flow, not all the way
            // down to the cell it drains to in one step.
            erodibility: 0.004,
            uplift: 0.0,
            flow_exponent: 0.45,
            time_step: 1.2,
            slope_creep: 0.01,
            uplift_limit: 1.0,
            ice_widening,
            ice_cutting,
            // On this coarse cube, the rain of forty cells: more than any
            // cell off the channel gathers, far less than the channel.
            canyon_flood_area: 40.0 * grid.cell_size() * grid.cell_size(),
            canyon_power: 4.0,
            scarp_retreat: 0.0,
        };
        let land = Land {
            is_base_level: &sea,
            rock_hardness: &vec![0.5; total],
            uplift_share: &vec![0.0; total],
            rainfall: &vec![1.0; total],
            ice: &ice,
            lake_evaporation: &vec![0.0; total],
            desert: &desert,
            grid: &grid,
        };
        let mut sediment = vec![0.0; total];
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
        (grid.clone(), ground, drainage.expect("at least one step"))
    }

    /// How much each cell of `row` on face 0 was lowered, by column.
    fn cuts_along(grid: &CubeGrid, before: &[f64], after: &[f64], row: usize) -> Vec<f64> {
        (0..N)
            .map(|column| {
                let cell = on_face_0(grid, row, column);
                before[cell] - after[cell]
            })
            .collect()
    }

    /// The groove runs between the face's two middle columns; the channel
    /// wanders between them from step to step.
    const GROOVE: std::ops::RangeInclusive<usize> = N / 2 - 2..=N / 2 + 1;
    /// Level ground well east of the groove, clear of the gutter that the
    /// flat drains into along the groove's rim.
    const FLAT: std::ops::RangeInclusive<usize> = N / 2 + 6..=N - 1;

    fn most(cuts: &[f64], columns: std::ops::RangeInclusive<usize>) -> f64 {
        columns.map(|column| cuts[column]).fold(f64::MIN, f64::max)
    }

    fn least(cuts: &[f64], columns: std::ops::RangeInclusive<usize>) -> f64 {
        columns.map(|column| cuts[column]).fold(f64::MAX, f64::min)
    }

    #[test]
    fn a_river_cuts_a_valley_deeper_than_the_ground_beside_it() {
        let (grid, after, _) = erode(5);
        let (before, _) = grooved_world(&grid);
        let cuts = cuts_along(&grid, &before, &after, 11);

        // The channel is cut far deeper than any cell of the level ground.
        let (in_valley, beside) = (most(&cuts, GROOVE), least(&cuts, FLAT));
        assert!(in_valley > 1.5 * beside, "{in_valley} vs {beside}");
    }

    #[test]
    fn ice_cuts_a_wider_valley_than_water() {
        let (grid, by_water, drainage) = erode_with(5, 0.0, 0.0, 1.0, 0.0);
        let (_, by_ice, _) = erode_with(5, 0.5, 0.0, 1.0, 0.0);
        // One column off the channel, low on face 0: the valley side.
        let valley_side = on_face_0(&grid, 11, channel_column(&grid, &drainage, 11) + 1);

        assert!(by_ice[valley_side] < by_water[valley_side]);
    }

    #[test]
    fn ice_fills_the_valley_and_digs_where_water_would_have_stopped() {
        let grid = CubeGrid::margin(N);
        let total = grid.cell_count();
        let (ground, sea) = grooved_world(&grid);
        let land = Land {
            is_base_level: &sea,
            rock_hardness: &vec![0.5; total],
            uplift_share: &vec![0.0; total],
            rainfall: &vec![1.0; total],
            ice: &vec![1.0; total],
            lake_evaporation: &vec![0.0; total],
            desert: &vec![0.0; total],
            grid: &grid,
        };
        // Ice lies in the groove and not on the high ground beside it.
        let thickness = ice_thickness(&ground, &land);
        assert!(thickness[on_face_0(&grid, 8, N / 2)] > thickness[on_face_0(&grid, 8, N / 2 + 4)]);

        // Where the valley meets the sea, ice digs below sea level; water
        // only cuts down to its outlet.
        let coast = (0..total)
            .filter(|&cell| !sea[cell] && grid.point(cell)[1].abs() < 0.05)
            .min_by(|&a, &b| ground[a].total_cmp(&ground[b]))
            .unwrap();
        let (_, by_water, _) = erode_with(40, 0.0, 0.0, 0.0, 0.0);
        let (_, by_ice, _) = erode_with(40, 0.0, 0.0, 1.0, 2.0);
        assert!(by_ice[coast] < by_water[coast]);
        assert!(by_ice[coast] >= FJORD_FLOOR);
    }

    #[test]
    fn in_desert_only_the_channel_is_cut() {
        let (grid, after, _) = erode_with(5, 0.0, 1.0, 0.0, 0.0);
        let (_, plain, _) = erode(5);
        let (before, _) = grooved_world(&grid);
        let desert_cuts = cuts_along(&grid, &before, &after, 11);
        let plain_cuts = cuts_along(&grid, &before, &plain, 11);

        // The channel, where the water has gathered, is cut: harder than a
        // river of the same flow would cut it.
        assert!(most(&desert_cuts, GROOVE) > most(&plain_cuts, GROOVE));
        // The level ground, where a cell has little more than its own rain,
        // is not cut at all; a river would have cut it a little.
        assert!(most(&desert_cuts, FLAT) < least(&plain_cuts, FLAT));
    }

    #[test]
    fn the_sea_stays_where_it_is_and_the_result_drains_to_it() {
        let (grid, after, drainage) = erode(5);
        let (before, sea) = grooved_world(&grid);

        for cell in 0..grid.cell_count() {
            if sea[cell] {
                assert_eq!(after[cell], before[cell]);
            }
        }
        let draining = drainage
            .receivers
            .iter()
            .filter(|&&r| r != NO_RECEIVER)
            .count();
        let land = sea.iter().filter(|&&s| !s).count();
        assert_eq!(draining, land);
    }
}
