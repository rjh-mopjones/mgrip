//! Drainage: which way water runs off the land, and how much gathers where.
//!
//! One solve serves both the erosion that carves the terrain and the river
//! network drawn on it, so rivers lie in the valleys that were cut for them.
//!
//! The method is priority-flood filling followed by steepest descent (Barnes
//! et al. 2014; Braun and Willett 2013): flood the land upwards from base
//! level, raising any hollow to the level at which it spills, then send each
//! cell's water to its lowest neighbour on that flooded surface.
//!
//! The land is the cubed sphere (spec 015): a cell's neighbours run across
//! face edges, slopes are measured over the true distance between cells,
//! and rain is gathered by area. Distances and flows are in mean cells, the
//! unit a flat grid would have used, so nothing calibrated on one needs
//! changing.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use mg_core::CubeGrid;

use crate::rivers::position_jitter;

/// Marks a cell that drains nowhere: it is base level (the sea).
pub const NO_RECEIVER: u32 = u32::MAX;

/// How much a hollow's floor is raised per cell so that water still runs
/// across it. Varied a little from cell to cell, or wide flats would drain
/// in straight lines along the grid.
const FLAT_SLOPE: f64 = 1e-4;

/// Rain that falls on the night side is locked up as ice; on the day side it
/// evaporates. Run-off is fullest in the terminus between.
const FROZEN_BELOW_LIGHT: f64 = 0.08;
const FULL_RUNOFF_FROM_LIGHT: f64 = 0.20;
const FULL_RUNOFF_TO_LIGHT: f64 = 0.70;
const EVAPORATED_ABOVE_LIGHT: f64 = 0.85;
const FROZEN_RUNOFF: f64 = 0.07;
const EVAPORATED_RUNOFF: f64 = 0.02;

pub struct Drainage {
    /// The ground with every hollow raised to the level at which it spills.
    pub filled: Vec<f64>,
    /// The cell each cell drains to, or `NO_RECEIVER` at base level.
    pub receivers: Vec<u32>,
    /// Cells from lowest to highest on the filled surface. A cell always
    /// comes after the cell it drains to.
    pub order: Vec<u32>,
    /// Rain gathered at each cell: its own and all that drains through it,
    /// less what evaporated from lakes on the way. In mean cells of run-off.
    pub flow: Vec<f64>,
}

impl Drainage {
    /// Depth of standing water at a cell: how far its hollow had to be
    /// filled before it spilled. Zero outside lakes.
    pub fn lake_depth(&self, ground: &[f64], cell: usize) -> f64 {
        (self.filled[cell] - ground[cell]).max(0.0)
    }
}

/// A hollow filled at least this deep is standing water, not a dip that
/// water runs across.
pub const LAKE_MIN_DEPTH: f64 = 0.002;

/// Water lost each step from a cell of open lake, in the units of `rainfall`
/// (the run-off of one well-watered cell). Lakes under ice lose nothing; in
/// the terminus little; towards the day side more than any stream brings.
pub fn lake_evaporation(light_level: f64) -> f64 {
    if light_level < FULL_RUNOFF_FROM_LIGHT {
        0.0
    } else if light_level < FULL_RUNOFF_TO_LIGHT {
        TERMINUS_LAKE_EVAPORATION
    } else {
        let dried = ((light_level - FULL_RUNOFF_TO_LIGHT)
            / (EVAPORATED_ABOVE_LIGHT - FULL_RUNOFF_TO_LIGHT))
            .min(1.0);
        TERMINUS_LAKE_EVAPORATION + (DAYSIDE_LAKE_EVAPORATION - TERMINUS_LAKE_EVAPORATION) * dried
    }
}

const TERMINUS_LAKE_EVAPORATION: f64 = 0.05;
const DAYSIDE_LAKE_EVAPORATION: f64 = 3.0;

/// How far a cell is under ice, from 0 (none) to 1 (ice sheet): the night
/// side, fading out towards the terminus as run-off fades in.
pub fn iciness(light_level: f64) -> f64 {
    ((FULL_RUNOFF_FROM_LIGHT - light_level) / (FULL_RUNOFF_FROM_LIGHT - FROZEN_BELOW_LIGHT))
        .clamp(0.0, 1.0)
}

/// How far a cell is desert, from 0 (none) to 1 (the deep day side): where
/// rain is so rare that only its floods do any work.
pub fn desert(light_level: f64) -> f64 {
    ((light_level - FULL_RUNOFF_TO_LIGHT) / (EVAPORATED_ABOVE_LIGHT - FULL_RUNOFF_TO_LIGHT))
        .clamp(0.0, 1.0)
}

/// Run-off from a cell, relative to a well-watered cell in the terminus.
pub fn rainfall(light_level: f64, humidity: f64) -> f64 {
    rainfall_with(light_level, humidity, FROZEN_RUNOFF, EVAPORATED_RUNOFF)
}

/// As `rainfall`, with the share of run-off that survives on the frozen
/// night side and on the evaporating day side given.
pub fn rainfall_with(
    light_level: f64,
    humidity: f64,
    frozen_runoff: f64,
    evaporated_runoff: f64,
) -> f64 {
    let reaches_rivers = if light_level < FROZEN_BELOW_LIGHT {
        frozen_runoff
    } else if light_level < FULL_RUNOFF_FROM_LIGHT {
        let thaw =
            (light_level - FROZEN_BELOW_LIGHT) / (FULL_RUNOFF_FROM_LIGHT - FROZEN_BELOW_LIGHT);
        frozen_runoff + (1.0 - frozen_runoff) * thaw
    } else if light_level < FULL_RUNOFF_TO_LIGHT {
        1.0
    } else if light_level < EVAPORATED_ABOVE_LIGHT {
        let dried =
            (light_level - FULL_RUNOFF_TO_LIGHT) / (EVAPORATED_ABOVE_LIGHT - FULL_RUNOFF_TO_LIGHT);
        1.0 - (1.0 - evaporated_runoff) * dried
    } else {
        evaporated_runoff
    };
    reaches_rivers * (0.5 + humidity.clamp(0.0, 1.0))
}

/// A cell waiting to be flooded, lowest first.
struct Flooding {
    level: f64,
    cell: u32,
}

impl PartialEq for Flooding {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Flooding {}
impl PartialOrd for Flooding {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Flooding {
    // Reversed, so the heap gives the lowest level first; ties by cell so the
    // result does not depend on insertion order.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .level
            .total_cmp(&self.level)
            .then(other.cell.cmp(&self.cell))
    }
}

/// Water can only run to one of a cell's eight neighbours. Always taking
/// the steepest makes valleys run dead straight along those eight directions
/// for as long as the slope allows, and the land comes out looking like a
/// grid. So the neighbour is drawn by lot among the downhill ones, steeper
/// ones being likelier by this power of their slope. Over many cells the
/// water then heads the way the ground really falls, whatever the angle.
const STEEPNESS_PREFERENCE: f64 = 1.0;

/// A number from 0 to 1 that is fixed for a cell and a `salt`.
fn routing_lot(cell: usize, salt: u32) -> f64 {
    position_jitter(
        (cell as u32).wrapping_mul(2_654_435_761) ^ salt.wrapping_mul(40_503),
        (cell as u32 >> 7).wrapping_add(salt.wrapping_mul(2_246_822_519)),
    )
}

/// A number from 0 to 1 fixed for a cell.
pub fn cell_jitter(cell: usize) -> f64 {
    position_jitter((cell % 65_536) as u32, (cell / 65_536) as u32)
}

/// Solve drainage over `ground`. Water runs to `is_base_level` cells (the
/// sea), which stay as they are. `rainfall` is the run-off each cell adds
/// per mean cell of area, and `evaporation` what a cell of open lake loses
/// (`lake_evaporation`). Land with no way to base level is left with no
/// receiver.
///
/// `salt` picks how the lots fall (see `STEEPNESS_PREFERENCE`): erosion
/// changes it every step so no one pattern is cut into the land.
pub fn solve_drainage(
    ground: &[f64],
    is_base_level: &[bool],
    rainfall: &[f64],
    evaporation: &[f64],
    grid: &CubeGrid,
    salt: u32,
) -> Drainage {
    let total = grid.cell_count();
    let mut filled = ground.to_vec();
    let mut reached = vec![false; total];
    let mut order = Vec::with_capacity(total);
    let mut waiting = BinaryHeap::with_capacity(total / 4);

    for cell in 0..total {
        if is_base_level[cell] {
            reached[cell] = true;
            waiting.push(Flooding {
                level: ground[cell],
                cell: cell as u32,
            });
        }
    }
    while let Some(Flooding { cell, .. }) = waiting.pop() {
        order.push(cell);
        let cell = cell as usize;
        for (neighbour, _) in grid.steps(cell) {
            if reached[neighbour] {
                continue;
            }
            reached[neighbour] = true;
            let spill = filled[cell] + FLAT_SLOPE * (0.3 + 1.4 * cell_jitter(neighbour));
            filled[neighbour] = ground[neighbour].max(spill);
            waiting.push(Flooding {
                level: filled[neighbour],
                cell: neighbour as u32,
            });
        }
    }

    // Each cell drains down its steepest slope on the filled surface.
    let receivers: Vec<u32> = (0..total)
        .map(|cell| {
            if is_base_level[cell] || !reached[cell] {
                return NO_RECEIVER;
            }
            let downhill: Vec<(usize, f64)> = grid
                .steps(cell)
                .map(|(neighbour, distance)| {
                    (neighbour, (filled[cell] - filled[neighbour]) / distance)
                })
                .filter(|&(_, slope)| slope > 0.0)
                .map(|(neighbour, slope)| (neighbour, slope.powf(STEEPNESS_PREFERENCE)))
                .collect();
            let total: f64 = downhill.iter().map(|&(_, chance)| chance).sum();
            let mut lot = routing_lot(cell, salt) * total;
            for &(neighbour, chance) in &downhill {
                lot -= chance;
                if lot <= 0.0 {
                    return neighbour as u32;
                }
            }
            downhill
                .last()
                .map_or(NO_RECEIVER, |&(neighbour, _)| neighbour as u32)
        })
        .collect();

    // Gather rain from the highest cells down. A cell adds its rain by its
    // area, so a smaller cell adds less.
    let mut flow: Vec<f64> = (0..total)
        .map(|cell| {
            if is_base_level[cell] {
                0.0
            } else {
                rainfall[cell] * grid.area_share(cell)
            }
        })
        .collect();
    for &cell in order.iter().rev() {
        let cell = cell as usize;
        // Water crossing a lake loses some of itself to the air; a lake
        // that loses all of it has no river out.
        if filled[cell] - ground[cell] >= LAKE_MIN_DEPTH {
            flow[cell] = (flow[cell] - evaporation[cell] * grid.area_share(cell)).max(0.0);
        }
        let receiver = receivers[cell];
        if receiver != NO_RECEIVER {
            flow[receiver as usize] += flow[cell];
        }
    }

    Drainage {
        filled,
        receivers,
        order,
        flow,
    }
}

#[cfg(test)]
pub(crate) mod test_world {
    //! A small cubed-sphere world for the solvers' tests: the sea is the
    //! southern cap, the land rises northwards with the height of the
    //! point above the equator.

    use mg_core::CubeGrid;

    pub const N: usize = 8;

    pub fn cube() -> CubeGrid {
        CubeGrid::margin(N)
    }

    /// Ground rising from the sea in the south to a summit at the north
    /// pole; sea where the point is below `sea_above` on the z axis.
    pub fn sloping_world(grid: &CubeGrid) -> (Vec<f64>, Vec<bool>) {
        let ground: Vec<f64> = (0..grid.cell_count())
            .map(|cell| grid.point(cell)[2].max(-0.6))
            .collect();
        let sea: Vec<bool> = (0..grid.cell_count())
            .map(|cell| grid.point(cell)[2] < -0.6)
            .collect();
        (ground, sea)
    }

    /// The cell at the middle of a face.
    pub fn middle_of(grid: &CubeGrid, face: usize) -> usize {
        grid.index(face, N / 2, N / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::test_world::*;
    use super::*;

    #[test]
    fn water_runs_downhill_to_the_sea_and_gathers_on_the_way() {
        let grid = cube();
        let (ground, sea) = sloping_world(&grid);
        let total = grid.cell_count();
        let drainage = solve_drainage(
            &ground,
            &sea,
            &vec![1.0; total],
            &vec![0.0; total],
            &grid,
            0,
        );

        // The sea drains nowhere; everything else drains somewhere lower.
        for cell in 0..total {
            if sea[cell] {
                assert_eq!(drainage.receivers[cell], NO_RECEIVER);
            } else {
                let to = drainage.receivers[cell] as usize;
                assert!(ground[to] <= ground[cell], "cell {cell}");
            }
        }
        // Flow gathers towards the sea: a cell just above the shore carries
        // more than the summit.
        let summit = middle_of(&grid, 4);
        let shore = (0..total)
            .filter(|&cell| !sea[cell])
            .min_by(|&a, &b| ground[a].total_cmp(&ground[b]))
            .unwrap();
        assert!(drainage.flow[shore] > drainage.flow[summit]);
    }

    #[test]
    fn a_hollow_is_filled_until_it_spills_and_water_crosses_it() {
        let grid = cube();
        let (mut ground, sea) = sloping_world(&grid);
        let total = grid.cell_count();
        // A pit in the middle of a side face, well below everything round it.
        let pit = middle_of(&grid, 1);
        ground[pit] -= 1.0;
        let drainage = solve_drainage(
            &ground,
            &sea,
            &vec![1.0; total],
            &vec![0.0; total],
            &grid,
            0,
        );

        assert!(drainage.filled[pit] > ground[pit] + 0.5);
        assert_ne!(drainage.receivers[pit], NO_RECEIVER);
        assert!(drainage.lake_depth(&ground, pit) >= LAKE_MIN_DEPTH);
    }

    #[test]
    fn a_lake_gives_up_water_to_the_air() {
        let grid = cube();
        let (mut ground, sea) = sloping_world(&grid);
        let total = grid.cell_count();
        let pit = middle_of(&grid, 1);
        ground[pit] -= 0.5;
        let flow = |evaporation: f64| {
            solve_drainage(
                &ground,
                &sea,
                &vec![1.0; total],
                &vec![evaporation; total],
                &grid,
                0,
            )
            .flow
        };
        let below = solve_drainage(
            &ground,
            &sea,
            &vec![1.0; total],
            &vec![0.0; total],
            &grid,
            0,
        )
        .receivers[pit] as usize;

        assert!(flow(0.5)[below] < flow(0.0)[below]);
        // A lake that loses all its water sends none on.
        assert_eq!(flow(100.0)[pit], 0.0);
        assert!(lake_evaporation(0.8) > lake_evaporation(0.4));
        assert_eq!(lake_evaporation(0.05), 0.0);
    }

    #[test]
    fn every_cell_comes_after_the_cell_it_drains_to() {
        let grid = cube();
        let (ground, sea) = sloping_world(&grid);
        let total = grid.cell_count();
        let drainage = solve_drainage(
            &ground,
            &sea,
            &vec![1.0; total],
            &vec![0.0; total],
            &grid,
            0,
        );
        let mut position = vec![usize::MAX; total];
        for (at, &cell) in drainage.order.iter().enumerate() {
            position[cell as usize] = at;
        }
        for cell in 0..total {
            let receiver = drainage.receivers[cell];
            if receiver != NO_RECEIVER {
                assert!(position[receiver as usize] < position[cell]);
            }
        }
    }

    #[test]
    fn run_off_is_fullest_in_the_terminus() {
        assert!(rainfall(0.4, 0.5) > rainfall(0.02, 0.5));
        assert!(rainfall(0.4, 0.5) > rainfall(0.95, 0.5));
        assert!(rainfall(0.4, 0.9) > rainfall(0.4, 0.1));
    }
}
