//! World storage. For now: a superflat overworld whose chunk packet body is built once.

use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::blocks::default_state as block;
use kiln_proto::WriteExt;

pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;
pub const SECTIONS: usize = (HEIGHT / 16) as usize;

/// Layers from the bottom of the world up (superflat "classic").
const LAYERS: [u16; 4] = [block::BEDROCK, block::DIRT, block::DIRT, block::GRASS_BLOCK];

/// Y coordinate a player stands at on top of the flat terrain.
pub const SURFACE_Y: f64 = (MIN_Y + LAYERS.len() as i32) as f64;

pub struct FlatWorld {
    /// Chunk Data body after the chunk coordinates; identical for every flat chunk.
    chunk_body: Bytes,
}

impl FlatWorld {
    pub fn new(plains_biome: i32) -> Self {
        Self { chunk_body: build_chunk_body(plains_biome) }
    }

    pub fn chunk_body(&self, _x: i32, _z: i32) -> &Bytes {
        &self.chunk_body
    }
}

/// Packs `values` (each < 2^bits) into longs the way paletted containers and heightmaps do:
/// first entry in the least significant bits, no entry spanning two longs.
fn pack(values: impl ExactSizeIterator<Item = u64>, bits: u32) -> Vec<i64> {
    let per_long = (64 / bits) as usize;
    let mut out = vec![0i64; values.len().div_ceil(per_long)];
    for (i, v) in values.enumerate() {
        out[i / per_long] |= (v << ((i % per_long) as u32 * bits)) as i64;
    }
    out
}

fn put_longs(b: &mut BytesMut, longs: &[i64]) {
    for l in longs {
        b.put_i64(*l);
    }
}

fn build_chunk_body(plains_biome: i32) -> Bytes {
    let mut b = BytesMut::with_capacity(64 * 1024);

    // Heightmaps: highest occupied block + 1, relative to MIN_Y.
    let height = LAYERS.len() as u64;
    let bits = u64::BITS - (HEIGHT as u64).leading_zeros(); // ceil(log2(HEIGHT + 1))
    let heights = pack(std::iter::repeat_n(height, 256), bits);
    const WORLD_SURFACE: i32 = 1;
    const MOTION_BLOCKING: i32 = 4;
    const MOTION_BLOCKING_NO_LEAVES: i32 = 5;
    b.put_varint(3);
    for kind in [WORLD_SURFACE, MOTION_BLOCKING, MOTION_BLOCKING_NO_LEAVES] {
        b.put_varint(kind);
        b.put_varint(heights.len() as i32);
        put_longs(&mut b, &heights);
    }

    // Sections.
    let mut data = BytesMut::new();
    for section in 0..SECTIONS {
        if section == 0 {
            // Indirect palette: [layer blocks..., air], 4 bits per entry.
            let mut palette: Vec<u16> = Vec::new();
            for l in LAYERS {
                if !palette.contains(&l) {
                    palette.push(l);
                }
            }
            palette.push(block::AIR);
            let index_of = |s: u16| palette.iter().position(|&p| p == s).unwrap() as u64;
            let entries = (0..4096).map(|i| {
                let y = i / 256;
                index_of(LAYERS.get(y).copied().unwrap_or(block::AIR))
            });
            data.put_i16((LAYERS.len() * 256) as i16); // non-air blocks
            data.put_i16(0); // fluids
            data.put_u8(4);
            data.put_varint(palette.len() as i32);
            for p in &palette {
                data.put_varint(*p as i32);
            }
            put_longs(&mut data, &pack(entries, 4));
        } else {
            data.put_i16(0);
            data.put_i16(0);
            data.put_u8(0); // single value
            data.put_varint(block::AIR as i32);
        }
        data.put_u8(0); // biomes: single value
        data.put_varint(plains_biome);
    }
    b.put_varint(data.len() as i32);
    b.put_slice(&data);

    b.put_varint(0); // block entities

    // Light: sections -1..=SECTIONS (bit 0 is the section below the world).
    let all: u64 = (1u64 << (SECTIONS + 2)) - 1;
    let sky_mask = all & !1; // world sections and the one above
    b.put_bitset(&[sky_mask]);
    b.put_bitset(&[0]); // block light mask
    b.put_bitset(&[1]); // empty sky light: below the world
    b.put_bitset(&[all]); // empty block light: everywhere
    b.put_varint(sky_mask.count_ones() as i32);
    for bit in 1..SECTIONS + 2 {
        b.put_varint(2048);
        if bit == 1 {
            // Bottom world section: dark inside the solid layers, full sky light above.
            let mut arr = [0xffu8; 2048];
            arr[..LAYERS.len() * 128].fill(0);
            b.put_slice(&arr);
        } else {
            b.put_bytes(0xff, 2048);
        }
    }
    b.put_varint(0); // block light arrays
    b.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_matches_protocol_example() {
        // 5 bits per entry example from the chunk format documentation.
        let vals = [1u64, 2, 2, 3, 4, 4, 5, 6, 6, 4, 8, 0, 7, 4, 3, 13, 15, 16, 9, 14, 10, 12, 0, 2];
        let longs = pack(vals.iter().copied(), 5);
        assert_eq!(longs, vec![0x0020863148418841, 0x01018A7260F68C87]);
    }

    #[test]
    fn heightmap_bits_for_384_blocks() {
        assert_eq!(u64::BITS - (HEIGHT as u64).leading_zeros(), 9);
    }
}
