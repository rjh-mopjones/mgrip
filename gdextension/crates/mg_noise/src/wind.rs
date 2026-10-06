//! Wind and the sand it moves (spec 013).
//!
//! On a tidally locked world the air at the surface runs from the cold side
//! to the hot side. It is fast over exposed ground and slack in hollows. In
//! the desert it picks sand up from worn rock and from the beds of dried
//! seas, carries it downwind, and drops it where it slackens. That is where
//! the sand seas are: not wherever a noise layer says, but in the basins the
//! wind cannot sweep clean.
//!
//! The land is the cubed sphere (spec 015): the wind is a tangent vector at
//! each cell, and sand goes to whichever neighbours lie downwind.

use mg_core::{cube::Tangent, CubeGrid};

use crate::biome_map::SEA_LEVEL;
use crate::drainage::desert;
use crate::erosion_sim::reach_cells;

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

/// The wind at the surface, one tangent vector per cell: its direction,
/// with its speed as its length (1 is a steady wind over open ground).
pub struct Wind {
    pub vector: Vec<Tangent>,
}

impl Wind {
    pub fn speed(&self, cell: usize) -> f64 {
        let v = self.vector[cell];
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }
}

fn length(v: Tangent) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The surface wind over `heightmap`: from dark towards light, turned aside
/// by rising ground, fast where the ground stands proud and slack where it
/// lies low.
pub fn surface_wind(light_level: &[f64], heightmap: &[f64], grid: &CubeGrid) -> Wind {
    let ground: Vec<f64> = heightmap.iter().map(|&h| h.max(SEA_LEVEL)).collect();
    let surroundings = grid.blur(&ground, reach_cells(grid, SHELTER_SCALE_WU));

    let vector = (0..grid.cell_count())
        .map(|cell| {
            let to_light = grid.gradient(light_level, cell);
            let strength = length(to_light).max(1e-12);
            // Slopes are per world unit, as the turning is.
            let rise = grid.gradient(&surroundings, cell);
            let v = [
                to_light[0] / strength - rise[0] * DEFLECTION,
                to_light[1] / strength - rise[1] * DEFLECTION,
                to_light[2] / strength - rise[2] * DEFLECTION,
            ];
            let len = length(v).max(1e-12);
            let speed = (1.0 + (ground[cell] - surroundings[cell]) * EXPOSURE_GAIN)
                .clamp(SLOWEST_WIND, FASTEST_WIND);
            [v[0] / len * speed, v[1] / len * speed, v[2] / len * speed]
        })
        .collect();
    Wind { vector }
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
    grid: &CubeGrid,
) -> Vec<f64> {
    let total = grid.cell_count();
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
    let downwind: Vec<[(usize, f64); 2]> = (0..total)
        .map(|cell| grid.downstream(cell, wind.vector[cell]))
        .collect();
    // Sand is a depth over a cell. Blown onto a cell of a different size it
    // lies deeper or shallower by the ratio of the two areas.
    let into = |from: usize, to: usize| grid.area_share(from) / grid.area_share(to);

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
                    next[to] += carried * share * into(cell, to);
                }
            }
        }
        for cell in 0..total {
            next[cell] *= 1.0 - SAND_LOST_OUTSIDE_DESERT * (1.0 - dryness[cell]);
        }
        sand = next;
    }
    // Soften the grid of eight directions out of the result.
    let settled = grid.blur(&sand, 1);
    (0..total)
        .map(|cell| {
            if is_water[cell] {
                0.0
            } else {
                (settled[cell] / FULL_SAND).min(1.0)
            }
        })
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
        let [cx, cz, cy] = mg_core::Sphere::MARGIN.noise_point_at(wx, wy, frequency);
        noise.get([cx + shift, cz, cy + shift])
    };

    // Distance from the sub-stellar point.
    let sphere = mg_core::Sphere::MARGIN;
    let from_sun = sphere.distance(sphere.point_at(wx, wy), crate::strategy::sun());
    let along_wind = from_sun / DUNE_SPACING_WU
        + wander(0.35, 0.0) * DUNE_SWING
        + wander(3.0, 700.0) * DUNE_KINK;
    // A dune is long and gentle on its windward side, short on its lee, and
    // rounded at crest and foot.
    let place = along_wind.rem_euclid(1.0);
    let rise = if place < 0.7 {
        place / 0.7
    } else {
        (1.0 - place) / 0.3
    };
    let profile = rise * rise * (3.0 - 2.0 * rise);
    // Dunes come and go along a crest, and stand higher in some fields.
    let patch = (0.55 + 0.6 * wander(1.6, 300.0)).clamp(0.0, 1.0);
    grown * drawable * DUNE_HEIGHT * profile * patch
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: usize = 16;

    fn cube() -> CubeGrid {
        CubeGrid::margin(N)
    }

    /// Light rising from the north pole (0) to the south pole (1): all
    /// desert in the south.
    fn light(grid: &CubeGrid) -> Vec<f64> {
        (0..grid.cell_count())
            .map(|cell| (1.0 - grid.point(cell)[2]) / 2.0)
            .collect()
    }

    #[test]
    fn the_wind_blows_from_the_dark_side_to_the_light() {
        let grid = cube();
        let wind = surface_wind(&light(&grid), &vec![0.2; grid.cell_count()], &grid);
        let middle = grid.index(0, N / 2, N / 2);
        let (east, south) = grid.tangents(middle);
        let v = wind.vector[middle];
        let southward = v[0] * south[0] + v[1] * south[1] + v[2] * south[2];
        let eastward = v[0] * east[0] + v[1] * east[1] + v[2] * east[2];

        assert!(southward > 0.9 * wind.speed(middle), "{southward}");
        assert!(eastward.abs() < 0.05, "{eastward}");
    }

    #[test]
    fn the_wind_is_slack_in_a_hollow() {
        let grid = cube();
        let mut ground = vec![0.2; grid.cell_count()];
        let hollow = grid.index(0, N / 2, 10);
        ground[hollow] = 0.05;
        let wind = surface_wind(&light(&grid), &ground, &grid);

        assert!(wind.speed(hollow) < wind.speed(grid.index(0, 2, 10)));
    }

    #[test]
    fn sand_gathers_in_desert_hollows_and_not_in_the_terminus() {
        let grid = cube();
        let total = grid.cell_count();
        let mut ground = vec![0.2; total];
        // A hollow on the sunlit south face, across the wind.
        let hollow = grid.index(5, N / 2, N / 2);
        for cell in [
            grid.neighbour(hollow, -1, 0).unwrap(),
            hollow,
            grid.neighbour(hollow, 1, 0).unwrap(),
        ] {
            ground[cell] = 0.02;
        }
        let wind = surface_wind(&light(&grid), &ground, &grid);
        let sand = drifted_sand(
            &wind,
            &ground,
            &vec![0.1; total],
            &light(&grid),
            &vec![false; total],
            &grid,
        );

        let open_desert = grid.index(5, 2, 2);
        let terminus = grid.index(0, N / 2, N / 2);
        assert!(
            sand[hollow] > sand[open_desert],
            "{} vs {}",
            sand[hollow],
            sand[open_desert]
        );
        assert!(sand[terminus] < 0.01, "{}", sand[terminus]);
    }

    #[test]
    fn dunes_stand_only_on_deep_sand_and_only_when_drawable() {
        assert_eq!(dune_height(0.1, 500.0, 400.0, 0.01), 0.0);
        assert_eq!(dune_height(1.0, 500.0, 400.0, 1.0), 0.0);
        let tallest = (0..200)
            .map(|i| dune_height(1.0, 500.0 + i as f64 * 0.01, 400.0, 0.01))
            .fold(0.0, f64::max);
        assert!(tallest > 0.0 && tallest <= DUNE_HEIGHT);
    }
}
