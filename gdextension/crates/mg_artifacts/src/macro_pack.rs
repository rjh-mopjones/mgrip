//! Macro pack: the part of the macro world map the game needs at run time, in
//! one compact file that can ship inside the game.
//!
//! Runtime chunks are anchored to the macro map (ocean mask, heights, rivers).
//! Generating that map takes several seconds, so the game loads it from a pack
//! instead. A pack holds the layers anchoring and macro sampling read, as
//! 32-bit floats, plus the river courses, gzip-compressed.
//!
//! A pack is only valid for the seed and the generator code it was made with.
//! It carries a low-resolution probe of the terrain; `matches_generator`
//! regenerates the probe and compares, so a pack left over from older
//! generator code is detected and can be ignored.

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use mg_core::TileType;
use mg_noise::{generate_macro_probe, BiomeMap, RiverCourse};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const MAGIC: &[u8; 6] = b"MGMP02";
/// Largest probe difference still counted as the same generator. Allows for
/// maths library differences between native and web builds.
const PROBE_TOLERANCE: f32 = 1.0e-4;

#[derive(Serialize, Deserialize)]
pub struct MacroPack {
    pub seed: u32,
    probe: Vec<f32>,
    width: u32,
    height: u32,
    world_width: f64,
    world_height: f64,
    continentalness: Vec<f32>,
    tectonic: Vec<f32>,
    humidity: Vec<f32>,
    rock_hardness: Vec<f32>,
    peaks_valleys: Vec<f32>,
    heightmap: Vec<f32>,
    rivers: Vec<f32>,
    temperature: Vec<f32>,
    aridity: Vec<f32>,
    biomes: Vec<TileType>,
    river_courses: Vec<RiverCourse>,
}

fn narrowed(layer: &[f64]) -> Vec<f32> {
    layer.iter().map(|&value| value as f32).collect()
}

fn widened(layer: &[f32]) -> Vec<f64> {
    layer.iter().map(|&value| value as f64).collect()
}

impl MacroPack {
    /// Pack a macro map generated for `seed`.
    pub fn from_macro_map(seed: u32, map: &BiomeMap) -> Self {
        Self {
            seed,
            probe: generate_macro_probe(seed),
            width: map.width as u32,
            height: map.height as u32,
            world_width: map.world_width,
            world_height: map.world_height,
            continentalness: narrowed(&map.continentalness),
            tectonic: narrowed(&map.tectonic),
            humidity: narrowed(&map.humidity),
            rock_hardness: narrowed(&map.rock_hardness),
            peaks_valleys: narrowed(&map.peaks_valleys),
            heightmap: narrowed(&map.heightmap),
            rivers: narrowed(&map.rivers),
            temperature: narrowed(&map.temperature),
            aridity: narrowed(&map.aridity),
            biomes: map.biomes.clone(),
            river_courses: map
                .river_network
                .as_ref()
                .map(|network| network.courses.clone())
                .unwrap_or_default(),
        }
    }

    /// The world's river courses, which chunks draw their rivers from.
    pub fn river_courses(&self) -> &[RiverCourse] {
        &self.river_courses
    }

    /// The macro map as the runtime uses it. Only the packed layers are
    /// filled in; every other layer of the returned map is zero.
    pub fn to_biome_map(&self) -> BiomeMap {
        let mut map = BiomeMap::empty(
            self.width as usize,
            self.height as usize,
            self.world_width,
            self.world_height,
        );
        map.continentalness = widened(&self.continentalness);
        map.tectonic = widened(&self.tectonic);
        map.humidity = widened(&self.humidity);
        map.rock_hardness = widened(&self.rock_hardness);
        map.peaks_valleys = widened(&self.peaks_valleys);
        map.heightmap = widened(&self.heightmap);
        map.rivers = widened(&self.rivers);
        map.temperature = widened(&self.temperature);
        map.aridity = widened(&self.aridity);
        map.biomes = self.biomes.clone();
        map
    }

    /// True if the generator in this build still produces the terrain this
    /// pack was made from.
    pub fn matches_generator(&self) -> bool {
        let current = generate_macro_probe(self.seed);
        current.len() == self.probe.len()
            && current
                .iter()
                .zip(&self.probe)
                .all(|(now, packed)| (now - packed).abs() <= PROBE_TOLERANCE)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let body = bincode::serialize(self).map_err(|e| format!("encoding macro pack: {e}"))?;
        let mut encoder = GzEncoder::new(MAGIC.to_vec(), Compression::default());
        encoder
            .write_all(&body)
            .and_then(|_| encoder.finish())
            .map_err(|e| format!("compressing macro pack: {e}"))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let compressed = bytes
            .strip_prefix(MAGIC)
            .ok_or_else(|| "not a macro pack, or an unsupported version".to_string())?;
        let mut body = Vec::new();
        GzDecoder::new(compressed)
            .read_to_end(&mut body)
            .map_err(|e| format!("decompressing macro pack: {e}"))?;
        bincode::deserialize(&body).map_err(|e| format!("decoding macro pack: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u32 = 42;

    /// A small stand-in for the macro map: packing does not care about size.
    fn small_map() -> BiomeMap {
        BiomeMap::generate(SEED, 0.0, 0.0, 1024.0, 512.0, 64, 32, 0, false, false, 1.0)
    }

    #[test]
    fn a_pack_survives_being_written_and_read_back() {
        let map = small_map();
        let bytes = MacroPack::from_macro_map(SEED, &map).to_bytes().unwrap();

        let restored = MacroPack::from_bytes(&bytes).unwrap();
        let restored_map = restored.to_biome_map();

        assert_eq!(restored.seed, SEED);
        assert_eq!((restored_map.width, restored_map.height), (64, 32));
        assert_eq!(restored_map.biomes, map.biomes);
        for (packed, original) in restored_map.heightmap.iter().zip(&map.heightmap) {
            assert!((packed - original).abs() < 1.0e-6);
        }
    }

    #[test]
    fn a_fresh_pack_matches_the_generator_and_a_tampered_one_does_not() {
        let mut pack = MacroPack::from_macro_map(SEED, &small_map());
        assert!(pack.matches_generator());

        // As if the generator had changed since the pack was made.
        pack.probe[0] += 0.5;
        assert!(!pack.matches_generator());
    }

    #[test]
    fn bytes_that_are_not_a_pack_are_rejected() {
        assert!(MacroPack::from_bytes(b"not a pack").is_err());
        assert!(MacroPack::from_bytes(b"").is_err());
    }
}
