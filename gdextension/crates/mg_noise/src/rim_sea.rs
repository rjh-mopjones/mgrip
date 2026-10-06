//! The rim sea: one unbroken stretch of liquid water all the way round the
//! world.
//!
//! Liquid sea lies only in the terminus, the ring between the frozen night
//! side and the dried-out day side. Left to the continents, that ring is a
//! string of separate seas with land between them. This opens the passages
//! that join them, so the whole rim can be sailed: it finds the cheapest
//! route round the world through water that would be liquid, and cuts a
//! strait wherever that route crosses land.
//!
//! It works on continentalness, before the land is grown, so the straits are
//! sea to everything that follows and get coasts like any other.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::biome_map::WORLD_WIDTH;
use crate::rivers::{D8_DISTANCES, D8_OFFSETS};

/// A strait is cut this far to either side of the route, in world units.
const STRAIT_HALF_WIDTH_WU: f64 = 2.5;
/// How far below sea level the middle of a strait is cut; it shallows to
/// the shore. Deeper than the shallows a hot sea dries out from (0.08), so
/// a strait on the day-side margin stays water.
pub const STRAIT_DEPTH: f64 = 0.1;
// What it costs the route to enter a cell, per cell of distance.
/// Open, liquid sea.
const SEA_COST: f64 = 1.0;
/// Land where the sea would be liquid: a strait can be cut here. Lower land
/// is cheaper to cut than higher.
const LAND_COST: f64 = 40.0;
const LAND_COST_PER_HEIGHT: f64 = 400.0;
/// Anywhere the sea would freeze or dry out: no use to a ship.
const DEAD_SEA_COST: f64 = 4000.0;

/// Open the rim sea in `continentalness`. `stays_liquid[cell]` says whether
/// sea at that cell would be liquid (neither frozen nor dried out). Returns
/// how many cells were turned from land to sea.
pub fn open_rim_sea(
    continentalness: &mut [f64],
    stays_liquid: &[bool],
    width: usize,
    height: usize,
    sea_level: f64,
) -> usize {
    let cost = |cell: usize| {
        if !stays_liquid[cell] {
            DEAD_SEA_COST
        } else if continentalness[cell] < sea_level {
            SEA_COST
        } else {
            LAND_COST + LAND_COST_PER_HEIGHT * (continentalness[cell] - sea_level)
        }
    };
    // The cheapest way across the map from its west edge to its east edge
    // need not end on the row it began on. Find where it ends, then find the
    // cheapest way that begins on that row and ends beside it: a closed ring.
    let Some(open_route) = cheapest_crossing(&cost, width, height, None) else {
        return 0;
    };
    let closing_row = open_route[open_route.len() - 1] / width;
    let route = cheapest_crossing(&cost, width, height, Some(closing_row)).unwrap_or(open_route);

    let reach = (STRAIT_HALF_WIDTH_WU * width as f64 / WORLD_WIDTH).round() as i32;
    let mut opened = 0;
    for &cell in &route {
        let (x, y) = ((cell % width) as i32, (cell / width) as i32);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let from_route = ((dx * dx + dy * dy) as f64).sqrt();
                let near_y = y + dy;
                if from_route > reach as f64 + 0.5 || near_y < 0 || near_y >= height as i32 {
                    continue;
                }
                let near = near_y as usize * width + (x + dx).rem_euclid(width as i32) as usize;
                // Deepest along the route, shallowing towards the shore.
                let bed = sea_level - STRAIT_DEPTH * (1.0 - from_route / (reach as f64 + 1.0));
                if continentalness[near] > bed {
                    if continentalness[near] >= sea_level {
                        opened += 1;
                    }
                    continentalness[near] = bed;
                }
            }
        }
    }
    opened
}

/// The cheapest path of cells from the map's west edge to its east edge,
/// stepping to any of eight neighbours and never round the seam. With
/// `closing_row`, the path starts on that row and ends on it or beside it,
/// so that its two ends meet across the seam.
fn cheapest_crossing(
    cost: &impl Fn(usize) -> f64,
    width: usize,
    height: usize,
    closing_row: Option<usize>,
) -> Option<Vec<usize>> {
    let mut best = vec![f64::INFINITY; width * height];
    let mut came_from = vec![usize::MAX; width * height];
    // Costs are ordered as integers: the heap needs a total order.
    let mut waiting = BinaryHeap::new();
    let scaled = |total: f64| (total * 1024.0) as u64;
    for row in 0..height {
        if closing_row.is_some_and(|start| start != row) {
            continue;
        }
        let cell = row * width;
        best[cell] = cost(cell);
        waiting.push((Reverse(scaled(best[cell])), cell));
    }
    let ends_here = |cell: usize| {
        cell % width == width - 1
            && closing_row.map_or(true, |start| (cell / width).abs_diff(start) <= 1)
    };

    while let Some((Reverse(reached_at), cell)) = waiting.pop() {
        if reached_at > scaled(best[cell]) {
            continue;
        }
        if ends_here(cell) {
            let mut path = vec![cell];
            while came_from[*path.last().expect("path has a cell")] != usize::MAX {
                path.push(came_from[*path.last().expect("path has a cell")]);
            }
            path.reverse();
            return Some(path);
        }
        let (x, y) = ((cell % width) as i32, (cell / width) as i32);
        for (&(dx, dy), distance) in D8_OFFSETS.iter().zip(D8_DISTANCES) {
            let (next_x, next_y) = (x + dx, y + dy);
            if next_x < 0 || next_x >= width as i32 || next_y < 0 || next_y >= height as i32 {
                continue;
            }
            let next = next_y as usize * width + next_x as usize;
            let total = best[cell] + cost(next) * distance;
            if total < best[next] {
                best[next] = total;
                came_from[next] = cell;
                waiting.push((Reverse(scaled(total)), next));
            }
        }
    }
    None
}

/// Whether `is_sea` holds a stretch of sea that goes all the way round the
/// world: one that can be followed from the west edge to the east edge and
/// arrives where it set out, across the seam.
pub fn sea_rings_the_world(is_sea: &[bool], width: usize, height: usize) -> bool {
    // Label each stretch of sea, without stepping across the seam.
    let mut stretch_of = vec![usize::MAX; width * height];
    let mut stretches = 0;
    for first in 0..width * height {
        if !is_sea[first] || stretch_of[first] != usize::MAX {
            continue;
        }
        stretch_of[first] = stretches;
        let mut pending = vec![first];
        while let Some(cell) = pending.pop() {
            let (x, y) = ((cell % width) as i32, (cell / width) as i32);
            for (dx, dy) in D8_OFFSETS {
                let (next_x, next_y) = (x + dx, y + dy);
                if next_x < 0 || next_x >= width as i32 || next_y < 0 || next_y >= height as i32 {
                    continue;
                }
                let next = next_y as usize * width + next_x as usize;
                if is_sea[next] && stretch_of[next] == usize::MAX {
                    stretch_of[next] = stretches;
                    pending.push(next);
                }
            }
        }
        stretches += 1;
    }
    // A ring is a stretch that touches both edges at rows that meet.
    (0..height).any(|row| {
        let west = stretch_of[row * width];
        west != usize::MAX
            && (row.saturating_sub(1)..=(row + 1).min(height - 1))
                .any(|east_row| stretch_of[east_row * width + width - 1] == west)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 24 by 9 world: sea along the middle row, with land across it from
    /// column 10 to column 13.
    fn blocked_rim() -> Vec<f64> {
        (0..24 * 9)
            .map(|cell| {
                let (x, y) = (cell % 24, cell / 24);
                if y == 4 && !(10..=13).contains(&x) { -0.2 } else { 0.3 }
            })
            .collect()
    }

    fn sea(continentalness: &[f64]) -> Vec<bool> {
        continentalness.iter().map(|&cont| cont < 0.0).collect()
    }

    #[test]
    fn a_sea_broken_by_land_does_not_ring_the_world() {
        assert!(!sea_rings_the_world(&sea(&blocked_rim()), 24, 9));
    }

    #[test]
    fn opening_the_rim_sea_cuts_through_the_land_in_its_way() {
        let mut continentalness = blocked_rim();
        let opened = open_rim_sea(&mut continentalness, &[true; 24 * 9], 24, 9, 0.0);

        assert!(opened >= 4);
        assert!(sea_rings_the_world(&sea(&continentalness), 24, 9));
        // The strait went through the gap, not somewhere else.
        assert!(continentalness[4 * 24 + 11] < 0.0);
        // Land well away from the route is untouched.
        assert_eq!(continentalness[0], 0.3);
    }

    #[test]
    fn the_route_keeps_to_water_that_stays_liquid() {
        // Two seas ring the world, but the northern one would freeze.
        let mut continentalness: Vec<f64> = (0..24 * 9)
            .map(|cell| {
                let (x, y) = (cell % 24, cell / 24);
                let blocked = y == 6 && (10..=13).contains(&x);
                if (y == 1 || y == 6) && !blocked { -0.2 } else { 0.3 }
            })
            .collect();
        let stays_liquid: Vec<bool> = (0..24 * 9).map(|cell| cell / 24 >= 4).collect();

        open_rim_sea(&mut continentalness, &stays_liquid, 24, 9, 0.0);

        // The strait is cut in the southern sea, though the northern is open.
        assert!(continentalness[6 * 24 + 11] < 0.0);
    }

    #[test]
    fn a_sea_that_meets_itself_across_the_seam_rings_the_world() {
        // Sea along row 4, stepping down to row 5 at the east edge.
        let is_sea: Vec<bool> = (0..24 * 9)
            .map(|cell| {
                let (x, y) = (cell % 24, cell / 24);
                (y == 4 && x < 23) || (y == 5 && x >= 22)
            })
            .collect();
        assert!(sea_rings_the_world(&is_sea, 24, 9));
    }
}
