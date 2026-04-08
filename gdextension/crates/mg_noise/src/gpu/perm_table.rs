//! Permutation table generation — must match the `noise` crate's OpenSimplex exactly
//! so GPU and CPU output are bit-identical (f32 precision aside).

use rand::{seq::SliceRandom, SeedableRng};
use rand_xorshift::XorShiftRng;

const TABLE_SIZE: usize = 256;

/// Generate a permutation table identical to `noise` crate's `PermutationTable::new()`.
pub fn generate_permutation_table(seed: u32) -> [u8; TABLE_SIZE] {
    let mut real = [0u8; 16];
    real[0] = 1;
    for i in 1..4 {
        real[i * 4] = seed as u8;
        real[i * 4 + 1] = (seed >> 8) as u8;
        real[i * 4 + 2] = (seed >> 16) as u8;
        real[i * 4 + 3] = (seed >> 24) as u8;
    }
    let mut rng: XorShiftRng = SeedableRng::from_seed(real);
    let mut table = [0u8; TABLE_SIZE];
    for (i, v) in table.iter_mut().enumerate() {
        *v = i as u8;
    }
    table.shuffle(&mut rng);
    table
}

/// Pad each u8 to u32 for simpler GPU buffer access.
pub fn permutation_table_to_u32(table: &[u8; TABLE_SIZE]) -> [u32; TABLE_SIZE] {
    let mut result = [0u32; TABLE_SIZE];
    for (i, &v) in table.iter().enumerate() {
        result[i] = v as u32;
    }
    result
}
