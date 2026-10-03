//! Jukeboxes: a music disc goes in (the clients get level event 1010 with the song, which is what
//! makes parrots dance), an empty hand takes it out (1011), a broken jukebox gives the disc back.

use bytes::Bytes;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::Reader;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    jukebox: [i32; 3],
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Bait", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats), jukebox: [0; 3] };
        w.console("gamerule minecraft:spawn_mobs false");
        w.console("gamemode survival Bait");
        w.ticks(5);
        let p = w.client.pos;
        w.jukebox = [p[0].floor() as i32 + 2, p[1].floor() as i32, p[2].floor() as i32];
        let [x, y, z] = w.jukebox;
        w.console(&format!("setblock {x} {y} {z} minecraft:jukebox"));
        w.console("give Bait minecraft:music_disc_13");
        w.ticks(2);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn use_jukebox(&mut self) {
        let pkt = PlayIn::UseItemOn { hand: 0, pos: self.jukebox, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 0 };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn has_record(&self) -> bool {
        let [x, y, z] = self.jukebox;
        kiln_blocks::state::get_bool(self.sim.block_at(x, y, z).expect("loaded"), "has_record")
    }

    /// The (event, data) of every Level Event packet sent since the last call.
    fn level_events(&mut self) -> Vec<(i32, i32)> {
        let log: Vec<Bytes> = std::mem::take(self.client.stats.log.lock().unwrap().as_mut().unwrap());
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

    fn items_on_ground(&self) -> Vec<String> {
        self.sim
            .entity_nbt()
            .into_iter()
            .filter(|t| t.get("id").and_then(|i| i.as_str()) == Some("minecraft:item"))
            .filter_map(|t| t.get("Item")?.get("id")?.as_str().map(str::to_owned))
            .collect()
    }

    fn held(&self) -> Option<String> {
        self.sim.inventory(1).unwrap()[36].map(|(id, _)| kiln_data::builtin_entries("minecraft:item").unwrap()[id as usize].to_string())
    }
}

#[test]
fn a_disc_goes_in_plays_and_comes_out_again() {
    let mut w = World::new();
    assert_eq!(w.held().as_deref(), Some("minecraft:music_disc_13"));
    w.level_events();
    w.use_jukebox();
    assert!(w.has_record(), "the jukebox holds the disc");
    assert_eq!(w.held(), None, "the disc left the hand");
    let events = w.level_events();
    let song = kiln_data::synced_id("minecraft:jukebox_song", "minecraft:13").expect("song") as i32;
    assert!(events.contains(&(1010, song)), "clients are told to play the song: {events:?}");

    // An empty hand takes it out.
    w.use_jukebox();
    assert!(!w.has_record());
    let events = w.level_events();
    assert!(events.iter().any(|e| e.0 == 1011), "and to stop it: {events:?}");
    w.ticks(3);
    assert_eq!(w.items_on_ground(), ["minecraft:music_disc_13"]);
}

#[test]
fn a_broken_jukebox_gives_the_disc_back() {
    let mut w = World::new();
    w.use_jukebox();
    assert!(w.has_record());
    let [x, y, z] = w.jukebox;
    w.level_events();
    w.console(&format!("setblock {x} {y} {z} minecraft:air destroy"));
    w.ticks(3);
    let mut items = w.items_on_ground();
    items.sort();
    assert_eq!(items, ["minecraft:jukebox", "minecraft:music_disc_13"]);
    assert!(w.level_events().iter().any(|e| e.0 == 1011));
}

/// A command that replaces the jukebox clears it first (`Clearable.tryClear`): the disc is gone
/// without dropping, and the music stops.
#[test]
fn a_jukebox_cleared_by_setblock_loses_its_disc_and_the_music_stops() {
    let mut w = World::new();
    w.use_jukebox();
    let [x, y, z] = w.jukebox;
    w.level_events();
    w.console(&format!("setblock {x} {y} {z} minecraft:air"));
    w.ticks(3);
    assert!(w.items_on_ground().is_empty());
    assert!(w.level_events().iter().any(|e| e.0 == 1011));
}

/// A playing jukebox saves how far its song has got, a finished song is not started again when
/// the block entity loads, and a song that runs out stops by itself with the disc staying
/// (`JukeboxSongPlayer.tick`: the length of the song and a second more). The songs' lengths are
/// the datapack's.
#[test]
fn a_song_ends_by_its_length_and_progress_is_saved() {
    let Some(dir) = std::env::var_os("KILN_DATAPACK") else {
        eprintln!("no vanilla datapack (KILN_DATAPACK): the song lengths are missing; skipped");
        return;
    };
    assert!(std::path::Path::new(&dir).join("data/minecraft/jukebox_song").is_dir());
    let mut w = World::new();
    let [x, y, z] = w.jukebox;
    w.console(&format!(
        r#"setblock {x} {y} {z} minecraft:jukebox[has_record=true]{{RecordItem:{{id:"minecraft:music_disc_11",count:1}},ticks_since_song_started:1400L}}"#
    ));
    w.ticks(2);
    let nbt = w.sim.block_entity_nbt(x, y, z).expect("jukebox block entity");
    let ticks = nbt.get("ticks_since_song_started").and_then(|t| t.as_i64()).expect("progress saved");
    assert!((1400..1410).contains(&ticks), "{ticks}");
    assert!(nbt.get("RecordItem").is_some());
    w.level_events();
    w.ticks(45);
    // The song is over (1420 ticks of music and a second more): the music stopped, the disc stayed.
    assert!(w.level_events().iter().any(|e| e.0 == 1011), "the clients are told to stop the music");
    assert!(w.has_record());
    let nbt = w.sim.block_entity_nbt(x, y, z).expect("jukebox block entity");
    assert!(nbt.get("ticks_since_song_started").is_none(), "{nbt:?}");
    assert!(nbt.get("RecordItem").is_some());
    // An empty hand still takes the disc out, with no music to stop.
    w.level_events();
    w.use_jukebox();
    assert!(!w.has_record());
    assert!(!w.level_events().iter().any(|e| e.0 == 1011));
}

/// A song that is over when it loads is not started (no stop, no power).
#[test]
fn a_finished_song_does_not_start_on_load() {
    let mut w = World::new();
    let [x, y, z] = w.jukebox;
    w.level_events();
    w.console(&format!(
        r#"setblock {x} {y} {z} minecraft:jukebox[has_record=true]{{RecordItem:{{id:"minecraft:music_disc_11",count:1}},ticks_since_song_started:100000L}}"#
    ));
    w.ticks(3);
    // (Without the datapack a song never ends; with it this one is long over.)
    let nbt = w.sim.block_entity_nbt(x, y, z).expect("jukebox block entity");
    if std::env::var_os("KILN_DATAPACK").is_some() {
        assert!(nbt.get("ticks_since_song_started").is_none(), "{nbt:?}");
        assert!(!w.level_events().iter().any(|e| e.0 == 1011));
    } else {
        assert!(nbt.get("ticks_since_song_started").is_some());
    }
}

/// A hopper above puts a disc in (it starts playing: level event 1010, `has_record`).
#[test]
fn a_hopper_puts_a_disc_in() {
    let mut w = World::new();
    let [x, y, z] = w.jukebox;
    w.level_events();
    w.console(&format!(
        r#"setblock {x} {} {z} minecraft:hopper[facing=down]{{Items:[{{Slot:2b,id:"minecraft:music_disc_13",count:1}}]}}"#,
        y + 1
    ));
    w.ticks(12);
    assert!(w.has_record(), "the hopper put the disc in");
    let song = kiln_data::synced_id("minecraft:jukebox_song", "minecraft:13").expect("song") as i32;
    let events = w.level_events();
    assert!(events.contains(&(1010, song)), "{events:?}");
    let nbt = w.sim.block_entity_nbt(x, y, z).expect("jukebox block entity");
    assert!(nbt.get("ticks_since_song_started").is_some(), "{nbt:?}");
}

/// A hopper under a jukebox whose song is over takes the disc out (`has_record` goes), while one
/// under a playing jukebox is locked by its power and takes nothing.
#[test]
fn a_hopper_takes_a_disc_out_unless_the_jukebox_locks_it() {
    let mut w = World::new();
    let [x, y, z] = w.jukebox;
    // The jukebox sits on a hopper that leads into a chest; its disc has long played out (without
    // datapack no song ends: the playing jukebox locks the hopper).
    w.console(&format!("setblock {x} {} {z} minecraft:chest", y - 2));
    w.console(&format!("setblock {x} {} {z} minecraft:hopper[facing=down]", y - 1));
    w.console(&format!("setblock {x} {y} {z} minecraft:air"));
    w.console(&format!(
        r#"setblock {x} {y} {z} minecraft:jukebox[has_record=true]{{RecordItem:{{id:"minecraft:music_disc_13",count:1}},ticks_since_song_started:100000L}}"#
    ));
    w.ticks(20);
    let datapack = std::env::var_os("KILN_DATAPACK").is_some();
    assert_eq!(w.has_record(), !datapack, "with the song over the hopper took the disc, else it stayed locked");
}
