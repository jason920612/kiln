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

/// A fresh world directory whose `datapacks` holds copies of the wp85 packs (`tools/wp85/packs`).
pub fn wp85_world(tag: &str, packs: &[&str]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("wp85-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    for pack in packs {
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/wp85/packs").join(pack), &dir.join("datapacks").join(pack));
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

/// A world with the test packs and one player, for tests that play.
pub struct Game {
    pub sim: kiln_sim::Sim,
    pub client: kiln_sim::testing::Client,
    pub dir: PathBuf,
    pub sync: std::sync::Arc<kiln_link::DataSync>,
}

impl Game {
    /// The player `name` (connection 1) in survival, spawn mobs off, after a few ticks.
    pub fn new(tag: &str, packs: &[&str], name: &str) -> Game {
        let dir = pack_world(tag, packs);
        let config = kiln_sim::SimConfig::new(4, 4, Some(dir.clone()));
        let sync = config.data_sync.clone();
        let mut sim = kiln_sim::Sim::new(config);
        let (msg, stats) = kiln_sim::testing::join(1, name, 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg]));
        let mut game = Game { sim, client: kiln_sim::testing::Client::new(1, stats), dir, sync };
        game.console("gamerule minecraft:spawn_mobs false");
        game.console(&format!("gamemode survival {name}"));
        game.ticks(5);
        game
    }

    pub fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    pub fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([kiln_link::ToSim::Console(cmd.into())]), "{cmd}");
    }

    /// The packets sent since the last call.
    pub fn log(&mut self) -> Vec<Bytes> {
        std::mem::take(self.client.stats.log.lock().unwrap().as_mut().unwrap())
    }

    /// The block position next to the player, `dx` east.
    pub fn spot(&self, dx: i32) -> [i32; 3] {
        let p = self.client.pos;
        [p[0].floor() as i32 + dx, p[1].floor() as i32, p[2].floor() as i32]
    }

    pub fn cleanup(self) {
        let dir = self.dir.clone();
        drop(self);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The `(event, data)` of the Level Event packets in `log`.
pub fn level_events(log: &[Bytes]) -> Vec<(i32, i32)> {
    log.iter()
        .filter_map(|p| {
            let mut r = Reader::new(p);
            (r.varint().ok()? == kiln_data::packets::play::clientbound::LEVEL_EVENT).then(|| {
                let event = r.i32().unwrap();
                let _pos = r.i64().unwrap();
                (event, r.i32().unwrap())
            })
        })
        .collect()
}

/// Whether the simulation finds the vanilla data (the loot tables and the like) where it looks for it: `KILN_DATAPACK`, else
/// `work/generated` under the current directory (`kiln_sim::datapack_dir`; a test runs in its crate's directory, so the checkout's
/// own `work/generated` is not read without `KILN_DATAPACK`).
pub fn vanilla_data() -> bool {
    let root = std::env::var_os("KILN_DATAPACK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("work/generated"));
    root.join("data/minecraft/loot_table").is_dir()
}
