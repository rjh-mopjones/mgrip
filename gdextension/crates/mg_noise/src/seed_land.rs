//! The seed: the flat map's land, which the sphere is grown from (spec
//! 017). Written by main's generator with `export seed-land`; read here,
//! never generated here.
//!
//! The file is the magic `MGSEED01`, the map's width, height and world
//! seed as little-endian u32, then the layers in `LAYERS` order, one
//! little-endian f32 per cell, row-major.

use crate::biome_map::{sample_field_bilinear, WORLD_HEIGHT, WORLD_WIDTH};

const MAGIC: &[u8; 8] = b"MGSEED02";
/// The layers a seed file holds, in order.
pub const LAYERS: [&str; 7] = [
    "continentalness",
    "tectonic",
    "tectonic_plate_ids",
    "rock_hardness",
    "peaks_valleys",
    "heightmap",
    "light_level",
];

pub struct SeedLand {
    pub width: usize,
    pub height: usize,
    pub seed: u32,
    pub continentalness: Vec<f64>,
    pub tectonic: Vec<f64>,
    pub tectonic_plate_ids: Vec<f64>,
    pub rock_hardness: Vec<f64>,
    pub peaks_valleys: Vec<f64>,
    pub heightmap: Vec<f64>,
    /// The flat map's light: a flat distance from its bottom centre, which
    /// is what put the terminus where it is.
    pub light_level: Vec<f64>,
}

impl SeedLand {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let body = bytes
            .strip_prefix(MAGIC)
            .ok_or_else(|| "not a seed land file, or an unsupported version".to_string())?;
        if body.len() < 12 {
            return Err("seed land file is truncated".into());
        }
        let word = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().expect("four bytes"));
        let (width, height, seed) = (word(0) as usize, word(4) as usize, word(8));
        let cells = width * height;
        let expected = 12 + LAYERS.len() * cells * 4;
        if body.len() != expected {
            return Err(format!(
                "seed land file holds {} bytes, expected {expected} for {width}x{height}",
                body.len()
            ));
        }
        let layer = |which: usize| -> Vec<f64> {
            let start = 12 + which * cells * 4;
            body[start..start + cells * 4]
                .chunks_exact(4)
                .map(|value| f32::from_le_bytes(value.try_into().expect("four bytes")) as f64)
                .collect()
        };
        Ok(Self {
            width,
            height,
            seed,
            continentalness: layer(0),
            tectonic: layer(1),
            tectonic_plate_ids: layer(2),
            rock_hardness: layer(3),
            peaks_valleys: layer(4),
            heightmap: layer(5),
            light_level: layer(6),
        })
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        Self::from_bytes(&bytes)
    }

    /// A layer at a world position, between cells in straight lines; the
    /// map joins east to west.
    pub fn sample(&self, field: &[f64], wx: f64, wy: f64) -> f64 {
        sample_field_bilinear(
            field,
            wx,
            wy,
            WORLD_WIDTH,
            WORLD_HEIGHT,
            self.width,
            self.height,
        )
    }

    /// A layer's value in the cell under a world position, for layers that
    /// are labels rather than quantities.
    pub fn nearest(&self, field: &[f64], wx: f64, wy: f64) -> f64 {
        let x = (wx / WORLD_WIDTH * self.width as f64).floor() as i64;
        let y = (wy / WORLD_HEIGHT * self.height as f64).floor() as i64;
        let x = x.rem_euclid(self.width as i64) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        field[y * self.width + x]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_file_is_read_back_layer_by_layer() {
        let (width, height) = (4usize, 2usize);
        let mut bytes = MAGIC.to_vec();
        bytes.extend((width as u32).to_le_bytes());
        bytes.extend((height as u32).to_le_bytes());
        bytes.extend(7u32.to_le_bytes());
        for layer in 0..LAYERS.len() {
            for cell in 0..width * height {
                bytes.extend(((layer * 10 + cell) as f32).to_le_bytes());
            }
        }
        let seed = SeedLand::from_bytes(&bytes).unwrap();
        assert_eq!((seed.width, seed.height, seed.seed), (4, 2, 7));
        assert_eq!(seed.continentalness[3], 3.0);
        assert_eq!(seed.heightmap[5], 55.0);
        assert_eq!(seed.light_level[0], 60.0);
        assert!(SeedLand::from_bytes(&bytes[..20]).is_err());
        assert!(SeedLand::from_bytes(b"junk").is_err());
    }
}
