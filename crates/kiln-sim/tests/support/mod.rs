//! Shared by the wp83 data pack tests: a world directory holding copies of the test packs of
//! `tools/wp83/packs`, and decoders for the packets that carry registries and tags.
#![allow(dead_code)]

use bytes::Bytes;
use kiln_proto::Reader;
use kiln_proto::nbt::Tag;
use std::path::{Path, PathBuf};

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let path = e.path();
        let target = to.join(e.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// The directory of the test pack `name` under `tools/wp83/packs`.
pub fn pack_source(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/wp83/packs").join(name)
}

/// A fresh world directory whose `datapacks` holds copies of the packs.
pub fn pack_world(tag: &str, packs: &[&str]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("wp83-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    for pack in packs {
        copy_dir(&pack_source(pack), &dir.join("datapacks").join(pack));
    }
    dir
}

/// A `Registry Data` packet: `(registry, [(entry, data)])`.
pub fn registry_data(p: &Bytes) -> (String, Vec<(String, Option<Tag>)>) {
    let mut r = Reader::new(p);
    assert_eq!(r.varint().unwrap(), kiln_data::packets::configuration::clientbound::REGISTRY_DATA);
    let registry = r.string(32767).unwrap().to_owned();
    let n = r.len().unwrap();
    let mut entries = Vec::new();
    for _ in 0..n {
        let name = r.string(32767).unwrap().to_owned();
        let data = if r.bool().unwrap() {
            let at = p.len() - r.remaining();
            let (tag, used) = kiln_proto::nbt::read_network(&p[at..]).unwrap();
            r.bytes(used).unwrap();
            Some(tag)
        } else {
            None
        };
        entries.push((name, data));
    }
    r.finish().unwrap();
    (registry, entries)
}

/// An `Update Tags` packet (configuration or play): `registry → tag → ids`.
pub fn update_tags(p: &Bytes) -> Vec<(String, Vec<(String, Vec<i32>)>)> {
    let mut r = Reader::new(p);
    let id = r.varint().unwrap();
    assert!(
        id == kiln_data::packets::configuration::clientbound::UPDATE_TAGS || id == kiln_data::packets::play::clientbound::UPDATE_TAGS,
        "packet {id} is not Update Tags"
    );
    let mut out = Vec::new();
    for _ in 0..r.len().unwrap() {
        let registry = r.string(32767).unwrap().to_owned();
        let mut tags = Vec::new();
        for _ in 0..r.len().unwrap() {
            let tag = r.string(32767).unwrap().to_owned();
            let ids = (0..r.len().unwrap()).map(|_| r.varint().unwrap()).collect();
            tags.push((tag, ids));
        }
        out.push((registry, tags));
    }
    r.finish().unwrap();
    out
}

pub fn tag_ids<'a>(tags: &'a [(String, Vec<(String, Vec<i32>)>)], registry: &str, tag: &str) -> Option<&'a [i32]> {
    tags.iter().find(|(r, _)| r == registry)?.1.iter().find(|(t, _)| t == tag).map(|(_, ids)| ids.as_slice())
}
