//! `cargo xtask completeness`: the protocol completeness list.
//!
//! `crates/kiln-proto/protocol.toml` classifies every packet of the generated `packets.json` as
//! implemented, ignored (received and deliberately dropped) or deferred. The check fails on
//! packets missing from the list and on entries for packets that no longer exist.

use crate::codegen::const_name;
use crate::read_json;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

pub const FILE: &str = "crates/kiln-proto/protocol.toml";

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Implemented,
    Ignored,
    Deferred,
}

const STATUSES: [Status; 3] = [Status::Implemented, Status::Ignored, Status::Deferred];

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub status: Status,
    pub note: String,
}

/// State -> direction -> packet name (without the `minecraft:` namespace) -> entry.
pub type Classification = BTreeMap<String, BTreeMap<String, BTreeMap<String, Entry>>>;

pub fn load(root: &Path) -> Result<Classification> {
    let path = root.join(FILE);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub state: String,
    pub dir: String,
    pub name: String,
    pub id: i64,
}

impl Packet {
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.state, self.dir, self.name)
    }

    pub fn entry<'a>(&self, c: &'a Classification) -> Option<&'a Entry> {
        c.get(&self.state)?.get(&self.dir)?.get(&self.name)
    }
}

/// Connection states in the order a client goes through them.
const STATE_ORDER: [&str; 5] = ["handshake", "status", "login", "configuration", "play"];

/// Every packet of a `packets.json` report, ordered by state, direction and id.
pub fn packets(report: &Value) -> Result<Vec<Packet>> {
    let mut out = Vec::new();
    for (state, dirs) in report.as_object().context("packets.json: states")? {
        for (dir, list) in dirs.as_object().context("packets.json: directions")? {
            for (name, body) in list.as_object().context("packets.json: packets")? {
                out.push(Packet {
                    state: state.clone(),
                    dir: dir.clone(),
                    name: name.strip_prefix("minecraft:").unwrap_or(name).to_string(),
                    id: body["protocol_id"].as_i64().with_context(|| format!("{name}: protocol_id"))?,
                });
            }
        }
    }
    let rank = |s: &str| STATE_ORDER.iter().position(|o| *o == s).unwrap_or(STATE_ORDER.len());
    out.sort_by(|a, b| (rank(&a.state), &a.state, &a.dir, a.id).cmp(&(rank(&b.state), &b.state, &b.dir, b.id)));
    Ok(out)
}

/// Unclassified packets, entries for packets that do not exist, and entries without a note.
pub fn problems(packets: &[Packet], c: &Classification) -> Vec<String> {
    let mut out = Vec::new();
    for p in packets {
        match p.entry(c) {
            None => out.push(format!("unclassified: {} (id {})", p.key(), p.id)),
            Some(e) if e.note.trim().is_empty() => out.push(format!("no note: {}", p.key())),
            Some(_) => {}
        }
    }
    let known: HashSet<String> = packets.iter().map(Packet::key).collect();
    for (state, dirs) in c {
        for (dir, names) in dirs {
            for name in names.keys() {
                let key = format!("{state}/{dir}/{name}");
                if !known.contains(&key) {
                    out.push(format!("stale: {key} is not in packets.json"));
                }
            }
        }
    }
    out
}

pub fn run(root: &Path, work: &Path) -> Result<()> {
    let report = read_json(&work.join("generated/reports/packets.json")).context("run `cargo xtask fetch` first")?;
    let version = read_json(&work.join("server_version.json")).ok();
    let packets = packets(&report)?;
    let classification = load(root)?;

    let label = version.as_ref().map_or("unknown version".to_string(), |v| {
        format!("{} (protocol {})", v["name"].as_str().unwrap_or("?"), v["protocol_version"])
    });
    println!("completeness: {label}, {} packets, classified in {FILE}", packets.len());
    println!("{:<15}{:<13}{:>12}{:>9}{:>10}", "state", "direction", "implemented", "ignored", "deferred");
    let mut rows: Vec<((&str, &str), [usize; 3])> = Vec::new();
    let mut total = [0; 3];
    for p in &packets {
        let group = (p.state.as_str(), p.dir.as_str());
        if rows.last().is_none_or(|(g, _)| *g != group) {
            rows.push((group, [0; 3]));
        }
        if let Some(e) = p.entry(&classification) {
            let i = STATUSES.iter().position(|s| *s == e.status).unwrap();
            rows.last_mut().unwrap().1[i] += 1;
            total[i] += 1;
        }
    }
    for ((state, dir), n) in &rows {
        println!("{state:<15}{dir:<13}{:>12}{:>9}{:>10}", n[0], n[1], n[2]);
    }
    println!("{:<28}{:>12}{:>9}{:>10}", "total", total[0], total[1], total[2]);

    for w in unreferenced(root, &packets, &classification)? {
        println!("warning: {w}");
    }
    let problems = problems(&packets, &classification);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("error: {p}");
        }
        bail!("{} problem(s) in {FILE}", problems.len());
    }
    println!("completeness: ok");
    Ok(())
}

/// Implemented packets whose `kiln-data` constant appears nowhere in the crates' sources: a cheap
/// guard against marking a packet implemented by mistake.
fn unreferenced(root: &Path, packets: &[Packet], c: &Classification) -> Result<Vec<String>> {
    let mut idents = HashSet::new();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files)?;
    for f in files {
        let text = fs::read_to_string(&f)?;
        idents.extend(text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(str::to_owned));
    }
    Ok(packets
        .iter()
        .filter(|p| p.entry(c).is_some_and(|e| e.status == Status::Implemented))
        .filter(|p| !idents.contains(&const_name(&p.name)))
        .map(|p| format!("{} is implemented but {} is not referenced in crates/", p.key(), const_name(&p.name)))
        .collect())
}

/// Rust sources under `dir`, skipping generated tables.
fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            if !(p.ends_with("gen") || p.ends_with("target")) {
                rust_files(&p, out)?;
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Value {
        serde_json::json!({
            "handshake": { "serverbound": { "minecraft:intention": { "protocol_id": 0 } } },
            "play": { "clientbound": {
                "minecraft:login": { "protocol_id": 1 },
                "minecraft:bundle_delimiter": { "protocol_id": 0 },
                "minecraft:debug/block_value": { "protocol_id": 2 },
            } },
        })
    }

    #[test]
    fn lists_packets_in_id_order() {
        let names: Vec<String> = packets(&report()).unwrap().iter().map(Packet::key).collect();
        assert_eq!(
            names,
            [
                "handshake/serverbound/intention",
                "play/clientbound/bundle_delimiter",
                "play/clientbound/login",
                "play/clientbound/debug/block_value"
            ]
        );
    }

    #[test]
    fn complete_list_has_no_problems() {
        let c: Classification = toml::from_str(
            r#"
            [handshake.serverbound]
            intention = { status = "implemented", note = "handshake" }
            [play.clientbound]
            bundle_delimiter = { status = "deferred", note = "entity bundles" }
            login = { status = "implemented", note = "join" }
            "debug/block_value" = { status = "ignored", note = "debug only" }
            "#,
        )
        .unwrap();
        assert_eq!(problems(&packets(&report()).unwrap(), &c), Vec::<String>::new());
    }

    #[test]
    fn reports_unclassified_stale_and_empty_notes() {
        let c: Classification = toml::from_str(
            r#"
            [handshake.serverbound]
            intention = { status = "implemented", note = "" }
            [play.clientbound]
            login = { status = "implemented", note = "join" }
            "debug/block_value" = { status = "deferred", note = "debug" }
            removed_packet = { status = "deferred", note = "gone" }
            [play.serverbound]
            chat = { status = "implemented", note = "wrong direction" }
            "#,
        )
        .unwrap();
        assert_eq!(
            problems(&packets(&report()).unwrap(), &c),
            [
                "no note: handshake/serverbound/intention",
                "unclassified: play/clientbound/bundle_delimiter (id 0)",
                "stale: play/clientbound/removed_packet is not in packets.json",
                "stale: play/serverbound/chat is not in packets.json",
            ]
        );
    }

    #[test]
    fn rejects_unknown_statuses_and_fields() {
        let bad_status = r#"[play.clientbound]
            login = { status = "done", note = "x" }"#;
        assert!(toml::from_str::<Classification>(bad_status).is_err());
        let missing_note = r#"[play.clientbound]
            login = { status = "deferred" }"#;
        assert!(toml::from_str::<Classification>(missing_note).is_err());
        let extra = r#"[play.clientbound]
            login = { status = "deferred", note = "x", owner = "me" }"#;
        assert!(toml::from_str::<Classification>(extra).is_err());
    }

    #[test]
    fn protocol_toml_parses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let c = load(root).unwrap();
        assert!(c["play"]["clientbound"]["level_chunk_with_light"].status == Status::Implemented);
    }
}
