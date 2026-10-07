//! Resolution and shape of the cell grid LifeGen runs on.

use std::collections::VecDeque;
use std::sync::Arc;

use mg_core::{CubeGrid, Sphere, SphereGrid};

/// The grid behind a terrain query. Margin's is the cubed sphere (spec 017):
/// the six faces of the cube stacked into one raster, `n` cells wide and
/// `6n` high, whose rows `fn..fn+n` are face `f`. A step off a face's edge
/// goes onto the face next to it, every cell is about the same size, and
/// no cell is a pole. A latitude-longitude raster over a sphere (spec 014)
/// and a flat patch (tests) are kept for the stages that still run on
/// them. Every distance between cells, every step to a neighbour and every
/// cell's area goes through this type, so the stages do not each need to
/// know the shape of the world.
#[derive(Debug, Clone)]
pub struct Grid {
    pub cells_per_world_unit: f64,
    shape: Shape,
}

#[derive(Debug, Clone)]
enum Shape {
    /// A flat patch whose edges are edges.
    Flat,
    /// A raster of longitude by latitude over the whole sphere.
    Sphere(SphereGrid),
    /// The cubed sphere, its faces stacked.
    Cube(Arc<CubeGrid>),
}

impl Grid {
    /// A flat patch whose edges are not joined.
    pub const fn flat(cells_per_world_unit: f64) -> Self {
        Self {
            cells_per_world_unit,
            shape: Shape::Flat,
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
            shape: Shape::Sphere(sphere.grid(width, height)),
        }
    }

    /// The cubed sphere, its faces stacked into a raster `n` wide and `6n`
    /// high. Distances and areas are in mean cells.
    pub fn cube(cube: Arc<CubeGrid>) -> Self {
        Self {
            cells_per_world_unit: cube.cells_per_world_unit(),
            shape: Shape::Cube(cube),
        }
    }

    /// The cube this grid is, if it is one.
    pub fn as_cube(&self) -> Option<&CubeGrid> {
        match &self.shape {
            Shape::Cube(cube) => Some(cube),
            _ => None,
        }
    }

    /// The raster a cube is stacked into: `n` wide, `6n` high.
    pub fn cube_raster_size(cube: &CubeGrid) -> (usize, usize) {
        (cube.n, 6 * cube.n)
    }

    fn cube_index(cube: &CubeGrid, x: usize, y: usize) -> usize {
        y * cube.n + x
    }

    fn cube_cell(cube: &CubeGrid, index: usize) -> (usize, usize) {
        (index % cube.n, index / cube.n)
    }

    /// Distance in cells between two cells: a great circle on a sphere, in
    /// equatorial cells for a raster and mean cells for the cube; a straight
    /// line on a flat patch.
    pub fn distance(&self, a: (usize, usize), b: (usize, usize)) -> f64 {
        match &self.shape {
            Shape::Sphere(grid) => grid.distance(a, b) * self.cells_per_world_unit,
            Shape::Cube(cube) => cube.distance_cells(
                Self::cube_index(cube, a.0, a.1),
                Self::cube_index(cube, b.0, b.1),
            ),
            Shape::Flat => {
                let dx = b.0 as f64 - a.0 as f64;
                let dy = b.1 as f64 - a.1 as f64;
                (dx * dx + dy * dy).sqrt()
            }
        }
    }

    /// The cell `dx` columns and `dy` rows from (x, y) on a grid `width` by
    /// `height` cells, or `None` off the edge of a flat patch or across a
    /// cube's corner.
    pub fn step(
        &self,
        x: usize,
        y: usize,
        dx: i32,
        dy: i32,
        width: usize,
        height: usize,
    ) -> Option<(usize, usize)> {
        match &self.shape {
            Shape::Sphere(grid) => Some(grid.neighbour(x, y, dx, dy)),
            Shape::Cube(cube) => cube
                .neighbour(Self::cube_index(cube, x, y), dx, dy)
                .map(|index| Self::cube_cell(cube, index)),
            Shape::Flat => {
                let (nx, ny) = (x as i64 + dx as i64, y as i64 + dy as i64);
                let inside = (0..width as i64).contains(&nx) && (0..height as i64).contains(&ny);
                inside.then_some((nx as usize, ny as usize))
            }
        }
    }

    /// The eight neighbours of a cell with the distance to each, in cells.
    pub fn neighbours(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> Vec<((usize, usize), f64)> {
        match &self.shape {
            Shape::Cube(cube) => cube
                .steps(Self::cube_index(cube, x, y))
                .map(|(index, distance)| (Self::cube_cell(cube, index), distance))
                .collect(),
            _ => OFFSETS
                .iter()
                .filter_map(|&(dx, dy)| {
                    let to = self.step(x, y, dx, dy, width, height)?;
                    Some((to, self.distance((x, y), to)))
                })
                .collect(),
        }
    }

    /// A cell's area as a share of a reference cell's: an equatorial cell
    /// on a raster, the mean cell on the cube, and 1 on a flat patch.
    pub fn area_share(&self, x: usize, y: usize) -> f64 {
        match &self.shape {
            Shape::Sphere(grid) => grid.area_share(y),
            Shape::Cube(cube) => cube.area_share(Self::cube_index(cube, x, y)),
            Shape::Flat => 1.0,
        }
    }

    /// Coarse buckets over the grid, each about `bucket_cells` cells across,
    /// for finding what lies near a cell without scanning everything.
    pub fn buckets(&self, bucket_cells: f64, width: usize, height: usize) -> Buckets {
        let bucket_cells = bucket_cells.max(1.0);
        match &self.shape {
            Shape::Cube(cube) => {
                let n = (cube.n as f64 / bucket_cells).ceil().max(1.0) as usize;
                Buckets::Cube {
                    fine: Arc::clone(cube),
                    coarse: CubeGrid::new(cube.sphere, n),
                }
            }
            shape => Buckets::Raster {
                wide: (width as f64 / bucket_cells).ceil() as usize,
                high: (height as f64 / bucket_cells).ceil() as usize,
                bucket_cells,
                wraps: matches!(shape, Shape::Sphere(_)),
            },
        }
    }
}

const OFFSETS: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

/// Coarse cells laid over a grid (see `Grid::buckets`).
pub enum Buckets {
    Raster {
        wide: usize,
        high: usize,
        bucket_cells: f64,
        wraps: bool,
    },
    Cube {
        fine: Arc<CubeGrid>,
        coarse: CubeGrid,
    },
}

impl Buckets {
    pub fn len(&self) -> usize {
        match self {
            Buckets::Raster { wide, high, .. } => wide * high,
            Buckets::Cube { coarse, .. } => coarse.cell_count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bucket a cell falls in.
    pub fn of(&self, x: usize, y: usize) -> usize {
        match self {
            Buckets::Raster {
                wide, bucket_cells, ..
            } => (y as f64 / bucket_cells) as usize * wide + (x as f64 / bucket_cells) as usize,
            Buckets::Cube { fine, coarse } => {
                coarse.cell_of(fine.point(Grid::cube_index(fine, x, y)))
            }
        }
    }

    /// Every bucket within `range` buckets of `bucket`, itself included.
    pub fn nearby(&self, bucket: usize, range: usize) -> Vec<usize> {
        match self {
            Buckets::Raster {
                wide, high, wraps, ..
            } => {
                let (bx, by) = ((bucket % wide) as i64, (bucket / wide) as i64);
                let range = range as i64;
                let mut found = Vec::new();
                for y in (by - range).max(0)..=(by + range).min(*high as i64 - 1) {
                    for dx in -range..=range {
                        let x = bx + dx;
                        let x = if *wraps {
                            x.rem_euclid(*wide as i64)
                        } else if (0..*wide as i64).contains(&x) {
                            x
                        } else {
                            continue;
                        };
                        found.push(y as usize * wide + x as usize);
                    }
                }
                found.sort_unstable();
                found.dedup();
                found
            }
            Buckets::Cube { coarse, .. } => {
                let mut seen = vec![false; coarse.cell_count()];
                let mut found = vec![bucket];
                seen[bucket] = true;
                let mut pending = VecDeque::from([(bucket, 0usize)]);
                while let Some((cell, steps)) = pending.pop_front() {
                    if steps == range {
                        continue;
                    }
                    for next in coarse.neighbours(cell).into_iter().flatten() {
                        if !seen[next] {
                            seen[next] = true;
                            found.push(next);
                            pending.push_back((next, steps + 1));
                        }
                    }
                }
                found
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_grid_measures_straight_across() {
        let grid = Grid::flat(1.0);

        assert_eq!(grid.distance((2, 0), (98, 0)), 96.0);
        assert_eq!(grid.area_share(0, 0), 1.0);
        assert_eq!(grid.step(0, 0, -1, 0, 100, 50), None);
        assert_eq!(grid.neighbours(0, 0, 100, 50).len(), 3);
    }

    #[test]
    fn a_sphere_measures_the_short_way_round() {
        // 100 cells round, 50 from pole to pole: rows 24 and 25 straddle
        // the equator, where cells are a world unit square.
        let grid = Grid::sphere(1.0, 100, 50);

        let across_the_seam = grid.distance((2, 25), (98, 28));
        assert!((across_the_seam - 5.0).abs() < 0.1, "{across_the_seam}");
        assert!(grid.area_share(0, 25) > 0.99 && grid.area_share(0, 0) < 0.1);
        // A step past the top edge comes down the far side of the pole.
        assert_eq!(grid.step(5, 0, 0, -1, 100, 50), Some((55, 0)));
    }

    #[test]
    fn a_cube_steps_across_its_face_edges_and_measures_true_distances() {
        let cube = Arc::new(CubeGrid::margin(8));
        let grid = Grid::cube(Arc::clone(&cube));
        let (width, height) = Grid::cube_raster_size(&cube);
        assert_eq!((width, height), (8, 48));

        // The last column of face 0 steps east onto face 1.
        let (x, y) = grid.step(7, 3, 1, 0, width, height).unwrap();
        assert_eq!(y / 8, 1, "face 1, got ({x}, {y})");
        // Cells are all about the same size.
        let shares: Vec<f64> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| grid.area_share(x, y))
            .collect();
        let (least, most) = shares
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        assert!(most / least < 2.0, "{least} to {most}");
        // A neighbour is about one cell away, and a cell on the far side of
        // the sphere about half the circumference.
        let near = grid.neighbours(3, 3, width, height);
        assert_eq!(near.len(), 8);
        assert!(near.iter().all(|&(_, d)| d > 0.5 && d < 1.6));
        let far = grid.distance((4, 4), (4, 2 * 8 + 4));
        let half_round = 1024.0 * grid.cells_per_world_unit / 2.0;
        assert!(
            (far - half_round).abs() / half_round < 0.1,
            "{far} vs {half_round}"
        );
    }

    #[test]
    fn buckets_find_their_neighbours_on_every_shape() {
        let flat = Grid::flat(1.0).buckets(10.0, 100, 50);
        assert_eq!(flat.of(0, 0), 0);
        assert_eq!(flat.nearby(0, 1).len(), 4);
        let sphere = Grid::sphere(1.0, 100, 50).buckets(10.0, 100, 50);
        assert_eq!(sphere.nearby(0, 1).len(), 6);

        let cube = Arc::new(CubeGrid::margin(16));
        let grid = Grid::cube(Arc::clone(&cube));
        let buckets = grid.buckets(4.0, 16, 96);
        let own = buckets.of(15, 15);
        assert!(buckets.nearby(own, 1).contains(&own));
        // A bucket at a face corner still has neighbours on the faces
        // across the edge.
        assert!(buckets.nearby(own, 1).len() >= 7);
        assert!(buckets.nearby(own, 2).len() > buckets.nearby(own, 1).len());
    }
}
