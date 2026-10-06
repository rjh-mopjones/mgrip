//! The planet's surface, and the equirectangular grids laid over it
//! (spec 014).
//!
//! Margin is a sphere. A world position is a longitude east and a latitude
//! south of the north pole, in world units: `wx` runs round the equator,
//! `wy` from the north pole (the anti-stellar point) to the south pole (the
//! sub-stellar point). A grid is a raster of cells over the whole sphere,
//! one row per band of latitude. Everything that measures a distance, an
//! area, a slope or a step to a neighbour does it through here, so this is
//! the only place that knows the projection.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// World units round the equator. One world unit is one chunk.
pub const WORLD_WIDTH: f64 = 1024.0;
/// World units from pole to pole: half the way round.
pub const WORLD_HEIGHT: f64 = WORLD_WIDTH / 2.0;

/// The eight neighbours of a cell as (dx, dy) steps, clockwise from north.
/// The order is angular, so a direction can be read as an index.
pub const D8_OFFSETS: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

/// A point on the unit sphere. `z` points at the north pole; `x` at
/// longitude zero on the equator.
pub type Point = [f64; 3];

/// The planet: a sphere whose equator is `circumference` world units round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub circumference: f64,
}

impl Sphere {
    /// Margin itself.
    pub const MARGIN: Sphere = Sphere {
        circumference: WORLD_WIDTH,
    };

    pub fn radius(&self) -> f64 {
        self.circumference / TAU
    }

    /// World units from pole to pole.
    pub fn height(&self) -> f64 {
        self.circumference / 2.0
    }

    /// Surface area, in square world units.
    pub fn area(&self) -> f64 {
        4.0 * PI * self.radius() * self.radius()
    }

    /// Longitude (0 to 2π, east) and latitude (−π/2 at the south pole to
    /// π/2 at the north) of a world position.
    pub fn lonlat_at(&self, wx: f64, wy: f64) -> (f64, f64) {
        let longitude = (wx / self.circumference * TAU).rem_euclid(TAU);
        let latitude = (FRAC_PI_2 - wy / self.height() * PI).clamp(-FRAC_PI_2, FRAC_PI_2);
        (longitude, latitude)
    }

    /// The point on the unit sphere under a world position. Continuous
    /// across the seam and over the poles.
    pub fn point_at(&self, wx: f64, wy: f64) -> Point {
        let (longitude, latitude) = self.lonlat_at(wx, wy);
        point_of(longitude, latitude)
    }

    /// The world position of a point: `wx` in [0, circumference), `wy` in
    /// [0, height].
    pub fn world_at(&self, point: Point) -> (f64, f64) {
        let (longitude, latitude) = lonlat_of(point);
        (
            longitude / TAU * self.circumference,
            (FRAC_PI_2 - latitude) / PI * self.height(),
        )
    }

    /// The angle at the centre between two points, 0 to π.
    pub fn angle(&self, a: Point, b: Point) -> f64 {
        angle_between(a, b)
    }

    /// Great-circle distance between two points, in world units.
    pub fn distance(&self, a: Point, b: Point) -> f64 {
        self.angle(a, b) * self.radius()
    }

    /// Great-circle distance between two world positions, in world units.
    pub fn distance_between(&self, a: (f64, f64), b: (f64, f64)) -> f64 {
        self.distance(self.point_at(a.0, a.1), self.point_at(b.0, b.1))
    }

    /// Where to sample a 3D noise so that it has `frequency` cycles per
    /// world unit along the ground: the point on a sphere of the right
    /// radius. Seamless everywhere.
    pub fn noise_point_at(&self, wx: f64, wy: f64, frequency: f64) -> Point {
        let scale = self.radius() * frequency;
        let point = self.point_at(wx, wy);
        [point[0] * scale, point[1] * scale, point[2] * scale]
    }

    /// The directions east and south along the ground at a point. At a
    /// pole, where there is no east, the x axis stands in.
    pub fn tangents(point: Point) -> (Point, Point) {
        let flat = (point[0] * point[0] + point[1] * point[1]).sqrt();
        let east = if flat < 1e-9 {
            [1.0, 0.0, 0.0]
        } else {
            [-point[1] / flat, point[0] / flat, 0.0]
        };
        let north = [
            point[1] * east[2] - point[2] * east[1],
            point[2] * east[0] - point[0] * east[2],
            point[0] * east[1] - point[1] * east[0],
        ];
        (east, [-north[0], -north[1], -north[2]])
    }

    /// A point moved along the ground by `east_wu` and `south_wu` world
    /// units: close for small moves, and a fair warp for larger ones.
    pub fn moved(&self, point: Point, east_wu: f64, south_wu: f64) -> Point {
        let (east, south) = Self::tangents(point);
        let (e, s) = (east_wu / self.radius(), south_wu / self.radius());
        let moved = [
            point[0] + east[0] * e + south[0] * s,
            point[1] + east[1] * e + south[1] * s,
            point[2] + east[2] * e + south[2] * s,
        ];
        let length = (moved[0] * moved[0] + moved[1] * moved[1] + moved[2] * moved[2]).sqrt();
        [moved[0] / length, moved[1] / length, moved[2] / length]
    }

    /// A raster of `width` by `height` cells over the whole sphere.
    pub fn grid(&self, width: usize, height: usize) -> SphereGrid {
        SphereGrid {
            sphere: *self,
            width,
            height,
        }
    }
}

/// A raster of cells over the whole sphere: `width` columns of longitude
/// round it, `height` rows of latitude from the north pole to the south.
/// Cell (x, y) is centred at world position (x + 0.5, y + 0.5) cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphereGrid {
    pub sphere: Sphere,
    pub width: usize,
    pub height: usize,
}

impl SphereGrid {
    /// Margin at one cell per world unit.
    pub const MARGIN: SphereGrid = SphereGrid {
        sphere: Sphere::MARGIN,
        width: WORLD_WIDTH as usize,
        height: WORLD_HEIGHT as usize,
    };

    pub fn cells_per_world_unit(&self) -> f64 {
        self.width as f64 / self.sphere.circumference
    }

    /// World units per cell along a meridian; the same on every row.
    pub fn cell_height(&self) -> f64 {
        self.sphere.height() / self.height as f64
    }

    /// World units a cell of row `y` spans along its parallel: an
    /// equatorial cell's width times the cosine of the latitude.
    pub fn cell_width(&self, y: usize) -> f64 {
        self.sphere.circumference / self.width as f64 * self.latitude(y).cos()
    }

    /// Area of a cell of row `y`, in square world units. Exact, so that the
    /// cells of a grid sum to the sphere's area.
    pub fn cell_area(&self, y: usize) -> f64 {
        let radius = self.sphere.radius();
        let step = TAU / self.width as f64;
        let (top, bottom) = (self.latitude_edge(y), self.latitude_edge(y + 1));
        radius * radius * step * (top.sin() - bottom.sin())
    }

    /// Latitude of the centre of row `y`.
    pub fn latitude(&self, y: usize) -> f64 {
        FRAC_PI_2 - (y as f64 + 0.5) / self.height as f64 * PI
    }

    /// Latitude of the northern edge of row `y`.
    fn latitude_edge(&self, y: usize) -> f64 {
        FRAC_PI_2 - y as f64 / self.height as f64 * PI
    }

    /// Longitude of the centre of column `x`.
    pub fn longitude(&self, x: usize) -> f64 {
        (x as f64 + 0.5) / self.width as f64 * TAU
    }

    /// The point on the unit sphere at the centre of a cell.
    pub fn point(&self, x: usize, y: usize) -> Point {
        point_of(self.longitude(x), self.latitude(y))
    }

    /// The cell a point falls in.
    pub fn cell_of(&self, point: Point) -> (usize, usize) {
        let (wx, wy) = self.sphere.world_at(point);
        self.cell_at(wx, wy)
    }

    /// The cell under a world position.
    pub fn cell_at(&self, wx: f64, wy: f64) -> (usize, usize) {
        let x = (wx * self.cells_per_world_unit()).floor() as i64;
        let y = (wy / self.cell_height()).floor() as i64;
        (
            self.wrap_column(x),
            y.clamp(0, self.height as i64 - 1) as usize,
        )
    }

    /// Column `x` brought back onto the grid, the world being round.
    pub fn wrap_column(&self, x: i64) -> usize {
        x.rem_euclid(self.width as i64) as usize
    }

    /// Any column and row brought onto the grid. East and west join. A row
    /// past a pole comes down the far side of it: the cell half a world
    /// away, as many rows in.
    pub fn wrap_cell(&self, x: i64, y: i64) -> (usize, usize) {
        let (mut column, mut row) = (x, y);
        if row < 0 {
            row = -row - 1;
            column += self.width as i64 / 2;
        } else if row >= self.height as i64 {
            row = 2 * self.height as i64 - 1 - row;
            column += self.width as i64 / 2;
        }
        (self.wrap_column(column), row as usize)
    }

    /// The cell `dx` columns east and `dy` rows south of (x, y).
    pub fn neighbour(&self, x: usize, y: usize, dx: i32, dy: i32) -> (usize, usize) {
        self.wrap_cell(x as i64 + dx as i64, y as i64 + dy as i64)
    }

    /// The eight neighbours of a cell, in `D8_OFFSETS` order.
    pub fn neighbours(&self, x: usize, y: usize) -> [(usize, usize); 8] {
        D8_OFFSETS.map(|(dx, dy)| self.neighbour(x, y, dx, dy))
    }

    /// A cell's place in a row-major field.
    pub fn index(&self, (x, y): (usize, usize)) -> usize {
        y * self.width + x
    }

    /// The cell at a place in a row-major field.
    pub fn cell(&self, index: usize) -> (usize, usize) {
        (index % self.width, index / self.width)
    }

    /// Great-circle distance between the centres of two cells, in world
    /// units.
    pub fn distance(&self, a: (usize, usize), b: (usize, usize)) -> f64 {
        self.sphere
            .distance(self.point(a.0, a.1), self.point(b.0, b.1))
    }

    /// Distance from a cell to each of its eight neighbours, in world
    /// units, for every row: the same all along a row. Indexed by row, then
    /// in `D8_OFFSETS` order.
    pub fn row_step_distances(&self) -> Vec<[f64; 8]> {
        (0..self.height)
            .map(|y| self.neighbours(0, y).map(|to| self.distance((0, y), to)))
            .collect()
    }

    /// The area of an equatorial cell: a cell's width at the equator times
    /// its height. The unit that areas and flows are counted in.
    pub fn equatorial_cell_area(&self) -> f64 {
        self.sphere.circumference / self.width as f64 * self.cell_height()
    }

    /// A row's cell area as a share of an equatorial cell's: 1 at the
    /// equator, near 0 at the poles.
    pub fn area_share(&self, y: usize) -> f64 {
        self.cell_area(y) / self.equatorial_cell_area()
    }

    /// The whole sphere's area in equatorial cells: what a count of every
    /// cell would be if they were all the size of one at the equator.
    pub fn cells_of_area(&self) -> f64 {
        self.sphere.area() / self.equatorial_cell_area()
    }

    /// How many cells along row `y` a reach of `cells` equatorial cells
    /// spans: more towards the poles, where cells are narrow, but never
    /// more than half the way round.
    pub fn reach_along_row(&self, y: usize, cells: i32) -> i32 {
        let widened = cells as f64 / self.latitude(y).cos().max(1e-9);
        (widened.round() as i32).min(self.width as i32 / 2)
    }

    /// Signed columns from `from_x` to `to_x`, the short way round.
    pub fn dx(&self, from_x: usize, to_x: usize) -> f64 {
        let direct = to_x as f64 - from_x as f64;
        let width = self.width as f64;
        if direct > width / 2.0 {
            direct - width
        } else if direct < -width / 2.0 {
            direct + width
        } else {
            direct
        }
    }

    /// Where to sample a 3D noise for a cell, with `frequency` cycles per
    /// world unit along the ground.
    pub fn noise_point(&self, x: usize, y: usize, frequency: f64) -> Point {
        self.sphere.noise_point_at(
            (x as f64 + 0.5) / self.cells_per_world_unit(),
            (y as f64 + 0.5) * self.cell_height(),
            frequency,
        )
    }
}

fn point_of(longitude: f64, latitude: f64) -> Point {
    let flat = latitude.cos();
    [
        flat * longitude.cos(),
        flat * longitude.sin(),
        latitude.sin(),
    ]
}

fn lonlat_of(point: Point) -> (f64, f64) {
    let longitude = point[1].atan2(point[0]).rem_euclid(TAU);
    let latitude = point[2].clamp(-1.0, 1.0).asin();
    (longitude, latitude)
}

fn angle_between(a: Point, b: Point) -> f64 {
    // atan2 of the cross and dot products is accurate at every angle,
    // where acos of the dot product loses precision near 0 and π.
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let sine = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    let cosine = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    sine.atan2(cosine)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: SphereGrid = SphereGrid::MARGIN;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    fn same_point(a: Point, b: Point) -> bool {
        angle_between(a, b) < 1e-9
    }

    #[test]
    fn the_cells_of_a_grid_cover_the_sphere_exactly() {
        let total: f64 = (0..GRID.height)
            .map(|y| GRID.cell_area(y) * GRID.width as f64)
            .sum();
        assert!(close(total, Sphere::MARGIN.area(), 1e-6 * total));
    }

    #[test]
    fn cells_are_square_at_the_equator_and_slivers_at_the_poles() {
        let equator = GRID.height / 2;
        assert!(close(GRID.cell_width(equator), GRID.cell_height(), 1e-4));
        assert!(GRID.cell_width(0) < GRID.cell_height() * 0.01);
        assert!(GRID.cell_area(0) < GRID.cell_area(equator) * 0.01);
    }

    #[test]
    fn an_east_neighbour_is_a_cell_width_away() {
        for y in [0, 100, GRID.height / 2, GRID.height - 1] {
            let expected = GRID.cell_width(y);
            let found = GRID.distance((10, y), (11, y));
            assert!(
                close(found, expected, expected * 1e-3),
                "row {y}: {found} vs {expected}"
            );
        }
    }

    #[test]
    fn half_way_round_the_equator_is_half_the_circumference() {
        let sphere = Sphere::MARGIN;
        let found = sphere.distance_between((0.0, 256.0), (512.0, 256.0));
        assert!(close(found, WORLD_WIDTH / 2.0, 1e-9));
        // Cell centres sit half a cell off the equator, so the great circle
        // between two opposite cells cuts the corner by about a cell.
        let equator = GRID.height / 2;
        let between_cells = GRID.distance((0, equator), (GRID.width / 2, equator));
        assert!(close(between_cells, WORLD_WIDTH / 2.0, GRID.cell_height()));
    }

    #[test]
    fn a_step_north_from_the_top_row_comes_down_the_far_side() {
        assert_eq!(GRID.neighbour(10, 0, 0, -1), (10 + GRID.width / 2, 0));
        assert_eq!(GRID.neighbour(10, 0, 1, -1), (11 + GRID.width / 2, 0));
        assert_eq!(
            GRID.neighbour(1000, GRID.height - 1, 0, 1),
            (1000 - GRID.width / 2, GRID.height - 1)
        );
        assert_eq!(GRID.neighbour(0, 5, -1, 0), (GRID.width - 1, 5));
        assert_eq!(GRID.neighbour(GRID.width - 1, 5, 1, 1), (0, 6));
    }

    #[test]
    fn neighbours_across_a_pole_are_really_next_to_each_other() {
        let (x, y) = (10, 0);
        let (nx, ny) = GRID.neighbour(x, y, 0, -1);
        // Two cells of the top row that face each other across the pole are
        // at most one cell apart along the meridian.
        assert!(GRID.distance((x, y), (nx, ny)) <= GRID.cell_height() * 1.01);
    }

    #[test]
    fn every_cell_is_found_under_its_own_centre() {
        for &(x, y) in &[
            (0, 0),
            (1023, 0),
            (512, 256),
            (0, 511),
            (1023, 511),
            (300, 17),
        ] {
            assert_eq!(GRID.cell_of(GRID.point(x, y)), (x, y));
        }
    }

    #[test]
    fn world_positions_are_continuous_across_the_seam_and_over_the_poles() {
        let sphere = Sphere::MARGIN;
        assert!(same_point(
            sphere.point_at(1024.0, 200.0),
            sphere.point_at(0.0, 200.0)
        ));
        assert!(same_point(
            sphere.point_at(-3.0, 200.0),
            sphere.point_at(1021.0, 200.0)
        ));
        assert!(same_point(
            sphere.point_at(0.0, 0.0),
            sphere.point_at(700.0, 0.0)
        ));
        assert!(same_point(
            sphere.point_at(100.0, 512.0),
            sphere.point_at(900.0, 512.0)
        ));
        let (wx, wy) = sphere.world_at(sphere.point_at(300.5, 128.25));
        assert!(close(wx, 300.5, 1e-9) && close(wy, 128.25, 1e-9));
    }

    #[test]
    fn the_poles_are_the_ends_of_the_map() {
        let sphere = Sphere::MARGIN;
        assert!(same_point(sphere.point_at(0.0, 0.0), [0.0, 0.0, 1.0]));
        assert!(same_point(sphere.point_at(0.0, 512.0), [0.0, 0.0, -1.0]));
        assert!(close(
            sphere.distance_between((0.0, 0.0), (0.0, 512.0)),
            WORLD_WIDTH / 2.0,
            1e-9
        ));
    }

    #[test]
    fn noise_points_a_world_unit_apart_are_a_frequency_apart() {
        let sphere = Sphere::MARGIN;
        let frequency = 0.05;
        let a = sphere.noise_point_at(100.0, 256.0, frequency);
        let b = sphere.noise_point_at(101.0, 256.0, frequency);
        let chord = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        assert!(close(chord, frequency, frequency * 1e-3));
    }

    #[test]
    fn area_shares_and_reaches_follow_the_latitude() {
        let equator = GRID.height / 2;
        assert!(close(GRID.area_share(equator), 1.0, 1e-4));
        assert!(GRID.area_share(0) < 0.01);
        assert_eq!(GRID.reach_along_row(equator, 3), 3);
        assert!(GRID.reach_along_row(10, 3) > 3);
        assert_eq!(GRID.reach_along_row(0, 3), GRID.width as i32 / 2);
        let steps = GRID.row_step_distances();
        // East and north steps at the equator are a cell; diagonals root two.
        assert!(close(steps[equator][2], 1.0, 1e-3));
        assert!(close(steps[equator][0], 1.0, 1e-3));
        assert!(close(steps[equator][1], 2f64.sqrt(), 1e-2));
        // The step over the pole lands a row's height away.
        assert!(steps[0][0] <= GRID.cell_height() * 1.01);
        assert_eq!(GRID.cell(GRID.index((7, 3))), (7, 3));
    }

    #[test]
    fn columns_are_measured_the_short_way_round() {
        assert_eq!(GRID.dx(2, 1022), -4.0);
        assert_eq!(GRID.dx(1022, 2), 4.0);
        assert_eq!(GRID.dx(10, 40), 30.0);
    }
}
