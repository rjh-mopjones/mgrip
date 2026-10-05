//! Terrain tiles for the website's map, compiled to WebAssembly.
//!
//! The map page starts from one image of the whole world. As the view zooms
//! in, it asks this module for sharper tiles of what is on screen. Tiles are
//! made by the same generator as the game's terrain, anchored to the same
//! macro pack, so the map shows the world the game will load.
//!
//! The interface is plain C functions over the module's memory, so the page
//! needs no generated bindings: see `site/assets/tile-worker.js`.

use std::sync::Mutex;

use mg_artifacts::MacroPack;
use mg_noise::{
    generate_map_tile, render_terrain, BiomeMap, MapTileDetail, NormalizationHints, RiverCourse,
};

/// Tiles this sharp or sharper (pixels per world unit) show the ground as
/// the game generates it; coarser tiles show the broad shape of the land.
const GROUND_DETAIL_FROM_PIXELS_PER_WU: f64 = 12.0;
/// Height is in units of 200 blocks and a world unit is 512 blocks, so this
/// turns a rise in height per world unit into a true slope.
const TRUE_SLOPE_PER_HEIGHT_PER_WU: f64 = 200.0 / 512.0;
/// Relief is exaggerated, as on any map: by this much on a tile of one pixel
/// per world unit, and by less the sharper the tile, so that both mountain
/// ranges from afar and single slopes up close read clearly.
const RELIEF_EXAGGERATION_AT_ONE_WU: f64 = 30.0;
const RELIEF_EXAGGERATION_FALLOFF: f64 = 0.54;
const RELIEF_MIN_EXAGGERATION: f64 = 1.5;
/// Direction towards the light: north-west and above.
const LIGHT: [f64; 3] = [-1.0, -1.0, 1.6];
/// Relief darkens to and lightens to at most these multiples of a colour.
const RELIEF_SHADE_RANGE: (f64, f64) = (0.6, 1.35);

struct World {
    seed: u32,
    macro_map: BiomeMap,
    river_courses: Vec<RiverCourse>,
    heights: NormalizationHints,
}

static WORLD: Mutex<Option<World>> = Mutex::new(None);
/// The last tile rendered, kept alive until the page has copied it out.
static TILE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

impl World {
    fn from_pack(pack: &MacroPack) -> Self {
        let macro_map = pack.to_biome_map();
        Self {
            seed: pack.seed,
            heights: NormalizationHints::for_macro_map(&macro_map),
            river_courses: pack.river_courses().to_vec(),
            macro_map,
        }
    }

    /// RGBA pixels of a square tile, `pixels` on a side, covering `size`
    /// world units from (`origin_x`, `origin_y`). Each pixel is the terrain
    /// at its centre, shaded by relief lit from the north-west.
    fn render_tile(&self, origin_x: f64, origin_y: f64, size: f64, pixels: usize) -> Vec<u8> {
        // Shading looks at neighbouring samples, so generate one extra sample
        // all round and cut it off; otherwise tile edges would show.
        let step = size / pixels as f64;
        let samples = pixels + 2;
        let (tile_x, tile_y) = (origin_x - step * 0.5, origin_y - step * 0.5);
        let detail = if pixels as f64 / size >= GROUND_DETAIL_FROM_PIXELS_PER_WU {
            MapTileDetail::Ground
        } else {
            MapTileDetail::Overview
        };
        let tile = generate_map_tile(
            &self.macro_map,
            &self.river_courses,
            self.seed,
            detail,
            tile_x,
            tile_y,
            step * (samples - 1) as f64,
            step * (samples - 1) as f64,
            samples,
            samples,
        );
        let rendered = render_terrain(&tile, Some(&self.heights));

        // Height to shade. The tile's heights rest on the macro heightmap
        // sampled bilinearly, which is creased along every chunk edge and
        // would shade as square facets; rest them on a smooth sampling of it
        // instead. The local detail on top is kept.
        let macro_heights = &self.macro_map.heightmap;
        let relief_heights: Vec<f64> = (0..samples * samples)
            .map(|cell| {
                let wx = tile_x + (cell % samples) as f64 * step;
                let wy = tile_y + (cell / samples) as f64 * step;
                tile.heightmap[cell] - self.macro_map.sample_field_at(macro_heights, wx, wy)
                    + self.macro_map.sample_field_smooth_at(macro_heights, wx, wy)
            })
            .collect();
        let height = |column: usize, row: usize| relief_heights[row * samples + column];
        let exaggeration = (RELIEF_EXAGGERATION_AT_ONE_WU * step.powf(RELIEF_EXAGGERATION_FALLOFF))
            .max(RELIEF_MIN_EXAGGERATION);
        let slope_scale = TRUE_SLOPE_PER_HEIGHT_PER_WU * exaggeration / (2.0 * step);
        let light_length = LIGHT.iter().map(|part| part * part).sum::<f64>().sqrt();

        let mut rgba = Vec::with_capacity(pixels * pixels * 4);
        for row in 1..=pixels {
            for column in 1..=pixels {
                let cell = row * samples + column;
                // How brightly the ground is lit, as a multiple of flat ground.
                // Water lies flat.
                let shade = if mg_noise::biome_map::tile_has_fluid_surface(tile.biomes[cell]) {
                    1.0
                } else {
                    let slope_x = (height(column + 1, row) - height(column - 1, row)) * slope_scale;
                    let slope_y = (height(column, row + 1) - height(column, row - 1)) * slope_scale;
                    let normal_length = (slope_x * slope_x + slope_y * slope_y + 1.0).sqrt();
                    let lit = (-slope_x * LIGHT[0] - slope_y * LIGHT[1] + LIGHT[2])
                        / (normal_length * light_length);
                    (lit / (LIGHT[2] / light_length)).clamp(RELIEF_SHADE_RANGE.0, RELIEF_SHADE_RANGE.1)
                };
                let pixel = &rendered[cell * 4..cell * 4 + 4];
                rgba.extend(pixel[..3].iter().map(|&channel| (channel as f64 * shade).min(255.0) as u8));
                rgba.push(255);
            }
        }
        rgba
    }
}

/// Reserve `len` bytes in the module's memory for the page to write into.
#[no_mangle]
pub extern "C" fn mg_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Release memory from `mg_alloc`.
///
/// # Safety
/// `pointer` and `len` must be exactly what one `mg_alloc(len)` call returned
/// and was given, and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn mg_free(pointer: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(pointer, 0, len));
}

/// Load a macro pack (the bytes of a `.mgmacro` file). Returns 1 on success,
/// 0 if the bytes are not a readable pack.
///
/// # Safety
/// `pointer` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn mg_load_pack(pointer: *const u8, len: usize) -> u32 {
    let bytes = std::slice::from_raw_parts(pointer, len);
    match MacroPack::from_bytes(bytes) {
        Ok(pack) => {
            *WORLD.lock().expect("world lock") = Some(World::from_pack(&pack));
            1
        }
        Err(_) => 0,
    }
}

/// Render a tile (see `World::render_tile`). Returns a pointer to
/// `pixels * pixels * 4` RGBA bytes, valid until the next call, or null if no
/// pack is loaded.
#[no_mangle]
pub extern "C" fn mg_render_tile(
    origin_x: f64,
    origin_y: f64,
    size: f64,
    pixels: u32,
) -> *const u8 {
    let world = WORLD.lock().expect("world lock");
    let Some(world) = world.as_ref() else {
        return std::ptr::null();
    };
    let mut tile = TILE.lock().expect("tile lock");
    *tile = world.render_tile(origin_x, origin_y, size, pixels as usize);
    tile.as_ptr()
}
