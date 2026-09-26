//! Anvil region files (`r.<x>.<z>.mca`): 32×32 chunks in 4 KiB sectors.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SECTOR: u64 = 4096;
/// Chunks stored outside the region file set this bit in the compression byte.
const EXTERNAL_FLAG: u8 = 0x80;
/// Upper bound on a decompressed chunk; vanilla chunks are far smaller.
pub const MAX_CHUNK_NBT: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RegionError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("corrupt region data: {0}")]
    Corrupt(&'static str),
    #[error("unsupported chunk compression {0}")]
    Compression(u8),
    #[error("chunk exceeds {MAX_CHUNK_NBT} bytes when decompressed")]
    TooLarge,
}

pub struct RegionFile {
    path: PathBuf,
    file: File,
    offsets: Box<[u32; 1024]>,
}

impl RegionFile {
    pub fn open(path: &Path) -> Result<Self, RegionError> {
        let mut file = File::open(path)?;
        let mut header = vec![0u8; 4096];
        file.read_exact(&mut header).map_err(|_| RegionError::Corrupt("short header"))?;
        let mut offsets = Box::new([0u32; 1024]);
        for (i, o) in offsets.iter_mut().enumerate() {
            *o = u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
        }
        Ok(Self { path: path.to_owned(), file, offsets })
    }

    /// The uncompressed NBT of the chunk at local coordinates (0..32), if present.
    pub fn read(&mut self, x: usize, z: usize) -> Result<Option<Vec<u8>>, RegionError> {
        let loc = self.offsets[(z << 5) | x];
        if loc == 0 {
            return Ok(None);
        }
        let (sector, count) = ((loc >> 8) as u64, (loc & 0xff) as u64);
        if sector < 2 || count == 0 {
            return Err(RegionError::Corrupt("bad sector offset"));
        }
        self.file.seek(SeekFrom::Start(sector * SECTOR))?;
        let mut head = [0u8; 5];
        self.file.read_exact(&mut head)?;
        let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
        let compression = head[4];
        let data = if compression & EXTERNAL_FLAG != 0 {
            // Oversized chunks live next to the region in c.<chunkX>.<chunkZ>.mcc.
            let (rx, rz) = region_coords(&self.path).ok_or(RegionError::Corrupt("region file name"))?;
            let ext = self.path.with_file_name(format!("c.{}.{}.mcc", rx * 32 + x as i32, rz * 32 + z as i32));
            if std::fs::metadata(&ext)?.len() > MAX_CHUNK_NBT as u64 {
                return Err(RegionError::TooLarge);
            }
            std::fs::read(ext)?
        } else {
            if len == 0 || len > count * SECTOR {
                return Err(RegionError::Corrupt("chunk length exceeds its sectors"));
            }
            let mut data = vec![0u8; len as usize - 1];
            self.file.read_exact(&mut data)?;
            data
        };
        decompress(compression & !EXTERNAL_FLAG, &data).map(Some)
    }
}

/// Zlib-compresses chunk NBT into the payload stored after the length field
/// (compression byte + data).
pub fn compress_chunk(nbt: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut out = vec![2u8];
    let mut enc = flate2::write::ZlibEncoder::new(&mut out, flate2::Compression::new(6));
    enc.write_all(nbt).expect("in-memory write");
    enc.finish().expect("in-memory write");
    out
}

/// Rewrites a region file with `updates` (local x, local z, payload from [`compress_chunk`])
/// replacing or adding chunks; other chunks are copied as stored. Writes a temporary
/// file and renames it over the old one.
pub fn write_region(path: &Path, updates: &[(usize, usize, Vec<u8>)], now: u32) -> Result<(), RegionError> {
    const MAX_SECTORS: usize = 255;
    let mut payloads: Vec<Option<Vec<u8>>> = vec![None; 1024];
    let mut stamps = vec![0u32; 1024];
    if path.exists() {
        let mut file = File::open(path)?;
        let mut header = vec![0u8; 8192];
        file.read_exact(&mut header).map_err(|_| RegionError::Corrupt("short header"))?;
        for i in 0..1024 {
            let loc = u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
            stamps[i] = u32::from_be_bytes(header[4096 + i * 4..4096 + i * 4 + 4].try_into().unwrap());
            if loc == 0 {
                continue;
            }
            let (sector, count) = ((loc >> 8) as u64, (loc & 0xff) as u64);
            file.seek(SeekFrom::Start(sector * SECTOR))?;
            let mut len = [0u8; 4];
            file.read_exact(&mut len)?;
            let len = u32::from_be_bytes(len) as u64;
            if len == 0 || len > count * SECTOR {
                return Err(RegionError::Corrupt("chunk length exceeds its sectors"));
            }
            let mut p = vec![0u8; len as usize];
            file.read_exact(&mut p)?;
            payloads[i] = Some(p);
        }
    }
    for (x, z, payload) in updates {
        let i = (z << 5) | x;
        payloads[i] = Some(payload.clone());
        stamps[i] = now;
    }

    let mut header = vec![0u8; 8192];
    let mut body = Vec::new();
    let mut sector = 2u32;
    for (i, p) in payloads.iter().enumerate() {
        let Some(p) = p else { continue };
        let mut record = Vec::with_capacity(p.len() + 4);
        let sectors = (p.len() + 4).div_ceil(SECTOR as usize);
        if sectors > MAX_SECTORS {
            // Too big for the region: store externally and keep a stub with the flag set.
            let (rx, rz) = region_coords(path).ok_or(RegionError::Corrupt("region file name"))?;
            let (x, z) = ((i & 31) as i32, (i >> 5) as i32);
            let ext = path.with_file_name(format!("c.{}.{}.mcc", rx * 32 + x, rz * 32 + z));
            std::fs::write(ext, &p[1..])?;
            record.extend_from_slice(&1u32.to_be_bytes());
            record.push(p[0] | EXTERNAL_FLAG);
        } else {
            record.extend_from_slice(&(p.len() as u32).to_be_bytes());
            record.extend_from_slice(p);
        }
        let count = record.len().div_ceil(SECTOR as usize);
        record.resize(count * SECTOR as usize, 0);
        header[i * 4..i * 4 + 4].copy_from_slice(&((sector << 8) | count as u32).to_be_bytes());
        header[4096 + i * 4..4096 + i * 4 + 4].copy_from_slice(&stamps[i].to_be_bytes());
        body.extend_from_slice(&record);
        sector += count as u32;
    }
    let tmp = path.with_extension("mca.tmp");
    {
        let mut f = File::create(&tmp)?;
        use std::io::Write;
        f.write_all(&header)?;
        f.write_all(&body)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Region coordinates from a `r.<x>.<z>.mca` file name.
fn region_coords(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_name()?.to_str()?;
    let mut parts = name.strip_prefix("r.")?.strip_suffix(".mca")?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn read_capped(mut r: impl Read) -> Result<Vec<u8>, RegionError> {
    let mut out = Vec::new();
    r.by_ref().take(MAX_CHUNK_NBT as u64 + 1).read_to_end(&mut out)?;
    if out.len() > MAX_CHUNK_NBT {
        return Err(RegionError::TooLarge);
    }
    Ok(out)
}

fn decompress(kind: u8, data: &[u8]) -> Result<Vec<u8>, RegionError> {
    match kind {
        1 => read_capped(flate2::read::GzDecoder::new(data)),
        2 => read_capped(flate2::read::ZlibDecoder::new(data)),
        3 => Ok(data.to_vec()),
        4 => lz4_block_stream(data),
        k => Err(RegionError::Compression(k)),
    }
}

/// lz4-java's `LZ4BlockOutputStream` format: blocks of
/// "LZ4Block" | token | compressed len (LE) | decompressed len (LE) | checksum | data.
fn lz4_block_stream(mut data: &[u8]) -> Result<Vec<u8>, RegionError> {
    const MAGIC: &[u8] = b"LZ4Block";
    const RAW: u8 = 0x10;
    const LZ4: u8 = 0x20;
    let mut out = Vec::new();
    while !data.is_empty() {
        if data.len() < 21 || &data[..8] != MAGIC {
            return Err(RegionError::Corrupt("bad LZ4 block header"));
        }
        let token = data[8];
        let clen = u32::from_le_bytes(data[9..13].try_into().unwrap()) as usize;
        let dlen = u32::from_le_bytes(data[13..17].try_into().unwrap()) as usize;
        data = &data[21..];
        if clen > data.len() || out.len() + dlen > MAX_CHUNK_NBT {
            return Err(RegionError::Corrupt("LZ4 block length"));
        }
        if dlen == 0 {
            break; // end-of-stream marker
        }
        let block = &data[..clen];
        match token & 0xf0 {
            RAW => out.extend_from_slice(block),
            LZ4 => {
                let start = out.len();
                out.resize(start + dlen, 0);
                let n = lz4_flex::block::decompress_into(block, &mut out[start..])
                    .map_err(|_| RegionError::Corrupt("LZ4 data"))?;
                if n != dlen {
                    return Err(RegionError::Corrupt("LZ4 length mismatch"));
                }
            }
            _ => return Err(RegionError::Corrupt("LZ4 block method")),
        }
        data = &data[clen..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_name_parsing() {
        assert_eq!(region_coords(Path::new("x/r.-1.2.mca")), Some((-1, 2)));
        assert_eq!(region_coords(Path::new("r.0.0.mcc")), None);
    }

    #[test]
    fn lz4_block_stream_roundtrip() {
        let payload: Vec<u8> = (0..10_000).map(|i| (i % 7) as u8).collect();
        let compressed = lz4_flex::block::compress(&payload);
        let mut stream = b"LZ4Block".to_vec();
        stream.push(0x20);
        stream.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        stream.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        stream.extend_from_slice(&0u32.to_le_bytes());
        stream.extend_from_slice(&compressed);
        stream.extend_from_slice(b"LZ4Block");
        stream.push(0x10);
        stream.extend_from_slice(&[0; 12]);
        assert_eq!(lz4_block_stream(&stream).unwrap(), payload);
    }
}
