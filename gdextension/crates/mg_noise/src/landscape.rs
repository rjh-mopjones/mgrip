//! Landscape: land grown from uplift and cut by rivers (spec 013).
//!
//! Land starts barely above the sea. Uplift raises it, rivers cut it down
//! (`erosion_sim`), and the two are run against each other until they
//! balance. Every ridge is then ground that rivers have not yet removed, and
//! every valley is where one ran: the shape of the land and its drainage
//! come out together.
//!
//! The land is the cubed sphere (spec 015). The parameters were chosen by
//! eye in the site's erosion sandbox.

use mg_core::{CubeGrid, Sphere};

use crate::biome_map::SEA_LEVEL;
use crate::drainage::{
    cell_jitter, desert, iciness, lake_evaporation, rainfall, solve_drainage, Drainage,
    LAKE_MIN_DEPTH,
};
use crate::erosion_sim::{crept, erosion_step, reach_cells, ErosionParams, Land};
use crate::rivers::sea_bodies;

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
/// Only the deeper troughs of the peaks-and-valleys layer (below minus this)
/// are places where the crust pulls apart.
const RIFT_FROM_VALLEY_DEPTH: f64 = 0.35;
/// The land is first grown on a cube of half the resolution, where a step is
/// four times cheaper, until uplift and erosion balance; then refined at
/// full resolution, which adds the smaller valleys.
const COARSE_STEPS: u32 = 400;
const FINE_STEPS: u32 = 40;
/// Routing variation used for the drainage of the finished land.
const FINAL_DRAINAGE: u32 = u32::MAX;
/// Faces narrower than this are too small to be worth halving.
const COARSE_MIN_FACE: usize = 64;
/// After the macro land is grown it is refined twice more, doubling the
/// resolution each time, to four cells per macro cell: valleys then exist
/// down to a few hundred blocks across. Steps of erosion at each doubling.
const REFINING_STEPS: [u32; 2] = [20, 8];
/// The finest cube is left with gullies a single cell wide, which read as a
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
    /// Sinking where the crust is pulled apart. Ground that sinks faster
    /// than rivers can cut its rim holds a lake.
    pub rifts: f64,
}

impl Default for UpliftMix {
    fn default() -> Self {
        Self {
            interior: 0.55,
            ranges: 0.75,
            faults: 0.70,
            rifts: 0.6,
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
    rifts: Vec<f64>,
}

impl UpliftSources {
    pub fn new(
        continentalness: &[f64],
        peaks_valleys: &[f64],
        tectonic: &[f64],
        grid: &CubeGrid,
    ) -> Self {
        // The tectonic layer is high where the crust is quiet.
        let stress: Vec<f64> = tectonic.iter().map(|quiet| (1.0 - quiet).powi(2)).collect();
        let faults = spread(&stress, grid, reach_cells(grid, FAULT_SPREAD_WU));
        // The crust pulls apart along boundaries where the peaks-and-valleys
        // layer runs deepest, as it is pushed up where that layer peaks.
        let rifts = (0..grid.cell_count())
            .map(|cell| {
                let trough = (-peaks_valleys[cell] - RIFT_FROM_VALLEY_DEPTH).max(0.0)
                    / (1.0 - RIFT_FROM_VALLEY_DEPTH);
                trough * faults[cell]
            })
            .collect();
        // Belts and faults are pushed up along crests, not evenly: a range
        // has a spine, and where nothing wears it down the spine shows.
        let crested = |field: Vec<f64>, shift: f64| -> Vec<f64> {
            (0..grid.cell_count())
                .map(|cell| {
                    let (wx, wy) = grid.world_position(cell);
                    field[cell] * (CREST_FLOOR + CREST_GAIN * crest(wx, wy, shift).powi(3))
                })
                .collect()
        };
        Self {
            interior: continentalness
                .iter()
                .map(|cont| {
                    ((cont - SEA_LEVEL) / INTERIOR_FULL_AT)
                        .clamp(0.0, 1.0)
                        .sqrt()
                })
                .collect(),
            // Mountain belts follow the ridges of the peaks-and-valleys layer.
            ranges: crested(
                peaks_valleys
                    .iter()
                    .map(|peaks| peaks.clamp(0.0, 1.0).powi(2))
                    .collect(),
                0.0,
            ),
            faults: crested(faults, 500.0),
            rifts,
        }
    }

    /// How fast each cell rises, as a share of the uplift rate.
    pub fn mixed(&self, mix: &UpliftMix) -> Vec<f64> {
        (0..self.interior.len())
            .map(|cell| {
                mix.interior * self.interior[cell]
                    + mix.ranges * self.ranges[cell]
                    + mix.faults * self.faults[cell]
                    - mix.rifts * self.rifts[cell]
            })
            .collect()
    }
}

/// Uplift along a belt or fault runs from `CREST_FLOOR` of its strength
/// between crests to `CREST_FLOOR + CREST_GAIN` on them.
const CREST_FLOOR: f64 = 0.35;
const CREST_GAIN: f64 = 1.4;
/// Crests are about this many world units apart at their broadest.
const CREST_SPACING_WU: f64 = 14.0;
const CREST_OCTAVES: usize = 3;

/// How near a world position is to a crest line, from 0 to 1: ridged noise
/// on the sphere. `shift` picks an unrelated pattern.
fn crest(wx: f64, wy: f64, shift: f64) -> f64 {
    use noise::NoiseFn;
    static CRESTS: std::sync::OnceLock<noise::OpenSimplex> = std::sync::OnceLock::new();
    let noise = CRESTS.get_or_init(|| noise::OpenSimplex::new(0xC4E5_7u32));
    let (mut sum, mut weight, mut total) = (0.0, 1.0, 0.0);
    for octave in 0..CREST_OCTAVES {
        let frequency = 2f64.powi(octave as i32) / CREST_SPACING_WU;
        let [cx, cz, cy] = Sphere::MARGIN.noise_point_at(wx, wy, frequency);
        sum += weight * (1.0 - noise.get([cx + shift, cz + shift, cy]).abs());
        total += weight;
        weight *= 0.5;
    }
    sum / total
}

/// A field blurred over `reach` cells each way, several times, and rescaled
/// to peak at 1.
fn spread(field: &[f64], grid: &CubeGrid, reach: usize) -> Vec<f64> {
    let mut current = field.to_vec();
    for _ in 0..FAULT_SPREAD_PASSES {
        current = grid.blur(&current, reach);
    }
    let peak = current.iter().copied().fold(1e-9, f64::max);
    current.iter().map(|value| value / peak).collect()
}

/// Flat land just above the sea: where a landscape starts from. Sea cells
/// keep their depth.
pub fn starting_ground(continentalness: &[f64], is_sea: &[bool]) -> Vec<f64> {
    (0..continentalness.len())
        .map(|cell| {
            if is_sea[cell] {
                return continentalness[cell].min(SEA_LEVEL);
            }
            SEA_LEVEL + STARTING_HEIGHT + STARTING_ROUGHNESS * cell_jitter(cell)
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
    pub grid: &'a CubeGrid,
    /// Land the landscape is grown from rather than from flat (spec 017).
    pub seed: Option<SeedGround<'a>>,
}

/// Given ground, one height per cell, and how far it holds: 1 where the
/// land is the seed's, 0 where it is the landscape's own, between where the
/// two blend. Where it holds fully the coarse pass pins the ground at it;
/// the fine pass starts from it and erodes on.
#[derive(Clone, Copy)]
pub struct SeedGround<'a> {
    pub heights: &'a [f64],
    pub hold: &'a [f64],
}

pub struct Landscape {
    /// Height of the ground, lake beds included. Sea cells keep their depth.
    pub heightmap: Vec<f64>,
    /// Level of standing water over each cell (see `water_levels`).
    pub water_level: Vec<f32>,
    /// Drainage over the finished land: the rivers that cut it.
    pub drainage: Drainage,
    /// Depth of rock removed from each cell.
    pub sediment: Vec<f64>,
}

/// One resolution's worth of inputs, owned, so a coarser or finer copy can
/// be made.
struct Level {
    continentalness: Vec<f64>,
    peaks_valleys: Vec<f64>,
    tectonic: Vec<f64>,
    rock_hardness: Vec<f64>,
    light_level: Vec<f64>,
    humidity: Vec<f64>,
    grid: CubeGrid,
    seed_heights: Option<Vec<f64>>,
    hold: Option<Vec<f64>>,
}

impl Level {
    fn from_inputs(inputs: &LandscapeInputs) -> Self {
        Self {
            continentalness: inputs.continentalness.to_vec(),
            peaks_valleys: inputs.peaks_valleys.to_vec(),
            tectonic: inputs.tectonic.to_vec(),
            rock_hardness: inputs.rock_hardness.to_vec(),
            light_level: inputs.light_level.to_vec(),
            humidity: inputs.humidity.to_vec(),
            grid: inputs.grid.clone(),
            seed_heights: inputs.seed.map(|seed| seed.heights.to_vec()),
            hold: inputs.seed.map(|seed| seed.hold.to_vec()),
        }
    }

    /// `ground` with every cell the seed holds fully put at the seed's height.
    fn pin(&self, ground: &mut [f64]) {
        let (Some(heights), Some(hold)) = (&self.seed_heights, &self.hold) else {
            return;
        };
        for cell in 0..ground.len() {
            if hold[cell] >= 1.0 {
                ground[cell] = heights[cell];
            }
        }
    }

    /// `ground` drawn towards the seed by how far the seed holds, on land.
    fn blend_to_seed(&self, ground: &mut [f64], is_sea: &[bool]) {
        let (Some(heights), Some(hold)) = (&self.seed_heights, &self.hold) else {
            return;
        };
        for cell in 0..ground.len() {
            if !is_sea[cell] {
                ground[cell] += (heights[cell] - ground[cell]) * hold[cell];
            }
        }
    }

    /// Half the resolution: each cell the mean of its four.
    fn halved(&self) -> Self {
        let coarse = self.grid.halved();
        let halve = |field: &[f64]| self.grid.halve_field(field, &coarse);
        Self {
            continentalness: halve(&self.continentalness),
            peaks_valleys: halve(&self.peaks_valleys),
            tectonic: halve(&self.tectonic),
            rock_hardness: halve(&self.rock_hardness),
            light_level: halve(&self.light_level),
            humidity: halve(&self.humidity),
            seed_heights: self.seed_heights.as_deref().map(halve),
            // A coarse cell holds only if all four of its cells do.
            hold: self.hold.as_deref().map(|hold| {
                halve(hold)
                    .into_iter()
                    .map(|mean| if mean >= 1.0 { 1.0 } else { mean.min(0.999) })
                    .collect()
            }),
            grid: coarse,
        }
    }

    /// Twice the resolution, each field filled in smoothly.
    fn doubled(&self) -> Self {
        let fine = self.grid.doubled();
        let double = |field: &[f64]| self.grid.double_field(field, &fine);
        Self {
            continentalness: double(&self.continentalness),
            peaks_valleys: double(&self.peaks_valleys),
            tectonic: double(&self.tectonic),
            rock_hardness: double(&self.rock_hardness),
            light_level: double(&self.light_level),
            humidity: double(&self.humidity),
            grid: fine,
            // The seed is a macro matter; refinement grows freely.
            seed_heights: None,
            hold: None,
        }
    }

    fn is_sea(&self) -> Vec<bool> {
        sea_bodies(&self.continentalness, &self.grid, SEA_LEVEL)
    }

    fn rainfall(&self) -> Vec<f64> {
        (0..self.grid.cell_count())
            .map(|cell| rainfall(self.light_level[cell], self.humidity[cell]))
            .collect()
    }

    fn lake_evaporation(&self) -> Vec<f64> {
        self.light_level
            .iter()
            .map(|&light| lake_evaporation(light))
            .collect()
    }

    /// Drainage of the finished land.
    fn final_drainage(&self, ground: &[f64], is_sea: &[bool]) -> Drainage {
        solve_drainage(
            ground,
            is_sea,
            &self.rainfall(),
            &self.lake_evaporation(),
            &self.grid,
            FINAL_DRAINAGE,
        )
    }

    /// Run `steps` of uplift and erosion on `ground`, draining to `is_sea`.
    /// With `pinned`, the cells the seed holds are put back at its heights
    /// after every step.
    fn erode(
        &self,
        ground: &mut Vec<f64>,
        sediment: &mut [f64],
        is_sea: &[bool],
        steps: u32,
        pinned: bool,
    ) {
        let uplift_share = UpliftSources::new(
            &self.continentalness,
            &self.peaks_valleys,
            &self.tectonic,
            &self.grid,
        )
        .mixed(&UpliftMix::default());
        let rainfall = self.rainfall();
        let ice: Vec<f64> = self
            .light_level
            .iter()
            .map(|&light| iciness(light))
            .collect();
        let lake_evaporation = self.lake_evaporation();
        let desert: Vec<f64> = self
            .light_level
            .iter()
            .map(|&light| desert(light))
            .collect();
        let land = Land {
            is_base_level: is_sea,
            rock_hardness: &self.rock_hardness,
            uplift_share: &uplift_share,
            rainfall: &rainfall,
            ice: &ice,
            lake_evaporation: &lake_evaporation,
            desert: &desert,
            grid: &self.grid,
        };
        let params = ErosionParams::default();
        for step in 0..steps {
            erosion_step(ground, sediment, &land, &params, step);
            if pinned {
                self.pin(ground);
            }
        }
    }
}

/// Grow the land from flat until uplift and erosion balance.
pub fn grow_landscape(inputs: &LandscapeInputs) -> Landscape {
    let fine = Level::from_inputs(inputs);
    let grid = &fine.grid;
    let is_sea = fine.is_sea();
    let mut ground = starting_ground(&fine.continentalness, &is_sea);
    let mut sediment = vec![0.0; grid.cell_count()];

    let can_halve = grid.n % 2 == 0 && grid.n >= COARSE_MIN_FACE;
    if can_halve {
        // The coarse pass grows the land from flat; with a seed, the land
        // the seed holds is pinned at the seed's heights throughout, so the
        // rest grows against it and drains through it.
        let coarse = fine.halved();
        let coarse_sea = coarse.is_sea();
        let mut coarse_ground = starting_ground(&coarse.continentalness, &coarse_sea);
        coarse.pin(&mut coarse_ground);
        let mut coarse_sediment = vec![0.0; coarse.grid.cell_count()];
        coarse.erode(
            &mut coarse_ground,
            &mut coarse_sediment,
            &coarse_sea,
            COARSE_STEPS,
            true,
        );

        // Carry the coarse land up. Where the two resolutions disagree about
        // the coast, the fine one's sea stays sea and its land stays land.
        let lifted = coarse.grid.double_field(&coarse_ground, grid);
        let removed = coarse.grid.double_field(&coarse_sediment, grid);
        for cell in 0..grid.cell_count() {
            if !is_sea[cell] {
                ground[cell] = ground[cell].max(lifted[cell]);
                sediment[cell] = removed[cell];
            }
        }
        // The fine pass starts from the seed where it holds and erodes
        // freely: the knit, which changes a mature landscape little.
        fine.blend_to_seed(&mut ground, &is_sea);
        fine.erode(&mut ground, &mut sediment, &is_sea, FINE_STEPS, false);
    } else {
        fine.pin(&mut ground);
        fine.erode(&mut ground, &mut sediment, &is_sea, COARSE_STEPS, true);
        fine.blend_to_seed(&mut ground, &is_sea);
        fine.erode(&mut ground, &mut sediment, &is_sea, FINE_STEPS, false);
    }

    for cell in ground.iter_mut() {
        *cell = cell.clamp(-1.0, 1.0);
    }
    let drainage = fine.final_drainage(&ground, &is_sea);
    let water_level = water_levels(&ground, &drainage);
    Landscape {
        heightmap: ground,
        water_level,
        drainage,
        sediment,
    }
}

/// The level of standing water over each cell: the height a lake's surface
/// stands at, or negative infinity where there is no lake.
fn water_levels(ground: &[f64], drainage: &Drainage) -> Vec<f32> {
    (0..ground.len())
        .map(|cell| {
            if drainage.lake_depth(ground, cell) >= LAKE_MIN_DEPTH {
                drainage.filled[cell] as f32
            } else {
                f32::NEG_INFINITY
            }
        })
        .collect()
}

/// The land's height on a cube finer than the macro map's, for everything
/// that looks closer than one cell per chunk: map tiles and the game's
/// chunks.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FineHeights {
    #[serde(with = "cube_grid_by_size")]
    pub grid: CubeGrid,
    pub heights: Vec<f32>,
    /// Level of standing water over each cell: a lake's surface, or negative
    /// infinity where there is none. Ground below it is lake bed.
    pub water_level: Vec<f32>,
}

impl FineHeights {
    /// No land at all: what a map read back without its fine heights has.
    pub fn empty() -> Self {
        let grid = CubeGrid::margin(1);
        Self {
            heights: vec![0.0; grid.cell_count()],
            water_level: vec![f32::NEG_INFINITY; grid.cell_count()],
            grid,
        }
    }

    /// Cells a side of the fine cube.
    pub fn n(&self) -> usize {
        self.grid.n
    }

    /// The level of the lake at a world position, if it is under one. A
    /// lake's level is the same all over it, so its shore is where the
    /// ground climbs through that level.
    pub fn lake_level(&self, wx: f64, wy: f64) -> Option<f64> {
        let point = self.grid.sphere.point_at(wx, wy);
        let home = self.grid.cell_of(point);
        let level = std::iter::once(Some(home))
            .chain(self.grid.neighbours(home))
            .flatten()
            .map(|cell| self.water_level[cell])
            .fold(f32::NEG_INFINITY, f32::max) as f64;
        (self.sample(wx, wy) < level).then_some(level)
    }

    /// Height at a world position, between cells in straight lines.
    pub fn sample(&self, wx: f64, wy: f64) -> f64 {
        self.sample_point(self.grid.sphere.point_at(wx, wy))
    }

    /// Height at a point on the sphere, as `sample`.
    pub fn sample_point(&self, point: mg_core::sphere::Point) -> f64 {
        self.grid
            .sample_by(point, |cell| self.heights[cell] as f64)
    }

    /// Height at a world position on a smooth curve (a cubic B-spline) over
    /// the cells: no creases along cell edges, so it shades cleanly as relief.
    pub fn sample_smooth(&self, wx: f64, wy: f64) -> f64 {
        self.grid.sample_smooth_by(self.grid.sphere.point_at(wx, wy), |cell| {
            self.heights[cell] as f64
        })
    }
}

/// A `CubeGrid` on Margin is fixed by its size alone, so that is all that
/// is written out.
pub(crate) mod cube_grid_by_size {
    use mg_core::CubeGrid;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(grid: &CubeGrid, serializer: S) -> Result<S::Ok, S::Error> {
        (grid.n as u64).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<CubeGrid, D::Error> {
        let n = u64::deserialize(deserializer)?;
        Ok(CubeGrid::margin(n as usize))
    }
}

/// The macro land refined to a finer cube, with the drainage that cut it.
pub struct FineLand {
    pub heights: FineHeights,
    pub drainage: Drainage,
    // The fields the river network needs, on the fine cube.
    pub ground: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub light_level: Vec<f64>,
    pub humidity: Vec<f64>,
    pub temperature: Vec<f64>,
}

/// How many times finer than the macro cube `refine_landscape` makes the land.
pub const REFINED_CELLS_PER_MACRO_CELL: usize = 1 << REFINING_STEPS.len();

/// Carry the macro land (`macro_heights`, from `grow_landscape`) to a finer
/// cube, eroding a little more at each doubling so the finer cube has
/// valleys of its own and not just the macro ones blurred. `temperature` is
/// carried along for the rivers.
pub fn refine_landscape(
    inputs: &LandscapeInputs,
    macro_heights: &[f64],
    temperature: &[f64],
) -> FineLand {
    let mut level = Level::from_inputs(inputs);
    let mut ground = macro_heights.to_vec();
    let mut temperature = temperature.to_vec();
    let mut is_sea = Vec::new();
    for steps in REFINING_STEPS {
        let finer = level.doubled();
        ground = level.grid.double_field(&ground, &finer.grid);
        temperature = level.grid.double_field(&temperature, &finer.grid);
        // The coast is where the land crosses sea level; ponds are land.
        is_sea = sea_bodies(&ground, &finer.grid, SEA_LEVEL);
        for cell in 0..ground.len() {
            if !is_sea[cell] {
                ground[cell] = ground[cell].max(SEA_LEVEL + STARTING_HEIGHT);
            }
        }
        let mut sediment = vec![0.0; ground.len()];
        finer.erode(&mut ground, &mut sediment, &is_sea, steps, false);
        level = finer;
    }

    for _ in 0..SETTLING_ROUNDS {
        ground = crept(&ground, &is_sea, &level.grid, SETTLING_CREEP);
    }
    for cell in ground.iter_mut() {
        *cell = cell.clamp(-1.0, 1.0);
    }
    let drainage = level.final_drainage(&ground, &is_sea);
    FineLand {
        heights: FineHeights {
            grid: level.grid.clone(),
            heights: ground.iter().map(|&height| height as f32).collect(),
            water_level: water_levels(&ground, &drainage),
        },
        drainage,
        ground,
        tectonic: level.tectonic,
        light_level: level.light_level,
        humidity: level.humidity,
        temperature,
    }
}

/// How flat the ground is at each cell of the cube, from 0 (steep) to 1
/// (level). Biome classification reads this to tell rugged country from
/// plains.
pub fn flatness_on_cube(heightmap: &[f64], grid: &CubeGrid) -> Vec<f64> {
    (0..grid.cell_count())
        .map(|cell| {
            let [x, y, z] = grid.gradient(heightmap, cell);
            flatness_of_slope((x * x + y * y + z * z).sqrt())
        })
        .collect()
}

/// How flat the ground is at each cell of a flat raster (a chunk), from 0
/// (steep) to 1 (level), given the size of a cell in world units. The
/// raster's edges are its edges.
pub fn flatness(heightmap: &[f64], width: usize, height: usize, cell_size_wu: f64) -> Vec<f64> {
    (0..width * height)
        .map(|cell| {
            let (x, y) = (cell % width, cell / width);
            let at = |x: usize, y: usize| heightmap[y * width + x];
            let (west, east) = (x.saturating_sub(1), (x + 1).min(width - 1));
            let (north, south) = (y.saturating_sub(1), (y + 1).min(height - 1));
            let rise_x = (at(east, y) - at(west, y)) / ((east - west).max(1) as f64 * cell_size_wu);
            let rise_y =
                (at(x, south) - at(x, north)) / ((south - north).max(1) as f64 * cell_size_wu);
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

    /// A cube of 16 cells a side with one island: land rising towards a
    /// point on the equator, sea everywhere else.
    fn island(grid: &CubeGrid) -> (Vec<f64>, Vec<f64>) {
        let middle = grid.point(grid.index(0, 8, 8));
        let continentalness: Vec<f64> = (0..grid.cell_count())
            .map(|cell| 0.4 - (grid.sphere.angle(grid.point(cell), middle) / 0.5).powi(2) * 0.5)
            .collect();
        (continentalness, vec![0.5; grid.cell_count()])
    }

    fn grown() -> (CubeGrid, Landscape, Vec<f64>) {
        let grid = CubeGrid::margin(16);
        let (continentalness, even) = island(&grid);
        let landscape = grow_landscape(&LandscapeInputs {
            continentalness: &continentalness,
            peaks_valleys: &even,
            tectonic: &even,
            rock_hardness: &even,
            light_level: &vec![0.4; grid.cell_count()],
            humidity: &even,
            grid: &grid,
            seed: None,
        });
        (grid, landscape, continentalness)
    }

    #[test]
    fn land_rises_above_the_sea_and_the_sea_keeps_its_depth() {
        let (grid, landscape, continentalness) = grown();
        let centre = grid.index(0, 8, 8);
        let far_side = grid.index(2, 8, 8);

        assert!(landscape.heightmap[centre] > SEA_LEVEL + 0.05);
        // Heights are kept within -1 to 1.
        assert_eq!(
            landscape.heightmap[far_side],
            continentalness[far_side].max(-1.0)
        );
    }

    #[test]
    fn all_land_drains_to_the_sea() {
        let (grid, landscape, continentalness) = grown();
        for cell in 0..grid.cell_count() {
            let is_land = continentalness[cell] > SEA_LEVEL;
            let drains = landscape.drainage.receivers[cell] != crate::drainage::NO_RECEIVER;
            assert_eq!(is_land, drains, "cell {cell}");
        }
    }

    #[test]
    fn level_ground_is_flat_and_steep_ground_is_not() {
        assert_eq!(flatness_of_slope(0.0), 1.0);
        assert_eq!(flatness_of_slope(STEEPEST_SLOPE * 2.0), 0.0);
        let slope = [0.0, 0.1, 0.2, 0.0, 0.1, 0.2];
        assert!(flatness(&slope, 3, 2, 1.0)[1] < 0.01);
        let grid = CubeGrid::margin(8);
        let level = vec![0.3; grid.cell_count()];
        assert!(flatness_on_cube(&level, &grid)
            .iter()
            .all(|&flat| flat > 0.99));
        let steep: Vec<f64> = (0..grid.cell_count())
            .map(|cell| grid.point(cell)[2] * 50.0)
            .collect();
        assert!(flatness_on_cube(&steep, &grid)[grid.index(0, 4, 4)] < 0.01);
    }
}
