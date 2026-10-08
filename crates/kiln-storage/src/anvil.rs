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

/// A section of stored chunk data, decoded: its light layers as stored (`None` when absent)
/// and, inside the build height, its blocks and biomes (`None` when absent).
pub(crate) struct SectionData {
    pub y: i32,
    pub blocks: Option<BlockContainer>,
    pub biomes: Option<Biomes>,
    pub sky: Option<Light>,
    pub block: Option<Light>,
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
    pending: HashMap<(i32, i32), Updates>,
    pub stats: crate::native::LoadStats,
    /// Threads reading, encoding and writing, once [`ChunkSource::set_background`] asked.
    background: Option<Background>,
    /// The dimension's shape, for chunks loaded in the background.
    pub dimension: Dimension,
}

use crate::region::{BackgroundWriter, REGION_WRITES, Updates, write_now};

/// The background side of an [`AnvilSource`]: loads on two reader threads (each with its own
/// [`AnvilSource`] for decoding), chunk encoding on an encoder thread, and region writes on a
/// writer thread. What the tick thread may need again stays on its side until written: the
/// chunks being encoded (`encoding`, waited for if read), the encoded ones (`pending`) and the
/// ones being written (`writing`).
struct Background {
    load_tx: std::sync::mpsc::Sender<(ChunkPos, Option<std::sync::Arc<[u8]>>)>,
    loaded_rx: std::sync::mpsc::Receiver<(ChunkPos, Option<(Chunk, Option<Tag>)>, std::time::Duration)>,
    encode_tx: std::sync::mpsc::Sender<(ChunkPos, Box<Chunk>, Option<Tag>)>,
    encoded_rx: std::sync::mpsc::Receiver<(ChunkPos, std::sync::Arc<[u8]>)>,
    encoding: std::collections::HashSet<ChunkPos>,
    writer: BackgroundWriter,
}

impl Background {
    fn start(region_dir: PathBuf, dimension: Dimension) -> Background {
        let (load_tx, load_rx) = std::sync::mpsc::channel::<(ChunkPos, Option<std::sync::Arc<[u8]>>)>();
        let load_rx = std::sync::Arc::new(std::sync::Mutex::new(load_rx));
        let (loaded_tx, loaded_rx) = std::sync::mpsc::channel();
        for i in 0..2 {
            let (rx, tx, dir) = (load_rx.clone(), loaded_tx.clone(), region_dir.clone());
            std::thread::Builder::new()
                .name(format!("kiln-load-{i}"))
                .spawn(move || {
                    let mut reader = AnvilSource::new(dir);
                    reader.dimension = dimension;
                    let mut seen_writes = REGION_WRITES.load(std::sync::atomic::Ordering::Acquire);
                    loop {
                        let next = rx.lock().unwrap().recv();
                        let Ok((pos, payload)) = next else { break };
                        // Region files rewritten since: open them again.
                        let writes = REGION_WRITES.load(std::sync::atomic::Ordering::Acquire);
                        if writes != seen_writes {
                            reader.regions.clear();
                            seen_writes = writes;
                        }
                        let start = std::time::Instant::now();
                        // A save not written yet comes with the job.
                        if let Some(p) = payload {
                            reader.queue(pos, p);
                        }
                        let chunk = reader.load_in(pos, dimension);
                        reader.pending.clear();
                        let kept = reader.preserved.remove(&pos);
                        if tx.send((pos, chunk.map(|c| (c, kept)), start.elapsed())).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawning a chunk loading thread");
        }
        // Encoders: a world save hands over thousands of chunks at once. A chunk is never with
        // two of them (a newer copy waits for the older one), so their order does not matter.
        let (encode_tx, encode_rx) = std::sync::mpsc::channel::<(ChunkPos, Box<Chunk>, Option<Tag>)>();
        let encode_rx = std::sync::Arc::new(std::sync::Mutex::new(encode_rx));
        let (encoded_tx, encoded_rx) = std::sync::mpsc::channel();
        for i in 0..3 {
            let (rx, tx) = (encode_rx.clone(), encoded_tx.clone());
            std::thread::Builder::new()
                .name(format!("kiln-encode-{i}"))
                .spawn(move || {
                    loop {
                        let next = rx.lock().unwrap().recv();
                        let Ok((pos, chunk, preserved)) = next else { break };
                        if tx.send((pos, encode_payload(pos, &chunk, preserved.as_ref()).into())).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawning a chunk encoding thread");
        }
        Background { load_tx, loaded_rx, encode_tx, encoded_rx, encoding: Default::default(), writer: BackgroundWriter::start("kiln-write") }
    }
}

/// A chunk's region file payload: 26.3 chunk NBT, compressed.
fn encode_payload(pos: ChunkPos, chunk: &Chunk, preserved: Option<&Tag>) -> Vec<u8> {
    let root = encode_chunk(pos, chunk, preserved);
    let mut buf = bytes::BytesMut::new();
    root.write_named("", &mut buf);
    crate::region::compress_chunk(&buf)
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
            stats: crate::native::LoadStats::new("anvil chunk storage"),
            background: None,
            dimension: kiln_world::OVERWORLD,
        }
    }

    fn read_nbt(&mut self, pos: ChunkPos) -> Option<Vec<u8>> {
        let key = (pos.x >> 5, pos.z >> 5);
        // A save not flushed yet is newer than the region file.
        let local = ((pos.x & 31) as usize, (pos.z & 31) as usize);
        if let Some((_, _, payload)) = self.pending.get(&key).and_then(|p| p.iter().find(|(x, z, _)| (*x, *z) == local)) {
            return crate::region::decompress_chunk(payload).map_err(|e| warn!("chunk {pos:?}: {e}")).ok();
        }
        // Handed to the writer and not written yet: the newest batch holding it.
        if let Some(payload) = self.background.as_ref().and_then(|bg| bg.writer.find(key, local)) {
            return crate::region::decompress_chunk(payload).map_err(|e| warn!("chunk {pos:?}: {e}")).ok();
        }
        let (dir, stats) = (&self.region_dir, &mut self.stats);
        let region = self.regions.entry(key).or_insert_with(|| {
            let start = std::time::Instant::now();
            let path = dir.join(format!("r.{}.{}.mca", key.0, key.1));
            let r = path.exists().then(|| RegionFile::open(&path)).and_then(|r| {
                r.map_err(|e| warn!("cannot open {}: {e}", path.display())).ok()
            });
            stats.record_open(start);
            r
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
        self.check_root(&root)?;
        let light_on = root.get("isLightOn").and_then(Tag::as_i64) == Some(1);
        let mut sections = Vec::new();
        for s in root.get("sections").and_then(Tag::as_list).ok_or(ChunkError::Field("sections"))? {
            sections.push(self.section_data(s, dim, light_on)?);
        }
        self.assemble(&root, sections, dim)
    }

    /// Warns once about another data version; fails unless the chunk is fully generated.
    pub(crate) fn check_root(&mut self, root: &Tag) -> Result<(), ChunkError> {
        let version = root.get("DataVersion").and_then(Tag::as_i64).unwrap_or(0);
        if version != DATA_VERSION && !self.warned_version {
            warn!("world data version {version} differs from {DATA_VERSION}; upgrade old worlds with vanilla --forceUpgrade");
            self.warned_version = true;
        }
        let status = root.get("Status").or_else(|| root.get("status")).and_then(Tag::as_str).unwrap_or("");
        if status != "minecraft:full" {
            return Err(ChunkError::NotFull(status.to_owned()));
        }
        Ok(())
    }

    /// One `sections` entry: light layers (when the chunk's light is on) and, inside the build
    /// height, blocks and biomes.
    pub(crate) fn section_data(&self, s: &Tag, dim: Dimension, light_on: bool) -> Result<SectionData, ChunkError> {
        let y = s.get("Y").and_then(Tag::as_i64).ok_or(ChunkError::Field("sections.Y"))? as i32;
        let mut d = SectionData { y, blocks: None, biomes: None, sky: None, block: None };
        if light_on {
            d.sky = s.get("SkyLight").and_then(Tag::as_byte_array).map(light_layer);
            d.block = s.get("BlockLight").and_then(Tag::as_byte_array).map(light_layer);
        }
        let li = y - (dim.min_y >> 4) + 1;
        if li >= 1 && li <= dim.height / 16 {
            d.blocks = s.get("block_states").map(decode_blocks).transpose()?;
            d.biomes = s.get("biomes").map(|b| self.decode_biomes(b)).transpose()?;
        }
        Ok(d)
    }

    /// A chunk from its sections and the other fields of its NBT (`isLightOn`, `xPos`/`zPos`,
    /// `block_entities`, `block_ticks`, `fluid_ticks`).
    pub(crate) fn assemble(&self, root: &Tag, data: Vec<SectionData>, dim: Dimension) -> Result<Chunk, ChunkError> {
        let n = (dim.height / 16) as usize;
        let min_section = dim.min_y >> 4;
        let mut sections = vec![Section::filled(default_state::AIR, self.default_biome); n];
        let mut sky: Vec<Option<Light>> = vec![None; n + 2];
        let mut block: Vec<Light> = vec![Light::Zero; n + 2];
        let light_on = root.get("isLightOn").and_then(Tag::as_i64) == Some(1);

        for s in data {
            let li = s.y - min_section + 1;
            if li < 0 || li as usize >= n + 2 {
                continue;
            }
            let li = li as usize;
            if light_on {
                if let Some(l) = s.sky {
                    sky[li] = Some(l);
                }
                if let Some(l) = s.block {
                    block[li] = l;
                }
            }
            if li == 0 || li == n + 1 {
                continue; // light-only sections outside the build height
            }
            let blocks = s.blocks.unwrap_or(BlockContainer::Single(default_state::AIR));
            let biomes = s.biomes.unwrap_or(Biomes::Single(self.default_biome));
            sections[li - 1] = Section::new(blocks, biomes);
        }

        let sky = if light_on { Some(fill_missing_sky(sky)) } else { None };
        let mut chunk = Chunk::with_light(sections, dim.min_y, sky, light_on.then_some(block));
        chunk.set_light_trusted(light_on);
        chunk.set_inhabited_time(root.get("InhabitedTime").and_then(Tag::as_i64).unwrap_or(0));
        let origin = match (root.get("xPos").and_then(Tag::as_i64), root.get("zPos").and_then(Tag::as_i64)) {
            (Some(x), Some(z)) => Some((x as i32, z as i32)),
            _ => None,
        };
        for entry in root.get("block_entities").and_then(Tag::as_list).unwrap_or(&[]) {
            load_block_entity(&mut chunk, entry, origin);
        }
        let ticks = |key| root.get(key).filter(|t| t.as_list().is_some_and(|l| !l.is_empty())).cloned();
        let (block_ticks, fluid_ticks) = (ticks("block_ticks"), ticks("fluid_ticks"));
        if block_ticks.is_some() || fluid_ticks.is_some() {
            let empty = || Tag::List(Vec::new());
            chunk.saved_ticks = Some(Box::new(kiln_world::chunk::SavedTicks {
                block: block_ticks.unwrap_or_else(empty),
                fluid: fluid_ticks.unwrap_or_else(empty),
            }));
        }
        Ok(chunk)
    }

    pub(crate) fn decode_biomes(&self, tag: &Tag) -> Result<Biomes, ChunkError> {
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

impl AnvilSource {
    /// The chunk at `pos` from the pending saves or the region file.
    fn load_in(&mut self, pos: ChunkPos, dim: Dimension) -> Option<Chunk> {
        let start = std::time::Instant::now();
        let data = self.read_nbt(pos)?;
        // One parse for the chunk and the fields kept for writing it back.
        let decoded = nbt::read_named(&data).map_err(ChunkError::from).and_then(|(_, root)| {
            self.check_root(&root)?;
            let light_on = root.get("isLightOn").and_then(Tag::as_i64) == Some(1);
            let mut sections = Vec::new();
            for s in root.get("sections").and_then(Tag::as_list).ok_or(ChunkError::Field("sections"))? {
                sections.push(self.section_data(s, dim, light_on)?);
            }
            let chunk = self.assemble(&root, sections, dim)?;
            Ok((chunk, root))
        });
        if decoded.is_ok() {
            self.stats.record(start);
        }
        match decoded {
            Ok((mut c, root)) => {
                if let Tag::Compound(mut fields) = root {
                    // Structure starts and references stay readable (location predicates).
                    if let Some((_, s)) = fields.iter().find(|(k, _)| k == "structures") {
                        c.structures = Some(Box::new(s.clone()));
                    }
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

    /// Queues an encoded chunk for the next flush (replacing an older copy).
    fn queue(&mut self, pos: ChunkPos, payload: std::sync::Arc<[u8]>) {
        let entry = self.pending.entry((pos.x >> 5, pos.z >> 5)).or_default();
        let (lx, lz) = ((pos.x & 31) as usize, (pos.z & 31) as usize);
        entry.retain(|(x, z, _)| (*x, *z) != (lx, lz));
        entry.push((lx, lz, payload));
    }

    /// Takes in what the background threads finished: encoded chunks (into `pending`) and
    /// written regions. With `wait_for`, blocks until that chunk's encoding is back; with
    /// `all`, until nothing is being encoded.
    fn collect(&mut self, wait_for: Option<ChunkPos>, all: bool) {
        loop {
            let Some(bg) = self.background.as_mut() else { return };
            let waiting = (all && !bg.encoding.is_empty()) || wait_for.is_some_and(|p| bg.encoding.contains(&p));
            let next = if waiting { bg.encoded_rx.recv().ok() } else { bg.encoded_rx.try_recv().ok() };
            let Some((pos, payload)) = next else { break };
            bg.encoding.remove(&pos);
            self.queue(pos, payload);
        }
        let Some(bg) = self.background.as_mut() else { return };
        // Written regions are new files: their read handles go.
        for key in bg.writer.written() {
            self.regions.remove(&key);
        }
    }


}

impl ChunkSource for AnvilSource {
    fn load(&mut self, pos: ChunkPos, dim: Dimension) -> Option<Chunk> {
        self.collect(Some(pos), false);
        self.load_in(pos, dim)
    }

    fn save(&mut self, pos: ChunkPos, chunk: &Chunk) {
        let payload = encode_payload(pos, chunk, self.preserved.get(&pos));
        // An older copy may still be with the encoder: it must not land after this one.
        self.collect(Some(pos), false);
        self.queue(pos, payload.into());
    }

    fn save_many(&mut self, chunks: &[(ChunkPos, &Chunk)]) {
        // Older copies still with the encoder land first.
        self.collect(None, true);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8);
        let per = chunks.len().div_ceil(threads).max(1);
        let preserved = &self.preserved;
        let payloads: Vec<Vec<Vec<u8>>> = std::thread::scope(|scope| {
            let workers: Vec<_> = chunks
                .chunks(per)
                .map(|part| scope.spawn(move || part.iter().map(|&(pos, chunk)| encode_payload(pos, chunk, preserved.get(&pos))).collect::<Vec<_>>()))
                .collect();
            workers.into_iter().map(|w| w.join().expect("encoding chunks")).collect()
        });
        for (&(pos, _), payload) in chunks.iter().zip(payloads.into_iter().flatten()) {
            self.queue(pos, payload.into());
        }
    }

    fn save_owned(&mut self, pos: ChunkPos, chunk: Chunk) {
        if self.background.is_none() {
            return self.save(pos, &chunk);
        }
        self.collect(Some(pos), false);
        let preserved = self.preserved.get(&pos).cloned();
        let bg = self.background.as_mut().unwrap();
        bg.encoding.insert(pos);
        let _ = bg.encode_tx.send((pos, Box::new(chunk), preserved));
    }

    fn unloaded(&mut self, pos: ChunkPos) {
        self.preserved.remove(&pos);
    }

    fn set_background(&mut self, on: bool) {
        if on && self.background.is_none() {
            self.background = Some(Background::start(self.region_dir.clone(), self.dimension));
        }
    }

    fn start_load(&mut self, pos: ChunkPos, dim: Dimension) -> kiln_world::StartLoad {
        use kiln_world::StartLoad;
        if self.background.is_none() || (dim.min_y, dim.height) != (self.dimension.min_y, self.dimension.height) {
            return StartLoad::Unsupported;
        }
        // A copy still with the encoder is waited for (it is newer than anything stored).
        self.collect(Some(pos), false);
        let key = (pos.x >> 5, pos.z >> 5);
        let local = ((pos.x & 31) as usize, (pos.z & 31) as usize);
        let bg = self.background.as_ref().unwrap();
        // A save not written yet goes to the loader with the job.
        let unwritten = self.pending.get(&key).and_then(|p| p.iter().find(|(x, z, _)| (*x, *z) == local)).map(|(_, _, p)| p).or_else(|| bg.writer.find(key, local));
        if let Some(payload) = unwritten {
            let _ = bg.load_tx.send((pos, Some(payload.clone())));
            return StartLoad::Started;
        }
        if bg.writer.writing(key) {
            // The region file is being replaced: read it once it is.
            return StartLoad::Unsupported;
        }
        let dir = &self.region_dir;
        let region = self.regions.entry(key).or_insert_with(|| {
            let path = dir.join(format!("r.{}.{}.mca", key.0, key.1));
            path.exists().then(|| RegionFile::open(&path)).and_then(Result::ok)
        });
        match region.as_ref() {
            Some(r) if r.contains(local.0, local.1) => {
                let _ = self.background.as_ref().unwrap().load_tx.send((pos, None));
                StartLoad::Started
            }
            _ => StartLoad::Missing,
        }
    }

    fn poll_loads(&mut self) -> Vec<(ChunkPos, Option<Chunk>, std::time::Duration)> {
        let Some(bg) = self.background.as_mut() else { return Vec::new() };
        let done: Vec<_> = bg.loaded_rx.try_iter().collect();
        done.into_iter()
            .map(|(pos, loaded, took)| {
                let chunk = loaded.map(|(c, kept)| {
                    if let Some(kept) = kept {
                        self.preserved.insert(pos, kept);
                    }
                    c
                });
                (pos, chunk, took)
            })
            .collect()
    }

    fn encoding(&mut self) -> usize {
        self.collect(None, false);
        self.background.as_ref().map_or(0, |bg| bg.encoding.len())
    }

    fn flush_ready(&mut self) {
        if self.background.is_none() {
            return;
        }
        self.collect(None, false);
        let bg = self.background.as_mut().unwrap();
        for (key, updates) in std::mem::take(&mut self.pending) {
            bg.writer.submit(key, self.region_dir.join(format!("r.{}.{}.mca", key.0, key.1)), updates);
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.background.is_some() {
            // Encodings still out land in `pending` first; then the regions go to the writer.
            self.collect(None, true);
            let bg = self.background.as_mut().unwrap();
            for (key, updates) in std::mem::take(&mut self.pending) {
                bg.writer.submit(key, self.region_dir.join(format!("r.{}.{}.mca", key.0, key.1)), updates);
            }
            return Ok(());
        }
        for ((rx, rz), updates) in std::mem::take(&mut self.pending) {
            let path = self.region_dir.join(format!("r.{rx}.{rz}.mca"));
            // Drop our read handle; the file is replaced and reopened on the next load.
            self.regions.remove(&(rx, rz));
            write_now(&path, &updates)?;
        }
        Ok(())
    }

    fn sync(&mut self) -> std::io::Result<()> {
        self.flush()?;
        if let Some(bg) = self.background.as_mut() {
            for key in bg.writer.wait() {
                self.regions.remove(&key);
            }
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
    let mut fields = chunk_fields(pos, chunk, preserved);
    let biome_names = biome_names();
    let sections = chunk_sections(chunk)
        .map(|(y, sec, sky, block)| {
            let mut s = vec![("Y".to_owned(), Tag::Byte(y as i8))];
            if let Some(sec) = sec {
                s.push(("block_states".into(), encode_blocks(&sec.blocks)));
                s.push(("biomes".into(), encode_biomes(&sec.biomes, biome_names)));
            }
            if let Some(sky) = sky {
                s.push(("SkyLight".into(), light_tag(sky)));
            }
            if let Some(block) = block {
                s.push(("BlockLight".into(), light_tag(block)));
            }
            Tag::Compound(s)
        })
        .collect();
    set(&mut fields, "sections", Tag::List(sections));
    Tag::Compound(fields)
}

pub(crate) fn light_tag(l: &Light) -> Tag {
    Tag::ByteArray(l.to_bytes().iter().map(|&b| b as i8).collect())
}

/// Biome names by registry id.
pub(crate) fn biome_names() -> &'static [&'static str] {
    kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:worldgen/biome").map(|(_, e)| *e).unwrap_or(&[])
}

/// The sections a saved chunk lists, in order: Y, the section inside the build height, and
/// the sky and block light layers written. Kiln keeps light up to date on every change, so
/// complete light is saved (a chunk that came without light goes back without it and vanilla
/// relights it); all-dark block light layers are left out.
#[allow(clippy::type_complexity)]
pub(crate) fn chunk_sections(chunk: &Chunk) -> impl Iterator<Item = (i32, Option<&Section>, Option<&Light>, Option<&Light>)> {
    let min_section = chunk.min_y() >> 4;
    let n = chunk.sections.len();
    let light_valid = chunk.light_trusted();
    (0..n + 2).filter_map(move |li| {
        let sec = (1..=n).contains(&li).then(|| &chunk.sections[li - 1]);
        let sky = light_valid.then(|| &chunk.sky_light()[li]);
        let block = light_valid.then(|| &chunk.block_light()[li]).filter(|b| !matches!(b, Light::Zero));
        (sec.is_some() || sky.is_some() || block.is_some()).then_some((min_section + li as i32 - 1, sec, sky, block))
    })
}

/// A saved chunk's fields, `sections` left an empty list in its place: `preserved` fields
/// from the loaded chunk, updated.
pub(crate) fn chunk_fields(pos: ChunkPos, chunk: &Chunk, preserved: Option<&Tag>) -> Vec<(String, Tag)> {
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
    set(&mut fields, "InhabitedTime", Tag::Long(chunk.inhabited_time()));
    set(&mut fields, "xPos", Tag::Int(pos.x));
    set(&mut fields, "zPos", Tag::Int(pos.z));
    set(&mut fields, "yPos", Tag::Int(min_section));
    set(&mut fields, "Status", Tag::String("minecraft:full".into()));
    set(&mut fields, "isLightOn", Tag::Byte(chunk.light_trusted() as i8));
    // Changed chunks go back without heightmaps (vanilla recomputes missing ones on load).
    if chunk.modified() {
        fields.retain(|(k, _)| k != "Heightmaps");
    }
    set(&mut fields, "sections", Tag::List(Vec::new()));
    let (bx, bz) = (pos.x * 16, pos.z * 16);
    let block_entities = chunk.block_entities().map(|((x, y, z), be)| be.saved([bx + x as i32, y, bz + z as i32])).collect();
    set(&mut fields, "block_entities", Tag::List(block_entities));
    if let Some(ticks) = &chunk.saved_ticks {
        set(&mut fields, "block_ticks", ticks.block.clone());
        set(&mut fields, "fluid_ticks", ticks.fluid.clone());
    }
    if let Some(structures) = &chunk.structures {
        set(&mut fields, "structures", (**structures).clone());
    }
    fields
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

/// A section's `block_states`: the palette in order of first use (as vanilla writes it), and
/// the indices packed at least 4 bits each.
pub(crate) fn encode_blocks(c: &BlockContainer) -> Tag {
    let mut palette: Vec<u16> = Vec::new();
    let mut idx = vec![0u64; 4096];
    let add = |palette: &mut Vec<u16>, s: u16| match palette.iter().position(|&p| p == s) {
        Some(i) => i as u16,
        None => {
            palette.push(s);
            (palette.len() - 1) as u16
        }
    };
    match c {
        BlockContainer::Single(s) => palette.push(*s),
        BlockContainer::Nibble { palette: pal, .. } | BlockContainer::Byte { palette: pal, .. } => {
            // Container palette index -> saved palette index, assigned on first use.
            let mut map = [u16::MAX; 256];
            for (i, e) in idx.iter_mut().enumerate() {
                let pi = match c {
                    BlockContainer::Nibble { indices, .. } => ((indices[i >> 1] >> ((i & 1) * 4)) & 0xf) as usize,
                    BlockContainer::Byte { indices, .. } => indices[i] as usize,
                    _ => unreachable!(),
                };
                if map[pi] == u16::MAX {
                    map[pi] = add(&mut palette, pal[pi]);
                }
                *e = map[pi] as u64;
            }
        }
        BlockContainer::Direct(d) => {
            let mut lookup: HashMap<u16, u16> = HashMap::new();
            for (e, &s) in idx.iter_mut().zip(d.iter()) {
                *e = *lookup.entry(s).or_insert_with(|| add(&mut palette, s)) as u64;
            }
        }
    }
    let mut fields = vec![("palette".to_owned(), Tag::heterogeneous_list(palette.iter().map(|&s| state_tag(s)).collect()))];
    if palette.len() > 1 {
        let bits = bits_for(palette.len()).max(4);
        let packed = kiln_world::section::pack(&idx, bits);
        fields.push(("data".into(), Tag::LongArray(packed.into_iter().map(|l| l as i64).collect())));
    }
    Tag::Compound(fields)
}

pub(crate) fn encode_biomes(b: &Biomes, names: &[&str]) -> Tag {
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

pub(crate) fn bits_for(n: usize) -> u32 {
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

pub(crate) fn decode_blocks(tag: &Tag) -> Result<BlockContainer, ChunkError> {
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

pub(crate) fn light_layer(bytes: &[i8]) -> Light {
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
