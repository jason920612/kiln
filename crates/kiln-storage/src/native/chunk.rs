//! Native chunk records: a chunk's sections in Kiln's own container layout (numeric state and
//! biome ids, light as stored) and its other fields as chunk NBT.
//!
//! A record converts to and from the chunk NBT vanilla stores in Anvil without loss: sections
//! keep their key order and which light layers they had, and a section this layout cannot
//! reproduce exactly (unknown keys, a palette in another order, ...) is kept as NBT.
//! [`NativeChunk::from_nbt`] checks the reproduction byte for byte and declines otherwise
//! (the chunk is then stored as NBT).
//!
//! Payload layout (little-endian): version `u8`; the other fields as named NBT (`sections`
//! left an empty list in its place), length-prefixed; section count `u16`; per section a
//! form byte, then either length-prefixed NBT or `Y`, flags, key count, key codes and each
//! key's value (containers as in memory, light as zero/full/nibbles).

use crate::anvil::{self, AnvilSource, ChunkError, SectionData};
use crate::native::registry::Remap;
use kiln_proto::nbt::{self, Tag};
use kiln_world::chunk::{Chunk, Light};
use kiln_world::section::{Biomes, BlockContainer};
use kiln_world::{ChunkPos, Dimension};

const VERSION: u8 = 1;

const KEY_Y: u8 = 0;
const KEY_BLOCKS: u8 = 1;
const KEY_BIOMES: u8 = 2;
const KEY_SKY: u8 = 3;
const KEY_BLOCK_LIGHT: u8 = 4;
const KEY_NAMES: [&str; 5] = ["Y", "block_states", "biomes", "SkyLight", "BlockLight"];
/// `data` comes before `palette` in the section's `block_states` / `biomes`.
const BLOCKS_DATA_FIRST: u8 = 1;
const BIOMES_DATA_FIRST: u8 = 2;

pub enum NSection {
    Native(Box<Native>),
    /// Kept as stored.
    Nbt(Tag),
}

pub struct Native {
    pub y: i8,
    pub flags: u8,
    /// Key codes in stored order (`Y` included).
    pub keys: Vec<u8>,
    pub blocks: Option<BlockContainer>,
    pub biomes: Option<Biomes>,
    pub sky: Option<Light>,
    pub block: Option<Light>,
}

pub struct NativeChunk {
    /// Every field but `sections`, which is an empty list in its place.
    pub rest: Tag,
    pub sections: Vec<NSection>,
}

/// A chunk's section as saved: key codes, Y, blocks and biomes, sky and block light.
type SectionRefs<'a> = (Vec<u8>, i8, Option<&'a kiln_world::section::Section>, Option<&'a Light>, Option<&'a Light>);

/// A section as the writer sees it (borrowed from a chunk or a record).
struct View<'a> {
    y: i8,
    flags: u8,
    keys: &'a [u8],
    blocks: Option<&'a BlockContainer>,
    biomes: Option<&'a Biomes>,
    sky: Option<&'a Light>,
    block: Option<&'a Light>,
}

impl NativeChunk {
    /// A native record for chunk NBT that it reproduces byte for byte; `None` otherwise.
    pub fn from_nbt(data: &[u8], codec: &AnvilSource) -> Option<NativeChunk> {
        let (name, root) = nbt::read_named(data).ok()?;
        // The NBT must write back as it was read (a native section is only taken when it
        // gives back the very tag it came from, and the rest is kept as tags).
        let mut back = bytes::BytesMut::new();
        root.write_named("", &mut back);
        if !name.is_empty() || back[..] != *data {
            return None;
        }
        let Tag::Compound(mut fields) = root else { return None };
        let (_, sections) = fields.iter_mut().find(|(k, _)| k == "sections")?;
        let Tag::List(list) = std::mem::replace(sections, Tag::List(Vec::new())) else { return None };
        let sections = list.into_iter().map(|s| native_section(&s, codec).map_or(NSection::Nbt(s), |n| NSection::Native(Box::new(n)))).collect();
        Some(NativeChunk { rest: Tag::Compound(fields), sections })
    }

    /// The chunk NBT this record stands for.
    pub fn to_nbt(&self) -> Tag {
        let names = anvil::biome_names();
        let sections: Vec<Tag> = self
            .sections
            .iter()
            .map(|s| match s {
                NSection::Nbt(t) => t.clone(),
                NSection::Native(n) => Tag::Compound(
                    n.keys
                        .iter()
                        .map(|&k| {
                            let v = match k {
                                KEY_Y => Tag::Byte(n.y),
                                KEY_BLOCKS => order(anvil::encode_blocks(n.blocks.as_ref().unwrap()), n.flags & BLOCKS_DATA_FIRST != 0),
                                KEY_BIOMES => {
                                    order(anvil::encode_biomes(n.biomes.as_ref().unwrap(), names), n.flags & BIOMES_DATA_FIRST != 0)
                                }
                                KEY_SKY => anvil::light_tag(n.sky.as_ref().unwrap()),
                                _ => anvil::light_tag(n.block.as_ref().unwrap()),
                            };
                            (KEY_NAMES[k as usize].to_owned(), v)
                        })
                        .collect(),
                ),
            })
            .collect();
        let mut rest = self.rest.clone();
        crate::put(&mut rest, "sections", Tag::List(sections));
        rest
    }

    /// Encodes a chunk as a record directly (what [`anvil::encode_chunk`] would store).
    pub fn encode_chunk(pos: ChunkPos, chunk: &Chunk, preserved: Option<&Tag>) -> Vec<u8> {
        let rest = Tag::Compound(anvil::chunk_fields(pos, chunk, preserved));
        let views: Vec<SectionRefs> = anvil::chunk_sections(chunk)
            .map(|(y, sec, sky, block)| {
                let mut keys = vec![KEY_Y];
                if sec.is_some() {
                    keys.extend([KEY_BLOCKS, KEY_BIOMES]);
                }
                if sky.is_some() {
                    keys.push(KEY_SKY);
                }
                if block.is_some() {
                    keys.push(KEY_BLOCK_LIGHT);
                }
                (keys, y as i8, sec, sky, block)
            })
            .collect();
        encode(
            &rest,
            views.len(),
            views.iter().map(|(keys, y, sec, sky, block)| {
                Ok(View { y: *y, flags: 0, keys, blocks: sec.map(|s| &s.blocks), biomes: sec.map(|s| &s.biomes), sky: *sky, block: *block })
            }),
        )
    }

    pub fn encode(&self) -> Vec<u8> {
        encode(
            &self.rest,
            self.sections.len(),
            self.sections.iter().map(|s| match s {
                NSection::Nbt(t) => Err(t),
                NSection::Native(n) => Ok(View {
                    y: n.y,
                    flags: n.flags,
                    keys: &n.keys,
                    blocks: n.blocks.as_ref(),
                    biomes: n.biomes.as_ref(),
                    sky: n.sky.as_ref(),
                    block: n.block.as_ref(),
                }),
            }),
        )
    }

    pub fn decode(data: &[u8]) -> Option<NativeChunk> {
        let mut r = Reader(data);
        if r.u8()? != VERSION {
            return None;
        }
        let rest = r.nbt()?;
        let n = r.u16()? as usize;
        let mut sections = Vec::with_capacity(n);
        for _ in 0..n {
            if r.u8()? == 1 {
                sections.push(NSection::Nbt(r.nbt()?));
                continue;
            }
            let y = r.u8()? as i8;
            let flags = r.u8()?;
            let nk = r.u8()? as usize;
            let keys = r.take(nk)?.to_vec();
            let mut s = Native { y, flags, keys: keys.clone(), blocks: None, biomes: None, sky: None, block: None };
            for k in keys {
                match k {
                    KEY_Y => {}
                    KEY_BLOCKS => s.blocks = Some(r.blocks()?),
                    KEY_BIOMES => s.biomes = Some(r.biomes()?),
                    KEY_SKY => s.sky = Some(r.light()?),
                    KEY_BLOCK_LIGHT => s.block = Some(r.light()?),
                    _ => return None,
                }
            }
            sections.push(NSection::Native(Box::new(s)));
        }
        r.0.is_empty().then_some(NativeChunk { rest, sections })
    }

    /// Replaces ids from another build's table with this build's.
    pub fn remap(&mut self, m: &Remap) {
        for s in &mut self.sections {
            let NSection::Native(n) = s else { continue };
            if let Some(b) = &mut n.blocks {
                match b {
                    BlockContainer::Single(s) => *s = m.state(*s),
                    BlockContainer::Nibble { palette, .. } | BlockContainer::Byte { palette, .. } => {
                        palette.iter_mut().for_each(|s| *s = m.state(*s))
                    }
                    BlockContainer::Direct(d) => d.iter_mut().for_each(|s| *s = m.state(*s)),
                }
            }
            match &mut n.biomes {
                Some(Biomes::Single(b)) => *b = m.biome(*b),
                Some(Biomes::Cells(c)) => c.iter_mut().for_each(|b| *b = m.biome(*b)),
                None => {}
            }
        }
    }

    /// The chunk this record holds for `dim`, and the fields to keep for its next save (as
    /// [`AnvilSource`] keeps them).
    pub fn into_chunk(self, codec: &mut AnvilSource, dim: Dimension) -> Result<(Chunk, Tag), ChunkError> {
        codec.check_root(&self.rest)?;
        let light_on = self.rest.get("isLightOn").and_then(Tag::as_i64) == Some(1);
        let mut data = Vec::with_capacity(self.sections.len());
        for s in self.sections {
            data.push(match s {
                NSection::Nbt(t) => codec.section_data(&t, dim, light_on)?,
                NSection::Native(n) => {
                    let n = *n;
                    SectionData {
                        y: n.y as i32,
                        blocks: n.blocks,
                        biomes: n.biomes,
                        sky: n.sky.filter(|_| light_on),
                        block: n.block.filter(|_| light_on),
                    }
                }
            });
        }
        let chunk = codec.assemble(&self.rest, data, dim)?;
        let mut preserved = self.rest;
        crate::remove(&mut preserved, "sections");
        crate::remove(&mut preserved, "block_entities");
        Ok((chunk, preserved))
    }
}

/// `block_states` / `biomes` with `data` moved before `palette` when `data_first`.
fn order(tag: Tag, data_first: bool) -> Tag {
    match tag {
        Tag::Compound(mut f) if data_first => {
            f.reverse();
            Tag::Compound(f)
        }
        t => t,
    }
}

/// A section in native form if the native form writes it back identically.
fn native_section(s: &Tag, codec: &AnvilSource) -> Option<Native> {
    let Tag::Compound(fields) = s else { return None };
    let mut n = Native { y: 0, flags: 0, keys: Vec::with_capacity(fields.len()), blocks: None, biomes: None, sky: None, block: None };
    for (k, v) in fields {
        let code = KEY_NAMES.iter().position(|n| n == k)? as u8;
        if n.keys.contains(&code) {
            return None;
        }
        n.keys.push(code);
        match code {
            KEY_Y => match v {
                Tag::Byte(y) => n.y = *y,
                _ => return None,
            },
            KEY_BLOCKS => {
                let c = anvil::decode_blocks(v).ok()?;
                if data_first(v)? {
                    n.flags |= BLOCKS_DATA_FIRST;
                }
                if order(anvil::encode_blocks(&c), data_first(v)?) != *v {
                    return None;
                }
                n.blocks = Some(c);
            }
            KEY_BIOMES => {
                let b = codec.decode_biomes(v).ok()?;
                if data_first(v)? {
                    n.flags |= BIOMES_DATA_FIRST;
                }
                if order(anvil::encode_biomes(&b, anvil::biome_names()), data_first(v)?) != *v {
                    return None;
                }
                n.biomes = Some(b);
            }
            _ => {
                let Tag::ByteArray(bytes) = v else { return None };
                if bytes.len() != 2048 {
                    return None;
                }
                let l = anvil::light_layer(bytes);
                if code == KEY_SKY {
                    n.sky = Some(l);
                } else {
                    n.block = Some(l);
                }
            }
        }
    }
    n.keys.contains(&KEY_Y).then_some(n)
}

/// Whether a `palette`/`data` compound lists `data` first; `None` for other keys.
fn data_first(t: &Tag) -> Option<bool> {
    let Tag::Compound(f) = t else { return None };
    match f.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>()[..] {
        ["palette"] | ["palette", "data"] => Some(false),
        ["data", "palette"] => Some(true),
        _ => None,
    }
}

fn encode<'a>(rest: &Tag, count: usize, sections: impl Iterator<Item = Result<View<'a>, &'a Tag>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 * 1024);
    out.push(VERSION);
    put_nbt(&mut out, rest);
    out.extend_from_slice(&(count as u16).to_le_bytes());
    for s in sections {
        let v = match s {
            Err(t) => {
                out.push(1);
                put_nbt(&mut out, t);
                continue;
            }
            Ok(v) => v,
        };
        out.push(0);
        out.push(v.y as u8);
        out.push(v.flags);
        out.push(v.keys.len() as u8);
        out.extend_from_slice(v.keys);
        for &k in v.keys {
            match k {
                KEY_BLOCKS => put_blocks(&mut out, v.blocks.expect("section with blocks")),
                KEY_BIOMES => match v.biomes.expect("section with biomes") {
                    Biomes::Single(b) => {
                        out.push(0);
                        out.extend_from_slice(&b.to_le_bytes());
                    }
                    Biomes::Cells(c) => {
                        out.push(1);
                        c.iter().for_each(|b| out.extend_from_slice(&b.to_le_bytes()));
                    }
                },
                KEY_SKY => put_light(&mut out, v.sky.expect("sky light")),
                KEY_BLOCK_LIGHT => put_light(&mut out, v.block.expect("block light")),
                _ => {}
            }
        }
    }
    out
}

fn put_nbt(out: &mut Vec<u8>, t: &Tag) {
    let mut b = bytes::BytesMut::new();
    t.write_named("", &mut b);
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(&b);
}

fn put_blocks(out: &mut Vec<u8>, c: &BlockContainer) {
    let palette = |out: &mut Vec<u8>, p: &[u16]| {
        out.extend_from_slice(&(p.len() as u16).to_le_bytes());
        p.iter().for_each(|s| out.extend_from_slice(&s.to_le_bytes()));
    };
    match c {
        BlockContainer::Single(s) => {
            out.push(0);
            out.extend_from_slice(&s.to_le_bytes());
        }
        BlockContainer::Nibble { palette: p, indices } => {
            out.push(1);
            palette(out, p);
            out.extend_from_slice(&indices[..]);
        }
        BlockContainer::Byte { palette: p, indices } => {
            out.push(2);
            palette(out, p);
            out.extend_from_slice(&indices[..]);
        }
        BlockContainer::Direct(d) => {
            out.push(3);
            d.iter().for_each(|s| out.extend_from_slice(&s.to_le_bytes()));
        }
    }
}

fn put_light(out: &mut Vec<u8>, l: &Light) {
    match l {
        Light::Zero => out.push(0),
        Light::Full => out.push(1),
        Light::Nibbles(n) => {
            out.push(2);
            out.extend_from_slice(&n[..]);
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (h, t) = self.0.split_at_checked(n)?;
        self.0 = t;
        Some(h)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn u16s(&mut self, n: usize) -> Option<Vec<u16>> {
        Some(self.take(n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }

    fn nbt(&mut self) -> Option<Tag> {
        let len = u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize;
        nbt::read_named(self.take(len)?).ok().map(|(_, t)| t)
    }

    fn blocks(&mut self) -> Option<BlockContainer> {
        Some(match self.u8()? {
            0 => BlockContainer::Single(self.u16()?),
            1 => {
                let n = self.u16()? as usize;
                let palette = self.u16s(n)?;
                let indices: Box<[u8; 2048]> = self.take(2048)?.to_vec().into_boxed_slice().try_into().ok()?;
                // Every index must be in the palette.
                if !(2..=16).contains(&n) || indices.iter().any(|b| (b & 15) as usize >= n || (b >> 4) as usize >= n) {
                    return None;
                }
                BlockContainer::Nibble { palette, indices }
            }
            2 => {
                let n = self.u16()? as usize;
                let palette = self.u16s(n)?;
                let indices: Box<[u8; 4096]> = self.take(4096)?.to_vec().into_boxed_slice().try_into().ok()?;
                if !(17..=256).contains(&n) || indices.iter().any(|&b| b as usize >= n) {
                    return None;
                }
                BlockContainer::Byte { palette, indices }
            }
            3 => {
                let d: Box<[u16; 4096]> = self.u16s(4096)?.into_boxed_slice().try_into().ok()?;
                BlockContainer::Direct(d)
            }
            _ => return None,
        })
    }

    fn biomes(&mut self) -> Option<Biomes> {
        Some(match self.u8()? {
            0 => Biomes::Single(self.u16()?),
            1 => Biomes::Cells(self.u16s(64)?.into_boxed_slice().try_into().ok()?),
            _ => return None,
        })
    }

    fn light(&mut self) -> Option<Light> {
        Some(match self.u8()? {
            0 => Light::Zero,
            1 => Light::Full,
            2 => Light::Nibbles(self.take(2048)?.to_vec().into_boxed_slice().try_into().ok()?),
            _ => return None,
        })
    }
}
