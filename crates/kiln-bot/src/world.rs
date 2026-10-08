//! The blocks a survival bot knows about: chunk columns decoded from Level Chunk packets and
//! kept up to date by Block Update and Section Blocks Update. Sections keep the packed form the
//! server sent; only sections the bot (or the server, near the bot) changes are unpacked.

use kiln_data::block_props;
use kiln_proto::{DecodeError, Reader};
use std::collections::HashMap;

/// Overworld: y -64..320.
pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;
const SECTIONS: usize = (HEIGHT / 16) as usize;

/// Cached ids of the blocks the bots care about.
pub struct Ids {
    pub water: (u16, u16),
    pub lava: (u16, u16),
}

pub fn ids() -> &'static Ids {
    static IDS: std::sync::OnceLock<Ids> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        let range = |name| {
            let b = kiln_data::blocks_types::block_by_name(name).expect("block");
            (b.first, b.last)
        };
        Ids { water: range("minecraft:water"), lava: range("minecraft:lava") }
    })
}

pub fn is_water(s: u16) -> bool {
    let (a, b) = ids().water;
    (a..=b).contains(&s) || (kiln_data::blocks_types::has_fluid(s) && !is_lava(s) && fluid_water_logged(s))
}

fn fluid_water_logged(s: u16) -> bool {
    kiln_data::blocks_types::block_of(s).property(s, "waterlogged") == Some("true")
        || matches!(
            kiln_data::blocks_types::block_of(s).name,
            "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass"
        )
}

pub fn is_lava(s: u16) -> bool {
    let (a, b) = ids().lava;
    (a..=b).contains(&s)
}

pub fn name(s: u16) -> &'static str {
    kiln_data::blocks_types::block_of(s).name
}

pub fn is_air(s: u16) -> bool {
    kiln_data::blocks_types::is_air(s)
}

/// A block with any collision box.
pub fn is_solid(s: u16) -> bool {
    !block_props::collision(s).is_empty()
}

/// One 16x16x16 section of block states.
#[derive(Clone)]
enum Section {
    Single(u16),
    /// `palette` is empty for direct (global) ids.
    Packed { bits: u8, palette: Box<[u16]>, data: Box<[u64]> },
    Direct(Box<[u16; 4096]>),
}

impl Section {
    fn get(&self, idx: usize) -> u16 {
        match self {
            Section::Single(s) => *s,
            Section::Packed { bits, palette, data } => {
                let bits = *bits as usize;
                let per_long = 64 / bits;
                let v = ((data[idx / per_long] >> ((idx % per_long) * bits)) & ((1u64 << bits) - 1)) as usize;
                if palette.is_empty() { v as u16 } else { palette.get(v).copied().unwrap_or(0) }
            }
            Section::Direct(d) => d[idx],
        }
    }

    fn set(&mut self, idx: usize, state: u16) {
        if let Section::Direct(d) = self {
            d[idx] = state;
            return;
        }
        if self.get(idx) == state {
            return;
        }
        let mut d = Box::new([0u16; 4096]);
        for (i, e) in d.iter_mut().enumerate() {
            *e = self.get(i);
        }
        d[idx] = state;
        *self = Section::Direct(d);
    }
}

fn skip_container(r: &mut Reader, biomes: bool) -> Result<(), DecodeError> {
    let bits = r.u8()? as usize;
    let max_indirect = if biomes { 3 } else { 8 };
    let entries = if biomes { 64usize } else { 4096 };
    match bits {
        0 => {
            r.varint()?;
            return Ok(());
        }
        1..=8 if bits <= max_indirect => {
            let n = r.varint()?;
            for _ in 0..n {
                r.varint()?;
            }
        }
        _ => {}
    }
    let per_long = 64 / bits;
    r.bytes(entries.div_ceil(per_long) * 8)?;
    Ok(())
}

fn read_blocks(r: &mut Reader) -> Result<Section, DecodeError> {
    let bits = r.u8()?;
    if bits == 0 {
        return Ok(Section::Single(r.varint()? as u16));
    }
    let palette: Box<[u16]> = if bits <= 8 {
        let n = r.varint()?;
        if !(1..=4096).contains(&n) {
            return Err(DecodeError::Invalid("bad palette size"));
        }
        let mut p = Vec::with_capacity(n as usize);
        for _ in 0..n {
            p.push(r.varint()? as u16);
        }
        p.into()
    } else {
        Box::new([])
    };
    let per_long = 64 / bits as usize;
    let raw = r.bytes(4096usize.div_ceil(per_long) * 8)?;
    let data: Box<[u64]> = raw.chunks_exact(8).map(|c| u64::from_be_bytes(c.try_into().unwrap())).collect();
    Ok(Section::Packed { bits, palette, data })
}

/// A chunk column.
#[derive(Clone)]
pub struct Column {
    sections: Vec<Section>,
}

impl Column {
    /// Reads the part of a Level Chunk packet body (after the packet id) that holds block
    /// states: `x`, `z`, heightmaps, then the section data. Block entities and light are ignored.
    pub fn parse(body: &[u8]) -> Result<(i32, i32, Column), DecodeError> {
        let mut r = Reader::new(body);
        let (x, z) = (r.i32()?, r.i32()?);
        let maps = r.varint()?;
        for _ in 0..maps {
            r.varint()?;
            let n = r.len()?;
            r.bytes(n.checked_mul(8).ok_or(DecodeError::Invalid("heightmap"))?)?;
        }
        let size = r.len()?;
        let mut data = Reader::new(r.bytes(size)?);
        let mut sections = Vec::with_capacity(SECTIONS);
        while data.remaining() > 0 && sections.len() < SECTIONS {
            let _non_air = data.i16()?;
            let _fluids = data.i16()?;
            sections.push(read_blocks(&mut data)?);
            skip_container(&mut data, true)?;
        }
        if sections.len() != SECTIONS {
            return Err(DecodeError::Invalid("chunk has the wrong number of sections"));
        }
        Ok((x, z, Column { sections }))
    }

    fn index(x: i32, y: i32, z: i32) -> (usize, usize) {
        let s = ((y - MIN_Y) >> 4) as usize;
        (s, (((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize)
    }
}

#[derive(Default)]
pub struct World {
    cols: HashMap<(i32, i32), Column>,
}

impl World {
    pub fn insert(&mut self, x: i32, z: i32, c: Column) {
        self.cols.insert((x, z), c);
    }

    pub fn remove(&mut self, x: i32, z: i32) {
        self.cols.remove(&(x, z));
    }

    pub fn clear(&mut self) {
        self.cols.clear();
    }

    pub fn has_chunk(&self, cx: i32, cz: i32) -> bool {
        self.cols.contains_key(&(cx, cz))
    }

    pub fn len(&self) -> usize {
        self.cols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }

    /// The block state at a position; `None` for chunks not received. Above and below the
    /// world it is air.
    pub fn get(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        if !(MIN_Y..MIN_Y + HEIGHT).contains(&y) {
            return Some(0);
        }
        let c = self.cols.get(&(x >> 4, z >> 4))?;
        let (s, i) = Column::index(x, y, z);
        Some(c.sections[s].get(i))
    }

    /// Like [`World::get`], with unknown chunks counting as stone (the bot never walks into them).
    pub fn block(&self, x: i32, y: i32, z: i32) -> u16 {
        self.get(x, y, z).unwrap_or(1)
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, state: u16) {
        if !(MIN_Y..MIN_Y + HEIGHT).contains(&y) {
            return;
        }
        if let Some(c) = self.cols.get_mut(&(x >> 4, z >> 4)) {
            let (s, i) = Column::index(x, y, z);
            c.sections[s].set(i, state);
        }
    }

    /// Topmost y whose block has collision or holds a fluid (grass, flowers and snow layers are
    /// no ground), searching down from `from`.
    pub fn surface_y(&self, x: i32, z: i32, from: i32) -> Option<i32> {
        let mut y = from;
        loop {
            y = self.top_block_y(x, z, y)?;
            let s = self.get(x, y, z)?;
            if is_solid(s) || kiln_data::blocks_types::has_fluid(s) {
                return Some(y);
            }
            y -= 1;
        }
    }

    /// Topmost y with a non-air block in the column, searching down from `from`.
    pub fn top_block_y(&self, x: i32, z: i32, from: i32) -> Option<i32> {
        let c = self.cols.get(&(x >> 4, z >> 4))?;
        let mut y = from.min(MIN_Y + HEIGHT - 1);
        while y >= MIN_Y {
            let (s, i) = Column::index(x, y, z);
            if let Section::Single(v) = &c.sections[s] {
                if is_air(*v) {
                    y = (y & !15) - 1;
                    continue;
                }
                return Some(y);
            }
            if !is_air(c.sections[s].get(i)) {
                return Some(y);
            }
            y -= 1;
        }
        None
    }
}

/// A world of stone below y=0 over chunks -2..2, built from chunk packets in the server's layout.
#[cfg(test)]
pub(crate) fn test_world() -> World {
    use bytes::{BufMut, BytesMut};
    use kiln_data::blocks::default_state as d;
    use kiln_proto::WriteExt;
    let mut w = World::default();
    for cx in -2..2 {
        for cz in -2..2 {
            let mut b = BytesMut::new();
            b.put_i32(cx);
            b.put_i32(cz);
            b.put_varint(0);
            let mut data = BytesMut::new();
            for s in 0..SECTIONS {
                data.put_i16(0);
                data.put_i16(0);
                data.put_u8(0);
                data.put_varint(if s < 4 { d::STONE } else { d::AIR } as i32);
                data.put_u8(0);
                data.put_varint(1);
            }
            b.put_varint(data.len() as i32);
            b.put_slice(&data);
            let (x, z, c) = Column::parse(&b).unwrap();
            w.insert(x, z, c);
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};
    use kiln_data::blocks::default_state as d;
    use kiln_proto::WriteExt;

    /// A chunk in the layout kiln-world sends: heightmaps, 24 sections (one holding stone, one
    /// with a 4-bit palette), no block entities, no light.
    fn packet(x: i32, z: i32) -> BytesMut {
        let mut b = BytesMut::new();
        b.put_i32(x);
        b.put_i32(z);
        b.put_varint(1);
        b.put_varint(4);
        b.put_varint(2);
        b.put_u64(0);
        b.put_u64(0);
        let mut data = BytesMut::new();
        for s in 0..SECTIONS {
            data.put_i16(0);
            data.put_i16(0);
            match s {
                1 => {
                    data.put_u8(0);
                    data.put_varint(d::STONE as i32);
                }
                2 => {
                    // 4 bits: palette [air, dirt, stone]; block 5 is dirt, block 4095 stone.
                    data.put_u8(4);
                    data.put_varint(3);
                    for p in [d::AIR, d::DIRT, d::STONE] {
                        data.put_varint(p as i32);
                    }
                    let mut longs = vec![0u64; 256];
                    longs[0] |= 1 << (5 * 4);
                    longs[255] |= 2 << (15 * 4);
                    for l in longs {
                        data.put_u64(l);
                    }
                }
                _ => {
                    data.put_u8(0);
                    data.put_varint(d::AIR as i32);
                }
            }
            // biomes: single, or a 1-bit palette of two with 4 longs (64 cells need one long)
            if s == 3 {
                data.put_u8(1);
                data.put_varint(2);
                data.put_varint(1);
                data.put_varint(2);
                data.put_u64(0);
            } else {
                data.put_u8(0);
                data.put_varint(1);
            }
        }
        b.put_varint(data.len() as i32);
        b.put_slice(&data);
        b.put_varint(0); // block entities
        b.put_slice(&[0xaa; 20]); // light, ignored
        b
    }

    #[test]
    fn reads_blocks_from_the_servers_chunk_layout() {
        let (x, z, col) = Column::parse(&packet(3, -2)).unwrap();
        assert_eq!((x, z), (3, -2));
        let mut w = World::default();
        w.insert(x, z, col);
        let (bx, bz) = (3 * 16, -2 * 16);
        assert_eq!(w.get(bx + 1, MIN_Y + 16 + 3, bz + 2), Some(d::STONE));
        // section 2 starts at y = -64 + 32 = -32; index 5 is x=5,y=0,z=0
        assert_eq!(w.get(bx + 5, -32, bz), Some(d::DIRT));
        assert_eq!(w.get(bx + 15, -32 + 15, bz + 15), Some(d::STONE));
        assert_eq!(w.get(bx, -32, bz), Some(d::AIR));
        assert_eq!(w.get(bx, 100, bz), Some(d::AIR));
        assert_eq!(w.get(0, 0, 0), None);
        assert_eq!(w.top_block_y(bx + 1, bz + 2, 300), Some(MIN_Y + 31));
        w.set(bx + 1, 100, bz + 2, d::COBBLESTONE);
        assert_eq!(w.get(bx + 1, 100, bz + 2), Some(d::COBBLESTONE));
        assert_eq!(w.top_block_y(bx + 1, bz + 2, 300), Some(100));
        w.set(bx + 5, -32, bz, d::AIR);
        assert_eq!(w.get(bx + 5, -32, bz), Some(d::AIR));
        assert_eq!(w.get(bx + 15, -32 + 15, bz + 15), Some(d::STONE));
    }

    #[test]
    fn rejects_truncated_chunks() {
        let p = packet(0, 0);
        assert!(Column::parse(&p[..p.len() / 2]).is_err());
    }

    #[test]
    fn reads_chunks_encoded_by_the_server() {
        use kiln_world::{Blocks, ChunkPos, OVERWORLD};
        let mut server = kiln_world::World::flat(OVERWORLD, 0, 67);
        let p = ChunkPos::new(5, -7);
        let (bx, bz) = (5 * 16, -7 * 16);
        server.set_block(bx + 3, 70, bz + 4, d::COBBLESTONE);
        server.set_block(bx + 4, -60, bz + 4, d::DIAMOND_ORE);
        for i in 0..40u16 {
            server.set_block(bx + 8, 20 + i as i32, bz + 8, d::STONE + i % 7);
        }
        let mut body = BytesMut::new();
        body.put_i32(p.x);
        body.put_i32(p.z);
        body.put_slice(&server.chunk_body(p));
        let (x, z, col) = Column::parse(&body).unwrap();
        assert_eq!((x, z), (5, -7));
        let mut w = World::default();
        w.insert(x, z, col);
        for (dx, y, dz) in [(3, 70, 4), (4, -60, 4), (0, 0, 0), (1, -64, 1), (2, 100, 2), (8, 21, 8), (8, 59, 8)] {
            assert_eq!(w.get(bx + dx, y, bz + dz), server.get_block(bx + dx, y, bz + dz), "{dx} {y} {dz}");
        }
    }
}
