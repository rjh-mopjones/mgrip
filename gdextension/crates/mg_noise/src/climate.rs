//! Climate from the land (spec 013, stages 3 and 4).
//!
//! Moisture comes off the liquid sea, is carried by the surface wind from
//! the dark side towards the light, and falls as rain: a little everywhere,
//! most where the air is forced up a slope. Beyond a range the air is dry.
//! So where it rains is decided by the sea, the wind and the mountains, not
//! by a noise layer.

use crate::biome_map::{SEA_LEVEL, WORLD_WIDTH};
use crate::erosion_sim::box_blurred;
use crate::rivers::D8_OFFSETS;
use crate::wind::Wind;

/// Moisture a cell of liquid sea gives the air each step, at a warm sea.
const SEA_EVAPORATION: f64 = 1.0;
/// Sea colder than this gives up nothing; warmer than `WARM_SEA_C` gives
/// the full amount.
const COLD_SEA_C: f64 = -10.0;
const WARM_SEA_C: f64 = 20.0;
/// Share of the air's moisture that falls each step over level ground.
const BASE_RAIN: f64 = 0.003;
/// Share that falls per unit of height the air is forced up on its way to
/// the next cell: air lifted drops its water, however far it travelled.
const OROGRAPHIC_RAIN: f64 = 1.0;
/// Over the frozen side, falling moisture is snow and the air is wrung dry
/// faster: this much more falls per step than over the terminus, fully by
/// `FROZEN_AIR_C`.
const COLD_RAIN_GAIN: f64 = 2.0;
const FROZEN_AIR_C: f64 = -20.0;
/// Air warmer than this, per degree, holds its moisture against falling:
/// hot air rains little. Full effect by `HOT_AIR_C`.
const WARM_AIR_C: f64 = 35.0;
const HOT_AIR_C: f64 = 80.0;
const HOT_AIR_RAIN_SHARE: f64 = 0.25;
/// Over hot ground the surface air rises, and takes this share of its
/// moisture up with it each step, into the high return flow this model does
/// not follow. Full by `HOT_AIR_C`. Without it the wind, which converges on
/// the sub-stellar point from every side, piles all the moisture it still
/// carries onto the day pole and rains it out there (spec 014).
const UPDRAFT_SHARE: f64 = 0.03;
/// Steps of carrying; moisture travels up to a cell a step. Enough to cross
/// the widest continent and settle.
const CARRY_STEPS: usize = 700;
/// Slopes are read from the land smoothed over this many world units, so
/// that a range casts a shadow and a gully does not.
const RELIEF_SCALE_WU: f64 = 4.0;
/// Air holds at most this much moisture, in units of a warm sea cell's
/// evaporation; where winds converge and pile it up, the excess falls at
/// once. Without this a convergence never stops gathering.
const AIR_CAPACITY: f64 = 3.0;
/// Share of the air's moisture that spreads to all eight neighbours each
/// step rather than going downwind: sea air reaches a shore upwind of the
/// sea too, as it does on any coast, though not far.
const MIXING: f64 = 0.45;
/// Rain at or above this is humidity 1; humidity rises as the square root
/// of rain below it, so a little rain counts for a lot and the wet coast
/// does not saturate everything near it. Set on seed 42 so that land in
/// the terminus averages about 0.5.
const FULL_RAIN: f64 = 0.12;
/// Rain is smoothed over this many world units at the end: the wind blows
/// straight here, and real plumes spread as they go.
const RAIN_SMOOTHING_WU: f64 = 10.0;
/// Snow that reaches the dark side by the high return flow, which this
/// model does not follow: a steady humidity there, so the night side has
/// its bogs and tundra and the cold shore upwind of the sea is not bone
/// dry. As a share of `FULL_RAIN`, full below `RETURN_FLOW_FULL_LIGHT` and
/// gone by `RETURN_FLOW_ENDS_LIGHT`.
const RETURN_FLOW_SNOW: f64 = 0.08;
const RETURN_FLOW_FULL_LIGHT: f64 = 0.1;
const RETURN_FLOW_ENDS_LIGHT: f64 = 0.34;

/// What the climate pass needs of the land.
pub struct Land<'a> {
    pub heightmap: &'a [f64],
    pub temperature: &'a [f64],
    pub is_liquid_sea: &'a [bool],
    pub width: usize,
    pub height: usize,
}

/// Rain falling on each cell per step, in units of a warm sea cell's
/// evaporation, from carrying moisture downwind until it settles.
pub fn rainfall(wind: &Wind, land: &Land) -> Vec<f64> {
    let (width, height) = (land.width, land.height);
    let total = width * height;
    let cells_per_wu = width as f64 / WORLD_WIDTH;
    let relief = box_blurred(
        &land
            .heightmap
            .iter()
            .map(|&h| h.max(SEA_LEVEL))
            .collect::<Vec<_>>(),
        width,
        height,
        (RELIEF_SCALE_WU * cells_per_wu / 2.0).round().max(1.0) as i32,
    );

    // Where each cell's air goes: shared between the two neighbours either
    // side of the wind's true direction, as sand is, or it would travel in
    // straight spokes.
    let grid = mg_core::Sphere::MARGIN.grid(width, height);
    let neighbour = |cell: usize, direction: usize| {
        let (dx, dy) = D8_OFFSETS[direction % 8];
        let (x, y) = grid.cell(cell);
        grid.index(grid.neighbour(x, y, dx, dy))
    };
    // Moisture is an amount over a cell. Carried onto a cell of a different
    // size it is thicker or thinner by the ratio of the two areas, so a
    // plume does not pile up as the cells narrow towards a pole.
    let shares: Vec<f64> = (0..height).map(|y| grid.area_share(y)).collect();
    let into = |from: usize, to: usize| shares[from / width] / shares[to / width];
    let downwind: Vec<[(usize, f64); 2]> = (0..total)
        .map(|cell| {
            let turn = wind.x[cell]
                .atan2(-wind.y[cell])
                .rem_euclid(std::f64::consts::TAU)
                / (std::f64::consts::TAU / 8.0);
            let (before, share) = (turn.floor() as usize, turn - turn.floor());
            [
                (neighbour(cell, before), 1.0 - share),
                (neighbour(cell, before + 1), share),
            ]
        })
        .collect();

    let source: Vec<f64> = (0..total)
        .map(|cell| {
            if !land.is_liquid_sea[cell] {
                return 0.0;
            }
            let warmth =
                ((land.temperature[cell] - COLD_SEA_C) / (WARM_SEA_C - COLD_SEA_C)).clamp(0.0, 1.0);
            SEA_EVAPORATION * warmth
        })
        .collect();
    // Share of the air's moisture that falls on each cell as it passes.
    let falls: Vec<f64> = (0..total)
        .map(|cell| {
            // How much the ground rises on the way to where the air goes.
            let rise: f64 = downwind[cell]
                .iter()
                .map(|&(to, share)| (relief[to] - relief[cell]).max(0.0) * share)
                .sum();
            let temperature = land.temperature[cell];
            let cold = (temperature / FROZEN_AIR_C).clamp(0.0, 1.0);
            let cold_gain = 1.0 + (COLD_RAIN_GAIN - 1.0) * cold;
            let hot = ((temperature - WARM_AIR_C) / (HOT_AIR_C - WARM_AIR_C)).clamp(0.0, 1.0);
            let hot_share = 1.0 - (1.0 - HOT_AIR_RAIN_SHARE) * hot;
            ((BASE_RAIN + OROGRAPHIC_RAIN * rise) * cold_gain * hot_share).min(0.95)
        })
        .collect();
    // How hot the ground is, 0 to 1: over hot ground the surface air rises.
    let hot_ground: Vec<f64> = (0..total)
        .map(|cell| {
            ((land.temperature[cell] - WARM_AIR_C) / (HOT_AIR_C - WARM_AIR_C)).clamp(0.0, 1.0)
        })
        .collect();

    let mut moisture = vec![0.0f64; total];
    let mut rain = vec![0.0f64; total];
    for _ in 0..CARRY_STEPS {
        let mut next = vec![0.0f64; total];
        for cell in 0..total {
            let gathered = moisture[cell] + source[cell];
            let in_air = gathered.min(AIR_CAPACITY);
            // More than the air can hold falls at once, except over hot
            // ground, where the excess rises away with the updraft instead.
            let excess = (gathered - in_air) * (1.0 - hot_ground[cell]);
            let fallen = in_air * falls[cell] + excess;
            rain[cell] = fallen;
            let carried = (in_air - fallen) * (1.0 - UPDRAFT_SHARE * hot_ground[cell]);
            for (to, share) in downwind[cell] {
                next[to] += carried * (1.0 - MIXING) * share * into(cell, to);
            }
            for direction in 0..8 {
                let to = neighbour(cell, direction);
                next[to] += carried * MIXING / 8.0 * into(cell, to);
            }
        }
        moisture = next;
    }
    // Spread the plumes, and soften the grid of eight directions out of them.
    let reach = (RAIN_SMOOTHING_WU * cells_per_wu / 2.0).round().max(1.0) as i32;
    box_blurred(&rain, width, height, reach)
}

/// Humidity, 0 to 1, from rainfall at a place with the given light level,
/// which decides how much of the return flow's snow it gets.
pub fn humidity_from_rain(rain: f64, light_level: f64) -> f64 {
    let return_flow = ((RETURN_FLOW_ENDS_LIGHT - light_level)
        / (RETURN_FLOW_ENDS_LIGHT - RETURN_FLOW_FULL_LIGHT))
        .clamp(0.0, 1.0);
    let with_snow = rain.max(RETURN_FLOW_SNOW * FULL_RAIN * return_flow);
    (with_snow / FULL_RAIN).clamp(0.0, 1.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wind::surface_wind;

    const WIDE: usize = 32;
    const HIGH: usize = 24;

    /// Light rising from north to south; liquid sea in rows 6 to 8; a range
    /// across the land at row 15.
    fn world(range_height: f64) -> (Vec<f64>, Vec<f64>, Vec<bool>, Vec<f64>) {
        let light: Vec<f64> = (0..WIDE * HIGH)
            .map(|cell| (cell / WIDE) as f64 / (HIGH - 1) as f64)
            .collect();
        let mut heights = vec![0.1; WIDE * HIGH];
        let mut sea = vec![false; WIDE * HIGH];
        for cell in 0..WIDE * HIGH {
            let row = cell / WIDE;
            if (6..=8).contains(&row) {
                heights[cell] = -0.2;
                sea[cell] = true;
            }
            if row == 15 {
                heights[cell] = range_height;
            }
        }
        let temperature = vec![15.0; WIDE * HIGH];
        (light, heights, sea, temperature)
    }

    #[test]
    fn over_hot_ground_the_air_rises_and_little_reaches_the_pole() {
        let (light, heights, sea, mut temperature) = world(0.1);
        let cool = rainfall(
            &surface_wind(&light, &heights, WIDE, HIGH),
            &Land {
                heightmap: &heights,
                temperature: &temperature,
                is_liquid_sea: &sea,
                width: WIDE,
                height: HIGH,
            },
        );
        // The land south of the sea is hot.
        for cell in 9 * WIDE..WIDE * HIGH {
            temperature[cell] = HOT_AIR_C;
        }
        let hot = rainfall(
            &surface_wind(&light, &heights, WIDE, HIGH),
            &Land {
                heightmap: &heights,
                temperature: &temperature,
                is_liquid_sea: &sea,
                width: WIDE,
                height: HIGH,
            },
        );
        let pole = (HIGH - 1) * WIDE + 16;
        assert!(
            hot[pole] < cool[pole] * 0.5,
            "{} vs {}",
            hot[pole],
            cool[pole]
        );
    }

    fn rain_over(range_height: f64) -> Vec<f64> {
        let (light, heights, sea, temperature) = world(range_height);
        let wind = surface_wind(&light, &heights, WIDE, HIGH);
        rainfall(
            &wind,
            &Land {
                heightmap: &heights,
                temperature: &temperature,
                is_liquid_sea: &sea,
                width: WIDE,
                height: HIGH,
            },
        )
    }

    #[test]
    fn rain_falls_downwind_of_the_sea_and_fades_inland() {
        let rain = rain_over(0.1);
        let at = |row: usize| rain[row * WIDE + 16];
        // Fades over the first rows inland. Further on, the sphere tells:
        // a wind blowing south everywhere converges on the south pole, and
        // what it still carries piles up there rather than fading.
        assert!(at(9) > at(10));
        assert!(at(10) > at(12));
        // Nothing upwind of the sea.
        assert!(at(2) < at(9) * 0.1);
    }

    #[test]
    fn a_range_takes_the_rain_and_leaves_a_shadow() {
        let (flat, ranged) = (rain_over(0.1), rain_over(1.5));
        let windward = 14 * WIDE + 16;
        let lee = 17 * WIDE + 16;
        assert!(ranged[windward] > flat[windward]);
        assert!(ranged[lee] < flat[lee] * 0.7);
    }
}
