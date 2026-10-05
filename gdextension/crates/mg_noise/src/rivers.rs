//! Two-tier river generation system (ported from Randlebrot).
//!
//! Rivers are computed once globally on a coarse heightmap, producing an immutable
//! `RiverNetwork` tree. Tiles query this tree at any LOD level — they never compute
//! rivers independently. This ensures river positions are identical at every zoom level.
//!
//! ## Architecture
//!
//! **Tier 1 — Global River Network** (runs once, immutable):
//! Computed on the macro heightmap via geology-aware D8 flow accumulation.
//! Produces a tree of `RiverSegment`s rooted at ocean outlets.
//!
//! **Tier 2 — LOD-Aware Tile Queries**:
//! Tiles call `RiverNetwork::query_chunk()` which returns segments filtered
//! by a drainage threshold that varies with LOD level.

use crate::biome_map::{WORLD_HEIGHT, WORLD_WIDTH};
use crate::drainage::{Drainage, NO_RECEIVER};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

// ─── D8 Constants ────────────────────────────────────────────────────────────

pub(crate) const D8_OFFSETS: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

pub(crate) const D8_DISTANCES: [f64; 8] = [
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
];

pub(crate) const NO_FLOW: u8 = 255;

// ─── LOD Drainage Thresholds ────────────────────────────────────────────────

pub const LOD_THRESHOLD_MACRO: u32 = 30;
pub const LOD_THRESHOLD_MESO: u32 = 4;
pub const LOD_THRESHOLD_MICRO: u32 = 2;

const MAX_RIVER_WORLD_HALF_WIDTH: f64 = 4.0;
// Macro flow grid width limits in WORLD UNITS. Multiplied by `pixels_per_wu`
// at render time so the same river width holds regardless of whether the
// macro grid is 1024×512 (1 px/wu) or 2048×1024 (2 px/wu). Without this
// scaling, doubling macro resolution halves visible river width — the exact
// bug that made rivers vanish after the res bump.
const FLOW_GRID_MIN_HALF_WIDTH_WU: f64 = 1.0;
// Per CLAUDE.md river invariants: rivers cannot be wider than 2 chunks (2 wu).
// Half-width = 1.0 wu → diameter = 2.0 wu = 2 chunks.
const FLOW_GRID_MAX_HALF_WIDTH_WU: f64 = 1.0;
// Runtime tile rasterization is at any pixels-per-wu. Min kept small so
// trickles look like trickles; max caps mains at ~20% of chunk width so
// rivers don't swallow entire 1×1 runtime tiles.
const TILE_RIVER_MIN_HALF_WIDTH_PX: f64 = 3.0;
const TILE_RIVER_MAX_HALF_WIDTH_PX: f64 = 56.0;

/// Macro-specific Strahler → WORLD-UNIT half-width lookup.
///
/// Values in wu — multiplied by `pixels_per_wu` at render time so the river
/// width is resolution-independent. At 1 px/wu these are 1–13 px; at 2 px/wu
/// they're 2–26 px. The hierarchy stays visible at any macro resolution.
fn macro_strahler_half_width_wu(strahler: u32) -> f64 {
    // Exponential progression: each order ~1.6× wider than previous.
    // S1 headwater trickle = 1.4 wu (barely visible at full map zoom).
    // S7 mainstem = 30 wu (clearly visible at full planet view).
    // The 20× ratio between S1 and S7+ makes the trunk rivers
    // dramatically wider than tributaries — readable as a real drainage
    // hierarchy from orbit.
    // Per CLAUDE.md: rivers max 2 chunks wide (half-width max 1.0 wu).
    // Table spans 0.08 (hairline headwater) to 1.0 (max mainstem).
    // ~12× ratio between S1 and S8 for visible hierarchy.
    match strahler.max(1) {
        1 => 0.08,
        2 => 0.12,
        3 => 0.18,
        4 => 0.28,
        5 => 0.42,
        6 => 0.60,
        7 => 0.80,
        _ => 1.00,
    }
}

/// Meander noise — shared instance used by both macro flow grid and runtime
/// chunk rasterization so the same river produces the same meander curve
/// regardless of render scale.
static MEANDER_NOISE: std::sync::OnceLock<noise::OpenSimplex> = std::sync::OnceLock::new();

fn meander_noise_instance() -> &'static noise::OpenSimplex {
    MEANDER_NOISE.get_or_init(|| noise::OpenSimplex::new(0xBEEF_u32))
}

/// Apply perpendicular meander displacement to a path.
///
/// D8 flow-solve paths run in fixed 8 directions and produce long straight
/// runs anywhere the gradient is consistent. Real rivers meander laterally
/// based on terrain slope, sediment, and discharge — this approximates that
/// by displacing each point perpendicular to the local tangent using
/// low-frequency OpenSimplex noise evaluated at the point's WORLD coord.
///
/// Noise frequency (`0.04`) targets ~25 wu meander wavelength. Amplitude is
/// provided in world units; typical values are 1.5-4× the river half-width.
///
/// Endpoint attenuation tapers the displacement at both ends so rivers join
/// smoothly at tributary junctions and river mouths.
fn meander_path(path: &[(f64, f64)], amplitude_wu: f64) -> Vec<(f64, f64)> {
    use noise::NoiseFn;
    if path.len() < 2 || amplitude_wu <= 0.0 {
        return path.to_vec();
    }
    let n = path.len();
    let noise = meander_noise_instance();
    let mut out = Vec::with_capacity(n);
    let endpoint_taper = 8.min(n / 4).max(1);
    for i in 0..n {
        let (wx, wy) = path[i];
        let prev = if i > 0 { path[i - 1] } else { path[i] };
        let next = if i + 1 < n { path[i + 1] } else { path[i] };
        let dx = next.0 - prev.0;
        let dy = next.1 - prev.1;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            out.push((wx, wy));
            continue;
        }
        // Left-hand normal (perpendicular to tangent).
        let nx = -dy / len;
        let ny = dx / len;
        // Two-harmonic meander: low-freq sweep + high-freq wiggle.
        // Low freq (0.05) = ~20 wu wavelength → broad bends.
        // High freq (0.18) = ~5.5 wu wavelength → short wiggles that break
        // up the D8 staircase pattern. Second harmonic at 40% strength so
        // it adds texture without dominating the sweep.
        let low = noise.get([wx * 0.05, wy * 0.05]);
        let high = noise.get([wx * 0.18 + 100.0, wy * 0.18 + 100.0]);
        let noise_value = low + high * 0.4;
        // Taper at endpoints so confluences stay anchored.
        let from_start = i.min(endpoint_taper) as f64 / endpoint_taper as f64;
        let from_end = (n - 1 - i).min(endpoint_taper) as f64 / endpoint_taper as f64;
        let taper = from_start.min(from_end);
        let offset = noise_value * amplitude_wu * taper;
        out.push((wx + nx * offset, wy + ny * offset));
    }
    out
}

/// Strahler-order-driven half-width in WORLD UNITS for visual rasterization.
///
/// Strahler order naturally encodes drainage hierarchy: order 1 = headwater
/// trickle, higher orders are mainstems with much higher discharge. This
/// produces a visible "thinner upstream, thicker downstream" gradient that
/// drainage area alone can't because the drainage range is 1000× but visual
/// width range needs to be 10×. Returns world units; callers convert to
/// pixels at their local `pixels_per_wu`.
///
/// Values are deliberately exaggerated for visual readability — at 1 px/wu
/// macro and 512 px/wu runtime, even Strahler 1 tributaries should look like
/// real channels rather than 1-pixel scratches.
fn strahler_world_half_width(strahler_order: u32) -> f64 {
    match strahler_order.max(1) {
        1 => 0.10,
        2 => 0.16,
        3 => 0.24,
        4 => 0.34,
        5 => 0.46,
        6 => 0.60,
        _ => 0.80,
    }
}
// Lower both knobs so the network includes short headwater tributaries.
// Dendritic drainage (see classic basin patterns) needs many Strahler-1
// trickles feeding into Strahler-2 confluences; with the old ratio 0.00012
// at 1024×512 the effective floor was ~63 cells which filtered out the
// fine-branching texture entirely.
// Channel initiation threshold — minimum catchment area (in cells) before
// a drainage path becomes a named river segment. Low values (4-8) create
// thousands of tiny S1-S2 segments that look like noise. Higher values
// (20+) produce fewer, cleaner channels where real catchment convergence
// has occurred — actual rivers, not every hillside trickle.
const MIN_RIVER_ACCUMULATION_RATIO: f64 = 0.00005;
const MIN_RIVER_ACCUMULATION_FLOOR: f64 = 20.0;

// ─── River Character ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiverCharacter {
    DryWadi,
    SeasonalFlow,
    Permanent,
    Frozen,
    BuriedIce,
}

impl RiverCharacter {
    pub fn classify(light_level: f64, humidity: f64, temperature: f64) -> Self {
        if light_level < 0.05 {
            RiverCharacter::BuriedIce
        } else if light_level < 0.1 && temperature < 0.0 {
            RiverCharacter::Frozen
        } else if light_level < 0.3 || humidity > 0.5 {
            RiverCharacter::Permanent
        } else if light_level < 0.7 || humidity > 0.2 {
            RiverCharacter::SeasonalFlow
        } else {
            RiverCharacter::DryWadi
        }
    }

    fn outside_surface_band(y: usize, height: usize) -> Self {
        if y < height / 5 {
            RiverCharacter::BuriedIce
        } else {
            RiverCharacter::DryWadi
        }
    }

    pub fn width_multiplier(&self) -> f64 {
        match self {
            RiverCharacter::DryWadi => 0.3,
            RiverCharacter::SeasonalFlow => 0.6,
            RiverCharacter::Permanent => 1.0,
            RiverCharacter::Frozen => 0.9,
            // Buried ice channels are subterranean drainage; render faintly so
            // runtime chunks and macro receipts both surface them as a subtle
            // hint instead of skipping them entirely (previously 0.0 made them
            // visible-but-invisible — `is_visible_channel` says yes, the
            // rasteriser skipped because half-width collapsed to zero).
            RiverCharacter::BuriedIce => 0.4,
        }
    }
}

/// Surface water freezes on land darker than this (the same light level at
/// which the sea freezes over).
const RIVER_MIN_LIGHT: f64 = 0.18;
/// Surface water evaporates on land brighter than this (the same light level
/// at which shallow sea dries out).
const RIVER_MAX_LIGHT: f64 = 0.62;
const RIVER_MIN_TEMPERATURE_C: f64 = 0.0;
const RIVER_MAX_TEMPERATURE_C: f64 = 42.0;

/// Whether a river on land holds liquid water: only in the terminus, neither
/// frozen nor evaporated.
fn river_water_is_liquid(light_level: f64, temperature_c: f64) -> bool {
    (RIVER_MIN_LIGHT..=RIVER_MAX_LIGHT).contains(&light_level)
        && (RIVER_MIN_TEMPERATURE_C..=RIVER_MAX_TEMPERATURE_C).contains(&temperature_c)
}

/// Set `surface_from` on every segment. `is_wet(x, y)` says whether liquid
/// water can lie at a point: on land in the terminus, or in liquid sea.
///
/// A river is drawn only where its water stays liquid all the way to the sea.
/// So a segment carries a surface river from the first point after which its
/// whole path is wet, provided every segment downstream of it is wet from end
/// to end and the last one ends in a body of water (`is_open_sea`), not in a
/// pond.
/// Rivers therefore start where the country turns wet enough, and never stop
/// on dry land.
fn mark_surface_rivers(
    segments: &mut [RiverSegment],
    is_wet: impl Fn(f64, f64) -> bool,
    is_open_sea: impl Fn(f64, f64) -> bool,
) {
    let meets_sea_at = |segment: &RiverSegment| {
        segment.path.iter().position(|&(x, y)| is_open_sea(x, y))
    };
    // First point from which the rest of the path is wet; None if the foot is
    // dry. The river ends where it meets a body of water: the path beyond
    // only anchors the mouth, and the water there may be frozen further out.
    let wet_from: Vec<Option<usize>> = segments
        .iter()
        .map(|segment| {
            let river = match meets_sea_at(segment) {
                Some(sea) => &segment.path[..=sea],
                None => &segment.path[..],
            };
            match river.iter().rposition(|&(x, y)| !is_wet(x, y)) {
                None => Some(0),
                Some(last_dry) if last_dry + 1 < river.len() => Some(last_dry + 1),
                Some(_) => None,
            }
        })
        .collect();

    let ends_in_open_sea = |segment: &RiverSegment| meets_sea_at(segment).is_some();
    // Whether water leaving a segment's foot stays liquid down to the sea.
    let reaches_sea: Vec<bool> = (0..segments.len())
        .map(|start| {
            let mut current = start;
            for _ in 0..segments.len() {
                match segments[current].downstream {
                    Some(next) if next < segments.len() => {
                        if wet_from[next] != Some(0) {
                            return false;
                        }
                        current = next;
                    }
                    _ => return ends_in_open_sea(&segments[current]),
                }
            }
            false
        })
        .collect();

    for (index, segment) in segments.iter_mut().enumerate() {
        segment.surface_from = wet_from[index].filter(|_| reaches_sea[index]);
    }
}

/// The direction (an index into `D8_OFFSETS`) from each cell to the cell it
/// drains to, or `NO_FLOW`.
fn flow_directions(drainage: &Drainage, width: usize) -> Vec<u8> {
    drainage
        .receivers
        .iter()
        .enumerate()
        .map(|(cell, &receiver)| {
            if receiver == NO_RECEIVER {
                return NO_FLOW;
            }
            let receiver = receiver as usize;
            let dy = (receiver / width) as i32 - (cell / width) as i32;
            // The map joins east to west, so a step can cross the seam.
            let raw_dx = (receiver % width) as i32 - (cell % width) as i32;
            let dx = if raw_dx.abs() > 1 { -raw_dx.signum() } else { raw_dx };
            D8_OFFSETS
                .iter()
                .position(|&offset| offset == (dx, dy))
                .map_or(NO_FLOW, |direction| direction as u8)
        })
        .collect()
}

// ─── River Segment ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiverSegment {
    pub id: usize,
    pub path: Vec<(f64, f64)>,
    pub drainage_area: u32,
    pub downstream: Option<usize>,
    pub upstream: Vec<usize>,
    pub character: RiverCharacter,
    pub meander_offsets: Vec<f64>,
    pub strahler_order: u32,
    /// Index of the first point of `path` from which liquid water runs on the
    /// surface all the way to the sea. `None` if this segment carries no such
    /// river: it lies in country too dry or too cold, or the water it would
    /// carry does not reach liquid sea. Only this part is drawn.
    pub surface_from: Option<usize>,
}

// ─── River Chain (connected headwater → mouth path) ────────────────────────

/// A continuous path from one headwater all the way to the ocean, built by
/// concatenating segment paths along the `downstream` pointers. Width grows
/// from headwater drainage to mouth drainage. Rendered as ONE rasterize call
/// so there are no visual gaps at confluences.
pub struct RiverChain {
    /// World-coord path from headwater (index 0) to mouth (last).
    pub path: Vec<(f64, f64)>,
    /// Per-point drainage for width modulation — grows monotonically from
    /// headwater upstream drainage to mouth segment drainage.
    pub drainage_per_point: Vec<u32>,
    /// Max drainage in this chain (= mouth segment drainage).
    pub max_drainage: u32,
    /// Max Strahler order along the chain (for width table lookup).
    pub max_strahler: u32,
    /// Character at the chain midpoint (for visibility + color).
    pub character: RiverCharacter,
}

/// Build river chains by following each headwater downstream to the mouth.
///
/// Each chain = continuous path from one headwater all the way to the ocean,
/// concatenating every segment's path along the `downstream` pointers. Trunk
/// segments near the mouth appear in MULTIPLE chains (one per tributary that
/// feeds into them) — the rasterise `max()` blend handles the overlap
/// correctly, and the visual result is a connected dendritic tree where width
/// grows smoothly from source to mouth.
pub fn build_river_chains(segments: &[RiverSegment]) -> Vec<RiverChain> {
    if segments.is_empty() {
        return vec![];
    }

    // Headwaters = segments with no upstream. Each starts a chain.
    let mut chains = Vec::new();
    for (idx, seg) in segments.iter().enumerate() {
        if !seg.upstream.is_empty() {
            continue; // not a headwater
        }

        let mut path = Vec::new();
        let mut drainage_per_point = Vec::new();
        let mut current = idx;
        let mut max_strahler = 0u32;
        let mut last_drainage = 0u32;
        let mut visited = 0usize;

        loop {
            if visited > segments.len() {
                break; // cycle guard
            }
            visited += 1;
            let s = &segments[current];
            max_strahler = max_strahler.max(s.strahler_order);

            // Include ALL segments' paths regardless of character visibility.
            // Previous versions skipped DryWadi/BuriedIce which cut chains
            // before they reached ocean — rivers ended mid-land. Now every
            // chain runs unbroken from headwater to sea. All rivers render
            // as solid blue (grey corridor color was removed earlier).
            let upstream_drain = s
                .upstream
                .iter()
                .filter_map(|&uid| segments.get(uid))
                .map(|u| u.drainage_area)
                .max()
                .unwrap_or(0);

            let n = s.path.len();
            for (i, &pt) in s.path.iter().enumerate() {
                let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 1.0 };
                let d = upstream_drain as f64
                    + (s.drainage_area as f64 - upstream_drain as f64) * t;
                path.push(pt);
                drainage_per_point.push(d as u32);
            }
            last_drainage = s.drainage_area;

            match s.downstream {
                Some(ds) if ds < segments.len() => {
                    current = ds;
                }
                _ => break,
            }
        }

        if path.len() >= 2 {
            // Use the character from the midpoint segment for visibility/color.
            let mid_char = segments.get(idx).map_or(RiverCharacter::Permanent, |s| s.character);
            chains.push(RiverChain {
                path,
                drainage_per_point,
                max_drainage: last_drainage,
                max_strahler,
                character: mid_char,
            });
        }
    }
    chains
}

// ─── Chunk Coordinate for Spatial Index ─────────────────────────────────────

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
struct RiverChunkCoord {
    x: i32,
    y: i32,
}

// ─── River Constraint ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct RiverConstraint {
    pub path: Vec<(f64, f64)>,
    pub drainage_area: u32,
    pub character: RiverCharacter,
    pub width: f64,
    pub depth: f64,
    pub strahler_order: u32,
    pub river_id: usize,
    pub segment_index: usize,
}

// ─── River Network ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct RiverNetwork {
    pub segments: Vec<RiverSegment>,
    #[serde(skip)]
    spatial_index: HashMap<RiverChunkCoord, Vec<usize>>,
    /// Precomputed chains: headwater → mouth continuous paths built by
    /// `build_river_chains`. Stored so both `to_flow_grid` and
    /// `rasterize_from_network` use the same connected river systems.
    #[serde(skip)]
    pub chains: Vec<RiverChain>,
    /// The rivers' final courses in world space, derived from `segments`.
    /// Every raster of the rivers, at any scale, is drawn from these.
    #[serde(skip)]
    pub courses: Vec<RiverCourse>,
    pub width: usize,
    pub height: usize,
}

impl std::fmt::Debug for RiverNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiverNetwork")
            .field("segments", &self.segments.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl Clone for RiverNetwork {
    fn clone(&self) -> Self {
        let mut cloned = Self {
            segments: self.segments.clone(),
            spatial_index: HashMap::new(),
            chains: Vec::new(),
            courses: Vec::new(),
            width: self.width,
            height: self.height,
        };
        cloned.rebuild_spatial_index();
        cloned
    }
}

impl RiverNetwork {
    pub fn empty(width: usize, height: usize) -> Self {
        Self {
            segments: Vec::new(),
            spatial_index: HashMap::new(),
            chains: Vec::new(),
            courses: Vec::new(),
            width,
            height,
        }
    }

    /// Generate the global river network from terrain and geological data.
    /// Build the river network from the drainage that carved the terrain
    /// (`erosion_sim`), so rivers lie in their valleys. `drainage` must have
    /// been solved with `sea_bodies` as its base level.
    pub fn generate(
        drainage: &Drainage,
        tectonic_stress: &[f64],
        continentalness: &[f64],
        light_level: &[f64],
        humidity: &[f64],
        temperature: &[f64],
        width: usize,
        height: usize,
        sea_level: f64,
    ) -> Self {
        let total = width * height;
        let world_width: f64 = 1024.0;
        let world_height: f64 = 512.0;

        // Rivers drain to bodies of water. A pond (a few cells below sea
        // level) is not one: for drainage it counts as land, so a river runs
        // through it and on to the sea instead of ending there.
        let in_sea_body = sea_bodies(continentalness, width, height, sea_level);
        let drainage_continentalness: Vec<f64> = continentalness
            .iter()
            .zip(&in_sea_body)
            .map(|(&cont, &in_body)| {
                if in_body || cont > sea_level { cont } else { sea_level + POND_RAISED_ABOVE_SEA }
            })
            .collect();
        let real_continentalness = continentalness;
        let continentalness = &drainage_continentalness[..];

        let flow_dir = flow_directions(drainage, width);
        let accumulation: Vec<u32> = drainage.flow.iter().map(|&flow| flow.round() as u32).collect();

        // Step 4: Build river tree
        let min_accumulation =
            ((total as f64) * MIN_RIVER_ACCUMULATION_RATIO).max(MIN_RIVER_ACCUMULATION_FLOOR) as u32;
        let mut segments = build_river_tree(
            &flow_dir, &accumulation, continentalness, width, height, sea_level, min_accumulation,
        );

        // Step 5: Classify river character at each segment midpoint
        for seg in &mut segments {
            if seg.path.is_empty() {
                continue;
            }
            let mid_idx = seg.path.len() / 2;
            let (mx, my) = seg.path[mid_idx];
            // Path coords are in WORLD units; convert to macro pixel indices
            // for the light_level/humidity/temperature array lookup.
            let px = ((mx / world_width * width as f64) as usize).min(width - 1);
            let py = ((my / world_height * height as f64) as usize).min(height - 1);
            let idx = py * width + px;
            let light = light_level.get(idx).copied().unwrap_or(0.5);
            let humid = humidity.get(idx).copied().unwrap_or(0.5);
            let temp = temperature.get(idx).copied().unwrap_or(15.0);
            // Narrow the forced-character bands to 20% each polar extreme.
            // The previous 33% bands made terminus-coast rivers (world y 100-170)
            // classify as BuriedIce and vanish after is_visible_channel filtering.
            seg.character = if py < height / 5 || py >= (height * 4) / 5 {
                RiverCharacter::outside_surface_band(py, height)
            } else {
                RiverCharacter::classify(light, humid, temp)
            };
        }

        // Step 5.5: Compute Strahler stream orders
        compute_strahler_orders(&mut segments);

        // Step 6: Smooth paths to remove D8 staircase.
        //
        // CRITICAL: unwrap x-coordinates before chaikin smoothing. D8 paths
        // that cross the world-x wrap boundary have consecutive points like
        // `(1023, y)` → `(0, y)` — physically adjacent across the seam but
        // numerically on opposite sides. Chaikin averages `(1023+0)/2 = 511.5`
        // and inserts that midpoint — a garbage point in the middle of the
        // world. Subsequent rasterizers then draw a straight horizontal line
        // from `(1023, y)` to `(511, y)` to `(0, y)`, visible as full-width
        // stripes on `rivers.png` at the wrap-crossing y row.
        for seg in &mut segments {
            let unwrapped = unwrap_path_x(&seg.path, width as f64);
            seg.path = chaikin_smooth(&unwrapped, 5);
            seg.meander_offsets = vec![0.0; seg.path.len()];
        }

        // Step 6.5: Keep surface rivers to where liquid water can run, and
        // only where it reaches the sea.
        let splines = crate::biome_splines::BiomeSplines::new(sea_level);
        let cell_at = |x: f64, y: f64| {
            let px = (x / world_width * width as f64).floor().rem_euclid(width as f64) as usize;
            let py = ((y / world_height * height as f64) as usize).min(height - 1);
            py * width + px
        };
        let is_wet = |x: f64, y: f64| {
            let idx = cell_at(x, y);
            let light = light_level.get(idx).copied().unwrap_or(0.5);
            let temp = temperature.get(idx).copied().unwrap_or(15.0);
            // A pond on the river's way holds water like the sea does.
            let cont = real_continentalness.get(idx).copied().unwrap_or(0.0);
            if cont < sea_level {
                let tectonic = tectonic_stress.get(idx).copied().unwrap_or(0.5);
                splines.sea_is_liquid(cont, temp, tectonic, light)
            } else {
                river_water_is_liquid(light, temp)
            }
        };
        let is_open_sea = |x: f64, y: f64| in_sea_body[cell_at(x, y)];
        mark_surface_rivers(&mut segments, is_wet, is_open_sea);

        // Diagnostics
        {
            let max_drainage = segments.iter().map(|s| s.drainage_area).max().unwrap_or(0);
            let segs_above_500 = segments.iter().filter(|s| s.drainage_area >= 500).count();
            let segs_above_100 = segments.iter().filter(|s| s.drainage_area >= 100).count();
            let visible = segments.iter().filter(|s| s.surface_from.is_some()).count();
            let buried = segments.iter().filter(|s| matches!(s.character, RiverCharacter::BuriedIce)).count();
            let dry = segments.iter().filter(|s| matches!(s.character, RiverCharacter::DryWadi)).count();
            let frozen = segments.iter().filter(|s| matches!(s.character, RiverCharacter::Frozen)).count();
            let seasonal = segments.iter().filter(|s| matches!(s.character, RiverCharacter::SeasonalFlow)).count();
            let permanent = segments.iter().filter(|s| matches!(s.character, RiverCharacter::Permanent)).count();
            eprintln!(
                "[rivers] {} segments, max drainage {max_drainage}, >=500: {segs_above_500}, >=100: {segs_above_100}  visible: {visible} (perm={permanent} season={seasonal} frozen={frozen}) hidden: buried={buried} dry={dry}",
                segments.len()
            );
        }

        // Step 7: Build spatial index + chains
        let spatial_index = build_spatial_index(&segments);
        let chains = build_river_chains(&segments);

        // Chain diagnostics
        {
            let total_chains = chains.len();
            let avg_len = if total_chains > 0 {
                chains.iter().map(|c| c.path.len()).sum::<usize>() / total_chains
            } else { 0 };
            let max_len = chains.iter().map(|c| c.path.len()).max().unwrap_or(0);
            let long_chains = chains.iter().filter(|c| c.path.len() > 20).count();
            // Check how many chains end near an ocean cell (last point near sea_level)
            let reaches_ocean = chains.iter().filter(|c| {
                if let Some(&(lx, ly)) = c.path.last() {
                    let px = ((lx / 1024.0 * width as f64) as usize).min(width - 1);
                    let py = ((ly / 512.0 * height as f64) as usize).min(height - 1);
                    continentalness.get(py * width + px).copied().unwrap_or(0.0) <= sea_level
                } else { false }
            }).count();
            eprintln!(
                "[chains] {} chains, avg_pts={avg_len}, max_pts={max_len}, long(>20)={long_chains}, reaches_ocean={reaches_ocean}/{}",
                total_chains, total_chains
            );
        }

        let mut network = Self {
            segments,
            spatial_index,
            chains,
            courses: Vec::new(),
            width,
            height,
        };
        network.courses = build_river_courses(&network);
        eprintln!("[rivers] {} courses drawn", network.courses.len());
        network
    }

    pub fn rebuild_spatial_index(&mut self) {
        self.spatial_index = build_spatial_index(&self.segments);
        self.chains = build_river_chains(&self.segments);
        self.courses = build_river_courses(self);
    }

    /// Returns the maximum drainage of any upstream tributary at the
    /// **start** of the segment (its upstream-facing end). Used by the
    /// rasterisers to lerp width along the path so rivers visibly widen
    /// toward the mouth: upstream-end width ~ upstream parent, downstream-end
    /// width ~ own drainage. Headwater segments (no upstream) return 0.
    pub fn upstream_drainage_for(&self, segment_index: usize) -> u32 {
        self.segments
            .get(segment_index)
            .map(|seg| {
                seg.upstream
                    .iter()
                    .filter_map(|&uid| self.segments.get(uid))
                    .map(|s| s.drainage_area)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }

    /// Query river segments intersecting rectangular bounds.
    pub fn query_chunk(
        &self,
        min_x: f64, min_y: f64,
        max_x: f64, max_y: f64,
        lod_drainage_threshold: u32,
    ) -> Vec<RiverConstraint> {
        let ix_min = min_x.floor() as i32;
        let iy_min = min_y.floor() as i32;
        let ix_max = max_x.ceil() as i32;
        let iy_max = max_y.ceil() as i32;

        let mut seen = vec![false; self.segments.len()];
        let mut constraints = Vec::new();

        for iy in iy_min..=iy_max {
            for ix in ix_min..=ix_max {
                let coord = RiverChunkCoord { x: ix, y: iy };
                if let Some(ids) = self.spatial_index.get(&coord) {
                    for &id in ids {
                        if id >= self.segments.len() || seen[id] {
                            continue;
                        }
                        seen[id] = true;
                        let seg = &self.segments[id];
                        if seg.drainage_area < lod_drainage_threshold {
                            continue;
                        }
                        if seg.path.len() < 2 {
                            continue;
                        }
                        // Keep the full segment path. Previously this clipped to a
                        // per-point filter around the query bbox, which dropped every
                        // point except the rare one that happened to land inside a 1×1
                        // chunk and left the caller with a single-point path — causing
                        // `rasterize_from_network` to skip rendering entirely on small
                        // runtime tiles. The downstream rasteriser already clips to
                        // tile pixel bounds, so passing the full polyline is correct
                        // and lets smooth lines draw through small chunks continuously.
                        constraints.push(RiverConstraint {
                            path: seg.path.clone(),
                            drainage_area: seg.drainage_area,
                            character: seg.character,
                            width: compute_river_width(seg.drainage_area, seg.character),
                            depth: compute_river_depth(seg.drainage_area),
                            strahler_order: seg.strahler_order,
                            river_id: seg.id,
                            segment_index: id,
                        });
                    }
                }
            }
        }
        constraints
    }

    /// Convert to a flat flow grid using smooth rasterisation.
    ///
    /// Renders CHAINS (headwater → mouth continuous paths) instead of
    /// individual segments. This produces connected river networks: one
    /// rasterize call per chain means no gaps at confluences, width grows
    /// smoothly from headwater to mouth, and meander applies to the whole
    /// river system as a coherent curve.
    /// The rivers on a grid covering the whole world. A cell's value is the
    /// size of the river running through it, as a share of the largest
    /// possible river (0.0 = no river, 1.0 = two world units wide). Drawn from
    /// the same courses as every other scale; on a coarse grid a river
    /// narrower than a cell still marks the cells it runs through.
    pub fn to_flow_grid(&self, width: usize, height: usize) -> Vec<f64> {
        paint_courses(
            &self.courses,
            (0.0, 0.0),
            (WORLD_WIDTH, WORLD_HEIGHT),
            (width, height),
            |_coverage, half_width| (half_width / COURSE_MAX_HALF_WIDTH_WU).clamp(0.01, 1.0),
        )
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

// ─── Width & Depth ──────────────────────────────────────────────────────────

fn compute_river_width(drainage_area: u32, character: RiverCharacter) -> f64 {
    ((drainage_area as f64).sqrt() * 0.075 * character.width_multiplier())
        .min(MAX_RIVER_WORLD_HALF_WIDTH)
}

fn compute_river_depth(drainage_area: u32) -> f64 {
    (drainage_area as f64).log10().max(0.0) * 0.5
}

// ─── Depression Filling (Priority-Flood) ────────────────────────────────────

#[derive(Clone, Copy)]
struct FloodCell {
    elevation: f64,
    index: usize,
}

impl PartialEq for FloodCell {
    fn eq(&self, other: &Self) -> bool { self.index == other.index }
}
impl Eq for FloodCell {}
impl PartialOrd for FloodCell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for FloodCell {
    fn cmp(&self, other: &Self) -> Ordering {
        other.elevation.partial_cmp(&self.elevation).unwrap_or(Ordering::Equal)
    }
}

pub(crate) fn position_jitter(x: u32, y: u32) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h ^= (y as u64).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 33;
    h = h.wrapping_mul(0x94D049BB133111EB);
    ((h >> 33) as f64) / (1u64 << 31) as f64
}

/// Unwrap a river path's x-coordinates to be continuous in coord space.
///
/// D8 flow paths cross the world x-wrap boundary. A segment flowing from
/// `(1023, y)` to `(1, y)` is physically adjacent across the seam but
/// numerically 1022 cells apart. The rasteriser doesn't know about wrap and
/// linearly interpolates across the whole width — visible as a solid
/// horizontal stripe from x=0 to x=width in every tile that touches that y
/// row. This helper rewrites each point so consecutive entries never jump
/// more than `world_width / 2`, carrying accumulated offsets forward. The
/// resulting coords may fall outside `[0, world_width)` on one side of the
/// wrap — that's intentional and handled by the tile-local pixel clip.
fn unwrap_path_x(path: &[(f64, f64)], world_width: f64) -> Vec<(f64, f64)> {
    if path.len() < 2 || world_width <= 0.0 {
        return path.to_vec();
    }
    let half_width = world_width * 0.5;
    let mut out = Vec::with_capacity(path.len());
    out.push(path[0]);
    for i in 1..path.len() {
        let raw_prev = path[i - 1];
        let unwrapped_prev = out[i - 1];
        let wrap_offset = unwrapped_prev.0 - raw_prev.0;
        let curr = path[i];
        let mut x = curr.0 + wrap_offset;
        // If raw consecutive points still straddle the wrap after carrying the
        // existing offset, add/subtract one more world_width so the difference
        // is the minimal one.
        while x - unwrapped_prev.0 > half_width {
            x -= world_width;
        }
        while x - unwrapped_prev.0 < -half_width {
            x += world_width;
        }
        out.push((x, curr.1));
    }
    out
}

/// Below sea level by more than this, ground is open water however the coast
/// is drawn: biome classification moves the coast by at most this much.
const MOUTH_OPEN_WATER_DEPTH: f64 = 0.05;
/// A river mouth is carried at most this many cells past the coast.
const MOUTH_MAX_CELLS_INTO_SEA: usize = 8;

/// Cells from `start` (below sea level, not included) to the nearest open
/// water, through cells below sea level. If no water in reach is that deep (a
/// shallow sea or a lake), the path goes to the deepest cell there is. Empty
/// if `start` is already the deepest.
///
/// Where the coast is drawn is not exactly where ground drops below sea
/// level (biome classification shifts it a little), so a river that stopped
/// at the first cell below sea level could end on dry land.
fn path_to_open_water(
    start: usize,
    continentalness: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<usize> {
    let depth = |cell: usize| sea_level - continentalness.get(cell).copied().unwrap_or(0.0);
    let mut came_from: HashMap<usize, usize> = HashMap::new();
    let path_to = |cell: usize, came_from: &HashMap<usize, usize>| {
        let mut path = Vec::new();
        let mut current = cell;
        while current != start {
            path.push(current);
            current = came_from[&current];
        }
        path.reverse();
        path
    };
    let mut deepest = start;
    let mut frontier = std::collections::VecDeque::from([(start, 0usize)]);
    while let Some((cell, steps)) = frontier.pop_front() {
        if depth(cell) > MOUTH_OPEN_WATER_DEPTH {
            return path_to(cell, &came_from);
        }
        if depth(cell) > depth(deepest) {
            deepest = cell;
        }
        if steps == MOUTH_MAX_CELLS_INTO_SEA {
            continue;
        }
        for (dx, dy) in D8_OFFSETS {
            let x = crate::wrap::wrap_grid_x((cell % width) as i32 + dx, width) as usize;
            let y = (cell / width) as i32 + dy;
            if y < 0 || y >= height as i32 {
                continue;
            }
            let neighbour = y as usize * width + x;
            if neighbour != start && depth(neighbour) >= 0.0 && !came_from.contains_key(&neighbour) {
                came_from.insert(neighbour, cell);
                frontier.push_back((neighbour, steps + 1));
            }
        }
    }
    path_to(deepest, &came_from)
}

/// A stretch of ground below sea level counts as a body of water, somewhere
/// a river can end, if it covers at least this many cells. Smaller ones are
/// ponds.
const SEA_BODY_MIN_CELLS: usize = 12;
/// For drainage, a pond's bed is treated as this far above sea level.
const POND_RAISED_ABOVE_SEA: f64 = 0.001;

/// For every cell, whether it lies in a body of water: a connected stretch
/// of at least `SEA_BODY_MIN_CELLS` cells below sea level. The map joins east
/// to west.
pub fn sea_bodies(continentalness: &[f64], width: usize, height: usize, sea_level: f64) -> Vec<bool> {
    let below_sea = |cell: usize| continentalness[cell] <= sea_level;
    let mut in_body = vec![false; width * height];
    let mut seen = vec![false; width * height];
    for first in 0..width * height {
        if seen[first] || !below_sea(first) {
            continue;
        }
        // Flood-fill this stretch of water.
        let mut stretch = vec![first];
        seen[first] = true;
        let mut next = 0;
        while next < stretch.len() {
            let cell = stretch[next];
            next += 1;
            for (dx, dy) in D8_OFFSETS {
                let x = crate::wrap::wrap_grid_x((cell % width) as i32 + dx, width) as usize;
                let y = (cell / width) as i32 + dy;
                if y < 0 || y >= height as i32 {
                    continue;
                }
                let neighbour = y as usize * width + x;
                if !seen[neighbour] && below_sea(neighbour) {
                    seen[neighbour] = true;
                    stretch.push(neighbour);
                }
            }
        }
        if stretch.len() >= SEA_BODY_MIN_CELLS {
            for cell in stretch {
                in_body[cell] = true;
            }
        }
    }
    in_body
}

// ─── River Tree Building ────────────────────────────────────────────────────

fn build_river_tree(
    flow_dir: &[u8], accumulation: &[u32], continentalness: &[f64],
    width: usize, height: usize, sea_level: f64, min_accumulation: u32,
) -> Vec<RiverSegment> {
    let total = width * height;
    // Convert pixel indices to world coordinates. At 1:1 macro (1024×512)
    // this is an identity transform. At 2:1 (2048×1024) it scales by 0.5.
    // Without this, path coordinates are in macro pixel space and the
    // runtime rasterizer (which works in world coords) draws nothing because
    // every path point is 2× too far from the chunk.
    let world_width: f64 = 1024.0;
    let world_height: f64 = 512.0;
    let px_to_wx = world_width / width as f64;
    let px_to_wy = world_height / height as f64;
    let is_river: Vec<bool> = accumulation.iter().map(|&a| a >= min_accumulation).collect();

    // Count river-cell inflows
    let mut inflow_count = vec![0u32; total];
    for idx in 0..total {
        if !is_river[idx] || flow_dir[idx] == NO_FLOW { continue; }
        let x = idx % width;
        let y = idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
        let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
        let ny = y as i32 + dy;
        if ny >= 0 && (ny as usize) < height {
            let nidx = ny as usize * width + nx;
            if is_river[nidx] { inflow_count[nidx] += 1; }
        }
    }

    // Find segment start points: headwaters (inflow=0) and confluences (inflow>=2)
    let mut starts: Vec<usize> = Vec::new();
    for idx in 0..total {
        if !is_river[idx] { continue; }
        if inflow_count[idx] == 0 || inflow_count[idx] >= 2 {
            starts.push(idx);
        }
    }
    starts.sort_unstable();
    starts.dedup();

    let mut segment_id_at: Vec<Option<usize>> = vec![None; total];
    let mut segments: Vec<RiverSegment> = Vec::new();

    for &start in &starts {
        if segment_id_at[start].is_some() && inflow_count[start] == 0 { continue; }

        let mut path = Vec::new();
        let mut current = start;

        loop {
            if current != start && segment_id_at[current].is_some() { break; }
            if current != start && inflow_count[current] >= 2 { break; }

            path.push((
                (current % width) as f64 * px_to_wx,
                (current / width) as f64 * px_to_wy,
            ));

            if continentalness.get(current).copied().unwrap_or(0.0) < sea_level { break; }
            if flow_dir[current] == NO_FLOW { break; }

            let x = current % width;
            let y = current / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[current] as usize];
            let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
            let ny = y as i32 + dy;
            if ny < 0 || ny >= height as i32 { break; }
            current = ny as usize * width + nx;
        }

        if path.len() < 2 { continue; }

        let seg_id = segments.len();
        let last = path.last().unwrap();
        let last_idx = last.1 as usize * width + last.0 as usize;
        let drainage = accumulation.get(last_idx).copied().unwrap_or(0);

        for &(px, py) in &path {
            let idx = py as usize * width + px as usize;
            if segment_id_at[idx].is_none() { segment_id_at[idx] = Some(seg_id); }
        }

        segments.push(RiverSegment {
            id: seg_id,
            meander_offsets: vec![0.0; path.len()],
            path,
            drainage_area: drainage,
            downstream: None,
            upstream: Vec::new(),
            character: RiverCharacter::Permanent,
            strahler_order: 1,
            surface_from: None,
        });
    }

    // Link segments downstream. When the immediate next cell has no segment
    // (drainage below min_accumulation), walk D8 flow until finding one or
    // reaching ocean. This bridges the gap between a river segment and its
    // downstream neighbor, adding the intermediate low-drainage cells to the
    // segment's path so chains run unbroken to ocean.
    for i in 0..segments.len() {
        let last = *segments[i].path.last().unwrap();
        let mut cur_idx = (last.1 * px_to_wy.recip()) as usize * width
            + (last.0 * px_to_wx.recip()) as usize;
        cur_idx = cur_idx.min(width * height - 1);
        let mut bridge_path: Vec<(f64, f64)> = Vec::new();
        let to_world =
            |cell: usize| ((cell % width) as f64 * px_to_wx, (cell / width) as f64 * px_to_wy);

        // A segment that already ends below sea level is at the sea: carry
        // it on to open water and look no further downstream.
        let ends_at_sea = continentalness.get(cur_idx).copied().unwrap_or(0.0) <= sea_level;
        if ends_at_sea {
            let onward = path_to_open_water(cur_idx, continentalness, width, height, sea_level);
            bridge_path.extend(onward.into_iter().map(to_world));
        }
        let downstream_steps = if ends_at_sea { 0 } else { width.max(height) };

        for _ in 0..downstream_steps {
            if flow_dir[cur_idx] == NO_FLOW { break; }
            let x = cur_idx % width;
            let y = cur_idx / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[cur_idx] as usize];
            let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
            let ny = y as i32 + dy;
            if ny < 0 || ny >= height as i32 { break; }
            let next = ny as usize * width + nx;

            if let Some(ds) = segment_id_at[next] {
                if ds != i {
                    segments[i].downstream = Some(ds);
                }
                break;
            }
            // At the sea, carry the river on to open water.
            if continentalness.get(next).copied().unwrap_or(0.0) <= sea_level {
                bridge_path.push(to_world(next));
                let onward = path_to_open_water(next, continentalness, width, height, sea_level);
                bridge_path.extend(onward.into_iter().map(to_world));
                break;
            }
            // Add bridge cell to this segment's path
            bridge_path.push((nx as f64 * px_to_wx, (ny as usize) as f64 * px_to_wy));
            segment_id_at[next] = Some(i);
            cur_idx = next;
        }

        if !bridge_path.is_empty() {
            segments[i].path.extend(bridge_path);
            // Update drainage at the new endpoint
            let new_last = cur_idx;
            if let Some(&acc) = accumulation.get(new_last) {
                segments[i].drainage_area = segments[i].drainage_area.max(acc);
            }
        }
    }

    // Build upstream links
    let downstream_links: Vec<(usize, Option<usize>)> = segments.iter().map(|s| (s.id, s.downstream)).collect();
    for (seg_id, downstream) in downstream_links {
        if let Some(ds) = downstream {
            if ds < segments.len() { segments[ds].upstream.push(seg_id); }
        }
    }

    segments
}

// ─── Strahler Stream Order ──────────────────────────────────────────────────

fn compute_strahler_orders(segments: &mut [RiverSegment]) {
    if segments.is_empty() { return; }
    let mut order: Vec<Option<u32>> = vec![None; segments.len()];
    let mut stack: Vec<usize> = Vec::new();

    for i in 0..segments.len() {
        if segments[i].upstream.is_empty() {
            order[i] = Some(1);
            stack.push(i);
        }
    }

    while let Some(seg_idx) = stack.pop() {
        let Some(downstream_id) = segments[seg_idx].downstream else { continue };
        if downstream_id >= segments.len() { continue; }

        let all_computed = segments[downstream_id].upstream.iter()
            .all(|&u| u >= segments.len() || order[u].is_some());
        if !all_computed { continue; }

        let upstream_orders: Vec<u32> = segments[downstream_id].upstream.iter()
            .filter_map(|&u| if u < segments.len() { order[u] } else { None })
            .collect();

        let new_order = if upstream_orders.is_empty() {
            1
        } else {
            let max_order = *upstream_orders.iter().max().unwrap();
            let count_max = upstream_orders.iter().filter(|&&o| o == max_order).count();
            if count_max >= 2 { max_order + 1 } else { max_order }
        };

        order[downstream_id] = Some(new_order);
        stack.push(downstream_id);
    }

    for (i, seg) in segments.iter_mut().enumerate() {
        seg.strahler_order = order[i].unwrap_or(1);
    }
}

// ─── Path Smoothing ─────────────────────────────────────────────────────────

fn chaikin_smooth(path: &[(f64, f64)], passes: usize) -> Vec<(f64, f64)> {
    if path.len() < 3 { return path.to_vec(); }
    let mut current = path.to_vec();
    for _ in 0..passes {
        let n = current.len();
        if n < 3 { break; }
        let mut smoothed = Vec::with_capacity(n * 2);
        smoothed.push(current[0]);
        for i in 0..n - 1 {
            let (ax, ay) = current[i];
            let (bx, by) = current[i + 1];
            if i > 0 { smoothed.push((0.75 * ax + 0.25 * bx, 0.75 * ay + 0.25 * by)); }
            if i + 1 < n - 1 { smoothed.push((0.25 * ax + 0.75 * bx, 0.25 * ay + 0.75 * by)); }
        }
        smoothed.push(current[n - 1]);
        current = smoothed;
    }
    current
}

fn subdivide_to_spacing(path: &[(f64, f64)], target: f64) -> Vec<(f64, f64)> {
    if path.len() < 2 || target <= 0.0 { return path.to_vec(); }
    let max_len = path.windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .fold(0.0f64, f64::max);
    if max_len <= target { return path.to_vec(); }
    let passes = ((max_len / target).log2().ceil() as usize).min(8);
    if passes == 0 { return path.to_vec(); }
    chaikin_smooth(path, passes)
}

fn interpolate_drainage(original: &[u32], new_len: usize) -> Vec<u32> {
    if original.len() == new_len || original.is_empty() { return original.to_vec(); }
    if original.len() == 1 { return vec![original[0]; new_len]; }
    let mut result = Vec::with_capacity(new_len);
    let scale = (original.len() - 1) as f64 / (new_len - 1).max(1) as f64;
    for i in 0..new_len {
        let t = i as f64 * scale;
        let lo = (t as usize).min(original.len() - 1);
        let hi = (lo + 1).min(original.len() - 1);
        let frac = t - lo as f64;
        result.push((original[lo] as f64 * (1.0 - frac) + original[hi] as f64 * frac) as u32);
    }
    result
}

// ─── Graduated Width Rasterisation ──────────────────────────────────────────

pub fn rasterise_smooth_line(
    grid: &mut [f64], width: usize, height: usize,
    path: &[(f64, f64)], drainage_per_point: &[u32],
    max_drainage: u32, max_half_width: f64,
) {
    rasterise_smooth_line_with_min(
        grid, width, height,
        path, drainage_per_point,
        max_drainage, max_half_width, 0.65,
    );
}

/// Like `rasterise_smooth_line` but with an explicit minimum half-width so
/// callers using a Strahler-derived `max_half_width` don't get their intended
/// width collapsed by the internal `norm_drain` modulation. The norm_drain
/// scaling stayed in to keep the small-drainage falloff but the floor is now
/// the caller's responsibility.
pub fn rasterise_smooth_line_with_min(
    grid: &mut [f64], width: usize, height: usize,
    path: &[(f64, f64)], drainage_per_point: &[u32],
    max_drainage: u32, max_half_width: f64,
    min_half_width: f64,
) {
    if path.len() < 2 || max_drainage == 0 { return; }
    let max_drain_f = max_drainage as f64;

    for i in 0..path.len() - 1 {
        let (x0, y0) = path[i];
        let (x1, y1) = path[i + 1];
        let d0 = drainage_per_point[i] as f64;
        let d1 = drainage_per_point[i + 1] as f64;

        let seg_dx = x1 - x0;
        let seg_dy = y1 - y0;
        let seg_len = (seg_dx * seg_dx + seg_dy * seg_dy).sqrt();
        if seg_len < 0.001 { continue; }

        let perp_x = -seg_dy / seg_len;
        let perp_y = seg_dx / seg_len;

        let steps = (seg_len / 0.5).ceil() as usize;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let cx = x0 + seg_dx * t;
            let cy = y0 + seg_dy * t;
            let drainage = d0 + (d1 - d0) * t;

            let norm_drain = (drainage / max_drain_f).sqrt();
            // Caller's `min_half_width` floors the rendered width so the
            // Strahler-derived target isn't shrunk by drainage modulation.
            let half_width = (norm_drain * max_half_width).max(min_half_width);
            let value = 1.0;

            let hw_ceil = half_width.ceil() as i32 + 1;
            let px_center = cx.round() as i32;
            let py_center = cy.round() as i32;

            for dy in -hw_ceil..=hw_ceil {
                for dx in -hw_ceil..=hw_ceil {
                    let px = px_center + dx;
                    let py = py_center + dy;
                    if px < 0 || px >= width as i32 || py < 0 || py >= height as i32 { continue; }

                    let rel_x = px as f64 - cx;
                    let rel_y = py as f64 - cy;
                    let perp_dist = (rel_x * perp_x + rel_y * perp_y).abs();
                    // Solid interior, single-pixel anti-aliased edge.
                    // 2 px AA edge (was 1) so downsampled macromap shows
                    // smoother river edges instead of pixel staircase.
                    let aa_width = 2.0_f64.min(half_width * 0.5);
                    let pixel_value = if perp_dist <= half_width - aa_width {
                        value
                    } else if perp_dist < half_width {
                        value * ((half_width - perp_dist) / aa_width)
                    } else {
                        continue;
                    };
                    let idx = py as usize * width + px as usize;
                    grid[idx] = grid[idx].max(pixel_value);
                }
            }
        }
    }
}

// ─── River courses: one geometry for every scale ────────────────────────────
//
// A river is the same river whether it is drawn on the world map, in a map
// tile or in a game chunk. Each segment's final course is built once, in
// world space, and every raster samples that geometry at its own resolution.
// Whether a point is in a river is then a function of its world position
// alone, so scales cannot disagree and neighbouring tiles meet exactly.

/// Distance between points along a course.
const COURSE_POINT_SPACING_WU: f64 = 0.05;
/// River systems that never reach this Strahler order are not drawn: they are
/// too numerous and too short to read as drainage.
const COURSE_MIN_SYSTEM_STRAHLER: u32 = 3;
/// Half-width of the smallest headwater and of the largest river: about 20
/// and 200 blocks across (a world unit is 512 blocks). CLAUDE.md allows a
/// river up to two world units wide; that is a limit, and a river that wide
/// swallows the valley it runs in.
const COURSE_MIN_HALF_WIDTH_WU: f64 = 0.02;
const COURSE_MAX_HALF_WIDTH_WU: f64 = 0.2;
/// Rivers are bent by warping the plane with noise: a broad sweep plus a
/// shorter wiggle. The warp depends only on position, so a tributary and the
/// river it joins are moved together and still meet. Amplitudes are small
/// enough for their wavelengths that the warp never folds the plane, which
/// would make rivers cross themselves.
const MEANDER_SWEEP_WU: f64 = 2.0;
const MEANDER_SWEEP_FREQUENCY: f64 = 0.05;
const MEANDER_WIGGLE_WU: f64 = 0.4;
const MEANDER_WIGGLE_FREQUENCY: f64 = 0.18;
/// At any resolution a river covers at least this many samples either side of
/// its centre line, so thin rivers do not vanish on coarse grids.
const COURSE_MIN_HALF_WIDTH_SAMPLES: f64 = 0.5;
/// A river's last stretch before the sea straightens out over this length,
/// so its meander cannot swing the mouth away from the water.
const MOUTH_STRAIGHTEN_WU: f64 = 6.0;

/// One river segment's final course.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiverCourse {
    /// World positions from upstream to downstream. Where a course crosses
    /// the east-west seam, x runs on past the edge of the world.
    pub points: Vec<(f32, f32)>,
    /// Half-width in world units at each point.
    pub half_widths: Vec<f32>,
}

/// Where the meander warp moves a world position to.
fn meander_warp(x: f64, y: f64) -> (f64, f64) {
    use noise::NoiseFn;
    let noise = meander_noise_instance();
    // Sampled on a cylinder so the warp is continuous across the seam.
    let offsets = |frequency: f64, amplitude: f64, shift: f64| {
        let [cx, cz, cy] =
            crate::wrap::cylindrical_noise_coords(x, y, frequency, 1.0, WORLD_WIDTH);
        (
            noise.get([cx + shift, cz, cy]) * amplitude,
            noise.get([cx, cz + shift, cy + shift]) * amplitude,
        )
    };
    let sweep = offsets(MEANDER_SWEEP_FREQUENCY, MEANDER_SWEEP_WU, 300.0);
    let wiggle = offsets(MEANDER_WIGGLE_FREQUENCY, MEANDER_WIGGLE_WU, 700.0);
    (x + sweep.0 + wiggle.0, y + sweep.1 + wiggle.1)
}

/// Points every `spacing` along a path, keeping its first and last point.
fn resample_evenly(path: &[(f64, f64)], spacing: f64) -> Vec<(f64, f64)> {
    if path.len() < 2 || spacing <= 0.0 {
        return path.to_vec();
    }
    let mut resampled = vec![path[0]];
    // Distance still to travel before the next point is due.
    let mut until_next = spacing;
    for pair in path.windows(2) {
        let (dx, dy) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
        let length = (dx * dx + dy * dy).sqrt();
        let mut travelled = 0.0;
        while length - travelled >= until_next {
            travelled += until_next;
            let along = travelled / length;
            resampled.push((pair[0].0 + dx * along, pair[0].1 + dy * along));
            until_next = spacing;
        }
        until_next -= length - travelled;
    }
    let last = path[path.len() - 1];
    if resampled.last() != Some(&last) {
        resampled.push(last);
    }
    resampled
}

/// Build the final course of every drawn river segment. Segments share their
/// junction points, and the warp moves a point the same way whichever segment
/// it belongs to, so courses meet end to end.
pub fn build_river_courses(network: &RiverNetwork) -> Vec<RiverCourse> {
    let segments = &network.segments;
    let largest_drainage = segments.iter().map(|s| s.drainage_area).max().unwrap_or(0) as f64;
    if largest_drainage <= 0.0 {
        return Vec::new();
    }

    // Highest Strahler order between a segment and the mouth of its system.
    let system_order = |start: usize| {
        let mut order = segments[start].strahler_order;
        let mut current = start;
        for _ in 0..segments.len() {
            match segments[current].downstream {
                Some(next) if next < segments.len() => {
                    current = next;
                    order = order.max(segments[current].strahler_order);
                }
                _ => break,
            }
        }
        order
    };
    // Drainage of the largest surface river flowing into a segment's head.
    let surface_inflow = |index: usize| {
        segments[index]
            .upstream
            .iter()
            .filter_map(|&upstream| segments.get(upstream))
            .filter(|upstream| upstream.surface_from.is_some())
            .map(|upstream| upstream.drainage_area)
            .max()
            .unwrap_or(0) as f64
    };
    // Width follows drainage, so a river widens downstream and a tributary
    // is never wider than the river it joins.
    let half_width = |drainage: f64, character: RiverCharacter| {
        let share = (drainage / largest_drainage).clamp(0.0, 1.0).sqrt();
        let width = COURSE_MIN_HALF_WIDTH_WU
            + (COURSE_MAX_HALF_WIDTH_WU - COURSE_MIN_HALF_WIDTH_WU) * share;
        width * character.width_multiplier()
    };

    segments
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| Some((index, segment, segment.surface_from?)))
        .filter(|&(index, segment, surface_from)| {
            segment.path.len() - surface_from >= 2
                && system_order(index) >= COURSE_MIN_SYSTEM_STRAHLER
        })
        .map(|(index, segment, surface_from)| {
            // A segment's path stops a cell short of the segment it flows
            // into. Carry it on to that segment's head so the river is unbroken.
            let mut raw_path = segment.path[surface_from..].to_vec();
            let downstream_head = segment
                .downstream
                .and_then(|next| segments.get(next))
                .and_then(|next| next.path.first());
            if let Some(&head) = downstream_head {
                if raw_path.last() != Some(&head) {
                    raw_path.push(head);
                }
            }
            let path = resample_evenly(
                &unwrap_path_x(&raw_path, WORLD_WIDTH),
                COURSE_POINT_SPACING_WU,
            );
            // Drainage grows along the segment from what flows in at its head
            // to its own total at its foot.
            // A river that starts partway along a segment starts from nothing.
            let inflow = if surface_from == 0 {
                surface_inflow(index)
            } else {
                0.0
            };
            let outflow = segment.drainage_area as f64;
            let last = (path.len() - 1).max(1) as f64;
            // Only a river's final segment straightens, and only by its foot:
            // its head keeps the full meander so tributaries still meet it.
            let length = last * COURSE_POINT_SPACING_WU;
            let straighten_over = MOUTH_STRAIGHTEN_WU.min(length);
            let meander_share = |point: usize| {
                if segment.downstream.is_some() || straighten_over <= 0.0 {
                    return 1.0;
                }
                let to_foot = (last - point as f64) * COURSE_POINT_SPACING_WU;
                (to_foot / straighten_over).min(1.0)
            };
            RiverCourse {
                points: path
                    .iter()
                    .enumerate()
                    .map(|(point, &(x, y))| {
                        let (warped_x, warped_y) = meander_warp(x, y);
                        let share = meander_share(point);
                        (
                            (x + (warped_x - x) * share) as f32,
                            (y + (warped_y - y) * share) as f32,
                        )
                    })
                    .collect(),
                half_widths: (0..path.len())
                    .map(|point| {
                        let drainage = inflow + (outflow - inflow) * point as f64 / last;
                        half_width(drainage, segment.character) as f32
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Draw river courses onto a grid covering a rectangle of the world. Samples
/// lie on the rectangle's edges as well as inside it (`tile_w` samples span
/// `world_w` inclusive), the same positions `BiomeMap::generate` uses, so
/// neighbouring tiles share their border samples and agree on them.
///
/// Returns 1.0 inside a river, fading to 0.0 at its bank over one sample.
pub fn rasterize_courses(
    courses: &[RiverCourse],
    origin_x: f64,
    origin_y: f64,
    world_w: f64,
    world_h: f64,
    tile_w: usize,
    tile_h: usize,
) -> Vec<f64> {
    paint_courses(
        courses,
        (origin_x, origin_y),
        (world_w, world_h),
        (tile_w, tile_h),
        |coverage, _half_width| coverage,
    )
}

/// Visit every sample covered by a river and keep the largest
/// `value(coverage, half_width)` for it. `coverage` is 1.0 inside the river,
/// fading to 0.0 at its bank; `half_width` is the river's own half-width
/// there, in world units.
fn paint_courses(
    courses: &[RiverCourse],
    (origin_x, origin_y): (f64, f64),
    (world_w, world_h): (f64, f64),
    (tile_w, tile_h): (usize, usize),
    value: impl Fn(f64, f64) -> f64,
) -> Vec<f64> {
    let mut grid = vec![0.0f64; tile_w * tile_h];
    if courses.is_empty() || tile_w < 2 || tile_h < 2 {
        return grid;
    }
    let step_x = world_w / (tile_w - 1) as f64;
    let step_y = world_h / (tile_h - 1) as f64;
    let sample_size = step_x.max(step_y);
    let min_half_width = COURSE_MIN_HALF_WIDTH_SAMPLES * sample_size;
    let (max_x, max_y) = (origin_x + world_w, origin_y + world_h);

    for course in courses {
        for lap in [-WORLD_WIDTH, 0.0, WORLD_WIDTH] {
            for (ends, widths) in course.points.windows(2).zip(course.half_widths.windows(2)) {
                let (ax, ay) = (ends[0].0 as f64 + lap, ends[0].1 as f64);
                let (bx, by) = (ends[1].0 as f64 + lap, ends[1].1 as f64);
                let (own_a, own_b) = (widths[0] as f64, widths[1] as f64);
                let half_a = own_a.max(min_half_width);
                let half_b = own_b.max(min_half_width);
                let reach = half_a.max(half_b);
                let (low_x, high_x) = (ax.min(bx) - reach, ax.max(bx) + reach);
                let (low_y, high_y) = (ay.min(by) - reach, ay.max(by) + reach);
                if high_x < origin_x || low_x > max_x || high_y < origin_y || low_y > max_y {
                    continue;
                }

                let first_px = ((low_x - origin_x) / step_x).floor().max(0.0) as usize;
                let last_px = (((high_x - origin_x) / step_x).ceil() as usize).min(tile_w - 1);
                let first_py = ((low_y - origin_y) / step_y).floor().max(0.0) as usize;
                let last_py = (((high_y - origin_y) / step_y).ceil() as usize).min(tile_h - 1);
                let (dx, dy) = (bx - ax, by - ay);
                let length_squared = dx * dx + dy * dy;

                for py in first_py..=last_py {
                    let wy = origin_y + py as f64 * step_y;
                    for px in first_px..=last_px {
                        let wx = origin_x + px as f64 * step_x;
                        // Nearest point on the segment, as a fraction along it.
                        let along = if length_squared > 0.0 {
                            (((wx - ax) * dx + (wy - ay) * dy) / length_squared).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let (off_x, off_y) = (wx - (ax + dx * along), wy - (ay + dy * along));
                        let distance = (off_x * off_x + off_y * off_y).sqrt();
                        let half_width = half_a + (half_b - half_a) * along;
                        if distance >= half_width {
                            continue;
                        }
                        let bank = sample_size.min(half_width * 0.5);
                        let coverage = ((half_width - distance) / bank).min(1.0);
                        let painted = value(coverage, own_a + (own_b - own_a) * along);
                        let cell = &mut grid[py * tile_w + px];
                        *cell = cell.max(painted);
                    }
                }
            }
        }
    }
    grid
}

// ─── Rasterize from Global Network ──────────────────────────────────────────

/// Rasterize rivers from the global network onto a tile grid.
/// Queries segments, applies Chaikin subdivision, then graduated rendering.
pub fn rasterize_from_network(
    network: &RiverNetwork,
    world_x: f64, world_y: f64, world_size: f64,
    output_size: usize,
    _lod_drainage_threshold: u32,
) -> Vec<f64> {
    let mut grid = vec![0.0f64; output_size * output_size];
    if network.chains.is_empty() {
        return grid;
    }

    let global_max = network.segments.iter().map(|s| s.drainage_area).max().unwrap_or(1);
    let scale = output_size as f64 / world_size;
    let pixels_per_wu = scale;
    let target_spacing = 0.08 / pixels_per_wu.max(0.0001);

    // Iterate chains (not individual segments). Each chain is a continuous
    // headwater→mouth path. Check if ANY point falls near this tile; if so,
    // render the entire chain (pixel clipping handles the rest). This
    // produces connected river visuals across tile boundaries.
    let margin = 4.0;
    let tile_min_x = world_x - margin;
    let tile_max_x = world_x + world_size + margin;
    let tile_min_y = world_y - margin;
    let tile_max_y = world_y + world_size + margin;

    for chain in &network.chains {
        if chain.path.len() < 2 {
            continue;
        }
        // Quick bbox reject: check if any chain point is near the tile.
        let hits_tile = chain.path.iter().any(|&(wx, wy)| {
            wx >= tile_min_x && wx <= tile_max_x && wy >= tile_min_y && wy <= tile_max_y
        });
        if !hits_tile {
            continue;
        }

        let unwrapped = unwrap_path_x(&chain.path, 1024.0);
        let smoothed = subdivide_to_spacing(&unwrapped, target_spacing);
        if smoothed.len() < 2 {
            continue;
        }

        // Runtime meander: small amplitude so rivers stay within chunk view.
        let world_half_raw = strahler_world_half_width(chain.max_strahler)
            * chain.character.width_multiplier();
        let meander_amplitude = 0.3 + world_half_raw * 1.5;
        let meandered = meander_path(&smoothed, meander_amplitude);

        let pixel_path: Vec<(f64, f64)> = meandered
            .iter()
            .map(|&(wx, wy)| ((wx - world_x) * scale, (wy - world_y) * scale))
            .collect();
        if pixel_path.len() < 2 {
            continue;
        }

        // Interpolate chain's per-point drainage to smoothed path length.
        let n = pixel_path.len();
        let orig_n = chain.drainage_per_point.len();
        let drainage_per_point: Vec<u32> = (0..n)
            .map(|i| {
                let t = i as f64 / (n - 1).max(1) as f64;
                let src = (t * (orig_n - 1).max(1) as f64) as usize;
                chain.drainage_per_point[src.min(orig_n - 1)]
            })
            .collect();

        let max_half_width = (world_half_raw * pixels_per_wu)
            .clamp(TILE_RIVER_MIN_HALF_WIDTH_PX, TILE_RIVER_MAX_HALF_WIDTH_PX);
        let min_half_width = (max_half_width * 0.15).clamp(TILE_RIVER_MIN_HALF_WIDTH_PX * 0.3, max_half_width);

        rasterise_smooth_line_with_min(
            &mut grid,
            output_size,
            output_size,
            &pixel_path,
            &drainage_per_point,
            global_max,
            max_half_width,
            min_half_width,
        );
    }
    grid
}

// ─── Legacy rasterize_to_tile (uses rasterize_from_network) ────────────────

/// Rasterize rivers onto a meso tile (backward-compatible interface).
pub fn rasterize_to_tile(
    network: &RiverNetwork,
    tile_w: usize, tile_h: usize,
    tile_world_x: f64, tile_world_y: f64,
    tile_world_w: f64, tile_world_h: f64,
    _macro_world_w: f64, _macro_world_h: f64,
    threshold: f64,
) -> Vec<f64> {
    // Delegate to rasterize_from_network for the square case.
    // For non-square tiles, use the max dimension.
    let size = tile_w.max(tile_h);
    let world_size = tile_world_w.max(tile_world_h);
    let grid = rasterize_from_network(network, tile_world_x, tile_world_y, world_size, size, threshold as u32);

    // If tile is square and matches output, return directly.
    if tile_w == size && tile_h == size { return grid; }

    // Otherwise crop to tile dimensions.
    let mut result = vec![0.0f64; tile_w * tile_h];
    for y in 0..tile_h.min(size) {
        for x in 0..tile_w.min(size) {
            result[y * tile_w + x] = grid[y * size + x];
        }
    }
    result
}

// ─── Spatial Index ──────────────────────────────────────────────────────────

fn build_spatial_index(segments: &[RiverSegment]) -> HashMap<RiverChunkCoord, Vec<usize>> {
    let mut index: HashMap<RiverChunkCoord, Vec<usize>> = HashMap::new();
    for seg in segments {
        for &(x, y) in &seg.path {
            let coord = RiverChunkCoord { x: x.floor() as i32, y: y.floor() as i32 };
            index.entry(coord).or_default().push(seg.id);
        }
    }
    for ids in index.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    index
}

/// Legacy flat-grid rasterization.
pub fn rasterize_from_network_flat(
    network: &RiverNetwork, width: usize, height: usize, _threshold: f64,
) -> Vec<f64> {
    network.to_flow_grid(width, height)
}

#[cfg(test)]
mod course_tests {
    use super::*;

    fn segment(
        id: usize,
        path: &[(f64, f64)],
        drainage_area: u32,
        downstream: Option<usize>,
        upstream: &[usize],
    ) -> RiverSegment {
        RiverSegment {
            id,
            path: path.to_vec(),
            drainage_area,
            downstream,
            upstream: upstream.to_vec(),
            character: RiverCharacter::Permanent,
            meander_offsets: Vec::new(),
            strahler_order: 3,
            surface_from: Some(0),
        }
    }

    /// Two tributaries joining at (500, 250) into a trunk running south.
    fn forked_network() -> RiverNetwork {
        let mut network = RiverNetwork::empty(1024, 512);
        network.segments = vec![
            segment(0, &[(490.0, 240.0), (500.0, 250.0)], 100, Some(2), &[]),
            segment(1, &[(510.0, 240.0), (500.0, 250.0)], 60, Some(2), &[]),
            segment(2, &[(500.0, 250.0), (500.0, 270.0)], 400, None, &[0, 1]),
        ];
        network.rebuild_spatial_index();
        network
    }

    fn straight_course(from: (f32, f32), to: (f32, f32), half_width: f32) -> RiverCourse {
        RiverCourse {
            points: vec![from, to],
            half_widths: vec![half_width, half_width],
        }
    }

    #[test]
    fn tributaries_end_exactly_where_the_river_they_join_begins() {
        let courses = forked_network().courses;

        assert_eq!(courses.len(), 3);
        let trunk_head = courses[2].points[0];
        assert_eq!(*courses[0].points.last().unwrap(), trunk_head);
        assert_eq!(*courses[1].points.last().unwrap(), trunk_head);
    }

    #[test]
    fn a_segment_that_stops_short_is_carried_on_to_the_river_it_flows_into() {
        let mut network = RiverNetwork::empty(1024, 512);
        network.segments = vec![
            // Ends at (500, 249), one cell before the trunk's head at (500, 250).
            segment(0, &[(490.0, 240.0), (500.0, 249.0)], 100, Some(1), &[]),
            segment(1, &[(500.0, 250.0), (500.0, 270.0)], 400, None, &[0]),
        ];
        network.rebuild_spatial_index();

        assert_eq!(
            network.courses[0].points.last(),
            network.courses[1].points.first()
        );
    }

    #[test]
    fn a_river_widens_downstream_and_a_tributary_is_no_wider_than_its_trunk() {
        let courses = forked_network().courses;
        let (tributary, trunk) = (&courses[0], &courses[2]);

        assert!(tributary.half_widths.first() < tributary.half_widths.last());
        assert!(trunk.half_widths.first() < trunk.half_widths.last());
        assert!(tributary.half_widths.last() <= trunk.half_widths.first());
        assert!(*trunk.half_widths.last().unwrap() <= COURSE_MAX_HALF_WIDTH_WU as f32);
    }

    /// A headwater (0) flowing into a trunk (1) that ends at the sea.
    fn headwater_and_trunk() -> Vec<RiverSegment> {
        vec![
            segment(0, &[(100.0, 100.0), (100.0, 110.0), (100.0, 120.0)], 100, Some(1), &[]),
            segment(1, &[(100.0, 121.0), (100.0, 130.0), (100.0, 140.0)], 400, None, &[0]),
        ]
    }

    #[test]
    fn a_river_in_wet_country_runs_its_whole_length() {
        let mut segments = headwater_and_trunk();
        mark_surface_rivers(&mut segments, |_, _| true, |_, y| y >= 140.0);

        assert_eq!(segments[0].surface_from, Some(0));
        assert_eq!(segments[1].surface_from, Some(0));
    }

    #[test]
    fn a_river_starts_where_the_country_turns_wet() {
        let mut segments = headwater_and_trunk();
        // Dry north of y = 105.
        mark_surface_rivers(&mut segments, |_, y| y > 105.0, |_, y| y >= 140.0);

        assert_eq!(segments[0].surface_from, Some(1));
        assert_eq!(segments[1].surface_from, Some(0));
    }

    #[test]
    fn no_river_is_drawn_above_a_dry_stretch() {
        let mut segments = headwater_and_trunk();
        // The trunk crosses dry ground at y = 130 before reaching the sea.
        mark_surface_rivers(&mut segments, |_, y| y != 130.0, |_, y| y >= 140.0);

        assert_eq!(segments[0].surface_from, None);
        assert_eq!(segments[1].surface_from, Some(2));
    }

    #[test]
    fn no_river_is_drawn_if_it_does_not_end_in_liquid_sea() {
        let mut segments = headwater_and_trunk();
        // The trunk's foot is frozen or dried out.
        mark_surface_rivers(&mut segments, |_, y| y < 140.0, |_, y| y >= 140.0);

        assert_eq!(segments[0].surface_from, None);
        assert_eq!(segments[1].surface_from, None);
    }

    #[test]
    fn no_river_is_drawn_if_it_ends_in_a_pond() {
        let mut segments = headwater_and_trunk();
        mark_surface_rivers(&mut segments, |_, _| true, |_, _| false);

        assert_eq!(segments[0].surface_from, None);
        assert_eq!(segments[1].surface_from, None);
    }

    #[test]
    fn a_river_is_drawn_even_if_the_sea_is_frozen_further_out() {
        let mut segments = headwater_and_trunk();
        segments[1].path.extend([(100.0, 150.0), (100.0, 160.0)]);
        // The sea begins at y = 140 and is frozen from y = 150.
        mark_surface_rivers(&mut segments, |_, y| y < 150.0, |_, y| y >= 140.0);

        assert_eq!(segments[0].surface_from, Some(0));
        assert_eq!(segments[1].surface_from, Some(0));
    }

    #[test]
    fn a_mouth_is_carried_through_the_shallows_to_open_water() {
        // One row, sea level 0: land, then two shallow cells, then open water.
        let continentalness = [0.2, -0.01, -0.02, -0.2, -0.3];
        assert_eq!(path_to_open_water(1, &continentalness, 5, 1, 0.0), vec![2, 3]);
        // Already in open water: nothing to add.
        assert!(path_to_open_water(3, &continentalness, 5, 1, 0.0).is_empty());
    }

    #[test]
    fn in_a_shallow_sea_a_mouth_is_carried_to_the_deepest_water_in_reach() {
        // Nothing here is deep enough to count as open water.
        let continentalness = [0.2, -0.01, -0.03, -0.02, 0.2, 0.2];
        assert_eq!(path_to_open_water(1, &continentalness, 6, 1, 0.0), vec![2]);
    }

    #[test]
    fn a_body_of_water_is_a_stretch_of_sea_of_some_size() {
        // A 20-cell row: one pond cell, land, then a 14-cell lake.
        let mut continentalness = vec![0.2; 20];
        continentalness[1] = -0.1;
        for cell in continentalness.iter_mut().skip(4).take(14) {
            *cell = -0.1;
        }

        let in_body = sea_bodies(&continentalness, 20, 1, 0.0);

        assert!(!in_body[1]);
        assert!(!in_body[2]);
        assert!(in_body[4] && in_body[17]);
    }

    #[test]
    fn only_the_surface_part_of_a_segment_becomes_a_course() {
        let mut network = RiverNetwork::empty(1024, 512);
        network.segments = headwater_and_trunk();
        network.segments[0].surface_from = None;
        network.segments[1].surface_from = Some(1);
        network.rebuild_spatial_index();

        assert_eq!(network.courses.len(), 1);
        let course = &network.courses[0];
        let (start_x, start_y) = meander_warp(100.0, 130.0);
        assert_eq!(course.points[0], (start_x as f32, start_y as f32));
        // It starts from nothing: as narrow as a river gets.
        assert_eq!(course.half_widths[0], COURSE_MIN_HALF_WIDTH_WU as f32);
    }

    #[test]
    fn rivers_run_only_in_the_terminus() {
        assert!(river_water_is_liquid(0.4, 15.0));
        assert!(!river_water_is_liquid(0.1, 15.0));
        assert!(!river_water_is_liquid(0.8, 15.0));
        assert!(!river_water_is_liquid(0.4, -5.0));
        assert!(!river_water_is_liquid(0.4, 60.0));
    }

    #[test]
    fn a_river_mouth_is_not_moved_by_the_meander() {
        let courses = forked_network().courses;
        let trunk = &courses[2];

        // The trunk runs from (500, 250) to its mouth at (500, 270).
        assert_eq!(*trunk.points.last().unwrap(), (500.0, 270.0));
        let (head_x, head_y) = meander_warp(500.0, 250.0);
        assert_eq!(trunk.points[0], (head_x as f32, head_y as f32));
    }

    #[test]
    fn resampling_spaces_points_evenly_and_keeps_both_ends() {
        let path = [(0.0, 0.0), (1.0, 0.0), (1.0, 0.5)];
        let resampled = resample_evenly(&path, 0.25);

        assert_eq!(resampled.first(), Some(&(0.0, 0.0)));
        assert_eq!(resampled.last(), Some(&(1.0, 0.5)));
        assert_eq!(resampled.len(), 7);
        assert_eq!(resampled[1], (0.25, 0.0));
        assert_eq!(resampled[5], (1.0, 0.25));
    }

    #[test]
    fn a_river_system_that_stays_small_is_not_drawn() {
        let mut network = RiverNetwork::empty(1024, 512);
        let mut stream = segment(0, &[(100.0, 100.0), (110.0, 100.0)], 30, None, &[]);
        stream.strahler_order = 2;
        network.segments = vec![stream];
        network.rebuild_spatial_index();

        assert!(network.courses.is_empty());
    }

    #[test]
    fn a_course_marks_the_samples_it_runs_through_and_nothing_far_away() {
        // 17 samples across 16 world units: one sample per world unit.
        let course = straight_course((0.0, 8.0), (16.0, 8.0), 1.0);
        let grid = rasterize_courses(&[course], 0.0, 0.0, 16.0, 16.0, 17, 17);

        for x in 0..17 {
            assert_eq!(grid[8 * 17 + x], 1.0, "on the river, column {x}");
            assert_eq!(grid[2 * 17 + x], 0.0, "far from the river, column {x}");
        }
    }

    #[test]
    fn a_river_thinner_than_a_sample_still_shows_on_a_coarse_grid() {
        let hairline = straight_course((0.0, 8.0), (16.0, 8.0), 0.01);
        let grid = rasterize_courses(&[hairline], 0.0, 0.0, 16.0, 16.0, 17, 17);

        assert!((0..17).all(|x| grid[8 * 17 + x] > 0.0));
    }

    #[test]
    fn neighbouring_tiles_agree_on_the_samples_along_their_shared_border() {
        let course = straight_course((3.0, 1.0), (13.0, 7.0), 0.6);
        let west = rasterize_courses(&[course.clone()], 0.0, 0.0, 8.0, 8.0, 33, 33);
        let east = rasterize_courses(&[course], 8.0, 0.0, 8.0, 8.0, 33, 33);

        for row in 0..33 {
            assert_eq!(west[row * 33 + 32], east[row * 33], "row {row}");
        }
        assert!((0..33).any(|row| west[row * 33 + 32] > 0.0));
    }

    #[test]
    fn the_world_grid_records_how_large_each_river_is() {
        let network = forked_network();
        let grid = network.to_flow_grid(1024, 512);
        let largest = grid.iter().cloned().fold(0.0f64, f64::max);
        let smallest_river = grid.iter().cloned().filter(|&v| v > 0.0).fold(1.0f64, f64::min);

        // The trunk drains the most, so it has the largest value on the grid.
        let trunk_foot = network.courses[2].half_widths.last().copied().unwrap() as f64;
        assert!((largest - trunk_foot / COURSE_MAX_HALF_WIDTH_WU).abs() < 0.02);
        assert!(smallest_river < largest);
    }

    #[test]
    fn a_course_that_crosses_the_seam_is_drawn_on_both_sides() {
        // Runs east past the edge of the world: x goes on beyond 1024.
        let course = straight_course((1020.0, 100.0), (1028.0, 100.0), 0.5);
        let east_edge = rasterize_courses(&[course.clone()], 1016.0, 96.0, 8.0, 8.0, 9, 9);
        let west_edge = rasterize_courses(&[course], 0.0, 96.0, 8.0, 8.0, 9, 9);

        assert_eq!(east_edge[4 * 9 + 6], 1.0);
        assert_eq!(west_edge[4 * 9 + 2], 1.0);
        assert_eq!(west_edge[4 * 9 + 7], 0.0);
    }
}
