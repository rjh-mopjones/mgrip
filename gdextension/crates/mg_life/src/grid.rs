//! Resolution and shape of the cell grid LifeGen runs on.

/// Margin's surface is a cylinder: the macro map's east and west edges are
/// neighbours, its north and south edges are not. Every distance between
/// cells and every step to a neighbouring column goes through this type, so
/// the stages do not each need to know about the seam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub cells_per_world_unit: f64,
    /// Width in cells, if the east and west edges are neighbours.
    pub wrap_width: Option<usize>,
}

impl Grid {
    /// A grid whose edges are not joined.
    pub fn flat(cells_per_world_unit: f64) -> Self {
        Self {
            cells_per_world_unit,
            wrap_width: None,
        }
    }

    /// A grid `width` cells around, joined east to west.
    pub fn ring(cells_per_world_unit: f64, width: usize) -> Self {
        Self {
            cells_per_world_unit,
            wrap_width: Some(width),
        }
    }

    /// Signed distance in cells from column `from_x` to column `to_x`, the
    /// short way round.
    pub fn dx(&self, from_x: usize, to_x: usize) -> f64 {
        let direct = to_x as f64 - from_x as f64;
        let Some(width) = self.wrap_width else {
            return direct;
        };
        let width = width as f64;
        if direct > width / 2.0 {
            direct - width
        } else if direct < -width / 2.0 {
            direct + width
        } else {
            direct
        }
    }

    /// Straight-line distance in cells between two cells, the short way round.
    pub fn distance(&self, a: (usize, usize), b: (usize, usize)) -> f64 {
        let dx = self.dx(a.0, b.0);
        let dy = b.1 as f64 - a.1 as f64;
        (dx * dx + dy * dy).sqrt()
    }

    /// The column `dx` columns from `x` in a row `width` columns wide.
    /// `None` if that is off the edge of a grid that is not joined.
    pub fn step_x(&self, x: usize, dx: i32, width: usize) -> Option<usize> {
        let stepped = x as i64 + dx as i64;
        if self.wrap_width.is_some() {
            Some(stepped.rem_euclid(width as i64) as usize)
        } else if (0..width as i64).contains(&stepped) {
            Some(stepped as usize)
        } else {
            None
        }
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
    }

    #[test]
    fn a_ring_measures_the_short_way_round() {
        let grid = Grid::ring(1.0, 100);

        assert_eq!(grid.dx(2, 98), -4.0);
        assert_eq!(grid.dx(98, 2), 4.0);
        assert_eq!(grid.dx(10, 40), 30.0);
        assert_eq!(grid.distance((2, 0), (98, 3)), 5.0);
    }

    #[test]
    fn stepping_off_the_edge_wraps_only_on_a_ring() {
        assert_eq!(Grid::flat(1.0).step_x(0, -1, 100), None);
        assert_eq!(Grid::flat(1.0).step_x(99, 1, 100), None);
        assert_eq!(Grid::flat(1.0).step_x(5, 1, 100), Some(6));
        assert_eq!(Grid::ring(1.0, 100).step_x(0, -1, 100), Some(99));
        assert_eq!(Grid::ring(1.0, 100).step_x(99, 1, 100), Some(0));
    }
}
