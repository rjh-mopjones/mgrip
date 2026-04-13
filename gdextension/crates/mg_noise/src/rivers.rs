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
const FLOW_GRID_MAX_HALF_WIDTH_WU: f64 = 18.0;
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
    // S3-S4 are thin tributary lines that provide branching texture.
    // S5+ are the visible mainstems. The jump at S5 is intentionally
    // large so mainstems read as clearly different features from
    // tributaries, not just slightly wider versions.
    match strahler.max(1) {
        1 => 0.5,
        2 => 0.7,
        3 => 1.0,
        4 => 1.5,
        5 => 3.5,
        6 => 6.0,
        7 => 10.0,
        _ => 15.0,
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
const MIN_RIVER_ACCUMULATION_RATIO: f64 = 0.00008;
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

    fn is_visible_channel(&self) -> bool {
        // Only surface-water channels render. DryWadi (substellar dayside:
        // too hot for surface water — dry geological channels only) and
        // BuriedIce (deep nightside: frozen underground drainage) are real
        // drainage features in the network but should not paint surface
        // water on the macromap or runtime chunks.
        matches!(
            self,
            RiverCharacter::SeasonalFlow
                | RiverCharacter::Permanent
                | RiverCharacter::Frozen
        )
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
            width: self.width,
            height: self.height,
        };
        cloned.rebuild_spatial_index();
        cloned.chains = build_river_chains(&cloned.segments);
        cloned
    }
}

impl RiverNetwork {
    pub fn empty(width: usize, height: usize) -> Self {
        Self {
            segments: Vec::new(),
            spatial_index: HashMap::new(),
            chains: Vec::new(),
            width,
            height,
        }
    }

    /// Generate the global river network from terrain and geological data.
    pub fn generate(
        heightmap: &[f64],
        rock_hardness: &[f64],
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

        // Step 0: Condition heightmap for coherent drainage
        let conditioned = condition_heightmap_for_drainage(
            heightmap, continentalness, tectonic_stress, width, height, sea_level,
        );

        // Step 1: Fill depressions (Priority-Flood)
        let filled = fill_depressions(&conditioned, width, height, sea_level, Some(continentalness));

        // Step 2: Geology-aware D8 flow direction
        let flow_dir = compute_geology_aware_flow(
            &filled, rock_hardness, tectonic_stress, width, height, sea_level, Some(continentalness),
        );

        // Step 3: Flow accumulation
        let mut accumulation = compute_flow_accumulation(&flow_dir, &filled, width, height);

        // Step 3.5: Climate-weighted accumulation.
        //
        // On a tidally locked world, precipitation peaks at the terminus
        // (moderate light_level ~0.3-0.6) where warm dayside air meets cold
        // nightside air. Deep dayside (light > 0.8) evaporates all surface
        // water. Deep nightside (light < 0.1) is frozen solid.
        //
        // Scale each cell's accumulated drainage by a "precipitation
        // effectiveness" factor so that drainage paths through dry/frozen
        // zones contribute less flow. This naturally reduces river formation
        // outside the terminus band without hard climate-zone cutoffs.
        for idx in 0..total {
            let light = light_level.get(idx).copied().unwrap_or(0.5);
            let precip_effectiveness = if light < 0.08 {
                // Deep nightside: frozen, minimal meltwater
                0.05
            } else if light < 0.20 {
                // Outer nightside: some seasonal melt
                0.15 + (light - 0.08) / 0.12 * 0.35
            } else if light < 0.70 {
                // Terminus band: full precipitation
                1.0
            } else if light < 0.85 {
                // Dayside margin: evaporation reduces effectiveness
                1.0 - (light - 0.70) / 0.15 * 0.7
            } else {
                // Deep dayside: nearly all water evaporates
                0.08
            };
            accumulation[idx] = (accumulation[idx] as f64 * precip_effectiveness) as u32;
        }

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

        // Diagnostics
        {
            let max_drainage = segments.iter().map(|s| s.drainage_area).max().unwrap_or(0);
            let segs_above_500 = segments.iter().filter(|s| s.drainage_area >= 500).count();
            let segs_above_100 = segments.iter().filter(|s| s.drainage_area >= 100).count();
            let visible = segments.iter().filter(|s| s.character.is_visible_channel()).count();
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

        Self { segments, spatial_index, chains, width, height }
    }

    pub fn rebuild_spatial_index(&mut self) {
        self.spatial_index = build_spatial_index(&self.segments);
        self.chains = build_river_chains(&self.segments);
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
    pub fn to_flow_grid(&self, width: usize, height: usize) -> Vec<f64> {
        let mut grid = vec![0.0f64; width * height];
        let global_max_drainage = self.segments.iter().map(|s| s.drainage_area).max().unwrap_or(1);
        let pixels_per_wu = width as f64 / 1024.0;
        let py_per_wu = height as f64 / 512.0;
        let target_spacing = 0.08 / pixels_per_wu.max(0.0001);
        let min_px = FLOW_GRID_MIN_HALF_WIDTH_WU * pixels_per_wu;
        let max_px = FLOW_GRID_MAX_HALF_WIDTH_WU * pixels_per_wu;

        let chains = build_river_chains(&self.segments);

        for chain in &chains {
            if chain.path.len() < 2 {
                continue;
            }
            // Only render chains with max Strahler >= 4 on the macro flow grid.
            // S1-S3 are too numerous at 1024×512 and produce scattered fragments
            // that don't read as connected drainage systems at full-map zoom.
            // S4+ chains are the mid-to-large rivers that form the visible
            // dendritic skeleton.
            if chain.max_strahler < 3 {
                continue;
            }
            let unwrapped = unwrap_path_x(&chain.path, 1024.0);
            let smoothed = subdivide_to_spacing(&unwrapped, target_spacing);
            if smoothed.len() < 2 {
                continue;
            }

            // Width from the chain's max Strahler order — the trunk drives
            // the visual weight; interior drainage modulation handles tapering.
            let max_half_width = (macro_strahler_half_width_wu(chain.max_strahler)
                * chain.character.width_multiplier()
                * pixels_per_wu)
                .clamp(min_px, max_px);
            let half_width_wu = max_half_width / pixels_per_wu;
            // Aggressive meander: base 10 wu + 3× river width. For S1 tribs
            // (hw=1.4 wu): 14 wu amplitude = 14 px at 1 px/wu. The D8
            // staircase step is 1 wu = 1 px. 14× displacement >> step →
            // meander visually dominates the staircase. For S7 mains
            // (hw=11 wu): 43 wu amplitude = big sweeping curves.
            let meander_amplitude = 10.0 + half_width_wu * 3.0;
            let smoothed = meander_path(&smoothed, meander_amplitude);

            // Interpolate drainage_per_point to match smoothed path length.
            let n = smoothed.len();
            let orig_n = chain.drainage_per_point.len();
            let drainage_per_point: Vec<u32> = (0..n)
                .map(|i| {
                    let t = i as f64 / (n - 1).max(1) as f64;
                    let src = (t * (orig_n - 1).max(1) as f64) as usize;
                    chain.drainage_per_point[src.min(orig_n - 1)]
                })
                .collect();

            let min_half_width = (max_half_width * 0.15).clamp(min_px * 0.3, max_half_width);

            // World → pixel conversion.
            let pixel_path: Vec<(f64, f64)> = smoothed
                .iter()
                .map(|&(wx, wy)| (wx * pixels_per_wu, wy * py_per_wu))
                .collect();
            rasterise_smooth_line_with_min(
                &mut grid,
                width,
                height,
                &pixel_path,
                &drainage_per_point,
                global_max_drainage,
                max_half_width,
                min_half_width,
            );
        }
        grid
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

pub(crate) fn fill_depressions(
    elevation: &[f64], width: usize, height: usize, sea_level: f64,
    continentalness: Option<&[f64]>,
) -> Vec<f64> {
    // Base epsilon for Priority Flood fill. Using a UNIFORM epsilon creates
    // perfectly monotonic gradients on flat plateaus — BFS-equidistant cells
    // get identical elevations, D8 ties break to the first-checked offset
    // (north), and large flat regions produce long straight axis-aligned
    // river chains visible as horizontal/vertical stripes on `rivers.png`.
    //
    // Position-hashed per-cell multiplier breaks the symmetry: two adjacent
    // cells that would otherwise receive the same epsilon now receive
    // slightly different increments, so D8 sees a real (if tiny) gradient
    // and picks varied neighbors. The hash is deterministic in world coords
    // so macro and runtime agree.
    let base_epsilon = 1e-4;
    let mut filled = elevation.to_vec();
    let mut resolved = vec![false; width * height];
    let mut heap = BinaryHeap::new();

    // ONLY initialise ocean cells as PF seeds so every drained cell flows
    // toward an actual sea, not the map's top/bottom edge. Previous behaviour
    // also seeded y=0 and y=height-1 as "boundary sinks", which let inland
    // drainage escape off-map to the polar edges without ever reaching
    // water. Rivers can now terminate only where they hit ocean (or drop
    // into an endorheic basin that PF raises to its spill level).
    //
    // For fully land-locked worlds this would leave cells unresolved; but
    // Margin's macromap has a terminator ocean belt, so every connected
    // land mass has an ocean path and PF covers the whole continent.
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let is_ocean = if let Some(cont) = continentalness {
                cont[idx] <= sea_level
            } else {
                elevation[idx] <= sea_level
            };
            if is_ocean {
                heap.push(FloodCell { elevation: elevation[idx], index: idx });
                resolved[idx] = true;
            }
        }
    }

    while let Some(cell) = heap.pop() {
        let x = cell.index % width;
        let y = cell.index / width;
        for &(dx, dy) in &D8_OFFSETS {
            let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width);
            let ny = y as i32 + dy;
            if ny < 0 || ny >= height as i32 { continue; }
            let nidx = ny as usize * width + nx as usize;
            if resolved[nidx] { continue; }
            resolved[nidx] = true;
            // Position-hashed per-cell jitter so PF-filled plateaus don't form
            // uniform monotonic gradients that D8 tie-breaks into straight
            // axis-aligned runs.
            let hash = position_jitter(nx as u32, ny as u32);
            let new_elev = if elevation[nidx] <= filled[cell.index] {
                filled[cell.index] + base_epsilon * (0.3 + 1.4 * hash)
            } else {
                elevation[nidx]
            };
            filled[nidx] = new_elev;
            heap.push(FloodCell { elevation: new_elev, index: nidx });
        }
    }
    filled
}

/// Deterministic coordinate-hashed jitter in `[0.0, 1.0)`. Breaks spatial
/// symmetry in priority-flood fill and D8 tie-breaks so flat regions don't
/// produce axis-aligned river chains. Also used for meander noise seeding
/// and anywhere else we need "small per-cell variation tied to world coord".
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

// ─── Heightmap Drainage Conditioning ────────────────────────────────────────

fn condition_heightmap_for_drainage(
    heightmap: &[f64], continentalness: &[f64], tectonic_stress: &[f64],
    width: usize, height: usize, sea_level: f64,
) -> Vec<f64> {
    let total = width * height;
    let smoothed = box_blur(heightmap, width, height, 48);
    // Blend factor: how much of the box-blurred heightmap to use for
    // drainage conditioning. Lower = more local relief preserved = more
    // varied D8 flow directions = rivers follow terrain features instead
    // of taking the straightest path downhill. 0.50 (was 0.80) keeps
    // enough smoothing to prevent noise-driven fragmentation while letting
    // valleys and ridges guide drainage.
    let blend = 0.50;
    let mut conditioned = Vec::with_capacity(total);
    for idx in 0..total {
        conditioned.push(heightmap[idx] * (1.0 - blend) + smoothed[idx] * blend);
    }
    let beta = 0.05;
    for idx in 0..total {
        if continentalness[idx] > sea_level {
            conditioned[idx] += tectonic_stress[idx] * beta;
        }
    }
    conditioned
}

pub(crate) fn box_blur(data: &[f64], width: usize, height: usize, radius: usize) -> Vec<f64> {
    let mut temp = data.to_vec();
    let mut output = data.to_vec();

    // Horizontal pass
    for y in 0..height {
        let mut sum = 0.0;
        let mut count = 0;
        for x in 0..=radius.min(width - 1) {
            sum += data[y * width + x];
            count += 1;
        }
        for x in 0..width {
            temp[y * width + x] = sum / count as f64;
            let right = x + radius + 1;
            if right < width { sum += data[y * width + right]; count += 1; }
            if x >= radius { sum -= data[y * width + (x - radius)]; count -= 1; }
        }
    }

    // Vertical pass
    for x in 0..width {
        let mut sum = 0.0;
        let mut count = 0;
        for y in 0..=radius.min(height - 1) {
            sum += temp[y * width + x];
            count += 1;
        }
        for y in 0..height {
            output[y * width + x] = sum / count as f64;
            let bottom = y + radius + 1;
            if bottom < height { sum += temp[bottom * width + x]; count += 1; }
            if y >= radius { sum -= temp[(y - radius) * width + x]; count -= 1; }
        }
    }
    output
}

// ─── Geology-Aware D8 Flow Direction ────────────────────────────────────────

fn compute_geology_aware_flow(
    elevation: &[f64], rock_hardness: &[f64], tectonic_stress: &[f64],
    width: usize, height: usize, sea_level: f64,
    continentalness: Option<&[f64]>,
) -> Vec<u8> {
    let mut flow_dir = vec![NO_FLOW; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let is_ocean = if let Some(cont) = continentalness {
                cont[idx] <= sea_level
            } else {
                elevation[idx] <= sea_level
            };
            if is_ocean { continue; }

            let mut max_slope = 0.0;
            let mut best_dir = NO_FLOW;
            // Cell-level jitter so tie-breaks between neighbors of equal slope
            // vary spatially. Without this the first-checked offset (north)
            // always wins on flat terrain and D8 emits long axis-aligned
            // chains.
            let cell_jitter = position_jitter(x as u32, y as u32);
            for (dir, &(dx, dy)) in D8_OFFSETS.iter().enumerate() {
                let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
                let ny = y as i32 + dy;
                if ny < 0 || ny >= height as i32 { continue; }
                let nidx = ny as usize * width + nx;
                let base_slope = (elevation[idx] - elevation[nidx]) / D8_DISTANCES[dir];
                let geo_factor = (1.0
                    - rock_hardness.get(nidx).copied().unwrap_or(0.5) * 0.5
                    + tectonic_stress.get(nidx).copied().unwrap_or(0.0) * 0.4)
                    .clamp(0.1, 2.0);
                // Hash per (cell, direction) so different directions get
                // different tiny bonuses — selects a varied direction on flat
                // terrain rather than always the first one tested.
                let dir_jitter = position_jitter(
                    x as u32 * 8 + dir as u32,
                    y as u32,
                );
                let tie_break = (cell_jitter + dir_jitter) * 1e-9;
                let adjusted = base_slope * geo_factor + tie_break;
                if adjusted > max_slope {
                    max_slope = adjusted;
                    best_dir = dir as u8;
                }
            }
            flow_dir[idx] = best_dir;
        }
    }
    flow_dir
}

// ─── Flow Accumulation ──────────────────────────────────────────────────────

pub(crate) fn compute_flow_accumulation(
    flow_dir: &[u8], elevation: &[f64], width: usize, height: usize,
) -> Vec<u32> {
    let total = width * height;
    let mut accumulation = vec![1u32; total];
    let mut sorted: Vec<usize> = (0..total).collect();
    sorted.sort_by(|&a, &b| elevation[b].partial_cmp(&elevation[a]).unwrap_or(Ordering::Equal));
    for &idx in &sorted {
        if flow_dir[idx] == NO_FLOW { continue; }
        let x = idx % width;
        let y = idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
        let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
        let ny = (y as i32 + dy) as usize;
        if ny < height {
            let target = ny * width + nx;
            accumulation[target] = accumulation[target].saturating_add(accumulation[idx]);
        }
    }
    accumulation
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

        for _ in 0..width.max(height) {
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
            // When we reach ocean, extend 5 cells INTO the water so the river
            // visually overlaps with the ocean render. A single cell wasn't
            // enough — the river mouth needs to extend into the ocean far
            // enough that the confluence overlay in terrain_render paints a
            // visible estuary/mouth where river meets sea.
            if continentalness.get(next).copied().unwrap_or(0.0) <= sea_level {
                bridge_path.push((nx as f64 * px_to_wx, (ny as usize) as f64 * px_to_wy));
                // Walk a few more cells into ocean
                let mut ocean_cur = next;
                for _ in 0..5 {
                    if flow_dir[ocean_cur] == NO_FLOW { break; }
                    let ox = ocean_cur % width;
                    let oy = ocean_cur / width;
                    let (odx, ody) = D8_OFFSETS[flow_dir[ocean_cur] as usize];
                    let onx = crate::wrap::wrap_grid_x(ox as i32 + odx, width) as usize;
                    let ony = oy as i32 + ody;
                    if ony < 0 || ony >= height as i32 { break; }
                    ocean_cur = ony as usize * width + onx;
                    bridge_path.push((onx as f64 * px_to_wx, (ony as usize) as f64 * px_to_wy));
                }
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

// ─── Legacy API (generate_river_network) ────────────────────────────────────

/// Legacy entry point — generates a RiverNetwork using the full pipeline.
pub fn generate_river_network(
    heightmap: &[f64], width: usize, height: usize,
    light_level: &[f64], humidity: &[f64], temperature: &[f64],
    _threshold: f64,
) -> RiverNetwork {
    // Use zeros for missing geological layers — the conditioning will still work
    // from the heightmap smoothing alone.
    let rock_hardness = vec![0.5; width * height];
    let tectonic_stress = vec![0.0; width * height];
    let continentalness = heightmap.to_vec(); // approximate: use heightmap as continentalness
    let sea_level = crate::biome_map::SEA_LEVEL;

    RiverNetwork::generate(
        heightmap, &rock_hardness, &tectonic_stress, &continentalness,
        light_level, humidity, temperature,
        width, height, sea_level,
    )
}

/// Legacy flat-grid rasterization.
pub fn rasterize_from_network_flat(
    network: &RiverNetwork, width: usize, height: usize, _threshold: f64,
) -> Vec<f64> {
    network.to_flow_grid(width, height)
}
