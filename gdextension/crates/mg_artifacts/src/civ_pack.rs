//! A civ pack: what LifeGen made of a world, in one compact file the game
//! can load (spec 011, spec 013 stage 6).
//!
//! The macro pack carries the land; this carries who lives on it: the
//! province of every chunk, the states, the settlements with their names,
//! the roads and where they cross rivers. It is made from a layers artifact
//! by `margins_grip export civ-pack` and loaded at startup beside the macro
//! pack. It is only valid for the terrain seed and civ seed it was made with.

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const MAGIC: &[u8; 6] = b"MGCP01";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CivPack {
    pub seed: u32,
    pub civ_seed: u32,
    /// Chunks across and down.
    pub width: u32,
    pub height: u32,
    /// Province id per chunk, 0 for none (sea). Indexed `y * width + x`.
    pub province_ids: Vec<u16>,
    /// Indexed by province id - 1.
    pub provinces: Vec<CivProvince>,
    /// Indexed by faction id - 1.
    pub factions: Vec<CivFaction>,
    pub settlements: Vec<CivSettlement>,
    pub roads: Vec<CivRoad>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CivProvince {
    pub name: String,
    /// Faction id, 0 for none.
    pub faction: u16,
    /// "claimed", "unclaimed" or "uninhabited".
    pub state: String,
    pub habitability: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CivFaction {
    pub name: String,
    pub capital_name: String,
    /// Whether the lore names this state, or LifeGen made it up.
    pub authored: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CivSettlement {
    pub name: String,
    /// Chunk it stands in.
    pub x: u16,
    pub y: u16,
    /// "Metropolis", "City", "Town", "Village", "Outpost" or "Ruins".
    pub size: String,
    pub province: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CivRoad {
    /// "Highway", "Road" or "Trail".
    pub kind: String,
    /// Chunks the road passes through, in order. Consecutive chunks may be
    /// on opposite edges of the map: the road crosses the seam.
    pub path: Vec<(u16, u16)>,
    /// Chunks where it crosses a river: a bridge or a ford at each.
    pub crossings: Vec<(u16, u16)>,
}

impl CivPack {
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let body = bincode::serialize(self).map_err(|e| format!("encoding civ pack: {e}"))?;
        let mut encoder = GzEncoder::new(MAGIC.to_vec(), Compression::default());
        encoder
            .write_all(&body)
            .and_then(|_| encoder.finish())
            .map_err(|e| format!("compressing civ pack: {e}"))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let compressed = bytes
            .strip_prefix(MAGIC)
            .ok_or_else(|| "not a civ pack (bad magic)".to_string())?;
        let mut body = Vec::new();
        GzDecoder::new(compressed)
            .read_to_end(&mut body)
            .map_err(|e| format!("decompressing civ pack: {e}"))?;
        bincode::deserialize(&body).map_err(|e| format!("decoding civ pack: {e}"))
    }

    /// The straight stretches of road that pass through a chunk, each as the
    /// two chunks it runs between (the far end unwrapped, so it may lie past
    /// the map's edge) and the road's kind. A road's path is simplified, so
    /// a stretch may cross many chunks.
    pub fn roads_through(&self, x: u16, y: u16) -> Vec<(&CivRoad, (i64, i64), (i64, i64))> {
        let width = self.width as i64;
        let mut found = Vec::new();
        for road in &self.roads {
            for pair in road.path.windows(2) {
                let (ax, ay) = (pair[0].0 as i64, pair[0].1 as i64);
                let mut bx = pair[1].0 as i64;
                let by = pair[1].1 as i64;
                // Go the short way round the seam.
                if bx - ax > width / 2 {
                    bx -= width;
                } else if ax - bx > width / 2 {
                    bx += width;
                }
                if line_crosses_cell((ax, ay), (bx, by), x as i64, y as i64, width) {
                    found.push((road, (ax, ay), (bx, by)));
                }
            }
        }
        found
    }

    pub fn province_at(&self, x: usize, y: usize) -> Option<&CivProvince> {
        if x >= self.width as usize || y >= self.height as usize {
            return None;
        }
        let id = self.province_ids[y * self.width as usize + x];
        (id != 0).then(|| &self.provinces[(id - 1) as usize])
    }

    /// The settlement nearest a chunk, with the distance to it in chunks,
    /// the short way round the map.
    pub fn nearest_settlement(&self, x: f64, y: f64) -> Option<(&CivSettlement, f64)> {
        let width = self.width as f64;
        self.settlements
            .iter()
            .map(|settlement| {
                let direct = (settlement.x as f64 + 0.5 - x).abs();
                let dx = direct.min(width - direct);
                let dy = settlement.y as f64 + 0.5 - y;
                (settlement, (dx * dx + dy * dy).sqrt())
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Whether the straight line between the centres of cells `a` and `b` passes
/// through cell `(x, y)`, on a grid `width` cells round. `b` may be past the
/// map's edge; the cell is tried at every lap.
fn line_crosses_cell(a: (i64, i64), b: (i64, i64), x: i64, y: i64, width: i64) -> bool {
    let (dx, dy) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
    let steps = dx.abs().max(dy.abs()).max(1.0);
    for lap in [-width, 0, width] {
        let (cx, cy) = (x + lap, y);
        // Does the line pass within half a cell of this cell's centre, along
        // its length? Sampling every half cell finds every cell it crosses.
        let count = (steps * 2.0) as i64;
        for step in 0..=count {
            let t = step as f64 / count as f64;
            let (px, py) = (a.0 as f64 + dx * t, a.1 as f64 + dy * t);
            if (px - cx as f64).abs() <= 0.5 && (py - cy as f64).abs() <= 0.5 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> CivPack {
        CivPack {
            seed: 42,
            civ_seed: 1,
            width: 4,
            height: 2,
            province_ids: vec![0, 1, 1, 2, 0, 1, 2, 2],
            provinces: vec![
                CivProvince { name: "Ashford".into(), faction: 1, state: "claimed".into(), habitability: 0.6 },
                CivProvince { name: "Rimwatch".into(), faction: 0, state: "unclaimed".into(), habitability: 0.3 },
            ],
            factions: vec![CivFaction { name: "Corazon".into(), capital_name: "Violetta".into(), authored: true }],
            settlements: vec![
                CivSettlement { name: "Violetta".into(), x: 1, y: 0, size: "Metropolis".into(), province: 1 },
                CivSettlement { name: "Rimwatch".into(), x: 3, y: 1, size: "Outpost".into(), province: 2 },
            ],
            roads: vec![CivRoad { kind: "Road".into(), path: vec![(1, 0), (2, 0), (3, 1)], crossings: vec![(2, 0)] }],
        }
    }

    #[test]
    fn a_civ_pack_survives_the_round_trip() {
        let original = pack();
        let bytes = original.to_bytes().unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert_eq!(CivPack::from_bytes(&bytes).unwrap(), original);
        assert!(CivPack::from_bytes(b"MGMP05junk").is_err());
    }

    #[test]
    fn roads_are_found_in_every_chunk_their_straight_stretches_cross() {
        let pack = pack();
        // The road's first stretch runs (1, 0) to (2, 0); its second (2, 0) to (3, 1).
        assert_eq!(pack.roads_through(1, 0).len(), 1);
        assert_eq!(pack.roads_through(2, 0).len(), 2);
        assert!(pack.roads_through(0, 1).is_empty());
        // A stretch across the seam is found at both ends.
        assert!(line_crosses_cell((3, 0), (4, 0), 0, 0, 4));
    }

    #[test]
    fn provinces_and_nearest_settlements_are_looked_up_by_chunk() {
        let pack = pack();
        assert_eq!(pack.province_at(1, 0).map(|p| p.name.as_str()), Some("Ashford"));
        assert!(pack.province_at(0, 0).is_none());
        // From chunk (0, 1) the outpost at (3, 1) is one chunk away round the seam.
        let (nearest, distance) = pack.nearest_settlement(0.5, 1.5).unwrap();
        assert_eq!(nearest.name, "Rimwatch");
        assert!((distance - 1.0).abs() < 1e-9);
    }
}
