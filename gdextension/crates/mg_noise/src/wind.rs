//! Wind and the sand it moves (spec 013).
//!
//! On a tidally locked world the air at the surface runs from the cold side
//! to the hot side. It is fast over exposed ground and slack in hollows. In
//! the desert it picks sand up from worn rock and from the beds of dried
//! seas, carries it downwind, and drops it where it slackens. That is where
//! the sand seas are: not wherever a noise layer says, but in the basins the
//! wind cannot sweep clean.

use crate::biome_map::{SEA_LEVEL, WORLD_HEIGHT, WORLD_WIDTH};
use crate::drainage::desert;
use crate::erosion_sim::box_blurred;
use crate::rivers::D8_OFFSETS;

/// Ground is exposed or sheltered relative to the land about this many
/// world units around it.
const SHELTER_SCALE_WU: f64 = 8.0;
/// How much the wind speeds up for each unit of height a place stands above
/// its surroundings, and slows for each unit below.
const EXPOSURE_GAIN: f64 = 9.0;
const SLOWEST_WIND: f64 = 0.1;
const FASTEST_WIND: f64 = 1.6;
/// How far rising ground turns the wind aside, per unit of slope.
const DEFLECTION: f64 = 10.0;

/// Sand a desert cell gives up each step: a little from any rock, more the
/// more rock erosion has loosened, most from the bed of a dried sea.
const SAND_FROM_ROCK: f64 = 0.004;
const SAND_FROM_WORN_ROCK: f64 = 0.03;
const SAND_FROM_SEA_BED: f64 = 0.03;
/// Share of a cell's loose sand that a full-speed wind moves on each step.
const SAND_CARRIED: f64 = 0.8;
/// Steps of carrying; sand travels up to a cell a step.
const DRIFT_STEPS: usize = 120;
/// Outside the desert, damp ground and what grows on it hold sand fast and
/// bury it: this share of it is lost each step.
const SAND_LOST_OUTSIDE_DESERT: f64 = 0.25;
/// This much sand on a cell is a full load, the value 1 of the result. Set
/// on seed 42 so that about an eighth of the desert holds enough (0.45 of
/// this) to be a sand sea; half the desert holds almost none.
const FULL_SAND: f64 = 5.0;
/// The result of `drifted_sand` at which sand lies as deep as it gets.
const SAND_SEA: f64 = 1.0;

/// Dune crests are about this far apart, in world units (90 blocks).
const DUNE_SPACING_WU: f64 = 0.18;
/// The tallest dunes stand this high (8 blocks).
const DUNE_HEIGHT: f64 = 0.04;
/// Crests wander from a perfect arc: in long swings of up to
/// `DUNE_SWING` spacings, and in short kinks of up to `DUNE_KINK`.
const DUNE_SWING: f64 = 5.0;
const DUNE_KINK: f64 = 0.6;
/// Dunes cannot be drawn by samples further apart than this share of their
/// spacing; they fade out as samples approach it, rather than show as
/// false stripes.
const DUNE_COARSEST_SAMPLING: f64 = 0.3;
/// Sand thinner than this lies flat; dunes grow to full height by `SAND_SEA`.
const DUNES_FROM_SAND: f64 = 0.3;

/// The wind at the surface, one vector per cell: its direction, with its
/// speed as its length (1 is a steady wind over open ground).
pub struct Wind {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

impl Wind {
    pub fn speed(&self, cell: usize) -> f64 {
        (self.x[cell] * self.x[cell] + self.y[cell] * self.y[cell]).sqrt()
    }
}

/// A field's slope at a cell, per cell. The map joins east to west.
fn slope(field: &[f64], cell: usize, width: usize, height: usize) -> (f64, f64) {
    let (x, y) = (cell % width, cell / width);
    let at = |x: usize, y: usize| field[y * width + x];
    let (north, south) = (y.saturating_sub(1), (y + 1).min(height - 1));
    (
        (at((x + 1) % width, y) - at((x + width - 1) % width, y)) / 2.0,
        (at(x, south) - at(x, north)) / (south - north).max(1) as f64,
    )
}

/// The surface wind over `heightmap`: from dark towards light, turned aside
/// by rising ground, fast where the ground stands proud and slack where it
/// lies low.
pub fn surface_wind(light_level: &[f64], heightmap: &[f64], width: usize, height: usize) -> Wind {
    let cells_per_wu = width as f64 / WORLD_WIDTH;
    let ground: Vec<f64> = heightmap.iter().map(|&h| h.max(SEA_LEVEL)).collect();
    let reach = (SHELTER_SCALE_WU * cells_per_wu / 2.0).round().max(1.0) as i32;
    let surroundings = box_blurred(&ground, width, height, reach);

    let (mut wind_x, mut wind_y) = (Vec::with_capacity(ground.len()), Vec::with_capacity(ground.len()));
    for cell in 0..ground.len() {
        let (to_light_x, to_light_y) = slope(light_level, cell, width, height);
        let to_light = (to_light_x * to_light_x + to_light_y * to_light_y).sqrt().max(1e-12);
        // Slopes are per cell; the turning is by slope per world unit.
        let (rise_x, rise_y) = slope(&surroundings, cell, width, height);
        let x = to_light_x / to_light - rise_x * cells_per_wu * DEFLECTION;
        let y = to_light_y / to_light - rise_y * cells_per_wu * DEFLECTION;
        let length = (x * x + y * y).sqrt().max(1e-12);
        let speed = (1.0 + (ground[cell] - surroundings[cell]) * EXPOSURE_GAIN)
            .clamp(SLOWEST_WIND, FASTEST_WIND);
        wind_x.push(x / length * speed);
        wind_y.push(y / length * speed);
    }
    Wind { x: wind_x, y: wind_y }
}

/// Where the sand ends up, from 0 (bare) to 1 (a sand sea), one value per
/// cell. `sediment` is the depth of rock erosion has removed from each cell
/// and `is_water` marks open water and ice, which hold no sand.
pub fn drifted_sand(
    wind: &Wind,
    heightmap: &[f64],
    sediment: &[f64],
    light_level: &[f64],
    is_water: &[bool],
    width: usize,
    height: usize,
) -> Vec<f64> {
    let total = width * height;
    let most_worn = sediment.iter().copied().fold(1e-9, f64::max);
    let dryness: Vec<f64> = light_level.iter().map(|&light| desert(light)).collect();
    let given_up: Vec<f64> = (0..total)
        .map(|cell| {
            if is_water[cell] {
                0.0
            } else if heightmap[cell] < SEA_LEVEL {
                dryness[cell] * SAND_FROM_SEA_BED
            } else {
                dryness[cell] * (SAND_FROM_ROCK + SAND_FROM_WORN_ROCK * sediment[cell] / most_worn)
            }
        })
        .collect();
    // Sand can only move to one of eight neighbours. Sent always to the
    // nearest in direction, it would travel in dead-straight streaks along
    // those eight; so it is shared between the two neighbours either side of
    // the wind's true direction, more to the nearer.
    let neighbour = |cell: usize, direction: usize| {
        let (dx, dy) = D8_OFFSETS[direction % 8];
        let to_y = ((cell / width) as i32 + dy).clamp(0, height as i32 - 1) as usize;
        to_y * width + ((cell % width) as i32 + dx).rem_euclid(width as i32) as usize
    };
    let downwind: Vec<[(usize, f64); 2]> = (0..total)
        .map(|cell| {
            // `D8_OFFSETS` runs clockwise from north, an eighth of a turn apart.
            let turn = wind.x[cell].atan2(-wind.y[cell]).rem_euclid(std::f64::consts::TAU)
                / (std::f64::consts::TAU / 8.0);
            let (before, share) = (turn.floor() as usize, turn - turn.floor());
            [(neighbour(cell, before), 1.0 - share), (neighbour(cell, before + 1), share)]
        })
        .collect();

    let mut sand = vec![0.0f64; total];
    for _ in 0..DRIFT_STEPS {
        let mut next = vec![0.0f64; total];
        for cell in 0..total {
            let loose = sand[cell] + given_up[cell];
            let carried = loose * SAND_CARRIED * wind.speed(cell).min(1.0);
            next[cell] += loose - carried;
            for (to, share) in downwind[cell] {
                // Sand blown onto water sinks; it is gone.
                if !is_water[to] {
                    next[to] += carried * share;
                }
            }
        }
        for cell in 0..total {
            next[cell] *= 1.0 - SAND_LOST_OUTSIDE_DESERT * (1.0 - dryness[cell]);
        }
        sand = next;
    }
    // Soften the grid of eight directions out of the result.
    let settled = box_blurred(&sand, width, height, 1);
    (0..total)
        .map(|cell| if is_water[cell] { 0.0 } else { (settled[cell] / FULL_SAND).min(1.0) })
        .collect()
}

/// How high dunes stand at a world position where `sand` (0 to 1) lies.
/// Crests run across the wind, which blows towards the sub-stellar point,
/// so they are arcs around it, swinging and breaking as real crests do. A
/// function of position alone, so neighbouring chunks agree.
///
/// `sample_spacing` is how far apart, in world units, the caller's samples
/// are: dunes too fine for it to draw are left out.
pub fn dune_height(sand: f64, wx: f64, wy: f64, sample_spacing: f64) -> f64 {
    use noise::NoiseFn;
    let grown = ((sand - DUNES_FROM_SAND) / (SAND_SEA - DUNES_FROM_SAND)).clamp(0.0, 1.0);
    let drawable =
        (1.0 - sample_spacing / (DUNE_SPACING_WU * DUNE_COARSEST_SAMPLING)).clamp(0.0, 1.0);
    if grown <= 0.0 || drawable <= 0.0 {
        return 0.0;
    }
    static WANDER: std::sync::OnceLock<noise::OpenSimplex> = std::sync::OnceLock::new();
    let noise = WANDER.get_or_init(|| noise::OpenSimplex::new(0xD0_0E5u32));
    let wander = |frequency: f64, shift: f64| {
        let [cx, cz, cy] = crate::wrap::cylindrical_noise_coords(wx, wy, frequency, 1.0, WORLD_WIDTH);
        noise.get([cx + shift, cz, cy + shift])
    };

    // Distance from the sub-stellar point (bottom centre of the map), the
    // short way round.
    let east = (wx - WORLD_WIDTH / 2.0).rem_euclid(WORLD_WIDTH);
    let across = east.min(WORLD_WIDTH - east);
    let from_sun = (across * across + (WORLD_HEIGHT - wy) * (WORLD_HEIGHT - wy)).sqrt();
    let along_wind =
        from_sun / DUNE_SPACING_WU + wander(0.35, 0.0) * DUNE_SWING + wander(3.0, 700.0) * DUNE_KINK;
    // A dune is long and gentle on its windward side, short on its lee, and
    // rounded at crest and foot.
    let place = along_wind.rem_euclid(1.0);
    let rise = if place < 0.7 { place / 0.7 } else { (1.0 - place) / 0.3 };
    let profile = rise * rise * (3.0 - 2.0 * rise);
    // Dunes come and go along a crest, and stand higher in some fields.
    let patch = (0.55 + 0.6 * wander(1.6, 300.0)).clamp(0.0, 1.0);
    grown * drawable * DUNE_HEIGHT * profile * patch
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: usize = 32;
    const HIGH: usize = 16;

    /// Light rising from north (0) to south (1): all desert in the south.
    fn light() -> Vec<f64> {
        (0..WIDE * HIGH).map(|cell| (cell / WIDE) as f64 / (HIGH - 1) as f64).collect()
    }

    #[test]
    fn the_wind_blows_from_the_dark_side_to_the_light() {
        let wind = surface_wind(&light(), &vec![0.2; WIDE * HIGH], WIDE, HIGH);
        let middle = 8 * WIDE + 16;

        assert!(wind.y[middle] > 0.9);
        assert!(wind.x[middle].abs() < 1e-9);
    }

    #[test]
    fn the_wind_is_slack_in_a_hollow() {
        let mut ground = vec![0.2; WIDE * HIGH];
        let hollow = 12 * WIDE + 16;
        ground[hollow] = 0.05;
        let wind = surface_wind(&light(), &ground, WIDE, HIGH);

        assert!(wind.speed(hollow) < wind.speed(12 * WIDE + 4));
    }

    #[test]
    fn sand_gathers_in_desert_hollows_and_not_in_the_terminus() {
        let mut ground = vec![0.2; WIDE * HIGH];
        let hollow = 13 * WIDE + 16;
        for cell in [hollow - 1, hollow, hollow + 1] {
            ground[cell] = 0.02;
        }
        let wind = surface_wind(&light(), &ground, WIDE, HIGH);
        let sand = drifted_sand(
            &wind,
            &ground,
            &vec![0.1; WIDE * HIGH],
            &light(),
            &vec![false; WIDE * HIGH],
            WIDE,
            HIGH,
        );

        // More in the hollow than on open ground beside it.
        assert!(sand[hollow] > sand[13 * WIDE + 4]);
        // None in the north, where it is not desert.
        assert_eq!(sand[2 * WIDE + 16], 0.0);
    }

    #[test]
    fn dunes_stand_only_where_sand_lies_and_run_in_crests() {
        assert_eq!(dune_height(0.1, 500.0, 400.0, 0.001), 0.0);
        // Too fine for samples half a world unit apart to draw.
        assert_eq!(dune_height(1.0, 512.0, 400.0, 0.5), 0.0);
        let along: Vec<f64> =
            (0..200).map(|step| dune_height(1.0, 512.0, 400.0 + step as f64 * 0.01, 0.001)).collect();
        let (lowest, highest) = along.iter().fold((f64::MAX, f64::MIN), |(low, high), &h| (low.min(h), high.max(h)));
        assert!(highest > lowest + 0.005);
        assert!(highest <= DUNE_HEIGHT);
    }
}
