//! Erosion sandbox: land grown from uplift and cut by rivers, on Margin's
//! real coastline, uplift and light, one step at a time.
//!
//! This is spec 013's landscape step run on its own so its parameters can be
//! chosen by eye before it replaces the generator's terrain. It works on a
//! coarser grid than the macro map so a step is quick enough to watch.

use mg_noise::biome_map::SEA_LEVEL;
use mg_noise::drainage::{
    desert, iciness, lake_evaporation, rainfall_with, solve_drainage, Drainage, LAKE_MIN_DEPTH,
};
use mg_noise::landscape::{starting_ground, UpliftMix, UpliftSources};
use mg_noise::rivers::sea_bodies;
use mg_noise::{erosion_step, BiomeMap, ErosionParams, Land};

/// The sandbox grid has one cell for this many macro cells each way.
const SHRINK: usize = 2;
/// Blocks of height per unit of heightmap (`VoxelMeshBuilder.HEIGHT_SCALE`).
const BLOCKS_PER_HEIGHT: f64 = 200.0;

/// What `Sandbox::render` draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Terrain,
    Uplift,
    Runoff,
}

/// The parameters on the page's sliders.
pub struct Settings {
    pub erosion: ErosionParams,
    /// Share of run-off that survives on the frozen night side and on the
    /// evaporating day side.
    pub night_runoff: f64,
    pub day_runoff: f64,
    pub uplift_mix: UpliftMix,
}

pub struct Sandbox {
    pub width: usize,
    pub height: usize,
    is_sea: Vec<bool>,
    sea_floor: Vec<f64>,
    rock_hardness: Vec<f64>,
    uplift_sources: UpliftSources,
    light_level: Vec<f64>,
    humidity: Vec<f64>,
    ground: Vec<f64>,
    sediment: Vec<f64>,
    pub steps: u32,
    /// Mean change in land height over the last step: near zero at steady state.
    pub last_change: f64,
}

impl Sandbox {
    pub fn new(macro_map: &BiomeMap, seed: u32) -> Self {
        let (width, height) = (macro_map.width / SHRINK, macro_map.height / SHRINK);
        let shrunk = |field: &[f64]| -> Vec<f64> {
            (0..width * height)
                .map(|cell| {
                    let (x, y) = (cell % width, cell / width);
                    field[y * SHRINK * macro_map.width + x * SHRINK]
                })
                .collect()
        };
        let continentalness = shrunk(&macro_map.continentalness);
        let mut sandbox = Self {
            width,
            height,
            is_sea: sea_bodies(&continentalness, width, height, SEA_LEVEL),
            sea_floor: continentalness.clone(),
            rock_hardness: shrunk(&macro_map.rock_hardness),
            uplift_sources: UpliftSources::new(
                &continentalness,
                &shrunk(&macro_map.peaks_valleys),
                &shrunk(&macro_map.tectonic),
                width,
                height,
            ),
            // The macro pack does not carry light level; it follows from position.
            light_level: (0..width * height)
                .map(|cell| {
                    let (x, y) = ((cell % width * SHRINK) as f64, (cell / width * SHRINK) as f64);
                    mg_noise::biome_map::light_level_at(seed, x, y)
                })
                .collect(),
            humidity: shrunk(&macro_map.humidity),
            ground: Vec::new(),
            sediment: Vec::new(),
            steps: 0,
            last_change: 0.0,
        };
        sandbox.reset();
        sandbox
    }

    /// Back to flat land just above the sea.
    pub fn reset(&mut self) {
        let width = self.width;
        self.ground = starting_ground(&self.sea_floor, &self.is_sea, width);
        self.sediment = vec![0.0; width * self.height];
        self.steps = 0;
        self.last_change = 0.0;
    }

    fn runoff(&self, settings: &Settings) -> Vec<f64> {
        (0..self.width * self.height)
            .map(|cell| {
                rainfall_with(
                    self.light_level[cell],
                    self.humidity[cell],
                    settings.night_runoff,
                    settings.day_runoff,
                )
            })
            .collect()
    }

    pub fn step(&mut self, count: u32, settings: &Settings) {
        let rainfall = self.runoff(settings);
        let uplift_share = self.uplift_share(settings);
        let ice: Vec<f64> = self.light_level.iter().map(|&light| iciness(light)).collect();
        let lake_evaporation = self.lake_evaporation();
        let desert: Vec<f64> = self.light_level.iter().map(|&light| desert(light)).collect();
        let land = Land {
            is_base_level: &self.is_sea,
            rock_hardness: &self.rock_hardness,
            uplift_share: &uplift_share,
            rainfall: &rainfall,
            ice: &ice,
            lake_evaporation: &lake_evaporation,
            desert: &desert,
            width: self.width,
            height: self.height,
        };
        for _ in 0..count {
            let before = self.ground.clone();
            erosion_step(&mut self.ground, &mut self.sediment, &land, &settings.erosion, self.steps);
            let land_cells = self.is_sea.iter().filter(|sea| !**sea).count().max(1);
            self.last_change = before
                .iter()
                .zip(&self.ground)
                .map(|(old, new)| (new - old).abs())
                .sum::<f64>()
                / land_cells as f64;
            self.steps += 1;
        }
    }

    fn lake_evaporation(&self) -> Vec<f64> {
        self.light_level.iter().map(|&light| lake_evaporation(light)).collect()
    }

    fn uplift_share(&self, settings: &Settings) -> Vec<f64> {
        self.uplift_sources.mixed(&settings.uplift_mix)
    }

    /// What lies on a cell below sea level: liquid sea, sea ice, or the dry
    /// bed of a sea that has evaporated.
    fn sea_colour(&self, cell: usize) -> [f64; 3] {
        let depth = ((SEA_LEVEL - self.sea_floor[cell]) / 0.4).clamp(0.0, 1.0);
        let light = self.light_level[cell];
        if light < SEA_FREEZES_BELOW_LIGHT {
            blend(SEA_ICE, DEEP_SEA_ICE, depth)
        } else if light > SEA_DRIES_ABOVE_LIGHT {
            blend(DRY_BASIN, DEEP_DRY_BASIN, depth)
        } else {
            blend(SHALLOW_SEA, DEEP_SEA, depth)
        }
    }

    fn drainage(&self, settings: &Settings) -> Drainage {
        solve_drainage(
            &self.ground,
            &self.is_sea,
            &self.runoff(settings),
            &self.lake_evaporation(),
            self.width,
            self.height,
            self.steps,
        )
    }

    /// Height of the highest land, in blocks above sea level.
    pub fn peak_blocks(&self) -> f64 {
        self.peak() * BLOCKS_PER_HEIGHT
    }

    fn peak(&self) -> f64 {
        self.ground
            .iter()
            .zip(&self.is_sea)
            .filter(|(_, sea)| !**sea)
            .map(|(height, _)| height - SEA_LEVEL)
            .fold(0.0, f64::max)
    }

    /// RGBA pixels, one per cell, and the number of river and lake cells.
    pub fn render(&self, view: View, river_threshold: f64, settings: &Settings) -> (Vec<u8>, u32, u32) {
        let drainage = self.drainage(settings);
        let peak = self.peak().max(0.01);
        let uplift_share = self.uplift_share(settings);
        let highest_uplift = uplift_share.iter().copied().fold(0.01, f64::max);
        let (mut rivers, mut lakes) = (0, 0);
        let mut rgba = Vec::with_capacity(self.width * self.height * 4);
        for cell in 0..self.width * self.height {
            let colour = if self.is_sea[cell] {
                self.sea_colour(cell)
            } else {
                match view {
                    View::Uplift => blend(FLAT_FIELD, UPLIFT_FIELD, uplift_share[cell] / highest_uplift),
                    View::Runoff => blend(FLAT_FIELD, RUNOFF_FIELD, (drainage.flow[cell].ln_1p() / 8.0).min(1.0)),
                    View::Terrain => {
                        // Land cut below the sea is under it: a fjord, or
                        // a river's drowned mouth.
                        let is_lake = drainage.filled[cell] - self.ground[cell] > LAKE_MIN_DEPTH
                            || self.ground[cell] < SEA_LEVEL;
                        let is_river = drainage.flow[cell] > river_threshold;
                        if is_lake {
                            lakes += 1;
                            LAKE
                        } else {
                            if is_river {
                                rivers += 1;
                            }
                            let land = self.land_colour(cell, peak);
                            // Bigger rivers show more strongly.
                            let strength = (drainage.flow[cell] / river_threshold).ln().clamp(0.0, 2.0) / 2.0;
                            if is_river { blend(land, RIVER, 0.55 + 0.45 * strength) } else { land }
                        }
                    }
                }
            };
            rgba.extend(colour.map(|channel| channel.clamp(0.0, 255.0) as u8));
            rgba.push(255);
        }
        (rgba, rivers, lakes)
    }

    /// Land coloured by height, shaded by relief lit from the north-west, and
    /// tinted cool on the night side and warm on the day side.
    fn land_colour(&self, cell: usize, peak: f64) -> [f64; 3] {
        let (width, height) = (self.width, self.height);
        let (x, y) = (cell % width, cell / width);
        let at = |x: usize, y: usize| self.ground[y * width + x];
        let elevation = ((self.ground[cell] - SEA_LEVEL) / peak).clamp(0.0, 1.0);
        let upper = HEIGHT_COLOURS.iter().position(|(stop, _)| elevation <= *stop).unwrap_or(HEIGHT_COLOURS.len() - 1).max(1);
        let ((low_stop, low), (high_stop, high)) = (HEIGHT_COLOURS[upper - 1], HEIGHT_COLOURS[upper]);
        let colour = blend(low, high, (elevation - low_stop) / (high_stop - low_stop));

        let rise_east = at((x + 1) % width, y) - at((x + width - 1) % width, y);
        let rise_south = at(x, (y + 1).min(height - 1)) - at(x, y.saturating_sub(1));
        let shade = (1.0 + (rise_east + rise_south) / peak * RELIEF_GAIN).clamp(0.5, 1.4);

        let light = self.light_level[cell];
        let zone_tint = if light < 0.2 {
            blend([1.0, 1.0, 1.0], NIGHT_TINT, (0.2 - light) / 0.2)
        } else if light > 0.62 {
            blend([1.0, 1.0, 1.0], DAY_TINT, ((light - 0.62) / 0.3).min(1.0))
        } else {
            [1.0, 1.0, 1.0]
        };
        [0, 1, 2].map(|channel| colour[channel] * shade * zone_tint[channel])
    }
}

// Height colours avoid green: the world has no green palette.
const HEIGHT_COLOURS: [(f64, [f64; 3]); 6] = [
    (0.0, [112.0, 98.0, 116.0]),
    (0.2, [156.0, 128.0, 104.0]),
    (0.45, [202.0, 168.0, 112.0]),
    (0.7, [156.0, 108.0, 88.0]),
    (0.88, [192.0, 188.0, 190.0]),
    (1.0, [246.0, 244.0, 242.0]),
];
// The light levels at which the generator freezes and dries out the sea.
const SEA_FREEZES_BELOW_LIGHT: f64 = 0.18;
const SEA_DRIES_ABOVE_LIGHT: f64 = 0.62;
const SEA_ICE: [f64; 3] = [206.0, 218.0, 230.0];
const DEEP_SEA_ICE: [f64; 3] = [150.0, 172.0, 198.0];
const DRY_BASIN: [f64; 3] = [214.0, 196.0, 168.0];
const DEEP_DRY_BASIN: [f64; 3] = [170.0, 146.0, 124.0];
const SHALLOW_SEA: [f64; 3] = [70.0, 112.0, 150.0];
const DEEP_SEA: [f64; 3] = [30.0, 58.0, 96.0];
const LAKE: [f64; 3] = [86.0, 138.0, 182.0];
const RIVER: [f64; 3] = [42.0, 100.0, 160.0];
const FLAT_FIELD: [f64; 3] = [236.0, 232.0, 224.0];
const UPLIFT_FIELD: [f64; 3] = [176.0, 62.0, 40.0];
const RUNOFF_FIELD: [f64; 3] = [34.0, 84.0, 150.0];
const NIGHT_TINT: [f64; 3] = [0.78, 0.86, 1.08];
const DAY_TINT: [f64; 3] = [1.1, 0.98, 0.82];
const RELIEF_GAIN: f64 = 6.0;

fn blend(from: [f64; 3], to: [f64; 3], share: f64) -> [f64; 3] {
    let share = share.clamp(0.0, 1.0);
    [0, 1, 2].map(|channel| from[channel] + (to[channel] - from[channel]) * share)
}

