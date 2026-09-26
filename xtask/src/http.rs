//! HTTPS downloads with SHA-1 verification.

use anyhow::{Context, Result, bail, ensure};
use sha1::{Digest, Sha1};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Identifies the tool only; never add user or machine details here.
const USER_AGENT: &str = concat!("kiln-xtask/", env!("CARGO_PKG_VERSION"));

pub struct Http(ureq::Agent);

impl Http {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(600)))
            .build();
        Self(config.into())
    }

    /// Fetches a small document (metadata, wiki pages).
    pub fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut resp = self.0.get(url).call().with_context(|| format!("GET {url}"))?;
        let body = resp.body_mut().with_config().limit(64 << 20).read_to_vec();
        body.with_context(|| format!("GET {url}"))
    }

    /// Streams `url` into `dest` and moves it into place only if the SHA-1 matches.
    pub fn download(&self, url: &str, dest: &Path, sha1: &str) -> Result<()> {
        let mut part = dest.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        let mut resp = self.0.get(url).call().with_context(|| format!("GET {url}"))?;
        let mut body = resp.body_mut().as_reader();
        let mut file = File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        let mut hasher = Sha1::new();
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = body.read(&mut buf).with_context(|| format!("GET {url}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
        }
        file.sync_all()?;
        drop(file);
        let got = hex(&hasher.finalize());
        if got != sha1 {
            let _ = fs::remove_file(&part);
            bail!("{url}: SHA-1 {got}, expected {sha1}");
        }
        fs::rename(&part, dest).with_context(|| format!("moving {} into place", part.display()))
    }
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha1::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

/// Checks a downloaded document against the SHA-1 published next to its URL.
pub fn verify_sha1(what: &str, bytes: &[u8], expected: &str) -> Result<()> {
    let got = sha1_hex(bytes);
    ensure!(got == expected, "{what}: SHA-1 {got}, expected {expected}");
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_known_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert!(verify_sha1("abc", b"abc", "a9993e364706816aba3e25717850c26c9cd0d89d").is_ok());
        assert!(verify_sha1("abc", b"abd", "a9993e364706816aba3e25717850c26c9cd0d89d").is_err());
    }
}
