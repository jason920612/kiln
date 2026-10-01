//! Cell files (`c.<x>.<z>.kcell`): the records of one cell (8×8 chunks) in a log-structured
//! file. Writes append records, then an index of every live record and a trailer pointing at
//! it, in one write; a file is only ever read through its last complete index, so a write cut
//! short by a crash leaves the previous state. Rewriting a file (compaction, conversion) goes
//! through a temporary file renamed over the old one. Compaction can run off the thread that
//! owns the file: [`CellFile::plan_compaction`] takes a snapshot, [`CompactionPlan::run`]
//! writes the compacted copy on any thread (the old file stays untouched and keeps taking
//! appends), and [`CellFile::finish_compaction`] carries the appends since the snapshot over
//! and renames the copy into place.
//!
//! Layout (little-endian):
//! - header: `KILNCELL`, format `u16`, 6 reserved bytes;
//! - record: header of [`REC_HEADER`] bytes (magic `KREC`, kind, slot, form, codec, dictionary
//!   id, registry fingerprint, timestamp, raw length, stored length, CRC-32 of the header's
//!   other bytes and the payload), then the payload;
//! - index: a record of kind [`INDEX`] listing (kind, slot, offset, length) per live record;
//! - trailer: index offset `u64`, `KEND`, CRC-32 of the offset.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const FILE_MAGIC: &[u8; 8] = b"KILNCELL";
/// Format version of cell files; readers refuse newer ones.
pub const FORMAT: u16 = 1;
const HEADER: u64 = 16;
const REC_MAGIC: u32 = u32::from_le_bytes(*b"KREC");
const END_MAGIC: u32 = u32::from_le_bytes(*b"KEND");
pub const REC_HEADER: usize = 32;
const TRAILER: usize = 16;
const INDEX_ENTRY: usize = 16;

/// Record kinds.
pub const CHUNK: u8 = 1;
pub const ENTITIES: u8 = 2;
pub const POI: u8 = 3;
/// Plugin cell data of the Anvil region holding the cell (the sidecar Anvil worlds keep in
/// `kiln/plugins/cells`), in the region's first cell.
pub const PLUGIN: u8 = 4;
const INDEX: u8 = 0xff;

/// A record's place in a cell: its kind and the chunk (`(z & 7) << 3 | (x & 7)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub kind: u8,
    pub slot: u8,
}

/// A record as stored: metadata and the (possibly compressed) payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// How the payload is to be read (for chunks: native or chunk NBT).
    pub form: u8,
    /// 0 stored, 1 zstd.
    pub codec: u8,
    /// Compression dictionary (0: none).
    pub dict: u32,
    /// State and biome id table the payload's ids refer to (0: none).
    pub registry: u32,
    /// Last write, epoch seconds (Anvil header timestamps survive conversion).
    pub stamp: u32,
    pub raw_len: u32,
    pub data: Vec<u8>,
}

impl Record {
    fn encode_into(&self, key: Key, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&REC_MAGIC.to_le_bytes());
        out.extend_from_slice(&[key.kind, key.slot, self.form, self.codec]);
        for v in [self.dict, self.registry, self.stamp, self.raw_len, self.data.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&self.data);
        let crc = record_crc(&out[start..start + REC_HEADER - 4], &out[start + REC_HEADER..]);
        out[start + REC_HEADER - 4..start + REC_HEADER].copy_from_slice(&crc.to_le_bytes());
    }

    /// Parses a record at the front of `buf` (header and payload); `None` if truncated or
    /// corrupt.
    fn decode(buf: &[u8]) -> Option<(Key, Record, usize)> {
        if buf.len() < REC_HEADER || u32::from_le_bytes(buf[0..4].try_into().unwrap()) != REC_MAGIC {
            return None;
        }
        let u = |i: usize| u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
        let len = u(24) as usize;
        let total = REC_HEADER.checked_add(len)?;
        if buf.len() < total || record_crc(&buf[..REC_HEADER - 4], &buf[REC_HEADER..total]) != u(28) {
            return None;
        }
        let key = Key { kind: buf[4], slot: buf[5] };
        let rec = Record {
            form: buf[6],
            codec: buf[7],
            dict: u(8),
            registry: u(12),
            stamp: u(16),
            raw_len: u(20),
            data: buf[REC_HEADER..total].to_vec(),
        };
        Some((key, rec, total))
    }
}

fn record_crc(header: &[u8], payload: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(header);
    h.update(payload);
    h.finalize()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    offset: u64,
    /// Whole record, header included.
    len: u32,
}

pub struct CellFile {
    path: PathBuf,
    file: File,
    index: BTreeMap<Key, Entry>,
    /// End of the last complete write; anything after it is the remains of an interrupted one.
    end: u64,
}

impl CellFile {
    /// Opens a cell file for reading and appending; `None` if there is none.
    pub fn open(path: &Path) -> io::Result<Option<CellFile>> {
        let mut file = match File::options().read(true).write(true).open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        let mut head = [0u8; HEADER as usize];
        file.read_exact(&mut head).map_err(|_| corrupt("short header"))?;
        if &head[..8] != FILE_MAGIC {
            return Err(corrupt("not a cell file"));
        }
        let format = u16::from_le_bytes([head[8], head[9]]);
        if format > FORMAT {
            return Err(corrupt("cell file from a newer Kiln"));
        }
        let mut cell = CellFile { path: path.to_owned(), file, index: BTreeMap::new(), end: HEADER };
        if !cell.read_tail(len)? {
            cell.recover(len)?;
        }
        Ok(Some(cell))
    }

    /// The index the trailer points at, when the file ends in a complete write.
    fn read_tail(&mut self, len: u64) -> io::Result<bool> {
        if len < HEADER + (REC_HEADER + TRAILER) as u64 {
            return Ok(len == HEADER);
        }
        let mut t = [0u8; TRAILER];
        self.file.seek(SeekFrom::Start(len - TRAILER as u64))?;
        self.file.read_exact(&mut t)?;
        let offset = u64::from_le_bytes(t[..8].try_into().unwrap());
        if u32::from_le_bytes(t[8..12].try_into().unwrap()) != END_MAGIC
            || u32::from_le_bytes(t[12..16].try_into().unwrap()) != crc32fast::hash(&t[..8])
            || offset < HEADER
            || offset > len - (REC_HEADER + TRAILER) as u64
        {
            return Ok(false);
        }
        let mut buf = vec![0u8; (len - TRAILER as u64 - offset) as usize];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buf)?;
        match Record::decode(&buf) {
            Some((Key { kind: INDEX, .. }, rec, n)) if n == buf.len() => match parse_index(&rec.data, offset) {
                Some(index) => {
                    self.index = index;
                    self.end = len;
                    Ok(true)
                }
                None => Ok(false),
            },
            _ => Ok(false),
        }
    }

    /// Scans the file from the start for the last complete write (the trailer was lost).
    fn recover(&mut self, len: u64) -> io::Result<()> {
        let mut buf = Vec::with_capacity(len as usize);
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_to_end(&mut buf)?;
        let mut pos = HEADER as usize;
        let mut last = None;
        while let Some((key, rec, n)) = Record::decode(&buf[pos..]) {
            let next = pos + n;
            if key.kind == INDEX {
                let t = buf.get(next..next + TRAILER);
                let complete = t.is_some_and(|t| {
                    u64::from_le_bytes(t[..8].try_into().unwrap()) == pos as u64
                        && u32::from_le_bytes(t[8..12].try_into().unwrap()) == END_MAGIC
                        && u32::from_le_bytes(t[12..16].try_into().unwrap()) == crc32fast::hash(&t[..8])
                });
                if !complete {
                    break;
                }
                match parse_index(&rec.data, pos as u64) {
                    Some(index) => last = Some((index, (next + TRAILER) as u64)),
                    None => break,
                }
                pos = next + TRAILER;
            } else {
                pos = next;
            }
        }
        tracing::warn!("cell file {} ends in an interrupted write; using its last complete one", self.path.display());
        (self.index, self.end) = last.unwrap_or((BTreeMap::new(), HEADER));
        Ok(())
    }

    /// Creates an empty cell file (replacing any).
    pub fn create(path: &Path) -> io::Result<CellFile> {
        write_new(path, std::iter::empty(), false)?;
        Ok(CellFile::open(path)?.expect("just created"))
    }

    pub fn contains(&self, key: Key) -> bool {
        self.index.contains_key(&key)
    }

    pub fn keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.index.keys().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Bytes of live records.
    pub fn live_bytes(&self) -> u64 {
        self.index.values().map(|e| e.len as u64).sum()
    }

    pub fn file_len(&self) -> u64 {
        self.end
    }

    pub fn read(&mut self, key: Key) -> io::Result<Option<Record>> {
        let Some(e) = self.index.get(&key).copied() else { return Ok(None) };
        let mut buf = vec![0u8; e.len as usize];
        self.file.seek(SeekFrom::Start(e.offset))?;
        self.file.read_exact(&mut buf)?;
        match Record::decode(&buf) {
            Some((k, rec, _)) if k == key => Ok(Some(rec)),
            _ => Err(corrupt("record fails its checksum")),
        }
    }

    /// The bytes of a stored record (header included), to copy into another file.
    fn read_raw(&mut self, e: Entry) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; e.len as usize];
        self.file.seek(SeekFrom::Start(e.offset))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Appends records (`None` removes one) and a new index in one write.
    pub fn commit(&mut self, updates: impl IntoIterator<Item = (Key, Option<Record>)>, sync: bool) -> io::Result<()> {
        let mut buf = Vec::new();
        for (key, rec) in updates {
            match rec {
                Some(rec) => {
                    let offset = self.end + buf.len() as u64;
                    rec.encode_into(key, &mut buf);
                    self.index.insert(key, Entry { offset, len: (self.end + buf.len() as u64 - offset) as u32 });
                }
                None => {
                    self.index.remove(&key);
                }
            }
        }
        let index_at = self.end + buf.len() as u64;
        append_index(&self.index, index_at, &mut buf);
        // Drop what an interrupted write left, so the scan of a later recovery sees this one.
        if self.file.metadata()?.len() != self.end {
            self.file.set_len(self.end)?;
        }
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&buf)?;
        if sync {
            self.file.sync_data()?;
        }
        self.end += buf.len() as u64;
        Ok(())
    }

    /// Whether stale records take enough room to rewrite the file.
    pub fn wants_compaction(&self) -> bool {
        let live = self.live_bytes() + HEADER;
        self.end > 2 * live && self.end - live > 256 * 1024
    }

    /// Rewrites the file with its live records only.
    pub fn compact(mut self, sync: bool) -> io::Result<Option<CellFile>> {
        let entries: Vec<(Key, Entry)> = self.index.iter().map(|(k, e)| (*k, *e)).collect();
        let mut raws = Vec::with_capacity(entries.len());
        for (k, e) in entries {
            raws.push((k, self.read_raw(e)?));
        }
        let path = self.path.clone();
        drop(self);
        if raws.is_empty() {
            std::fs::remove_file(&path)?;
            return Ok(None);
        }
        write_raw(&path, raws, sync)?;
        CellFile::open(&path)
    }

    /// A snapshot of the live records, to compact on another thread while this file goes on
    /// taking appends.
    pub fn plan_compaction(&self) -> CompactionPlan {
        CompactionPlan { path: self.path.clone(), snapshot: self.index.clone() }
    }

    /// Appends records as stored (headers included) and drops keys, with a new index, in one
    /// write.
    fn commit_raw(&mut self, put: Vec<(Key, Vec<u8>)>, remove: &[Key], sync: bool) -> io::Result<()> {
        let mut buf = Vec::new();
        for (key, raw) in put {
            let offset = self.end + buf.len() as u64;
            self.index.insert(key, Entry { offset, len: raw.len() as u32 });
            buf.extend_from_slice(&raw);
        }
        for key in remove {
            self.index.remove(key);
        }
        let index_at = self.end + buf.len() as u64;
        append_index(&self.index, index_at, &mut buf);
        if self.file.metadata()?.len() != self.end {
            self.file.set_len(self.end)?;
        }
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&buf)?;
        if sync {
            self.file.sync_data()?;
        }
        self.end += buf.len() as u64;
        Ok(())
    }

    /// Ends a compaction: the records written since the plan was taken (new, replaced or
    /// removed) are carried into the compacted copy with a fresh index, and the copy is renamed
    /// over this file. Returns the new file (`None` when nothing is left, then both files are
    /// gone). On an error this file is untouched and the copy is removed.
    pub fn finish_compaction(self, done: Compacted, sync: bool) -> io::Result<Option<CellFile>> {
        let path = self.path.clone();
        let result = self.swap_in(&done, sync);
        if result.as_ref().map_or(true, Option::is_none) {
            let _ = std::fs::remove_file(&done.tmp);
        }
        if let Ok(None) = result {
            let _ = std::fs::remove_file(&path);
        }
        result
    }

    fn swap_in(mut self, done: &Compacted, sync: bool) -> io::Result<Option<CellFile>> {
        // What differs from the snapshot: appended and replaced records, removed keys.
        let mut put = Vec::new();
        for (k, e) in self.index.clone() {
            if done.snapshot.get(&k) != Some(&e) {
                put.push((k, self.read_raw(e)?));
            }
        }
        let removed: Vec<Key> = done.snapshot.keys().filter(|k| !self.index.contains_key(k)).copied().collect();
        if self.index.is_empty() {
            return Ok(None);
        }
        let mut copy = CellFile::open(&done.tmp)?.ok_or_else(|| corrupt("compacted copy is gone"))?;
        if !put.is_empty() || !removed.is_empty() {
            copy.commit_raw(put, &removed, sync)?;
        }
        drop(copy);
        // Windows cannot replace a file this process still has open for writing.
        let path = self.path.clone();
        drop(self);
        std::fs::rename(&done.tmp, &path)?;
        CellFile::open(&path)
    }
}

/// The work of a compaction that does not need the file's owner: which records are live.
pub struct CompactionPlan {
    path: PathBuf,
    snapshot: BTreeMap<Key, Entry>,
}

/// A compacted copy waiting to be swapped in ([`CellFile::finish_compaction`]).
pub struct Compacted {
    tmp: PathBuf,
    snapshot: BTreeMap<Key, Entry>,
}

impl CompactionPlan {
    /// The temporary copy's path (`<cell file>.compact`); leftovers are deleted when a store
    /// opens.
    pub fn temp_path(path: &Path) -> PathBuf {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".compact");
        PathBuf::from(tmp)
    }

    /// Where this plan's copy is written.
    pub fn temp_file(&self) -> PathBuf {
        CompactionPlan::temp_path(&self.path)
    }

    /// Writes the compacted copy: reads the snapshot's records through its own handle (records
    /// are never modified in place, so appends to the file meanwhile do not matter), writes
    /// them and a complete index to the temporary path and syncs it. The cell file itself is
    /// not touched.
    pub fn run(self, sync: bool) -> io::Result<Compacted> {
        self.run_limited(sync, None)
    }

    /// [`CompactionPlan::run`] that stops after writing `torn_at` bytes of the copy and reports
    /// an error, leaving the torn copy behind: what a crash in the middle of a compaction
    /// leaves (for tests).
    #[doc(hidden)]
    pub fn run_torn(self, torn_at: usize) -> io::Result<Compacted> {
        self.run_limited(false, Some(torn_at))
    }

    fn run_limited(self, sync: bool, torn_at: Option<usize>) -> io::Result<Compacted> {
        let mut src = File::open(&self.path)?;
        let mut buf = Vec::new();
        buf.extend_from_slice(FILE_MAGIC);
        buf.extend_from_slice(&FORMAT.to_le_bytes());
        buf.extend_from_slice(&[0; 6]);
        let mut index = BTreeMap::new();
        for (k, e) in &self.snapshot {
            let mut raw = vec![0u8; e.len as usize];
            src.seek(SeekFrom::Start(e.offset))?;
            src.read_exact(&mut raw)?;
            // A record that no longer checks out must not be carried into the copy.
            match Record::decode(&raw) {
                Some((key, _, n)) if key == *k && n == raw.len() => {}
                _ => return Err(corrupt("record fails its checksum")),
            }
            index.insert(*k, Entry { offset: buf.len() as u64, len: raw.len() as u32 });
            buf.extend_from_slice(&raw);
        }
        append_index(&index, buf.len() as u64, &mut buf);
        let tmp = CompactionPlan::temp_path(&self.path);
        let mut f = File::create(&tmp)?;
        if let Some(n) = torn_at {
            f.write_all(&buf[..n.min(buf.len())])?;
            return Err(io::Error::other("compaction interrupted"));
        }
        f.write_all(&buf)?;
        if sync {
            f.sync_all()?;
        }
        Ok(Compacted { tmp, snapshot: self.snapshot })
    }
}

impl Compacted {
    /// Deletes the copy (a compaction that will not be finished).
    pub fn discard(self) {
        let _ = std::fs::remove_file(&self.tmp);
    }
}

/// Writes a new cell file holding `records`, through a temporary file renamed into place.
pub fn write_new(path: &Path, records: impl IntoIterator<Item = (Key, Record)>, sync: bool) -> io::Result<()> {
    let raws = records.into_iter().map(|(k, r)| {
        let mut b = Vec::with_capacity(REC_HEADER + r.data.len());
        r.encode_into(k, &mut b);
        (k, b)
    });
    write_raw(path, raws.collect(), sync)
}

fn write_raw(path: &Path, records: Vec<(Key, Vec<u8>)>, sync: bool) -> io::Result<()> {
    let mut buf = Vec::with_capacity(records.iter().map(|(_, b)| b.len()).sum::<usize>() + 4096);
    buf.extend_from_slice(FILE_MAGIC);
    buf.extend_from_slice(&FORMAT.to_le_bytes());
    buf.extend_from_slice(&[0; 6]);
    let mut index = BTreeMap::new();
    for (k, raw) in records {
        index.insert(k, Entry { offset: buf.len() as u64, len: raw.len() as u32 });
        buf.extend_from_slice(&raw);
    }
    append_index(&index, buf.len() as u64, &mut buf);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&buf)?;
        if sync {
            f.sync_all()?;
        }
    }
    std::fs::rename(&tmp, path)
}

fn append_index(index: &BTreeMap<Key, Entry>, at: u64, buf: &mut Vec<u8>) {
    let mut data = Vec::with_capacity(index.len() * INDEX_ENTRY + 4);
    data.extend_from_slice(&(index.len() as u32).to_le_bytes());
    for (k, e) in index {
        data.extend_from_slice(&[k.kind, k.slot, 0, 0]);
        data.extend_from_slice(&e.offset.to_le_bytes());
        data.extend_from_slice(&e.len.to_le_bytes());
    }
    let rec = Record { form: 0, codec: 0, dict: 0, registry: 0, stamp: 0, raw_len: data.len() as u32, data };
    rec.encode_into(Key { kind: INDEX, slot: 0 }, buf);
    buf.extend_from_slice(&at.to_le_bytes());
    buf.extend_from_slice(&END_MAGIC.to_le_bytes());
    let crc = crc32fast::hash(&at.to_le_bytes());
    buf.extend_from_slice(&crc.to_le_bytes());
}

/// An index record's entries; every record must lie before the index itself.
fn parse_index(data: &[u8], index_at: u64) -> Option<BTreeMap<Key, Entry>> {
    let n = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
    if data.len() != 4 + n * INDEX_ENTRY {
        return None;
    }
    let mut index = BTreeMap::new();
    for e in data[4..].chunks_exact(INDEX_ENTRY) {
        let offset = u64::from_le_bytes(e[4..12].try_into().unwrap());
        let len = u32::from_le_bytes(e[12..16].try_into().unwrap());
        if offset < HEADER || offset + len as u64 > index_at || (len as usize) < REC_HEADER {
            return None;
        }
        index.insert(Key { kind: e[0], slot: e[1] }, Entry { offset, len });
    }
    Some(index)
}

fn corrupt(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(n: u8, len: usize) -> Record {
        Record { form: 0, codec: 0, dict: 0, registry: 7, stamp: 99, raw_len: len as u32, data: vec![n; len] }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kiln-cellfile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn commit_reopen_and_remove() {
        let d = dir("commit");
        let path = d.join("c.0.0.kcell");
        let a = Key { kind: CHUNK, slot: 3 };
        let b = Key { kind: ENTITIES, slot: 63 };
        let mut f = CellFile::create(&path).unwrap();
        f.commit([(a, Some(rec(1, 100))), (b, Some(rec(2, 10)))], true).unwrap();
        f.commit([(a, Some(rec(3, 50)))], true).unwrap();
        drop(f);
        let mut f = CellFile::open(&path).unwrap().unwrap();
        assert_eq!(f.read(a).unwrap(), Some(rec(3, 50)));
        assert_eq!(f.read(b).unwrap(), Some(rec(2, 10)));
        f.commit([(b, None)], false).unwrap();
        drop(f);
        let mut f = CellFile::open(&path).unwrap().unwrap();
        assert_eq!(f.read(b).unwrap(), None);
        assert_eq!(f.keys().collect::<Vec<_>>(), vec![a]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn interrupted_write_keeps_the_last_complete_one() {
        let d = dir("crash");
        let path = d.join("c.0.0.kcell");
        let a = Key { kind: CHUNK, slot: 0 };
        let mut f = CellFile::create(&path).unwrap();
        f.commit([(a, Some(rec(1, 1000)))], true).unwrap();
        let good = f.file_len();
        f.commit([(a, Some(rec(2, 1000)))], true).unwrap();
        let full = f.file_len();
        drop(f);
        // Every cut of the second write, down to nothing of it.
        let bytes = std::fs::read(&path).unwrap();
        for cut in [full - 1, full - TRAILER as u64, good + 40, good + 1] {
            std::fs::write(&path, &bytes[..cut as usize]).unwrap();
            let mut f = CellFile::open(&path).unwrap().unwrap();
            assert_eq!(f.read(a).unwrap(), Some(rec(1, 1000)), "cut at {cut}");
            // Writing after recovery drops the partial write first.
            f.commit([(a, Some(rec(4, 10)))], false).unwrap();
            drop(f);
            let mut f = CellFile::open(&path).unwrap().unwrap();
            assert_eq!(f.read(a).unwrap(), Some(rec(4, 10)));
        }
        // A corrupt byte inside the latest write's record: its checksum fails.
        let mut bad = bytes.clone();
        bad[good as usize + REC_HEADER + 5] ^= 0xff;
        std::fs::write(&path, &bad).unwrap();
        let mut f = CellFile::open(&path).unwrap().unwrap();
        assert!(f.read(a).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn key(slot: u8) -> Key {
        Key { kind: CHUNK, slot }
    }

    /// A file with 20 generations of 8 records, mostly stale.
    fn stale_file(path: &Path) -> CellFile {
        let mut f = CellFile::create(path).unwrap();
        for i in 0..20u8 {
            f.commit((0..8u8).map(|s| (key(s), Some(rec(i, 4000)))), false).unwrap();
        }
        f
    }

    #[test]
    fn background_compaction_carries_appends_over() {
        let d = dir("bg");
        let path = d.join("c.0.0.kcell");
        let mut f = stale_file(&path);
        let before = f.file_len();
        let plan = f.plan_compaction();
        // Writes after the snapshot: a replaced record, a new one, a removed one, in two commits.
        f.commit([(key(0), Some(rec(100, 500))), (key(9), Some(rec(101, 600)))], false).unwrap();
        f.commit([(key(1), None)], false).unwrap();
        let copy = plan.run(true).unwrap();
        // ... and one more while the copy waits to be swapped in.
        f.commit([(key(2), Some(rec(102, 700)))], false).unwrap();
        let mut f = f.finish_compaction(copy, true).unwrap().unwrap();
        assert!(f.file_len() < before / 5, "{} of {before}", f.file_len());
        assert!(!CompactionPlan::temp_path(&path).exists());
        assert_eq!(f.read(key(0)).unwrap(), Some(rec(100, 500)));
        assert_eq!(f.read(key(1)).unwrap(), None);
        assert_eq!(f.read(key(2)).unwrap(), Some(rec(102, 700)));
        assert_eq!(f.read(key(3)).unwrap(), Some(rec(19, 4000)));
        assert_eq!(f.read(key(9)).unwrap(), Some(rec(101, 600)));
        // The swapped-in file is an ordinary one: it takes appends and reopens.
        f.commit([(key(3), Some(rec(103, 10)))], false).unwrap();
        drop(f);
        let mut f = CellFile::open(&path).unwrap().unwrap();
        assert_eq!(f.keys().map(|k| k.slot).collect::<Vec<_>>(), vec![0, 2, 3, 4, 5, 6, 7, 9]);
        assert_eq!(f.read(key(3)).unwrap(), Some(rec(103, 10)));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn compaction_of_a_file_emptied_meanwhile_removes_it() {
        let d = dir("bg-empty");
        let path = d.join("c.0.0.kcell");
        let mut f = stale_file(&path);
        let plan = f.plan_compaction();
        f.commit((0..8u8).map(|s| (key(s), None)), false).unwrap();
        let copy = plan.run(false).unwrap();
        assert!(f.finish_compaction(copy, false).unwrap().is_none());
        assert!(!path.exists());
        assert!(!CompactionPlan::temp_path(&path).exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_copy_torn_anywhere_leaves_the_file_intact() {
        let d = dir("torn");
        let path = d.join("c.0.0.kcell");
        drop(stale_file(&path));
        let copy_len = 16 + 8 * (REC_HEADER + 4000);
        for cut in [0, 1, 15, 16, 100, copy_len / 2, copy_len - 1, copy_len + 10] {
            let f = CellFile::open(&path).unwrap().unwrap();
            assert!(f.plan_compaction().run_torn(cut).is_err());
            drop(f);
            // The crash left a torn copy behind; the cell file is what it was.
            assert!(CompactionPlan::temp_path(&path).exists());
            let mut f = CellFile::open(&path).unwrap().unwrap();
            for s in 0..8u8 {
                assert_eq!(f.read(key(s)).unwrap(), Some(rec(19, 4000)), "cut at {cut}");
            }
        }
        // A later compaction starts over on the same temporary path.
        let f = CellFile::open(&path).unwrap().unwrap();
        let copy = f.plan_compaction().run(false).unwrap();
        let mut f = f.finish_compaction(copy, false).unwrap().unwrap();
        for s in 0..8u8 {
            assert_eq!(f.read(key(s)).unwrap(), Some(rec(19, 4000)));
        }
        assert!(!CompactionPlan::temp_path(&path).exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn compaction_keeps_live_records() {
        let d = dir("compact");
        let path = d.join("c.1.-1.kcell");
        let mut f = CellFile::create(&path).unwrap();
        for i in 0..20u8 {
            f.commit((0..8u8).map(|s| (Key { kind: CHUNK, slot: s }, Some(rec(i, 4000)))), false).unwrap();
        }
        assert!(f.wants_compaction());
        let before = f.file_len();
        let mut f = f.compact(true).unwrap().unwrap();
        assert!(f.file_len() < before / 10);
        for s in 0..8u8 {
            assert_eq!(f.read(Key { kind: CHUNK, slot: s }).unwrap(), Some(rec(19, 4000)));
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
