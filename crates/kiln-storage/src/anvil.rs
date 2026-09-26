//! Loading vanilla 26.x chunks (Anvil + chunk NBT) into the world representation.

use crate::region::RegionFile;
use kiln_data::blocks::default_state;
use kiln_data::blocks_types::block_by_name;
use kiln_proto::nbt::{self, Tag};
use kiln_world::block_entity::BlockEntity;
use kiln_world::chunk::{Chunk, Light};
use kiln_world::section::{BlockContainer, Biomes, Section};
use kiln_world::{ChunkPos, ChunkSource, Dimension};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

/// World data version this build reads without upgrading (26.3).
pub const DATA_VERSION: i64 = 5023;

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error(transparent)]
    Nbt(#[from] nbt::NbtError),
    #[error("missing or malformed field {0}")]
    Field(&'static str),
    #[error("chunk is not fully generated (status {0})")]
    NotFull(String),
}

/// Chunks from a dimension's `region/` directory.
pub struct AnvilSource {
    region_dir: PathBuf,
    regions: HashMap<(i32, i32), Option<RegionFile>>,
    biomes: HashMap<&'static str, u16>,
    default_biome: u16,
    warned_version: bool,
    /// Loaded chunks' NBT minus `sections` and `block_entities`: fields we do not model yet
    /// (structure references, scheduled ticks, ...) are written back unchanged.
    preserved: HashMap<ChunkPos, Tag>,
    /// Encoded chunks waiting for `flush`, per region.
    pending: HashMap<(i32, i32), Vec<(usize, usize, Vec<u8>)>>,
}

impl AnvilSource {
    pub fn new(region_dir: impl Into<PathBuf>) -> Self {
        let biomes = kiln_data::registries::SYNCHRONIZED
            .iter()
            .find(|(r, _)| *r == "minecraft:worldgen/biome")
            .map(|(_, e)| e.iter().enumerate().map(|(i, &n)| (n, i as u16)).collect())
            .unwrap_or_default();
        let default_biome = kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains").unwrap_or(0) as u16;
        Self {
            region_dir: region_dir.into(),
            regions: HashMap::new(),
            biomes,
            default_biome,
            warned_version: false,
            preserved: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    fn read_nbt(&mut self, pos: ChunkPos) -> Option<Vec<u8>> {
        let key = (pos.x >> 5, pos.z >> 5);
        // A save not flushed yet is newer than the region file.
        let local = ((pos.x & 31) as usize, (pos.z & 31) as usize);
        if let Some((_, _, payload)) = self.pending.get(&key).and_then(|p| p.iter().find(|(x, z, _)| (*x, *z) == local)) {
            return crate::region::decompress_chunk(payload).map_err(|e| warn!("chunk {pos:?}: {e}")).ok();
        }
        let dir = &self.region_dir;
        let region = self.regions.entry(key).or_insert_with(|| {
            let path = dir.join(format!("r.{}.{}.mca", key.0, key.1));
            path.exists().then(|| RegionFile::open(&path)).and_then(|r| {
                r.map_err(|e| warn!("cannot open {}: {e}", path.display())).ok()
            })
        });
        match region.as_mut()?.read((pos.x & 31) as usize, (pos.z & 31) as usize) {
            Ok(data) => data,
            Err(e) => {
                warn!("chunk {pos:?}: {e}");
                None
            }
        }
    }

    /// Parses chunk NBT into a chunk for `dim`.
    pub fn decode(&mut self, data: &[u8], dim: Dimension) -> Result<Chunk, ChunkError> {
        let (_, root) = nbt::read_named(data)?;
        let version = root.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0);
        if version != DATA_VERSION && !self.warned_version {
            warn!("world data version {version} differs from {DATA_VERSION}; upgrade old worlds with vanilla --forceUpgrade");
            self.warned_version = true;
        }
        let status = root.get("Status").or_else(|| root.get("status")).and_then(Tag::as_str).unwrap_or("");
        if status != "minecraft:full" {
            return Err(ChunkError::NotFull(status.to_owned()));
        }

        let n = (dim.height / 16) as usize;
        let min_section = dim.min_y >> 4;
        let mut sections = vec![Section::filled(default_state::AIR, self.default_biome); n];
        let mut sky: Vec<Option<Light>> = vec![None; n + 2];
        let mut block: Vec<Light> = vec![Light::Zero; n + 2];
        let light_on = root.get("isLightOn").and_then(Tag::as_i64) == Some(1);

        for s in root.get("sections").and_then(Tag::as_list).ok_or(ChunkError::Field("sections"))? {
            let y = s.get("Y").and_then(Tag::as_i64).ok_or(ChunkError::Field("sections.Y"))? as i32;
            let li = y - min_section + 1;
            if li < 0 || li as usize >= n + 2 {
                continue;
            }
            let li = li as usize;
            if light_on {
                if let Some(l) = s.get("SkyLight").and_then(Tag::as_byte_array) {
                    sky[li] = Some(light_layer(l));
                }
                if let Some(l) = s.get("BlockLight").and_then(Tag::as_byte_array) {
                    block[li] = light_layer(l);
                }
            }
            if li == 0 || li == n + 1 {
                continue; // light-only sections outside the build height
            }
            let blocks = match s.get("block_states") {
                Some(b) => decode_blocks(b)?,
                None => BlockContainer::Single(default_state::AIR),
            };
            let biomes = match s.get("biomes") {
                Some(b) => self.decode_biomes(b)?,
                None => Biomes::Single(self.default_biome),
            };
            sections[li - 1] = Section::new(blocks, biomes);
        }

        let sky = if light_on { Some(fill_missing_sky(sky)) } else { None };
        let mut chunk = Chunk::with_light(sections, dim.min_y, sky, light_on.then_some(block));
        chunk.set_light_trusted(light_on);
        let origin = match (root.get("xPos").and_then(Tag::as_i64), root.get("zPos").and_then(Tag::as_i64)) {
            (Some(x), Some(z)) => Some((x as i32, z as i32)),
            _ => None,
        };
        for entry in root.get("block_entities").and_then(Tag::as_list).unwrap_or(&[]) {
            load_block_entity(&mut chunk, entry, origin);
        }
        Ok(chunk)
    }

    fn decode_biomes(&self, tag: &Tag) -> Result<Biomes, ChunkError> {
        let palette: Vec<u16> = tag
            .get("palette")
            .and_then(Tag::as_list)
            .ok_or(ChunkError::Field("biomes.palette"))?
            .iter()
            .map(|t| {
                let name = t.unwrap_list_element().as_str().unwrap_or("");
                self.biomes.get(name).copied().unwrap_or(self.default_biome)
            })
            .collect();
        if palette.len() <= 1 {
            return Ok(Biomes::Single(palette.first().copied().unwrap_or(self.default_biome)));
        }
        let data = tag.get("data").and_then(Tag::as_long_array).ok_or(ChunkError::Field("biomes.data"))?;
        let idx = unpack(data, bits_for(palette.len()), 64).ok_or(ChunkError::Field("biomes.data length"))?;
        let mut cells = Box::new([0u16; 64]);
        for (c, i) in cells.iter_mut().zip(idx) {
            *c = *palette.get(i).ok_or(ChunkError::Field("biome index"))?;
        }
        Ok(Biomes::Cells(cells))
    }
}

impl ChunkSource for AnvilSource {
    fn load(&mut self, pos: ChunkPos, dim: Dimension) -> Option<Chunk> {
        let data = self.read_nbt(pos)?;
        match self.decode(&data, dim) {
            Ok(c) => {
                if let Ok((_, Tag::Compound(mut fields))) = nbt::read_named(&data) {
                    fields.retain(|(k, _)| k != "sections" && k != "block_entities");
                    self.preserved.insert(pos, Tag::Compound(fields));
                }
                Some(c)
            }
            Err(ChunkError::NotFull(_)) => None,
            Err(e) => {
                warn!("chunk {pos:?}: {e}");
                None
            }
        }
    }

    fn save(&mut self, pos: ChunkPos, chunk: &Chunk) {
        let root = encode_chunk(pos, chunk, self.preserved.get(&pos));
        let mut buf = bytes::BytesMut::new();
        root.write_named("", &mut buf);
        let payload = crate::region::compress_chunk(&buf);
        let entry = self.pending.entry((pos.x >> 5, pos.z >> 5)).or_default();
        let (lx, lz) = ((pos.x & 31) as usize, (pos.z & 31) as usize);
        entry.retain(|(x, z, _)| (*x, *z) != (lx, lz));
        entry.push((lx, lz, payload));
    }

    fn unloaded(&mut self, pos: ChunkPos) {
        self.preserved.remove(&pos);
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.region_dir)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        for ((rx, rz), updates) in std::mem::take(&mut self.pending) {
            let path = self.region_dir.join(format!("r.{rx}.{rz}.mca"));
            // Drop our read handle; the file is replaced and reopened on the next load.
            self.regions.remove(&(rx, rz));
            crate::region::write_region(&path, &updates, now).map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

fn set(fields: &mut Vec<(String, Tag)>, key: &str, value: Tag) {
    match fields.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value,
        None => fields.push((key.to_owned(), value)),
    }
}

/// Encodes a chunk as 26.3 chunk NBT, keeping `preserved` fields from the loaded chunk.
pub fn encode_chunk(pos: ChunkPos, chunk: &Chunk, preserved: Option<&Tag>) -> Tag {
    let mut fields = match preserved {
        Some(Tag::Compound(f)) => f.clone(),
        _ => vec![
            ("block_entities".into(), Tag::List(Vec::new())),
            ("block_ticks".into(), Tag::List(Vec::new())),
            ("fluid_ticks".into(), Tag::List(Vec::new())),
            ("InhabitedTime".into(), Tag::Long(0)),
            ("LastUpdate".into(), Tag::Long(0)),
            (
                "structures".into(),
                Tag::Compound(vec![
                    ("References".into(), Tag::Compound(Vec::new())),
                    ("starts".into(), Tag::Compound(Vec::new())),
                ]),
            ),
        ],
    };
    let min_section = chunk.min_y() >> 4;
    set(&mut fields, "DataVersion", Tag::Int(DATA_VERSION as i32));
    set(&mut fields, "xPos", Tag::Int(pos.x));
    set(&mut fields, "zPos", Tag::Int(pos.z));
    set(&mut fields, "yPos", Tag::Int(min_section));
    set(&mut fields, "Status", Tag::String("minecraft:full".into()));
    // Kiln keeps light up to date on every change, so complete light is saved (a chunk that
    // came without light goes back without it and vanilla relights it). Changed chunks go
    // back without heightmaps (vanilla recomputes missing ones on load).
    let light_valid = chunk.light_trusted();
    set(&mut fields, "isLightOn", Tag::Byte(light_valid as i8));
    if chunk.modified() {
        fields.retain(|(k, _)| k != "Heightmaps");
    }

    let biome_names = kiln_data::registries::SYNCHRONIZED
        .iter()
        .find(|(r, _)| *r == "minecraft:worldgen/biome")
        .map(|(_, e)| *e)
        .unwrap_or(&[]);
    let n = chunk.sections.len();
    let mut sections = Vec::with_capacity(n + 2);
    for li in 0..n + 2 {
        let mut s = vec![("Y".to_owned(), Tag::Byte((min_section + li as i32 - 1) as i8))];
        if (1..=n).contains(&li) {
            let sec = &chunk.sections[li - 1];
            s.push(("block_states".into(), encode_blocks(&sec.blocks)));
            s.push(("biomes".into(), encode_biomes(&sec.biomes, biome_names)));
        }
        if light_valid {
            let sky = &chunk.sky_light()[li];
            s.push(("SkyLight".into(), Tag::ByteArray(sky.to_bytes().iter().map(|&b| b as i8).collect())));
            let block = &chunk.block_light()[li];
            if !matches!(block, Light::Zero) {
                s.push(("BlockLight".into(), Tag::ByteArray(block.to_bytes().iter().map(|&b| b as i8).collect())));
            }
        }
        if s.len() > 1 {
            sections.push(Tag::Compound(s));
        }
    }
    set(&mut fields, "sections", Tag::List(sections));
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let block_entities = chunk.block_entities().map(|((x, y, z), be)| be.saved([bx + x as i32, y, bz + z as i32])).collect();
    set(&mut fields, "block_entities", Tag::List(block_entities));
    Tag::Compound(fields)
}

/// Adds a `block_entities` entry to the chunk if it belongs to the block at its position, as
/// vanilla does when it promotes the saved entries (others are dropped with a warning).
fn load_block_entity(chunk: &mut Chunk, entry: &Tag, origin: Option<(i32, i32)>) {
    let coord = |k| entry.get(k).and_then(Tag::as_i64).map(|v| v as i32);
    let (Some(x), Some(y), Some(z)) = (coord("x"), coord("y"), coord("z")) else {
        warn!("block entity without a position: {entry:?}");
        return;
    };
    if origin.is_some_and(|o| o != (x >> 4, z >> 4)) {
        warn!("block entity at {x},{y},{z} lies outside its chunk; dropped");
        return;
    }
    let Some(be) = BlockEntity::from_saved(entry.clone()) else {
        warn!("block entity of unknown type {:?} at {x},{y},{z}; dropped", entry.get("id"));
        return;
    };
    let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
    let state = chunk.get(lx, y, lz);
    if kiln_data::block_props::block_entity_type(state) != Some(be.kind) {
        let block = kiln_data::blocks_types::block_of(state).name;
        warn!("block entity {} at {x},{y},{z} does not match {block}; dropped", kiln_world::block_entity::type_name(be.kind));
        return;
    }
    chunk.load_block_entity(lx, y, lz, be);
}

/// A palette entry: the bare block id for a block's default state, else `{id, properties}`.
fn state_tag(state: u16) -> Tag {
    let block = kiln_data::blocks_types::block_of(state);
    if state == block.default || block.properties.is_empty() {
        return Tag::String(block.name.to_owned());
    }
    let props = block
        .properties
        .iter()
        .zip(block.property_indices(state))
        .map(|(p, i)| (p.name.to_owned(), Tag::String(p.values[i].to_owned())))
        .collect();
    Tag::Compound(vec![("id".into(), Tag::String(block.name.to_owned())), ("properties".into(), Tag::Compound(props))])
}

fn encode_blocks(c: &BlockContainer) -> Tag {
    let mut palette: Vec<u16> = Vec::new();
    let mut idx = vec![0usize; 4096];
    let mut lookup: HashMap<u16, usize> = HashMap::new();
    for (i, e) in idx.iter_mut().enumerate() {
        let s = c.get(i);
        *e = *lookup.entry(s).or_insert_with(|| {
            palette.push(s);
            palette.len() - 1
        });
    }
    let mut fields = vec![("palette".to_owned(), Tag::heterogeneous_list(palette.iter().map(|&s| state_tag(s)).collect()))];
    if palette.len() > 1 {
        let bits = bits_for(palette.len()).max(4);
        let packed = kiln_world::section::pack(&idx.iter().map(|&i| i as u64).collect::<Vec<_>>(), bits);
        fields.push(("data".into(), Tag::LongArray(packed.into_iter().map(|l| l as i64).collect())));
    }
    Tag::Compound(fields)
}

fn encode_biomes(b: &Biomes, names: &[&str]) -> Tag {
    let cells: Vec<u16> = match b {
        Biomes::Single(s) => vec![*s],
        Biomes::Cells(c) => c.to_vec(),
    };
    let mut palette: Vec<u16> = Vec::new();
    let idx: Vec<u64> = cells
        .iter()
        .map(|c| match palette.iter().position(|p| p == c) {
            Some(i) => i as u64,
            None => {
                palette.push(*c);
                (palette.len() - 1) as u64
            }
        })
        .collect();
    let name = |id: u16| Tag::String(names.get(id as usize).copied().unwrap_or("minecraft:plains").to_owned());
    let mut fields = vec![("palette".to_owned(), Tag::List(palette.iter().map(|&p| name(p)).collect()))];
    if palette.len() > 1 {
        let packed = kiln_world::section::pack(&idx, bits_for(palette.len()));
        fields.push(("data".into(), Tag::LongArray(packed.into_iter().map(|l| l as i64).collect())));
    }
    Tag::Compound(fields)
}

fn bits_for(n: usize) -> u32 {
    usize::BITS - (n - 1).leading_zeros()
}

/// Unpacks `count` entries of `bits` each (no entry spans two longs).
fn unpack(data: &[i64], bits: u32, count: usize) -> Option<Vec<usize>> {
    let per = (64 / bits) as usize;
    if data.len() != count.div_ceil(per) {
        return None;
    }
    let mask = (1u64 << bits) - 1;
    Some((0..count).map(|i| ((data[i / per] as u64 >> ((i % per) as u32 * bits)) & mask) as usize).collect())
}

/// A block state from a palette entry: a bare block id for the default state (26.x), or a
/// compound with the id and properties (`id`/`properties`, or the older `Name`/`Properties`).
fn palette_state(entry: &Tag) -> Option<u16> {
    let entry = entry.unwrap_list_element();
    if let Some(name) = entry.as_str() {
        return Some(block_by_name(name)?.default);
    }
    let name = entry.get("id").or_else(|| entry.get("Name"))?.as_str()?;
    let block = block_by_name(name)?;
    let mut state = block.default;
    if let Some(Tag::Compound(props)) = entry.get("properties").or_else(|| entry.get("Properties")) {
        for (k, v) in props {
            if let Some(v) = v.as_str() {
                state = block.with_property(state, k, v).unwrap_or(state);
            }
        }
    }
    Some(state)
}

fn decode_blocks(tag: &Tag) -> Result<BlockContainer, ChunkError> {
    let palette: Vec<u16> = tag
        .get("palette")
        .and_then(Tag::as_list)
        .ok_or(ChunkError::Field("block_states.palette"))?
        .iter()
        .map(|e| {
            palette_state(e).unwrap_or_else(|| {
                warn!("unknown block state {e:?}; using air");
                default_state::AIR
            })
        })
        .collect();
    if palette.len() <= 1 {
        return Ok(BlockContainer::Single(palette.first().copied().unwrap_or(default_state::AIR)));
    }
    let data = tag.get("data").and_then(Tag::as_long_array).ok_or(ChunkError::Field("block_states.data"))?;
    let bits = bits_for(palette.len()).max(4);
    let idx = unpack(data, bits, 4096).ok_or(ChunkError::Field("block_states.data length"))?;
    if idx.iter().any(|&i| i >= palette.len()) {
        return Err(ChunkError::Field("block palette index"));
    }
    Ok(BlockContainer::from_palette(&palette, |i| idx[i]))
}

fn light_layer(bytes: &[i8]) -> Light {
    if bytes.len() != 2048 {
        return Light::Zero;
    }
    if bytes.iter().all(|&b| b == 0) {
        return Light::Zero;
    }
    if bytes.iter().all(|&b| b == -1) {
        return Light::Full;
    }
    let mut n = Box::new([0u8; 2048]);
    for (d, s) in n.iter_mut().zip(bytes) {
        *d = *s as u8;
    }
    Light::Nibbles(n)
}

/// Vanilla omits sky light layers that carry no information: above the topmost stored layer
/// the sky is fully lit, and a missing layer below takes each column's value from the bottom
/// row of the nearest stored layer above it.
fn fill_missing_sky(mut sky: Vec<Option<Light>>) -> Vec<Light> {
    let mut above: Option<Light> = None;
    for li in (0..sky.len()).rev() {
        match &sky[li] {
            Some(l) => above = Some(l.clone()),
            None => {
                let derived = match &above {
                    None => Light::Full,
                    Some(l) => bottom_row_extended(l),
                };
                sky[li] = Some(derived.clone());
                above = Some(derived);
            }
        }
    }
    sky.into_iter().map(|l| l.unwrap_or(Light::Full)).collect()
}

fn bottom_row_extended(l: &Light) -> Light {
    match l {
        Light::Zero => Light::Zero,
        Light::Full => Light::Full,
        Light::Nibbles(n) => {
            let mut out = Box::new([0u8; 2048]);
            // The bottom row (y = 0) is the first 256 entries = 128 bytes; repeat it 16 times.
            for y in 0..16 {
                out[y * 128..y * 128 + 128].copy_from_slice(&n[..128]);
            }
            light_or_uniform(out)
        }
    }
}

fn light_or_uniform(n: Box<[u8; 2048]>) -> Light {
    if n.iter().all(|&b| b == 0) {
        Light::Zero
    } else if n.iter().all(|&b| b == 0xff) {
        Light::Full
    } else {
        Light::Nibbles(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_entries_in_26_3_forms() {
        let stone = block_by_name("minecraft:stone").unwrap().default;
        assert_eq!(palette_state(&Tag::String("minecraft:stone".into())), Some(stone));
        let wrapped = Tag::Compound(vec![(String::new(), Tag::String("minecraft:stone".into()))]);
        assert_eq!(palette_state(&wrapped), Some(stone));
        let fence = Tag::Compound(vec![
            ("id".into(), Tag::String("minecraft:oak_fence".into())),
            ("properties".into(), Tag::Compound(vec![("north".into(), Tag::String("true".into()))])),
        ]);
        let s = palette_state(&fence).unwrap();
        assert_eq!(block_by_name("minecraft:oak_fence").unwrap().property(s, "north"), Some("true"));
    }

    #[test]
    fn unpack_respects_long_boundaries() {
        // 5 bits: 12 entries per long.
        let data = [0x0020863148418841u64 as i64, 0x01018A7260F68C87u64 as i64];
        let v = unpack(&data, 5, 24).unwrap();
        assert_eq!(&v[..6], &[1, 2, 2, 3, 4, 4]);
        assert!(unpack(&data, 5, 36).is_none());
    }
}
