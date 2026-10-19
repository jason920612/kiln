use super::*;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::io::Write;

/// A zip with the given entries: (name, bytes, deflate).
fn zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data, deflate) in entries {
        let body = if *deflate {
            let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else {
            data.to_vec()
        };
        let offset = out.len() as u32;
        let method: u16 = if *deflate { 8 } else { 0 };
        // local header
        out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        out.extend_from_slice(&[20, 0, 0, 0]);
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // time, date, crc
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);
        // central directory entry
        central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0]);
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&(body.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&[0; 4]); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
fn reads_stored_and_deflated_entries() {
    let z = zip(&[("a.txt", b"hello", false), ("dir/b.txt", &b"deflated ".repeat(100), true)]);
    assert_eq!(read_entry(&z, |n| n == "a.txt").unwrap().1, b"hello");
    assert_eq!(read_entry(&z, |n| n == "dir/b.txt").unwrap().1, b"deflated ".repeat(100));
    assert!(read_entry(&z, |n| n == "missing").is_none());
    assert!(read_entry(b"not a zip", |_| true).is_none());
}

#[test]
fn finds_the_language_in_a_jar_and_in_a_bundler_jar() {
    let lang = br#"{"commands.list.players":"There are %s of a max of %s players online: %s"}"#;
    let dir = std::env::temp_dir().join(format!("kiln-jar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = dir.join("server-26.3.jar");
    std::fs::write(&plain, zip(&[("assets/minecraft/lang/en_us.json", lang, true)])).unwrap();
    assert_eq!(lang_from_jar(&plain).unwrap().as_bytes(), lang);
    let inner = zip(&[("assets/minecraft/lang/en_us.json", lang, true)]);
    let bundler = dir.join("server.jar");
    std::fs::write(&bundler, zip(&[("META-INF/versions/26.3/server-26.3.jar", &inner, false)])).unwrap();
    assert_eq!(lang_from_jar(&bundler).unwrap().as_bytes(), lang);
    // find_language: KILN_JAR wins; a missing file falls through to the next candidate.
    let found = find_language(|k| (k == "KILN_JAR").then(|| bundler.display().to_string()));
    assert_eq!(found.unwrap().as_bytes(), lang);
    std::fs::remove_dir_all(&dir).unwrap();
}
