//! Resolution and shape of the cell grid LifeGen runs on.

use mg_core::{Sphere, SphereGrid};

/// The grid behind a terrain query. Margin's lies over the sphere (spec
/// 014): east and west join, a step past a pole comes down the far side of
/// it, cells narrow towards the poles and distances are great circles.
/// Tests run on a flat patch instead, whose edges are edges. Every distance
/// between cells and every step to a neighbour goes through this type, so
/// the stages do not each need to know the shape of the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub cells_per_world_unit: f64,
    /// The sphere the grid lies over, or none for a flat patch.
    sphere: Option<SphereGrid>,
}

impl Grid {
    /// A flat patch whose edges are not joined.
    pub const fn flat(cells_per_world_unit: f64) -> Self {
        Self {
            cells_per_world_unit,
            sphere: None,
        }
    }

    /// A grid of `width` by `height` cells over a whole sphere: `width`
    /// cells round its equator at this resolution.
    pub fn sphere(cells_per_world_unit: f64, width: usize, height: usize) -> Self {
        let sphere = Sphere {
            circumference: width as f64 / cells_per_world_unit,
        };
        Self {
            cells_per_world_unit,
            sphere: Some(sphere.grid(width, height)),
        }
    }

    pub fn is_sphere(&self) -> bool {
        self.sphere.is_some()
    }

    /// Signed columns from `from_x` to `to_x`, the short way round.
    pub fn dx(&self, from_x: usize, to_x: usize) -> f64 {
        match self.sphere {
            Some(grid) => grid.dx(from_x, to_x),
            None => to_x as f64 - from_x as f64,
        }
    }

    /// Distance in cells between two cells: a great circle on the sphere,
    /// in equatorial cells; a straight line on a flat patch.
    pub fn distance(&self, a: (usize, usize), b: (usize, usize)) -> f64 {
        match self.sphere {
            Some(grid) => grid.distance(a, b) * self.cells_per_world_unit,
            None => {
                let dx = b.0 as f64 - a.0 as f64;
                let dy = b.1 as f64 - a.1 as f64;
                (dx * dx + dy * dy).sqrt()
            }
        }
    }

    /// The cell `dx` columns and `dy` rows from (x, y) on a grid `width` by
    /// `height` cells, or `None` off the edge of a flat patch.
    pub fn step(
        &self,
        x: usize,
        y: usize,
        dx: i32,
        dy: i32,
        width: usize,
        height: usize,
    ) -> Option<(usize, usize)> {
        match self.sphere {
            Some(grid) => Some(grid.neighbour(x, y, dx, dy)),
            None => {
                let (nx, ny) = (x as i64 + dx as i64, y as i64 + dy as i64);
                let inside = (0..width as i64).contains(&nx) && (0..height as i64).contains(&ny);
                inside.then_some((nx as usize, ny as usize))
            }
        }
    }

    /// The column `dx` columns from `x` in a row `width` columns wide, for
    /// grids laid over this one (buckets). `None` off the edge of a flat
    /// patch.
    pub fn step_x(&self, x: usize, dx: i32, width: usize) -> Option<usize> {
        let stepped = x as i64 + dx as i64;
        if self.sphere.is_some() {
            Some(stepped.rem_euclid(width as i64) as usize)
        } else if (0..width as i64).contains(&stepped) {
            Some(stepped as usize)
        } else {
            None
        }
    }

    /// A row's cell area as a share of an equatorial cell's: 1 on a flat
    /// patch, and at the equator; near 0 at the poles.
    pub fn area_share(&self, y: usize) -> f64 {
        self.sphere.map_or(1.0, |grid| grid.area_share(y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_grid_measures_straight_across() {
        let grid = Grid::flat(1.0);

        assert_eq!(grid.dx(2, 98), 96.0);
        assert_eq!(grid.distance((2, 0), (98, 0)), 96.0);
        assert_eq!(grid.area_share(0), 1.0);
    }

    #[test]
    fn a_sphere_measures_the_short_way_round() {
        // 100 cells round, 50 from pole to pole: rows 24 and 25 straddle
        // the equator, where cells are a world unit square.
        let grid = Grid::sphere(1.0, 100, 50);

        assert_eq!(grid.dx(2, 98), -4.0);
        assert_eq!(grid.dx(98, 2), 4.0);
        assert_eq!(grid.dx(10, 40), 30.0);
        let across_the_seam = grid.distance((2, 25), (98, 28));
        assert!((across_the_seam - 5.0).abs() < 0.1, "{across_the_seam}");
        assert!(grid.area_share(25) > 0.99 && grid.area_share(0) < 0.1);
    }

    #[test]
    fn stepping_off_the_edge_wraps_only_on_a_sphere() {
        let flat = Grid::flat(1.0);
        let sphere = Grid::sphere(1.0, 100, 50);
        assert_eq!(flat.step_x(0, -1, 100), None);
        assert_eq!(flat.step_x(99, 1, 100), None);
        assert_eq!(flat.step_x(5, 1, 100), Some(6));
        assert_eq!(sphere.step_x(0, -1, 100), Some(99));
        assert_eq!(sphere.step_x(99, 1, 100), Some(0));
        // A step past the top edge: off a flat patch, over a sphere's pole.
        assert_eq!(flat.step(5, 0, 0, -1, 100, 50), None);
        assert_eq!(sphere.step(5, 0, 0, -1, 100, 50), Some((55, 0)));
    }
}
