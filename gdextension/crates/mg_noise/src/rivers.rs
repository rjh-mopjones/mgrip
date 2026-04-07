//! Two-tier river generation. Global D8 flow network computed once at macro scale.

use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

pub(crate) const D8_OFFSETS: [(i32, i32); 8] = [
    (0, -1), (1, -1), (1, 0), (1, 1),
    (0, 1), (-1, 1), (-1, 0), (-1, -1),
];

pub(crate) const D8_DISTANCES: [f64; 8] = [
    1.0, std::f64::consts::SQRT_2, 1.0, std::f64::consts::SQRT_2,
    1.0, std::f64::consts::SQRT_2, 1.0, std::f64::consts::SQRT_2,
];

pub(crate) const NO_FLOW: u8 = 255;

pub const LOD_THRESHOLD_MACRO: f64 = 500.0;
pub const LOD_THRESHOLD_MESO: f64 = 50.0;
pub const LOD_THRESHOLD_MICRO: f64 = 5.0;

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
        if light_level < 0.05 { RiverCharacter::BuriedIce }
        else if light_level < 0.1 && temperature < 0.0 { RiverCharacter::Frozen }
        else if light_level < 0.3 || humidity > 0.5 { RiverCharacter::Permanent }
        else if light_level < 0.7 || humidity > 0.2 { RiverCharacter::SeasonalFlow }
        else { RiverCharacter::DryWadi }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiverSegment {
    pub path: Vec<(usize, usize)>,
    pub drainage_area: f64,
    pub downstream: Option<usize>,
    pub upstream: Vec<usize>,
    pub character: RiverCharacter,
    pub strahler_order: u32,
}

/// Global river network — computed once on the macro heightmap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiverNetwork {
    pub segments: Vec<RiverSegment>,
    /// Map from (chunk_x, chunk_y) → segment indices that pass through
    pub spatial_index: HashMap<(usize, usize), Vec<usize>>,
    pub width: usize,
    pub height: usize,
}

impl RiverNetwork {
    pub fn empty(width: usize, height: usize) -> Self {
        Self {
            segments: Vec::new(),
            spatial_index: HashMap::new(),
            width,
            height,
        }
    }

    /// Query flow accumulation at a pixel, using threshold appropriate for LOD.
    pub fn flow_at(&self, _x: usize, _y: usize) -> f64 {
        0.0
    }
}

/// Rasterize river network into a flow-value grid.
/// Each cell gets the drainage_area of the largest segment passing through it.
pub fn rasterize_from_network(network: &RiverNetwork, width: usize, height: usize, threshold: f64) -> Vec<f64> {
    let mut grid = vec![0.0f64; width * height];
    for seg in &network.segments {
        if seg.drainage_area < threshold { continue; }
        for &(x, y) in &seg.path {
            if x < width && y < height {
                let idx = y * width + x;
                if seg.drainage_area > grid[idx] {
                    grid[idx] = seg.drainage_area;
                }
            }
        }
    }
    grid
}

// ─── Priority-flood depression filling ───────────────────────────────────────

#[derive(PartialEq)]
struct HeapEntry(f64, usize);

impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Fill topographic depressions using Priority-Flood (Wang & Liu 2006).
pub fn fill_depressions(elevation: &mut Vec<f64>, width: usize, height: usize) {
    let total = width * height;
    let epsilon = 1e-5;
    let mut open: BinaryHeap<HeapEntry> = BinaryHeap::new();
    let mut closed = vec![false; total];

    // Seed with all border cells
    for y in 0..height {
        for x in 0..width {
            if x == 0 || x == width - 1 || y == 0 || y == height - 1 {
                let idx = y * width + x;
                open.push(HeapEntry(elevation[idx], idx));
                closed[idx] = true;
            }
        }
    }

    while let Some(HeapEntry(elev, idx)) = open.pop() {
        let x = idx % width;
        let y = idx / width;
        for &(dx, dy) in &D8_OFFSETS {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 { continue; }
            let nidx = ny as usize * width + nx as usize;
            if closed[nidx] { continue; }
            closed[nidx] = true;
            if elevation[nidx] < elev + epsilon {
                elevation[nidx] = elev + epsilon;
            }
            open.push(HeapEntry(elevation[nidx], nidx));
        }
    }
}

/// Compute D8 flow directions and accumulation.
pub fn compute_flow_accumulation(elevation: &[f64], width: usize, height: usize) -> (Vec<u8>, Vec<u32>) {
    let total = width * height;
    let mut flow_dir = vec![NO_FLOW; total];
    let mut accumulation = vec![1u32; total];

    // Compute D8 flow directions
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let h = elevation[idx];
            let mut best_slope = 0.0;
            let mut best_dir = NO_FLOW;

            for (d, &(dx, dy)) in D8_OFFSETS.iter().enumerate() {
                let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
                let ny = y as i32 + dy;
                if ny < 0 || ny >= height as i32 { continue; }
                let nidx = ny as usize * width + nx;
                let slope = (h - elevation[nidx]) / D8_DISTANCES[d];
                if slope > best_slope { best_slope = slope; best_dir = d as u8; }
            }
            flow_dir[idx] = best_dir;
        }
    }

    // Sort cells high-to-low, accumulate drainage
    let mut order: Vec<usize> = (0..total).collect();
    order.sort_unstable_by(|&a, &b| elevation[b].partial_cmp(&elevation[a]).unwrap_or(Ordering::Equal));

    for &idx in &order {
        let dir = flow_dir[idx];
        if dir == NO_FLOW { continue; }
        let x = (idx % width) as i32;
        let y = (idx / width) as i32;
        let (dx, dy) = D8_OFFSETS[dir as usize];
        let nx = crate::wrap::wrap_grid_x(x + dx, width) as usize;
        let ny = y + dy;
        if ny >= 0 && (ny as usize) < height {
            let nidx = ny as usize * width + nx;
            accumulation[nidx] += accumulation[idx];
        }
    }

    (flow_dir, accumulation)
}

/// Box blur for smoothing drainage area maps (used by erosion sim).
pub fn box_blur(input: &[f64], width: usize, height: usize, radius: usize) -> Vec<f64> {
    let mut output = vec![0.0f64; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            let mut count = 0;
            for dy in -(radius as i32)..=(radius as i32) {
                for dx in -(radius as i32)..=(radius as i32) {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
                        sum += input[ny as usize * width + nx as usize];
                        count += 1;
                    }
                }
            }
            output[y * width + x] = sum / count as f64;
        }
    }
    output
}

/// Generate the global river network from a post-erosion heightmap.
pub fn generate_river_network(
    heightmap: &[f64],
    width: usize,
    height: usize,
    light_level: &[f64],
    humidity: &[f64],
    temperature: &[f64],
    threshold: f64,
) -> RiverNetwork {
    let mut elev = heightmap.to_vec();
    fill_depressions(&mut elev, width, height);
    let (flow_dir, accumulation) = compute_flow_accumulation(&elev, width, height);

    let mut network = RiverNetwork::empty(width, height);
    let mut spatial_index: HashMap<(usize, usize), Vec<usize>> = HashMap::new();

    // Extract segments where accumulation exceeds threshold
    // Simple extraction: each cell above threshold is a river segment of length 1
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let drain = accumulation[idx] as f64;
            if drain < threshold { continue; }

            let light = if idx < light_level.len() { light_level[idx] } else { 0.5 };
            let humid = if idx < humidity.len() { humidity[idx] } else { 0.5 };
            let temp = if idx < temperature.len() { temperature[idx] } else { 15.0 };
            let character = RiverCharacter::classify(light, humid, temp);

            // Find downstream cell
            let downstream_seg = if flow_dir[idx] != NO_FLOW {
                let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
                let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width) as usize;
                let ny = y as i32 + dy;
                if ny >= 0 && (ny as usize) < height {
                    Some(ny as usize * width + nx)
                } else { None }
            } else { None };

            let seg_id = network.segments.len();
            spatial_index.entry((x, y)).or_default().push(seg_id);

            network.segments.push(RiverSegment {
                path: vec![(x, y)],
                drainage_area: drain,
                downstream: downstream_seg,
                upstream: Vec::new(),
                character,
                strahler_order: 1,
            });
        }
    }

    network.spatial_index = spatial_index;
    network
}

/// Rasterize the global river network (in macro pixel coords) into a meso tile buffer.
///
/// Each macro segment pixel's world footprint is projected onto the tile's pixel grid.
/// One macro pixel covers `(macro_world_w / network.width)` × `(macro_world_h / network.height)`
/// world units, which expands to an N×M footprint at meso resolution.
pub fn rasterize_to_tile(
    network: &RiverNetwork,
    tile_w: usize,
    tile_h: usize,
    tile_world_x: f64,
    tile_world_y: f64,
    tile_world_w: f64,
    tile_world_h: f64,
    macro_world_w: f64,
    macro_world_h: f64,
    threshold: f64,
) -> Vec<f64> {
    let macro_px_w = macro_world_w / network.width as f64;
    let macro_px_h = macro_world_h / network.height as f64;
    let meso_ppw = tile_w as f64 / tile_world_w;   // meso pixels per world unit X
    let meso_pph = tile_h as f64 / tile_world_h;   // meso pixels per world unit Y

    let mut grid = vec![0.0f64; tile_w * tile_h];

    for seg in &network.segments {
        if seg.drainage_area < threshold { continue; }
        for &(mx, my) in &seg.path {
            // World bounding box of this macro pixel
            let wx0 = mx as f64 * macro_px_w;
            let wy0 = my as f64 * macro_px_h;
            let wx1 = wx0 + macro_px_w;
            let wy1 = wy0 + macro_px_h;
            // Clip to this tile's world extent
            let rx0 = (wx0 - tile_world_x).max(0.0);
            let ry0 = (wy0 - tile_world_y).max(0.0);
            let rx1 = (wx1 - tile_world_x).min(tile_world_w);
            let ry1 = (wy1 - tile_world_y).min(tile_world_h);
            if rx0 >= rx1 || ry0 >= ry1 { continue; }
            // Convert to meso pixel ranges
            let px0 = (rx0 * meso_ppw) as usize;
            let py0 = (ry0 * meso_pph) as usize;
            let px1 = ((rx1 * meso_ppw) as usize + 1).min(tile_w);
            let py1 = ((ry1 * meso_pph) as usize + 1).min(tile_h);
            for py in py0..py1 {
                for px in px0..px1 {
                    let idx = py * tile_w + px;
                    if seg.drainage_area > grid[idx] {
                        grid[idx] = seg.drainage_area;
                    }
                }
            }
        }
    }
    grid
}

/// Constraint used when generating chunk-level rivers locked to the global network.
pub struct RiverConstraint {
    pub x: usize,
    pub y: usize,
    pub flow_value: f64,
}
