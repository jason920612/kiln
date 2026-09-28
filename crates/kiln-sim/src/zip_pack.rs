//! Zip data packs (`FilePackResources`): vanilla reads `.zip` packs in `<world>/datapacks` in
//! place. Kiln's loaders read directories, so a zip pack is unpacked once into a cache
//! directory keyed by the file's path, size and modification time, and loaded from there.
//!
//! Only what data packs use is supported: stored and deflated entries, no zip64, no
//! encryption.

use std::io::Read;
use std::path::{Path, PathBuf};

const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
const CENTRAL_FILE_HEADER: u32 = 0x0201_4b50;
const LOCAL_FILE_HEADER: u32 = 0x0403_4b50;

/// One file in a zip archive.
pub(crate) struct Entry {
    pub name: String,
    method: u16,
    compressed: usize,
    size: usize,
    local_header: usize,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A zip archive read into memory.
pub(crate) struct Zip {
    bytes: Vec<u8>,
    pub entries: Vec<Entry>,
}

impl Zip {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Self::parse(std::fs::read(path)?).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a zip file"))
    }

    pub fn parse(bytes: Vec<u8>) -> Option<Self> {
        // The end record is the last 22 bytes, before a comment of up to 65535 bytes.
        let lowest = bytes.len().saturating_sub(22 + 0xFFFF);
        let end = (lowest..=bytes.len().checked_sub(22)?).rev().find(|&i| u32_at(&bytes, i) == Some(END_OF_CENTRAL_DIRECTORY))?;
        let count = u16_at(&bytes, end + 10)? as usize;
        let mut at = u32_at(&bytes, end + 16)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(&bytes, at)? != CENTRAL_FILE_HEADER {
                return None;
            }
            let method = u16_at(&bytes, at + 10)?;
            let compressed = u32_at(&bytes, at + 20)? as usize;
            let size = u32_at(&bytes, at + 24)? as usize;
            let name_len = u16_at(&bytes, at + 28)? as usize;
            let extra_len = u16_at(&bytes, at + 30)? as usize;
            let comment_len = u16_at(&bytes, at + 32)? as usize;
            let local_header = u32_at(&bytes, at + 42)? as usize;
            let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + name_len)?).into_owned();
            entries.push(Entry { name, method, compressed, size, local_header });
            at += 46 + name_len + extra_len + comment_len;
        }
        Some(Zip { bytes, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// An entry's contents.
    pub fn read(&self, e: &Entry) -> std::io::Result<Vec<u8>> {
        let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad zip entry {}", e.name));
        let at = e.local_header;
        if u32_at(&self.bytes, at) != Some(LOCAL_FILE_HEADER) {
            return Err(bad());
        }
        let name_len = u16_at(&self.bytes, at + 26).ok_or_else(bad)? as usize;
        let extra_len = u16_at(&self.bytes, at + 28).ok_or_else(bad)? as usize;
        let start = at + 30 + name_len + extra_len;
        let data = self.bytes.get(start..start + e.compressed).ok_or_else(bad)?;
        match e.method {
            0 => Ok(data.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(e.size);
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut out)?;
                Ok(out)
            }
            m => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, format!("zip method {m} for {}", e.name))),
        }
    }
}

/// A relative path inside the pack, or `None` for names that would leave it.
fn safe_relative(name: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p if p.contains(':') => return None,
            p => out.push(p),
        }
    }
    Some(out)
}

/// FNV-1a, to name the cache directory of a zip.
fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in *part {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Where Kiln unpacks zip packs (`KILN_PACK_CACHE`, default `<temp>/kiln-zip-packs`).
fn cache_root() -> PathBuf {
    std::env::var_os("KILN_PACK_CACHE").map_or_else(|| std::env::temp_dir().join("kiln-zip-packs"), PathBuf::from)
}

/// Whether `path` is a zip pack (`pack.mcmeta` at its root).
pub(crate) fn is_pack(path: &Path) -> bool {
    Zip::open(path).is_ok_and(|z| z.find("pack.mcmeta").is_some())
}

/// The directory holding the unpacked contents of the zip pack at `path` (unpacked on first
/// use, and again whenever the file changes).
pub(crate) fn unpacked(path: &Path) -> std::io::Result<PathBuf> {
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = fnv(&[absolute.to_string_lossy().as_bytes(), &meta.len().to_le_bytes(), &modified.to_le_bytes()]);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("pack");
    let dir = cache_root().join(format!("{stem}-{key:016x}"));
    let done = dir.join(".kiln-unpacked");
    if done.is_file() {
        return Ok(dir);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let zip = Zip::open(path)?;
    for e in &zip.entries {
        if e.name.ends_with('/') {
            continue;
        }
        let Some(rel) = safe_relative(&e.name) else { continue };
        let out = dir.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out, zip.read(e)?)?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(done, b"")?;
    Ok(dir)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A zip with the given files, deflated when `deflate`.
    pub(crate) fn build(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        use std::io::Write;
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let body = if deflate {
                let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(data).unwrap();
                enc.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend(LOCAL_FILE_HEADER.to_le_bytes());
            out.extend([20, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0; 8]); // time, date, crc (not checked)
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&body);
            central.extend(CENTRAL_FILE_HEADER.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0; 8]);
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]); // extra, comment, disk, internal and external attributes
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = out.len() as u32;
        let n = files.len() as u16;
        out.extend(&central);
        out.extend(END_OF_CENTRAL_DIRECTORY.to_le_bytes());
        out.extend([0; 4]);
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        for deflate in [false, true] {
            let bytes = build(&[("pack.mcmeta", b"{}"), ("data/k/function/a.mcfunction", b"say hi\nsay hi again\n")], deflate);
            let zip = Zip::parse(bytes).unwrap();
            assert_eq!(zip.entries.len(), 2);
            let e = zip.find("data/k/function/a.mcfunction").unwrap();
            assert_eq!(zip.read(e).unwrap(), b"say hi\nsay hi again\n");
        }
        assert!(Zip::parse(b"not a zip".to_vec()).is_none());
    }

    #[test]
    fn unsafe_names_stay_inside() {
        assert_eq!(safe_relative("data/a/b.json"), Some(PathBuf::from("data").join("a").join("b.json")));
        assert_eq!(safe_relative("../evil"), None);
        assert_eq!(safe_relative("C:/evil"), None);
    }
}
