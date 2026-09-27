//! 16×16×16 chunk sections: block states and biomes in paletted containers whose
//! in-memory layout matches the wire format, so encoding is a byte swap plus a header.

use bytes::{BufMut, BytesMut};
use kiln_data::block_props::randomly_ticks;
use kiln_data::blocks_types::{has_fluid, is_air};
use kiln_proto::WriteExt;

/// Index of a block within a section: y, then z, then x (x fastest), as on the wire.
#[inline]
pub fn block_index(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

/// Block states of one section.
#[derive(Clone)]
pub enum BlockContainer {
    Single(u16),
    /// Up to 16 states, 4-bit indices; entry `i` is nibble `i % 2` (low first) of byte `i / 2`,
    /// which is the little-endian image of the wire's longs.
    Nibble { palette: Vec<u16>, indices: Box<[u8; 2048]> },
    /// Up to 256 states, one byte per entry.
    Byte { palette: Vec<u16>, indices: Box<[u8; 4096]> },
    /// Global state ids, 16 bits per entry.
    Direct(Box<[u16; 4096]>),
}

impl BlockContainer {
    /// Builds the most compact container from a palette and per-block palette indices.
    /// Duplicate palette entries are allowed; indices must be in range.
    pub fn from_palette(palette: &[u16], indices: impl Fn(usize) -> usize) -> Self {
        let mut dedup: Vec<u16> = Vec::with_capacity(palette.len());
        let remap: Vec<u8> = palette
            .iter()
            .map(|s| match dedup.iter().position(|d| d == s) {
                Some(i) => i as u8,
                None => {
                    dedup.push(*s);
                    (dedup.len() - 1).min(255) as u8
                }
            })
            .collect();
        match dedup.len() {
            0 => Self::Single(0),
            1 => Self::Single(dedup[0]),
            2..=16 => {
                let mut n = Box::new([0u8; 2048]);
                for i in 0..4096 {
                    n[i >> 1] |= remap[indices(i)] << ((i & 1) * 4);
                }
                Self::Nibble { palette: dedup, indices: n }
            }
            17..=256 => {
                let mut b = Box::new([0u8; 4096]);
                for (i, e) in b.iter_mut().enumerate() {
                    *e = remap[indices(i)];
                }
                Self::Byte { palette: dedup, indices: b }
            }
            _ => {
                let mut d = Box::new([0u16; 4096]);
                for (i, e) in d.iter_mut().enumerate() {
                    *e = palette[indices(i)];
                }
                Self::Direct(d)
            }
        }
    }

    pub fn get(&self, i: usize) -> u16 {
        match self {
            Self::Single(s) => *s,
            Self::Nibble { palette, indices } => palette[((indices[i >> 1] >> ((i & 1) * 4)) & 0xf) as usize],
            Self::Byte { palette, indices } => palette[indices[i] as usize],
            Self::Direct(d) => d[i],
        }
    }

    /// Sets entry `i`, growing the representation when the palette is full.
    /// Returns the previous state.
    pub fn set(&mut self, i: usize, state: u16) -> u16 {
        let old = self.get(i);
        if old == state {
            return old;
        }
        loop {
            match self {
                Self::Single(s) => {
                    let base = *s;
                    *self = Self::Nibble { palette: vec![base], indices: Box::new([0; 2048]) };
                }
                Self::Nibble { palette, indices } => {
                    let p = match palette.iter().position(|&x| x == state) {
                        Some(p) => p,
                        None if palette.len() < 16 => {
                            palette.push(state);
                            palette.len() - 1
                        }
                        None => {
                            self.grow();
                            continue;
                        }
                    };
                    let shift = (i & 1) * 4;
                    let b = &mut indices[i >> 1];
                    *b = (*b & !(0xf << shift)) | ((p as u8) << shift);
                    return old;
                }
                Self::Byte { palette, indices } => {
                    let p = match palette.iter().position(|&x| x == state) {
                        Some(p) => p,
                        None if palette.len() < 256 => {
                            palette.push(state);
                            palette.len() - 1
                        }
                        None => {
                            self.grow();
                            continue;
                        }
                    };
                    indices[i] = p as u8;
                    return old;
                }
                Self::Direct(d) => {
                    d[i] = state;
                    return old;
                }
            }
        }
    }

    fn grow(&mut self) {
        let next = match &*self {
            Self::Nibble { palette, indices: nibbles } => {
                // Palette indices carry over unchanged; only their width grows.
                let mut indices = Box::new([0u8; 4096]);
                for (i, e) in indices.iter_mut().enumerate() {
                    *e = (nibbles[i >> 1] >> ((i & 1) * 4)) & 0xf;
                }
                Self::Byte { palette: palette.clone(), indices }
            }
            Self::Byte { .. } => {
                let mut d = Box::new([0u16; 4096]);
                for (i, e) in d.iter_mut().enumerate() {
                    *e = self.get(i);
                }
                Self::Direct(d)
            }
            Self::Single(_) | Self::Direct(_) => return,
        };
        *self = next;
    }

    /// Writes the paletted container in wire format.
    pub fn encode(&self, b: &mut BytesMut) {
        match self {
            Self::Single(s) => {
                b.put_u8(0);
                b.put_varint(*s as i32);
            }
            Self::Nibble { palette, indices } => {
                b.put_u8(4);
                put_palette(b, palette);
                put_le_longs(b, &indices[..]);
            }
            Self::Byte { palette, indices } => {
                b.put_u8(8);
                put_palette(b, palette);
                put_le_longs(b, &indices[..]);
            }
            Self::Direct(d) => {
                b.put_u8(16);
                for chunk in d.chunks_exact(4) {
                    let long = chunk.iter().enumerate().fold(0u64, |acc, (k, &v)| acc | ((v as u64) << (16 * k)));
                    b.put_u64(long);
                }
            }
        }
    }
}

fn put_palette(b: &mut BytesMut, palette: &[u16]) {
    b.put_varint(palette.len() as i32);
    for p in palette {
        b.put_varint(*p as i32);
    }
}

/// Our index arrays are the little-endian image of the wire's big-endian longs.
fn put_le_longs(b: &mut BytesMut, bytes: &[u8]) {
    for chunk in bytes.chunks_exact(8) {
        b.put_u64(u64::from_le_bytes(chunk.try_into().unwrap()));
    }
}

/// Biomes of one section (4×4×4 cells).
#[derive(Clone)]
pub enum Biomes {
    Single(u16),
    Cells(Box<[u16; 64]>),
}

impl Biomes {
    /// Encodes as the smallest valid container: single, indirect with 1-3 bits, or direct.
    pub fn encode(&self, b: &mut BytesMut, biome_count: usize) {
        match self {
            Self::Single(s) => {
                b.put_u8(0);
                b.put_varint(*s as i32);
            }
            Self::Cells(cells) => {
                let mut palette: Vec<u16> = Vec::new();
                for c in cells.iter() {
                    if !palette.contains(c) {
                        palette.push(*c);
                    }
                }
                if palette.len() == 1 {
                    b.put_u8(0);
                    b.put_varint(palette[0] as i32);
                    return;
                }
                let indirect_bits = bits_for(palette.len());
                let (bits, values): (u32, Vec<u64>) = if indirect_bits <= 3 {
                    b.put_u8(indirect_bits as u8);
                    put_palette(b, &palette);
                    (indirect_bits, cells.iter().map(|c| palette.iter().position(|p| p == c).unwrap() as u64).collect())
                } else {
                    let bits = bits_for(biome_count);
                    b.put_u8(bits as u8);
                    (bits, cells.iter().map(|&c| c as u64).collect())
                };
                for long in pack(&values, bits) {
                    b.put_u64(long);
                }
            }
        }
    }
}

/// Bits needed to index `n` distinct values (at least 1).
pub fn bits_for(n: usize) -> u32 {
    (usize::BITS - (n.max(2) - 1).leading_zeros()).max(1)
}

/// Packs values into longs: first entry in the low bits, no entry spans two longs.
pub fn pack(values: &[u64], bits: u32) -> Vec<u64> {
    let per_long = (64 / bits) as usize;
    let mut out = vec![0u64; values.len().div_ceil(per_long)];
    for (i, v) in values.iter().enumerate() {
        out[i / per_long] |= v << ((i % per_long) as u32 * bits);
    }
    out
}

#[derive(Clone)]
pub struct Section {
    pub blocks: BlockContainer,
    pub biomes: Biomes,
    non_air: u16,
    fluids: u16,
    /// Blocks that tick randomly (`LevelChunkSection.tickingBlockCount` plus fluids).
    ticking: u16,
}

impl Section {
    pub fn filled(state: u16, biome: u16) -> Self {
        Self::new(BlockContainer::Single(state), Biomes::Single(biome))
    }

    pub fn new(blocks: BlockContainer, biomes: Biomes) -> Self {
        let (non_air, fluids, ticking) = match &blocks {
            BlockContainer::Single(s) => {
                let all = |b: bool| if b { 4096 } else { 0 };
                (all(!is_air(*s)), all(has_fluid(*s)), all(randomly_ticks(*s)))
            }
            _ => (0..4096).fold((0u16, 0u16, 0u16), |(n, f, t), i| {
                let s = blocks.get(i);
                (n + !is_air(s) as u16, f + has_fluid(s) as u16, t + randomly_ticks(s) as u16)
            }),
        };
        Self { blocks, biomes, non_air, fluids, ticking }
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> u16 {
        self.blocks.get(block_index(x, y, z))
    }

    /// Returns the previous state.
    pub fn set(&mut self, x: usize, y: usize, z: usize, state: u16) -> u16 {
        let old = self.blocks.set(block_index(x, y, z), state);
        if old != state {
            self.non_air = self.non_air + !is_air(state) as u16 - !is_air(old) as u16;
            self.fluids = self.fluids + has_fluid(state) as u16 - has_fluid(old) as u16;
            self.ticking = self.ticking + randomly_ticks(state) as u16 - randomly_ticks(old) as u16;
        }
        old
    }

    pub fn is_empty(&self) -> bool {
        self.non_air == 0
    }

    /// Whether any block (or fluid) in the section ticks randomly
    /// (`LevelChunkSection.isRandomlyTicking`).
    pub fn is_randomly_ticking(&self) -> bool {
        self.ticking > 0
    }

    pub fn encode(&self, b: &mut BytesMut, biome_count: usize) {
        b.put_i16(self.non_air as i16);
        b.put_i16(self.fluids as i16);
        self.blocks.encode(b);
        self.biomes.encode(b, biome_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn grows_through_all_representations_and_keeps_values() {
        let mut c = BlockContainer::Single(d::AIR);
        let states: Vec<u16> = (0..300).map(|i| 1 + i as u16 * 7).collect();
        for (i, &s) in states.iter().enumerate() {
            c.set(i * 13 % 4096, s);
            if i == 10 {
                assert!(matches!(c, BlockContainer::Nibble { .. }));
            }
            if i == 100 {
                assert!(matches!(c, BlockContainer::Byte { .. }));
            }
        }
        assert!(matches!(c, BlockContainer::Direct(_)));
        for (i, &s) in states.iter().enumerate() {
            assert_eq!(c.get(i * 13 % 4096), s);
        }
        assert_eq!(c.get(1), d::AIR);
    }

    #[test]
    fn nibble_layout_matches_packing() {
        let mut c = BlockContainer::Single(d::AIR);
        for i in 0..4096 {
            c.set(i, [d::AIR, d::STONE, d::DIRT][i % 3]);
        }
        let mut wire = BytesMut::new();
        c.encode(&mut wire);
        let BlockContainer::Nibble { palette, .. } = &c else { panic!() };
        let idx: Vec<u64> = (0..4096).map(|i| palette.iter().position(|&p| p == c.get(i)).unwrap() as u64).collect();
        let mut expected = BytesMut::new();
        expected.put_u8(4);
        put_palette(&mut expected, palette);
        for l in pack(&idx, 4) {
            expected.put_u64(l);
        }
        assert_eq!(wire, expected);
    }

    #[test]
    fn section_counts() {
        let mut s = Section::filled(d::AIR, 0);
        s.set(0, 0, 0, d::STONE);
        s.set(1, 0, 0, d::WATER);
        assert_eq!((s.non_air, s.fluids), (2, 1));
        s.set(0, 0, 0, d::AIR);
        assert_eq!((s.non_air, s.fluids), (1, 1));
        // Still water does not tick randomly; grass does.
        assert!(!s.is_randomly_ticking());
        s.set(2, 0, 0, d::GRASS_BLOCK);
        assert!(s.is_randomly_ticking());
        s.set(2, 0, 0, d::DIRT);
        assert!(!s.is_randomly_ticking());
    }

    #[test]
    fn pack_matches_protocol_example() {
        let vals = [1u64, 2, 2, 3, 4, 4, 5, 6, 6, 4, 8, 0, 7, 4, 3, 13, 15, 16, 9, 14, 10, 12, 0, 2];
        assert_eq!(pack(&vals, 5), vec![0x0020863148418841, 0x01018A7260F68C87]);
        assert_eq!(bits_for(1), 1);
        assert_eq!(bits_for(2), 1);
        assert_eq!(bits_for(3), 2);
        assert_eq!(bits_for(67), 7);
        assert_eq!(bits_for(385), 9);
    }
}
