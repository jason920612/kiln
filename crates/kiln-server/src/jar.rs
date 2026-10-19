//! Reading a file out of the vanilla server jar (a zip), for the texts the console prints:
//! `assets/minecraft/lang/en_us.json`. The jar is the user's own (Kiln ships no Mojang data); it
//! is found as `KILN_JAR`, or `server.jar` and `versions/*/server-*.jar` in the working directory
//! and in `KILN_WORK`. A bundler jar (the `server.jar` of a download) holds the real one in
//! `META-INF/versions/<version>/server-<version>.jar`.

use flate2::read::DeflateDecoder;
use std::io::Read;
use std::path::{Path, PathBuf};

const LANG_ENTRY: &str = "assets/minecraft/lang/en_us.json";

fn u16_at(b: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize)
}

fn u32_at(b: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize)
}

/// One entry of a zip held in memory (stored or deflated), matched by `pick`.
pub fn read_entry(zip: &[u8], pick: impl Fn(&str) -> bool) -> Option<(String, Vec<u8>)> {
    // The end of central directory record: signature 0x06054b50, within the last 64 KiB.
    let tail = zip.len().saturating_sub(65_557);
    let eocd = (tail..zip.len().checked_sub(22)? + 1).rev().find(|&i| zip.get(i..i + 4) == Some(&[0x50, 0x4b, 0x05, 0x06]))?;
    let entries = u16_at(zip, eocd + 10)?;
    let mut at = u32_at(zip, eocd + 16)?;
    for _ in 0..entries {
        if zip.get(at..at + 4)? != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let method = u16_at(zip, at + 10)?;
        let compressed = u32_at(zip, at + 20)?;
        let name_len = u16_at(zip, at + 28)?;
        let extra_len = u16_at(zip, at + 30)?;
        let comment_len = u16_at(zip, at + 32)?;
        let local = u32_at(zip, at + 42)?;
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + name_len)?).into_owned();
        if pick(&name) {
            // The local header repeats the name and extra field lengths (they may differ from the central ones).
            let data_at = local + 30 + u16_at(zip, local + 26)? + u16_at(zip, local + 28)?;
            let raw = zip.get(data_at..data_at + compressed)?;
            let bytes = match method {
                0 => raw.to_vec(),
                8 => {
                    let mut out = Vec::new();
                    DeflateDecoder::new(raw).read_to_end(&mut out).ok()?;
                    out
                }
                _ => return None,
            };
            return Some((name, bytes));
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    None
}

/// The text of `en_us.json` from the jar at `path` (a bundler jar's inner server jar included).
pub fn lang_from_jar(path: &Path) -> Option<String> {
    let zip = std::fs::read(path).ok()?;
    if let Some((_, json)) = read_entry(&zip, |n| n == LANG_ENTRY) {
        return String::from_utf8(json).ok();
    }
    let (_, inner) = read_entry(&zip, |n| n.starts_with("META-INF/versions/") && n.ends_with(".jar"))?;
    let (_, json) = read_entry(&inner, |n| n == LANG_ENTRY)?;
    String::from_utf8(json).ok()
}

/// Where a server jar may be: `KILN_JAR`, then the working directory, then `KILN_WORK`.
pub fn candidate_jars(var: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = var("KILN_JAR").map(PathBuf::from).into_iter().collect();
    let mut dirs = vec![PathBuf::from(".")];
    dirs.extend(var("KILN_WORK").map(PathBuf::from));
    for dir in dirs {
        // The unpacked jar of a bundler run first: it is the one that has the assets directly.
        if let Ok(versions) = std::fs::read_dir(dir.join("versions")) {
            let mut found: Vec<PathBuf> = versions
                .filter_map(Result::ok)
                .flat_map(|v| std::fs::read_dir(v.path()).into_iter().flatten().filter_map(Result::ok).map(|f| f.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "jar"))
                .collect();
            found.sort();
            out.extend(found);
        }
        out.push(dir.join("server.jar"));
    }
    out
}

/// The vanilla language file: `KILN_LANG` (a json file), `lang/en_us.json`, or the server jar's.
pub fn find_language(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    for path in var("KILN_LANG").into_iter().chain(["lang/en_us.json".to_owned()]) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Some(text);
        }
    }
    candidate_jars(var).iter().find_map(|jar| lang_from_jar(jar))
}

#[cfg(test)]
mod tests;
