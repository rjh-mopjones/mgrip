//! The cubed sphere: six square faces of cells laid over the sphere
//! (spec 015).
//!
//! The cube's axis is the sphere's: faces 4 and 5 are centred on the north
//! and south poles, faces 0 to 3 on the equator at longitudes 0°, 90°, 180°
//! and 270°. Each face is `n` by `n` cells in equiangular coordinates: a
//! cell's centre is at angles (α, β) in (−π/4, π/4), evenly spaced, and its
//! point on the sphere is the face's frame applied to (tan α, tan β, 1),
//! normalised. Cells are nearly square everywhere; edges differ by at most
//! 1.3 to 1 over the sphere, and nothing is a sliver.
//!
//! Cells are stored face-major, row-major within a face. The eight
//! neighbours of a cell run across face edges; the twenty-four cells at
//! the cube's corners have seven. Everything that steps between cells,
//! measures them or samples a field between them goes through here.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use crate::sphere::{Point, Sphere, D8_OFFSETS};

/// A face's frame: its centre on the sphere, and the directions its `u`
/// (rightward) and `v` (downward) raster axes point along at the centre.
struct Frame {
    centre: Point,
    right: Point,
    down: Point,
}

/// Faces 0 to 3 round the equator from longitude 0° eastward, each with
/// north at the top; face 4 the north pole, face 5 the south pole.
const FRAMES: [Frame; 6] = [
    Frame {
        centre: [1.0, 0.0, 0.0],
        right: [0.0, 1.0, 0.0],
        down: [0.0, 0.0, -1.0],
    },
    Frame {
        centre: [0.0, 1.0, 0.0],
        right: [-1.0, 0.0, 0.0],
        down: [0.0, 0.0, -1.0],
    },
    Frame {
        centre: [-1.0, 0.0, 0.0],
        right: [0.0, -1.0, 0.0],
        down: [0.0, 0.0, -1.0],
    },
    Frame {
        centre: [0.0, -1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        down: [0.0, 0.0, -1.0],
    },
    // Looking down on the north pole with longitude 0° at the bottom.
    Frame {
        centre: [0.0, 0.0, 1.0],
        right: [0.0, 1.0, 0.0],
        down: [1.0, 0.0, 0.0],
    },
    // Looking up at the south pole with longitude 0° at the top.
    Frame {
        centre: [0.0, 0.0, -1.0],
        right: [0.0, 1.0, 0.0],
        down: [-1.0, 0.0, 0.0],
    },
];

/// The cubed sphere at one resolution, with its per-face tables.
#[derive(Clone)]
pub struct CubeGrid {
    pub sphere: Sphere,
    /// Cells along a face's side.
    pub n: usize,
    /// Great-circle distance from each cell of a face to its eight
    /// neighbours, in world units, `D8_OFFSETS` order; the same on every
    /// face. `f64::NAN` where there is no neighbour.
    steps: Vec<[f64; 8]>,
    /// Area of each cell of a face, in square world units; the same on
    /// every face.
    areas: Vec<f64>,
}

impl CubeGrid {
    pub fn new(sphere: Sphere, n: usize) -> Self {
        let mut grid = Self {
            sphere,
            n,
            steps: Vec::new(),
            areas: Vec::new(),
        };
        grid.steps = (0..n * n)
            .map(|cell| {
                D8_OFFSETS.map(|(dx, dy)| match grid.neighbour(cell, dx, dy) {
                    Some(to) => grid.sphere.distance(grid.point(cell), grid.point(to)),
                    None => f64::NAN,
                })
            })
            .collect();
        grid.areas = (0..n * n).map(|cell| grid.exact_area(cell)).collect();
        grid
    }

    /// Margin at `n` cells a side.
    pub fn margin(n: usize) -> Self {
        Self::new(Sphere::MARGIN, n)
    }

    pub fn cell_count(&self) -> usize {
        6 * self.n * self.n
    }

    pub fn index(&self, face: usize, u: usize, v: usize) -> usize {
        (face * self.n + v) * self.n + u
    }

    /// (face, u, v) of a cell.
    pub fn cell(&self, index: usize) -> (usize, usize, usize) {
        let per_face = self.n * self.n;
        (
            index / per_face,
            index % self.n,
            (index % per_face) / self.n,
        )
    }

    /// The angles of a cell's centre on its face.
    fn angles(&self, u: f64, v: f64) -> (f64, f64) {
        let step = FRAC_PI_2 / self.n as f64;
        ((u + 0.5) * step - FRAC_PI_4, FRAC_PI_4 - (v + 0.5) * step)
    }

    /// The point on the unit sphere at angles (α, β) of a face, which may
    /// lie beyond the face's edges.
    fn point_at_angles(face: usize, alpha: f64, beta: f64) -> Point {
        let frame = &FRAMES[face];
        let (a, b) = (alpha.tan(), beta.tan());
        normalised([
            frame.centre[0] + frame.right[0] * a - frame.down[0] * b,
            frame.centre[1] + frame.right[1] * a - frame.down[1] * b,
            frame.centre[2] + frame.right[2] * a - frame.down[2] * b,
        ])
    }

    /// The point on the unit sphere at angles `(alpha, beta)` on `face`,
    /// each in (-π/4, π/4): the face's own frame, which an image of the
    /// face is laid out in.
    pub fn point_on_face(face: usize, alpha: f64, beta: f64) -> Point {
        Self::point_at_angles(face, alpha, beta)
    }

    /// A cell's centre on the unit sphere.
    pub fn point(&self, index: usize) -> Point {
        let (face, u, v) = self.cell(index);
        let (alpha, beta) = self.angles(u as f64, v as f64);
        Self::point_at_angles(face, alpha, beta)
    }

    /// The face a point falls on, with its angles there.
    fn face_of(point: Point) -> (usize, f64, f64) {
        let [x, y, z] = point;
        let face = if x.abs() >= y.abs() && x.abs() >= z.abs() {
            if x >= 0.0 {
                0
            } else {
                2
            }
        } else if y.abs() >= z.abs() {
            if y >= 0.0 {
                1
            } else {
                3
            }
        } else if z >= 0.0 {
            4
        } else {
            5
        };
        let frame = &FRAMES[face];
        let depth = dot(point, frame.centre);
        let alpha = (dot(point, frame.right) / depth).atan();
        let beta = (-dot(point, frame.down) / depth).atan();
        (face, alpha, beta)
    }

    /// Where a point falls: its face, and its position in cells along and
    /// down that face, continuous, with cell centres at whole numbers plus
    /// a half.
    fn locate(&self, point: Point) -> (usize, f64, f64) {
        let (face, alpha, beta) = Self::face_of(point);
        let step = FRAC_PI_2 / self.n as f64;
        (face, (alpha + FRAC_PI_4) / step, (FRAC_PI_4 - beta) / step)
    }

    /// The cell a point falls in.
    pub fn cell_of(&self, point: Point) -> usize {
        let (face, u, v) = self.locate(point);
        let last = self.n as f64 - 1.0;
        self.index(
            face,
            u.floor().clamp(0.0, last) as usize,
            v.floor().clamp(0.0, last) as usize,
        )
    }

    /// The cell `dx` along and `dy` down from a cell, across a face edge if
    /// need be: the step is taken in the cell's own face's angles and the
    /// cell under the result is found. `None` across a cube corner, where
    /// there is no cell. Exact for a step of one; a cell's own neighbour for
    /// longer walks, which is close enough for blurs and halos.
    pub fn neighbour(&self, index: usize, dx: i32, dy: i32) -> Option<usize> {
        let (face, u, v) = self.cell(index);
        let (nu, nv) = (u as i64 + dx as i64, v as i64 + dy as i64);
        let n = self.n as i64;
        if (0..n).contains(&nu) && (0..n).contains(&nv) {
            return Some(self.index(face, nu as usize, nv as usize));
        }
        let past_corner = (nu < 0 || nu >= n) && (nv < 0 || nv >= n);
        if past_corner && dx.abs() == 1 && dy.abs() == 1 {
            // A cube corner: three faces meet and the diagonal cell is not
            // there. (A longer walk past a corner is answered as best it can
            // be, below.)
            let at_corner = (u == 0 || u == self.n - 1) && (v == 0 || v == self.n - 1);
            if at_corner {
                return None;
            }
        }
        let (alpha, beta) = self.angles(nu as f64, nv as f64);
        if alpha.abs() >= FRAC_PI_2 || beta.abs() >= FRAC_PI_2 {
            return None;
        }
        Some(self.cell_of(Self::point_at_angles(face, alpha, beta)))
    }

    /// The eight neighbours of a cell in `D8_OFFSETS` order, `None` where
    /// a cube corner leaves a gap.
    pub fn neighbours(&self, index: usize) -> [Option<usize>; 8] {
        D8_OFFSETS.map(|(dx, dy)| self.neighbour(index, dx, dy))
    }

    /// Great-circle distance from a cell to each neighbour, in world units,
    /// `D8_OFFSETS` order; `NAN` where there is none.
    pub fn step_distances(&self, index: usize) -> [f64; 8] {
        self.steps[index % (self.n * self.n)]
    }

    /// A cell's area in square world units.
    pub fn area(&self, index: usize) -> f64 {
        self.areas[index % (self.n * self.n)]
    }

    /// The area of a cell as a share of the mean cell's.
    pub fn area_share(&self, index: usize) -> f64 {
        self.area(index) / (self.sphere.area() / self.cell_count() as f64)
    }

    /// Area of a cell from its four corners: a spherical quadrilateral.
    fn exact_area(&self, cell: usize) -> f64 {
        let (u, v) = (cell % self.n, cell / self.n);
        let step = FRAC_PI_2 / self.n as f64;
        let corner = |du: f64, dv: f64| {
            let alpha = (u as f64 + du) * step - FRAC_PI_4;
            let beta = FRAC_PI_4 - (v as f64 + dv) * step;
            Self::point_at_angles(0, alpha, beta)
        };
        let (a, b, c, d) = (
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        );
        let radius = self.sphere.radius();
        (triangle_excess(a, b, c) + triangle_excess(a, c, d)) * radius * radius
    }

    /// A field's value at a point, interpolated in straight lines between
    /// the four cells around it, across face edges.
    pub fn sample(&self, field: &[f64], point: Point) -> f64 {
        self.sample_by(point, |cell| field[cell])
    }

    /// As `sample`, reading the field through `value`, so a field held in
    /// another type (or computed) is sampled without being copied.
    pub fn sample_by(&self, point: Point, value: impl Fn(usize) -> f64) -> f64 {
        let (face, u, v) = self.locate(point);
        let (u, v) = (u - 0.5, v - 0.5);
        let (u0, v0) = (u.floor(), v.floor());
        let (tu, tv) = (u - u0, v - v0);
        let last = self.n as f64 - 1.0;
        let home = self.index(
            face,
            u0.clamp(0.0, last) as usize,
            v0.clamp(0.0, last) as usize,
        );
        let (du, dv) = (
            (u0 - u0.clamp(0.0, last)) as i32,
            (v0 - v0.clamp(0.0, last)) as i32,
        );
        let at = |dx: i32, dy: i32| value(self.neighbour(home, du + dx, dv + dy).unwrap_or(home));
        let top = at(0, 0) + (at(1, 0) - at(0, 0)) * tu;
        let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * tu;
        top + (bottom - top) * tv
    }

    /// A field's value at a point on a smooth curve (a cubic B-spline) over
    /// the cells around it: no creases along cell edges, for shading.
    pub fn sample_smooth(&self, field: &[f64], point: Point) -> f64 {
        self.sample_smooth_by(point, |cell| field[cell])
    }

    /// As `sample_smooth`, reading the field through `value`.
    pub fn sample_smooth_by(&self, point: Point, value: impl Fn(usize) -> f64) -> f64 {
        let (face, u, v) = self.locate(point);
        let (u, v) = (u - 0.5, v - 0.5);
        let (u0, v0) = (u.floor(), v.floor());
        let (tu, tv) = (u - u0, v - v0);
        let last = self.n as f64 - 1.0;
        let home = self.index(
            face,
            u0.clamp(0.0, last) as usize,
            v0.clamp(0.0, last) as usize,
        );
        let (du, dv) = (
            (u0 - u0.clamp(0.0, last)) as i32,
            (v0 - v0.clamp(0.0, last)) as i32,
        );
        let weights = |t: f64| {
            let s = 1.0 - t;
            [
                s * s * s / 6.0,
                (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0,
                (3.0 * s * s * s - 6.0 * s * s + 4.0) / 6.0,
                t * t * t / 6.0,
            ]
        };
        let (wu, wv) = (weights(tu), weights(tv));
        let mut sum = 0.0;
        for (row, weight_v) in wv.iter().enumerate() {
            for (column, weight_u) in wu.iter().enumerate() {
                let cell = self
                    .neighbour(home, du + column as i32 - 1, dv + row as i32 - 1)
                    .unwrap_or(home);
                sum += value(cell) * weight_u * weight_v;
            }
        }
        sum
    }

    /// One face of a field, padded on every side by `width` cells of its
    /// neighbours, as a raster `n + 2 * width` square: a kernel written for
    /// a raster runs on it unchanged. Returns the padded raster.
    pub fn halo(&self, field: &[f64], face: usize, width: usize) -> Vec<f64> {
        let wide = self.n + 2 * width;
        let mut padded = vec![0.0; wide * wide];
        for row in 0..wide {
            for column in 0..wide {
                let (du, dv) = (column as i64 - width as i64, row as i64 - width as i64);
                let (u, v) = (
                    du.clamp(0, self.n as i64 - 1),
                    dv.clamp(0, self.n as i64 - 1),
                );
                let home = self.index(face, u as usize, v as usize);
                let cell = self
                    .neighbour(home, (du - u) as i32, (dv - v) as i32)
                    .unwrap_or(home);
                padded[row * wide + column] = field[cell];
            }
        }
        padded
    }

    /// The cube at half the resolution.
    pub fn halved(&self) -> CubeGrid {
        CubeGrid::new(self.sphere, self.n / 2)
    }

    /// The cube at twice the resolution.
    pub fn doubled(&self) -> CubeGrid {
        CubeGrid::new(self.sphere, self.n * 2)
    }

    /// A field of this cube on the cube of half its resolution: each coarse
    /// cell the mean of its four.
    pub fn halve_field(&self, field: &[f64], coarse: &CubeGrid) -> Vec<f64> {
        (0..coarse.cell_count())
            .map(|cell| {
                let (face, u, v) = coarse.cell(cell);
                let fine = |du: usize, dv: usize| field[self.index(face, 2 * u + du, 2 * v + dv)];
                (fine(0, 0) + fine(1, 0) + fine(0, 1) + fine(1, 1)) / 4.0
            })
            .collect()
    }

    /// A field of this cube on the cube of twice its resolution, filled in
    /// smoothly: each fine cell samples the coarse field at its centre.
    pub fn double_field(&self, field: &[f64], fine: &CubeGrid) -> Vec<f64> {
        (0..fine.cell_count())
            .map(|cell| self.sample(field, fine.point(cell)))
            .collect()
    }
}

impl std::fmt::Debug for CubeGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CubeGrid").field("n", &self.n).finish()
    }
}

/// A tangent vector at a point on the sphere, in world units.
pub type Tangent = Point;

impl CubeGrid {
    /// The mean cell edge, in world units: the unit that distances and
    /// areas are counted in when a calibration speaks of "cells".
    pub fn cell_size(&self) -> f64 {
        (self.sphere.area() / self.cell_count() as f64).sqrt()
    }

    /// Mean cells per world unit.
    pub fn cells_per_world_unit(&self) -> f64 {
        1.0 / self.cell_size()
    }

    /// A cell's centre as a world position.
    pub fn world_position(&self, index: usize) -> (f64, f64) {
        self.sphere.world_at(self.point(index))
    }

    /// Distance between two cells, in mean cells.
    pub fn distance_cells(&self, a: usize, b: usize) -> f64 {
        self.sphere.distance(self.point(a), self.point(b)) * self.cells_per_world_unit()
    }

    /// The eight neighbours of a cell with the distance to each in mean
    /// cells, skipping the gap at a cube corner.
    pub fn steps(&self, index: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let cells_per_wu = self.cells_per_world_unit();
        self.neighbours(index)
            .into_iter()
            .zip(self.step_distances(index))
            .filter_map(move |(to, wu)| to.map(|to| (to, wu * cells_per_wu)))
    }

    /// `field` averaged over a square of `reach` cells each way, run on each
    /// face padded with its neighbours' cells, so the blur crosses face
    /// edges. Separable: rows then columns.
    pub fn blur(&self, field: &[f64], reach: usize) -> Vec<f64> {
        if reach == 0 {
            return field.to_vec();
        }
        let n = self.n;
        let wide = n + 2 * reach;
        let span = (2 * reach + 1) as f64;
        let mut out = vec![0.0; self.cell_count()];
        for face in 0..6 {
            let padded = self.halo(field, face, reach);
            // Along rows, by running sums.
            let mut rows = vec![0.0; wide * wide];
            for row in 0..wide {
                let mut running = vec![0.0; wide + 1];
                for column in 0..wide {
                    running[column + 1] = running[column] + padded[row * wide + column];
                }
                for column in reach..wide - reach {
                    rows[row * wide + column] =
                        (running[column + reach + 1] - running[column - reach]) / span;
                }
            }
            // Down columns, into the face's own cells.
            for v in 0..n {
                for u in 0..n {
                    let column = u + reach;
                    let sum: f64 = (0..=2 * reach)
                        .map(|row| rows[(v + row) * wide + column])
                        .sum();
                    out[self.index(face, u, v)] = sum / span;
                }
            }
        }
        out
    }

    /// The directions east and south along the ground at a cell.
    pub fn tangents(&self, index: usize) -> (Tangent, Tangent) {
        Sphere::tangents(self.point(index))
    }

    /// How `field` rises per world unit at a cell, as a tangent vector: a
    /// least-squares fit over the differences to its neighbours.
    pub fn gradient(&self, field: &[f64], index: usize) -> Tangent {
        let here = self.point(index);
        let (east, south) = self.tangents(index);
        let radius = self.sphere.radius();
        // Normal equations for g = (ge, gs) in the tangent basis.
        let (mut aa, mut ab, mut bb, mut ae, mut be) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for to in self.neighbours(index).into_iter().flatten() {
            let there = self.point(to);
            let offset = [
                (there[0] - here[0]) * radius,
                (there[1] - here[1]) * radius,
                (there[2] - here[2]) * radius,
            ];
            let (de, ds) = (dot(offset, east), dot(offset, south));
            let rise = field[to] - field[index];
            aa += de * de;
            ab += de * ds;
            bb += ds * ds;
            ae += de * rise;
            be += ds * rise;
        }
        let det = aa * bb - ab * ab;
        if det.abs() < 1e-12 {
            return [0.0, 0.0, 0.0];
        }
        let ge = (ae * bb - be * ab) / det;
        let gs = (be * aa - ae * ab) / det;
        [
            east[0] * ge + south[0] * gs,
            east[1] * ge + south[1] * gs,
            east[2] * ge + south[2] * gs,
        ]
    }

    /// The two neighbours a flow along `direction` (a tangent vector) goes
    /// to, with the share each gets: the nearest in direction and the next,
    /// shared by angle, so flow does not run in spokes along eight
    /// directions.
    pub fn downstream(&self, index: usize, direction: Tangent) -> [(usize, f64); 2] {
        let here = self.point(index);
        let speed = dot(direction, direction).sqrt();
        if speed < 1e-12 {
            return [(index, 0.5), (index, 0.5)];
        }
        let mut best: [(usize, f64); 2] = [(index, -2.0), (index, -2.0)];
        for to in self.neighbours(index).into_iter().flatten() {
            let there = self.point(to);
            let step = normalised([there[0] - here[0], there[1] - here[1], there[2] - here[2]]);
            let alignment = dot(step, direction) / speed;
            if alignment > best[0].1 {
                best[1] = best[0];
                best[0] = (to, alignment);
            } else if alignment > best[1].1 {
                best[1] = (to, alignment);
            }
        }
        // Shares by how far off each lies: the better aligned gets more.
        let (a, b) = (best[0].1.acos(), best[1].1.acos());
        let total = (a + b).max(1e-9);
        [(best[0].0, b / total), (best[1].0, a / total)]
    }
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalised(a: Point) -> Point {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}

fn angle(a: Point, b: Point) -> f64 {
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    dot(cross, cross).sqrt().atan2(dot(a, b))
}

/// The spherical excess (area on the unit sphere) of a triangle, by
/// l'Huilier's theorem.
fn triangle_excess(a: Point, b: Point, c: Point) -> f64 {
    let (x, y, z) = (angle(a, b), angle(b, c), angle(c, a));
    let s = (x + y + z) / 2.0;
    let product =
        (s / 2.0).tan() * ((s - x) / 2.0).tan() * ((s - y) / 2.0).tan() * ((s - z) / 2.0).tan();
    4.0 * product.max(0.0).sqrt().atan()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> CubeGrid {
        CubeGrid::margin(16)
    }

    #[test]
    fn every_cell_is_found_under_its_own_centre() {
        let cube = grid();
        for cell in 0..cube.cell_count() {
            assert_eq!(cube.cell_of(cube.point(cell)), cell, "cell {cell}");
        }
    }

    #[test]
    fn the_cells_cover_the_sphere_and_are_nearly_square() {
        let cube = grid();
        let total: f64 = (0..cube.cell_count()).map(|cell| cube.area(cell)).sum();
        assert!((total - cube.sphere.area()).abs() < 1e-6 * total, "{total}");
        let mut longest: f64 = 0.0;
        let mut shortest = f64::MAX;
        for cell in 0..cube.n * cube.n {
            for (direction, step) in cube.step_distances(cell).iter().enumerate() {
                if direction % 2 == 0 && step.is_finite() {
                    longest = longest.max(*step);
                    shortest = shortest.min(*step);
                }
            }
        }
        assert!(longest / shortest < 1.35, "{longest} / {shortest}");
    }

    #[test]
    fn neighbours_are_eight_except_at_the_corners() {
        let cube = grid();
        let mut sevens = 0;
        for cell in 0..cube.cell_count() {
            let count = cube.neighbours(cell).iter().filter(|n| n.is_some()).count();
            match count {
                8 => {}
                7 => sevens += 1,
                _ => panic!("cell {cell} has {count} neighbours"),
            }
        }
        assert_eq!(sevens, 24);
    }

    #[test]
    fn a_neighbour_across_an_edge_is_next_door_and_steps_back() {
        let cube = grid();
        let n = cube.n;
        for face in 0..6 {
            for v in 0..n {
                let cell = cube.index(face, n - 1, v);
                let east = cube.neighbour(cell, 1, 0).expect("an edge neighbour");
                assert_ne!(cube.cell(east).0, face);
                let distance = cube.sphere.distance(cube.point(cell), cube.point(east));
                assert!(distance < cube.step_distances(cell)[0] * 1.5, "{distance}");
                // Stepping back lands home.
                let back = cube
                    .neighbours(east)
                    .iter()
                    .any(|&beside| beside == Some(cell));
                assert!(back, "face {face} row {v}");
            }
        }
    }

    #[test]
    fn a_halo_holds_the_neighbours_cells() {
        let cube = grid();
        let field: Vec<f64> = (0..cube.cell_count()).map(|cell| cell as f64).collect();
        let padded = cube.halo(&field, 2, 1);
        let wide = cube.n + 2;
        // The padded raster's interior is the face itself.
        assert_eq!(padded[1 * wide + 1], cube.index(2, 0, 0) as f64);
        // Its west border is the east neighbour's cells.
        let west = cube.neighbour(cube.index(2, 0, 3), -1, 0).unwrap();
        assert_eq!(padded[4 * wide], west as f64);
    }

    #[test]
    fn sampling_is_exact_on_a_constant_and_close_on_a_smooth_field() {
        let cube = grid();
        let constant = vec![3.5; cube.cell_count()];
        let linear: Vec<f64> = (0..cube.cell_count())
            .map(|cell| cube.point(cell)[2])
            .collect();
        for &(x, y) in &[
            (0.0, 0.0),
            (300.0, 10.0),
            (700.0, 500.0),
            (1023.9, 256.0),
            (128.0, 128.0),
        ] {
            let point = cube.sphere.point_at(x, y);
            assert!((cube.sample(&constant, point) - 3.5).abs() < 1e-12);
            assert!((cube.sample_smooth(&constant, point) - 3.5).abs() < 1e-12);
            assert!(
                (cube.sample(&linear, point) - point[2]).abs() < 0.05,
                "at ({x}, {y})"
            );
        }
    }

    #[test]
    fn a_blur_crosses_face_edges_and_keeps_the_mean() {
        let cube = grid();
        let mut field = vec![0.0; cube.cell_count()];
        let hot = cube.index(0, cube.n - 1, 5);
        field[hot] = 100.0;
        let blurred = cube.blur(&field, 1);
        let east = cube.neighbour(hot, 1, 0).unwrap();
        assert_ne!(cube.cell(east).0, 0);
        assert!(blurred[east] > 5.0, "{}", blurred[east]);
        let total: f64 = blurred.iter().sum();
        assert!((total - 100.0).abs() < 1e-6, "{total}");
    }

    #[test]
    fn the_gradient_of_height_points_north_and_flow_goes_down_it() {
        let cube = grid();
        // Height rises with z: the gradient points north everywhere.
        let field: Vec<f64> = (0..cube.cell_count())
            .map(|cell| cube.point(cell)[2])
            .collect();
        for face in 0..4 {
            let cell = cube.index(face, 7, 7);
            let g = cube.gradient(&field, cell);
            let (_, south) = cube.tangents(cell);
            assert!(dot(g, south) < 0.0, "face {face}");
            assert!((dot(g, g).sqrt() - 1.0 / cube.sphere.radius()).abs() < 0.05);
            // Downhill is south: the two downstream cells both lie south.
            let downhill = [-g[0], -g[1], -g[2]];
            for (to, share) in cube.downstream(cell, downhill) {
                assert!(field[to] < field[cell]);
                assert!(share >= 0.0 && share <= 1.0);
            }
        }
        assert!((cube.cells_per_world_unit() * cube.cell_size() - 1.0).abs() < 1e-12);
        let (wx, wy) = cube.world_position(cube.index(0, 7, 7));
        assert!(wx >= 0.0 && wx < 1024.0 && wy > 0.0 && wy < 512.0);
    }

    #[test]
    fn halving_then_doubling_a_smooth_field_returns_it_closely() {
        let cube = grid();
        let coarse = cube.halved();
        let field: Vec<f64> = (0..cube.cell_count())
            .map(|cell| cube.point(cell)[0])
            .collect();
        let halved = cube.halve_field(&field, &coarse);
        let doubled = coarse.double_field(&halved, &cube);
        let worst = field
            .iter()
            .zip(&doubled)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(worst < 0.08, "{worst}");
    }
}
