//! Landscape: land grown from uplift and cut by rivers (spec 013).
//!
//! Land starts barely above the sea. Uplift raises it, rivers cut it down
//! (`erosion_sim`), and the two are run against each other until they
//! balance. Every ridge is then ground that rivers have not yet removed, and
//! every valley is where one ran: the shape of the land and its drainage
//! come out together.
//!
//! The parameters were chosen by eye in the site's erosion sandbox.

use crate::biome_map::{SEA_LEVEL, WORLD_WIDTH};
use crate::drainage::{rainfall, solve_drainage, Drainage};
use crate::erosion_sim::{crept, erosion_step, ErosionParams, Land};
use crate::rivers::{position_jitter, sea_bodies};

/// Land starts this far above sea level, with a little unevenness so water
/// has somewhere to start running.
const STARTING_HEIGHT: f64 = 0.002;
const STARTING_ROUGHNESS: f64 = 0.002;
/// Continentalness above sea level at which land counts as deep interior.
const INTERIOR_FULL_AT: f64 = 0.4;
/// Plate boundaries are spread into belts about this many world units wide,
/// by repeated blurring.
const FAULT_SPREAD_WU: f64 = 6.0;
const FAULT_SPREAD_PASSES: usize = 3;
/// The land is first grown on a grid of half the resolution, where a step is
/// four times cheaper, until uplift and erosion balance; then refined at
/// full resolution, which adds the smaller valleys.
const COARSE_STEPS: u32 = 400;
const FINE_STEPS: u32 = 40;
/// Routing variation used for the drainage of the finished land.
const FINAL_DRAINAGE: u32 = u32::MAX;
/// Grids narrower than this are too small to be worth halving.
const COARSE_MIN_WIDTH: usize = 128;
/// After the macro land is grown it is refined twice more, doubling the
/// resolution each time, to four cells per world unit: valleys then exist
/// down to a few hundred blocks across. Steps of erosion at each doubling.
const REFINING_STEPS: [u32; 2] = [20, 8];
/// The finest grid is left with gullies a single cell wide, which read as a
/// rash of bumps when shaded. A few rounds of slope creep alone, with no
/// cutting, close those and leave every valley wider than a cell or two.
const SETTLING_ROUNDS: usize = 4;
const SETTLING_CREEP: f64 = 0.5;

/// How much each source of uplift contributes, as shares of the uplift rate.
pub struct UpliftMix {
    /// Whole continents rising, most in the middle.
    pub interior: f64,
    /// Rising along mountain belts.
    pub ranges: f64,
    /// Rising along plate boundaries.
    pub faults: f64,
}

impl Default for UpliftMix {
    fn default() -> Self {
        Self {
            interior: 0.55,
            ranges: 0.75,
            faults: 0.70,
        }
    }
}

/// Where the crust is pushed up, each field 0 to 1.
///
/// The tectonic layer alone will not do: it marks plate boundaries as thin
/// lines, and used as uplift it raises narrow ridges out of flat land.
pub struct UpliftSources {
    interior: Vec<f64>,
    ranges: Vec<f64>,
    faults: Vec<f64>,
}

impl UpliftSources {
    pub fn new(
        continentalness: &[f64],
        peaks_valleys: &[f64],
        tectonic: &[f64],
        width: usize,
        height: usize,
    ) -> Self {
        // The tectonic layer is high where the crust is quiet.
        let stress: Vec<f64> = tectonic.iter().map(|quiet| (1.0 - quiet).powi(2)).collect();
        let reach = (FAULT_SPREAD_WU * width as f64 / WORLD_WIDTH / 2.0).round().max(1.0) as i32;
        Self {
            interior: continentalness
                .iter()
                .map(|cont| ((cont - SEA_LEVEL) / INTERIOR_FULL_AT).clamp(0.0, 1.0).sqrt())
                .collect(),
            // Mountain belts follow the ridges of the peaks-and-valleys layer.
            ranges: peaks_valleys.iter().map(|peaks| peaks.clamp(0.0, 1.0).powi(2)).collect(),
            faults: spread(&stress, width, height, reach),
        }
    }

    /// How fast each cell rises, as a share of the uplift rate.
    pub fn mixed(&self, mix: &UpliftMix) -> Vec<f64> {
        (0..self.interior.len())
            .map(|cell| {
                mix.interior * self.interior[cell]
                    + mix.ranges * self.ranges[cell]
                    + mix.faults * self.faults[cell]
            })
            .collect()
    }
}

/// A field blurred over `reach` cells each way and rescaled to peak at 1.
/// The map joins east to west.
fn spread(field: &[f64], width: usize, height: usize, reach: i32) -> Vec<f64> {
    let span = (2 * reach + 1) as f64;
    let mut current = field.to_vec();
    for _ in 0..FAULT_SPREAD_PASSES {
        // A box blur along rows, then along columns.
        let rows: Vec<f64> = (0..width * height)
            .map(|cell| {
                let (x, y) = ((cell % width) as i32, cell / width);
                (-reach..=reach)
                    .map(|dx| current[y * width + (x + dx).rem_euclid(width as i32) as usize])
                    .sum::<f64>()
                    / span
            })
            .collect();
        current = (0..width * height)
            .map(|cell| {
                let (x, y) = (cell % width, (cell / width) as i32);
                (-reach..=reach)
                    .map(|dy| rows[(y + dy).clamp(0, height as i32 - 1) as usize * width + x])
                    .sum::<f64>()
                    / span
            })
            .collect();
    }
    let peak = current.iter().copied().fold(1e-9, f64::max);
    current.iter().map(|value| value / peak).collect()
}

/// Flat land just above the sea: where a landscape starts from. Sea cells
/// keep their depth.
pub fn starting_ground(continentalness: &[f64], is_sea: &[bool], width: usize) -> Vec<f64> {
    (0..continentalness.len())
        .map(|cell| {
            if is_sea[cell] {
                return continentalness[cell].min(SEA_LEVEL);
            }
            let roughness = position_jitter((cell % width) as u32, (cell / width) as u32);
            SEA_LEVEL + STARTING_HEIGHT + STARTING_ROUGHNESS * roughness
        })
        .collect()
}

/// What a landscape is grown from, one value per cell.
pub struct LandscapeInputs<'a> {
    pub continentalness: &'a [f64],
    pub peaks_valleys: &'a [f64],
    pub tectonic: &'a [f64],
    pub rock_hardness: &'a [f64],
    pub light_level: &'a [f64],
    pub humidity: &'a [f64],
    pub width: usize,
    pub height: usize,
}

pub struct Landscape {
    /// Height of the ground. Sea cells keep their depth.
    pub heightmap: Vec<f64>,
    /// Drainage over the finished land: the rivers that cut it.
    pub drainage: Drainage,
    /// Depth of rock removed from each cell.
    pub sediment: Vec<f64>,
}

/// One grid's worth of inputs, owned, so a coarser copy can be made.
struct Grid {
    continentalness: Vec<f64>,
    peaks_valleys: Vec<f64>,
    tectonic: Vec<f64>,
    rock_hardness: Vec<f64>,
    light_level: Vec<f64>,
    humidity: Vec<f64>,
    width: usize,
    height: usize,
}

impl Grid {
    fn from_inputs(inputs: &LandscapeInputs) -> Self {
        Self {
            continentalness: inputs.continentalness.to_vec(),
            peaks_valleys: inputs.peaks_valleys.to_vec(),
            tectonic: inputs.tectonic.to_vec(),
            rock_hardness: inputs.rock_hardness.to_vec(),
            light_level: inputs.light_level.to_vec(),
            humidity: inputs.humidity.to_vec(),
            width: inputs.width,
            height: inputs.height,
        }
    }

    /// Every second cell each way.
    fn halved(&self) -> Self {
        let (width, height) = (self.width / 2, self.height / 2);
        let halve = |field: &[f64]| -> Vec<f64> {
            (0..width * height)
                .map(|cell| field[(cell / width) * 2 * self.width + (cell % width) * 2])
                .collect()
        };
        Self {
            continentalness: halve(&self.continentalness),
            peaks_valleys: halve(&self.peaks_valleys),
            tectonic: halve(&self.tectonic),
            rock_hardness: halve(&self.rock_hardness),
            light_level: halve(&self.light_level),
            humidity: halve(&self.humidity),
            width,
            height,
        }
    }

    /// Twice the resolution, each field filled in smoothly.
    fn doubled(&self) -> Self {
        let (width, height) = (self.width * 2, self.height * 2);
        let double = |field: &[f64]| doubled(field, self.width, self.height, width, height);
        Self {
            continentalness: double(&self.continentalness),
            peaks_valleys: double(&self.peaks_valleys),
            tectonic: double(&self.tectonic),
            rock_hardness: double(&self.rock_hardness),
            light_level: double(&self.light_level),
            humidity: double(&self.humidity),
            width,
            height,
        }
    }

    fn is_sea(&self) -> Vec<bool> {
        sea_bodies(&self.continentalness, self.width, self.height, SEA_LEVEL)
    }

    fn rainfall(&self) -> Vec<f64> {
        (0..self.width * self.height)
            .map(|cell| rainfall(self.light_level[cell], self.humidity[cell]))
            .collect()
    }

    /// Run `steps` of uplift and erosion on `ground`, draining to `is_sea`.
    fn erode(&self, ground: &mut Vec<f64>, sediment: &mut [f64], is_sea: &[bool], steps: u32) {
        let uplift_share = UpliftSources::new(
            &self.continentalness,
            &self.peaks_valleys,
            &self.tectonic,
            self.width,
            self.height,
        )
        .mixed(&UpliftMix::default());
        let rainfall = self.rainfall();
        let land = Land {
            is_base_level: is_sea,
            rock_hardness: &self.rock_hardness,
            uplift_share: &uplift_share,
            rainfall: &rainfall,
            width: self.width,
            height: self.height,
        };
        let params = ErosionParams::default();
        for step in 0..steps {
            erosion_step(ground, sediment, &land, &params, step);
        }
    }
}

/// `coarse` (half the resolution) sampled smoothly onto a grid of `width`.
fn doubled(coarse: &[f64], coarse_width: usize, coarse_height: usize, width: usize, height: usize) -> Vec<f64> {
    (0..width * height)
        .map(|cell| {
            // Coarse cell (x, y) sits on fine cell (2x, 2y).
            let (fx, fy) = ((cell % width) as f64 / 2.0, (cell / width) as f64 / 2.0);
            let (x0, y0) = (fx.floor() as usize, (fy.floor() as usize).min(coarse_height - 1));
            let (x1, y1) = ((x0 + 1) % coarse_width, (y0 + 1).min(coarse_height - 1));
            let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
            let at = |x: usize, y: usize| coarse[y * coarse_width + x];
            let north = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
            let south = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
            north + (south - north) * ty
        })
        .collect()
}

/// Grow the land from flat until uplift and erosion balance.
pub fn grow_landscape(inputs: &LandscapeInputs) -> Landscape {
    let fine = Grid::from_inputs(inputs);
    let (width, height) = (fine.width, fine.height);
    let is_sea = fine.is_sea();
    let mut ground = starting_ground(&fine.continentalness, &is_sea, width);
    let mut sediment = vec![0.0; width * height];

    let can_halve = width % 2 == 0 && height % 2 == 0 && width >= COARSE_MIN_WIDTH;
    if can_halve {
        let coarse = fine.halved();
        let coarse_sea = coarse.is_sea();
        let mut coarse_ground = starting_ground(&coarse.continentalness, &coarse_sea, coarse.width);
        let mut coarse_sediment = vec![0.0; coarse.width * coarse.height];
        coarse.erode(&mut coarse_ground, &mut coarse_sediment, &coarse_sea, COARSE_STEPS);

        // Carry the coarse land up. Where the two grids disagree about the
        // coast, the fine grid's sea stays sea and its land stays land.
        let lifted = doubled(&coarse_ground, coarse.width, coarse.height, width, height);
        let removed = doubled(&coarse_sediment, coarse.width, coarse.height, width, height);
        for cell in 0..width * height {
            if !is_sea[cell] {
                ground[cell] = ground[cell].max(lifted[cell]);
                sediment[cell] = removed[cell];
            }
        }
        fine.erode(&mut ground, &mut sediment, &is_sea, FINE_STEPS);
    } else {
        fine.erode(&mut ground, &mut sediment, &is_sea, COARSE_STEPS);
    }

    for cell in ground.iter_mut() {
        *cell = cell.clamp(-1.0, 1.0);
    }
    let drainage = solve_drainage(&ground, &is_sea, &fine.rainfall(), width, height, FINAL_DRAINAGE);
    // Any hollow left is filled level, so water runs across it.
    let heightmap = (0..width * height)
        .map(|cell| if is_sea[cell] { ground[cell] } else { drainage.filled[cell] })
        .collect();
    Landscape {
        heightmap,
        drainage,
        sediment,
    }
}

/// The land's height on a grid finer than the macro map, for everything that
/// looks closer than one cell per chunk: map tiles and the game's chunks.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FineHeights {
    pub cells_per_wu: usize,
    pub width: usize,
    pub height: usize,
    pub heights: Vec<f32>,
}

impl FineHeights {
    /// The four cells around a world position and how far between them it
    /// lies. The map joins east to west; north and south edges are clamped.
    fn around(&self, wx: f64, wy: f64) -> (i64, i64, f64, f64) {
        let fx = wx * self.cells_per_wu as f64;
        let fy = (wy * self.cells_per_wu as f64).clamp(0.0, (self.height - 1) as f64);
        (fx.floor() as i64, fy.floor() as i64, fx - fx.floor(), fy - fy.floor())
    }

    fn at(&self, x: i64, y: i64) -> f64 {
        let x = x.rem_euclid(self.width as i64) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        self.heights[y * self.width + x] as f64
    }

    /// Height at a world position, between cells in straight lines.
    pub fn sample(&self, wx: f64, wy: f64) -> f64 {
        let (x, y, tx, ty) = self.around(wx, wy);
        let north = self.at(x, y) + (self.at(x + 1, y) - self.at(x, y)) * tx;
        let south = self.at(x, y + 1) + (self.at(x + 1, y + 1) - self.at(x, y + 1)) * tx;
        north + (south - north) * ty
    }

    /// Height at a world position on a smooth curve (a cubic B-spline) over
    /// the cells: no creases along cell edges, so it shades cleanly as relief.
    pub fn sample_smooth(&self, wx: f64, wy: f64) -> f64 {
        let (x, y, tx, ty) = self.around(wx, wy);
        let weights = |t: f64| {
            let u = 1.0 - t;
            [
                u * u * u / 6.0,
                (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0,
                (3.0 * u * u * u - 6.0 * u * u + 4.0) / 6.0,
                t * t * t / 6.0,
            ]
        };
        let (weights_x, weights_y) = (weights(tx), weights(ty));
        let mut height = 0.0;
        for (row, weight_y) in weights_y.iter().enumerate() {
            for (column, weight_x) in weights_x.iter().enumerate() {
                height += self.at(x + column as i64 - 1, y + row as i64 - 1) * weight_x * weight_y;
            }
        }
        height
    }
}

/// The macro land refined to a finer grid, with the drainage that cut it.
pub struct FineLand {
    pub heights: FineHeights,
    pub drainage: Drainage,
    // The fields the river network needs, on the fine grid.
    pub ground: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub light_level: Vec<f64>,
    pub humidity: Vec<f64>,
}

/// `field` (on a grid `width` across) filled in smoothly on a grid `factor`
/// times finer, `factor` a power of two.
pub fn refined_field(field: &[f64], width: usize, height: usize, factor: usize) -> Vec<f64> {
    let (mut current, mut current_width, mut current_height) = (field.to_vec(), width, height);
    while current_width < width * factor {
        current = doubled(&current, current_width, current_height, current_width * 2, current_height * 2);
        current_width *= 2;
        current_height *= 2;
    }
    current
}

/// How many times finer than the macro grid `refine_landscape` makes the land.
pub const REFINED_CELLS_PER_MACRO_CELL: usize = 1 << REFINING_STEPS.len();

/// Carry the macro land (`macro_heights`, from `grow_landscape`) to a finer
/// grid, eroding a little more at each doubling so the finer grid has
/// valleys of its own and not just the macro ones blurred.
pub fn refine_landscape(inputs: &LandscapeInputs, macro_heights: &[f64]) -> FineLand {
    let mut grid = Grid::from_inputs(inputs);
    let mut ground = macro_heights.to_vec();
    let mut is_sea = Vec::new();
    for steps in REFINING_STEPS {
        let finer = grid.doubled();
        ground = doubled(&ground, grid.width, grid.height, finer.width, finer.height);
        // The coast is where the land crosses sea level; ponds are land.
        is_sea = sea_bodies(&ground, finer.width, finer.height, SEA_LEVEL);
        for cell in 0..ground.len() {
            if !is_sea[cell] {
                ground[cell] = ground[cell].max(SEA_LEVEL + STARTING_HEIGHT);
            }
        }
        let mut sediment = vec![0.0; ground.len()];
        finer.erode(&mut ground, &mut sediment, &is_sea, steps);
        grid = finer;
    }

    for _ in 0..SETTLING_ROUNDS {
        ground = crept(&ground, &is_sea, grid.width, grid.height, SETTLING_CREEP);
    }
    for cell in ground.iter_mut() {
        *cell = cell.clamp(-1.0, 1.0);
    }
    let drainage = solve_drainage(&ground, &is_sea, &grid.rainfall(), grid.width, grid.height, FINAL_DRAINAGE);
    // Any hollow left is filled level, so water runs across it.
    let ground: Vec<f64> = (0..ground.len())
        .map(|cell| if is_sea[cell] { ground[cell] } else { drainage.filled[cell] })
        .collect();
    FineLand {
        heights: FineHeights {
            cells_per_wu: (grid.width as f64 / WORLD_WIDTH).round() as usize,
            width: grid.width,
            height: grid.height,
            heights: ground.iter().map(|&height| height as f32).collect(),
        },
        drainage,
        ground,
        tectonic: grid.tectonic,
        light_level: grid.light_level,
        humidity: grid.humidity,
    }
}

/// How flat the ground is at each cell, from 0 (steep) to 1 (level), given
/// the size of a cell in world units. Biome classification reads this to
/// tell rugged country from plains.
pub fn flatness(heightmap: &[f64], width: usize, height: usize, cell_size_wu: f64) -> Vec<f64> {
    (0..width * height)
        .map(|cell| {
            let (x, y) = (cell % width, cell / width);
            let at = |x: usize, y: usize| heightmap[y * width + x];
            let (west, east) = (x.saturating_sub(1), (x + 1).min(width - 1));
            let (north, south) = (y.saturating_sub(1), (y + 1).min(height - 1));
            let rise_x = (at(east, y) - at(west, y)) / ((east - west).max(1) as f64 * cell_size_wu);
            let rise_y = (at(x, south) - at(x, north)) / ((south - north).max(1) as f64 * cell_size_wu);
            flatness_of_slope((rise_x * rise_x + rise_y * rise_y).sqrt())
        })
        .collect()
}

/// A rise of this much height per world unit, or more, is as steep as
/// ground gets for classification.
pub const STEEPEST_SLOPE: f64 = 0.06;

/// Flatness (0 steep to 1 level) of ground with the given slope, in height
/// per world unit.
pub fn flatness_of_slope(slope: f64) -> f64 {
    1.0 - (slope / STEEPEST_SLOPE).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 32 by 16 island: sea all round, land rising in the middle.
    fn island() -> (Vec<f64>, Vec<f64>) {
        let (width, height) = (32, 16);
        let continentalness: Vec<f64> = (0..width * height)
            .map(|cell| {
                let (x, y) = ((cell % width) as f64, (cell / width) as f64);
                let from_middle = ((x - 16.0) / 12.0).powi(2) + ((y - 8.0) / 6.0).powi(2);
                0.4 - from_middle * 0.5
            })
            .collect();
        (continentalness, vec![0.5; width * height])
    }

    fn grown() -> (Landscape, Vec<f64>) {
        let (continentalness, even) = island();
        let landscape = grow_landscape(&LandscapeInputs {
            continentalness: &continentalness,
            peaks_valleys: &even,
            tectonic: &even,
            rock_hardness: &even,
            light_level: &vec![0.4; 512],
            humidity: &even,
            width: 32,
            height: 16,
        });
        (landscape, continentalness)
    }

    #[test]
    fn land_rises_above_the_sea_and_the_sea_keeps_its_depth() {
        let (landscape, continentalness) = grown();
        let (centre, corner) = (8 * 32 + 16, 0);

        assert!(landscape.heightmap[centre] > SEA_LEVEL + 0.05);
        // Heights are kept within -1 to 1.
        assert_eq!(landscape.heightmap[corner], continentalness[corner].max(-1.0));
    }

    #[test]
    fn all_land_drains_to_the_sea() {
        let (landscape, continentalness) = grown();
        for cell in 0..512 {
            let is_land = continentalness[cell] > SEA_LEVEL;
            let drains = landscape.drainage.receivers[cell] != crate::drainage::NO_RECEIVER;
            assert_eq!(is_land, drains, "cell {cell}");
        }
    }

    #[test]
    fn doubling_keeps_coarse_values_and_fills_in_between() {
        // 2 by 1 doubled to 4 by 2.
        let fine = doubled(&[0.0, 1.0], 2, 1, 4, 2);
        assert_eq!(fine[0], 0.0);
        assert_eq!(fine[2], 1.0);
        assert_eq!(fine[1], 0.5);
        // East of the last coarse column it runs on round the seam.
        assert_eq!(fine[3], 0.5);
    }

    #[test]
    fn level_ground_is_flat_and_steep_ground_is_not() {
        assert_eq!(flatness_of_slope(0.0), 1.0);
        assert_eq!(flatness_of_slope(STEEPEST_SLOPE * 2.0), 0.0);
        let slope = [0.0, 0.1, 0.2, 0.0, 0.1, 0.2];
        assert!(flatness(&slope, 3, 2, 1.0)[1] < 0.01);
    }
}
