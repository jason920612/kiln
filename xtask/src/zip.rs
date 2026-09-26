//! Just enough of the zip format to list and read entries of Mojang's jars
//! (stored or deflated entries, no zip64, no encryption).

use anyhow::{Context, Result, bail, ensure};
use std::io::Read;
use std::path::Path;

pub struct Zip {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

struct Entry {
    name: String,
    method: u16,
    crc: u32,
    compressed: usize,
    size: usize,
    header: usize,
}

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

fn u16_at(d: &[u8], at: usize) -> Result<u16> {
    let b = d.get(at..at + 2).context("truncated zip")?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32> {
    let b = d.get(at..at + 4).context("truncated zip")?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl Zip {
    pub fn open(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::new(data).with_context(|| format!("reading {}", path.display()))
    }

    pub fn new(data: Vec<u8>) -> Result<Self> {
        // The end of central directory record sits in the last 22 + 65535 (comment) bytes.
        let min = data.len().saturating_sub(22 + 0xffff);
        let eocd = (min..=data.len().saturating_sub(22))
            .rev()
            .find(|&i| u32_at(&data, i).is_ok_and(|s| s == EOCD_SIG))
            .context("not a zip file")?;
        let count = u16_at(&data, eocd + 10)?;
        let offset = u32_at(&data, eocd + 16)?;
        if count == 0xffff || offset == 0xffff_ffff {
            bail!("zip64 archives are not supported");
        }

        let mut entries = Vec::with_capacity(count as usize);
        let mut at = offset as usize;
        for _ in 0..count {
            ensure!(u32_at(&data, at)? == CENTRAL_SIG, "bad central directory entry");
            let name_len = u16_at(&data, at + 28)? as usize;
            let extra_len = u16_at(&data, at + 30)? as usize;
            let comment_len = u16_at(&data, at + 32)? as usize;
            let name = data.get(at + 46..at + 46 + name_len).context("truncated zip")?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method: u16_at(&data, at + 10)?,
                crc: u32_at(&data, at + 16)?,
                compressed: u32_at(&data, at + 20)? as usize,
                size: u32_at(&data, at + 24)? as usize,
                header: u32_at(&data, at + 42)? as usize,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { data, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = self.entries.iter().find(|e| e.name == name).with_context(|| format!("no {name} in zip"))?;
        let h = e.header;
        ensure!(u32_at(&self.data, h)? == LOCAL_SIG, "bad local header for {name}");
        let start = h + 30 + u16_at(&self.data, h + 26)? as usize + u16_at(&self.data, h + 28)? as usize;
        let raw = self.data.get(start..start + e.compressed).context("truncated zip")?;
        let out = match e.method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(e.size);
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut out)?;
                out
            }
            m => bail!("{name}: unsupported compression method {m}"),
        };
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        ensure!(out.len() == e.size && crc.sum() == e.crc, "{name}: size or CRC mismatch");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a zip the way `jar`/`zip` tools lay it out.
    fn build(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, content, deflate) in files {
            let body = if *deflate {
                let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(content).unwrap();
                enc.finish().unwrap()
            } else {
                content.to_vec()
            };
            let mut crc = flate2::Crc::new();
            crc.update(content);
            let method: u16 = if *deflate { 8 } else { 0 };
            let header = out.len() as u32;
            let common = |v: &mut Vec<u8>| {
                v.extend(method.to_le_bytes());
                v.extend([0u8; 4]); // time, date
                v.extend(crc.sum().to_le_bytes());
                v.extend((body.len() as u32).to_le_bytes());
                v.extend((content.len() as u32).to_le_bytes());
                v.extend((name.len() as u16).to_le_bytes());
            };
            out.extend(LOCAL_SIG.to_le_bytes());
            out.extend([20, 0, 0, 0]); // version needed, flags
            common(&mut out);
            out.extend(3u16.to_le_bytes()); // extra field, skipped by readers
            out.extend(name.as_bytes());
            out.extend([0xaa; 3]);
            out.extend(&body);

            central.extend(CENTRAL_SIG.to_le_bytes());
            central.extend([20, 0, 20, 0, 0, 0]); // version made by, needed, flags
            common(&mut central);
            central.extend([0u8; 12]); // extra, comment, disk, attributes
            central.extend(header.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend(&central);
        out.extend(EOCD_SIG.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend(4u16.to_le_bytes());
        out.extend(b"note");
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let json = br#"{"id": "26.3", "protocol_version": 777}"#.repeat(20);
        let zip = Zip::new(build(&[
            ("META-INF/versions.list", b"abc\t26.3\t26.3/server-26.3.jar", false),
            ("version.json", &json, true),
        ]))
        .unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["META-INF/versions.list", "version.json"]);
        assert_eq!(zip.read("META-INF/versions.list").unwrap(), b"abc\t26.3\t26.3/server-26.3.jar");
        assert_eq!(zip.read("version.json").unwrap(), json);
        assert!(zip.read("missing").is_err());
    }

    #[test]
    fn detects_corruption() {
        let mut data = build(&[("a.txt", b"hello world", false)]);
        let at = data.windows(11).position(|w| w == b"hello world").unwrap();
        data[at] = b'j';
        assert!(Zip::new(data).unwrap().read("a.txt").is_err());
        assert!(Zip::new(b"not a zip at all, just some bytes".to_vec()).is_err());
    }
}
