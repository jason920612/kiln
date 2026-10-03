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
    w.console(&format!("setblock {x} {y} {z} minecraft:air"));
    w.ticks(3);
    assert_eq!(w.items_on_ground(), ["minecraft:music_disc_13"]);
    assert!(w.level_events().iter().any(|e| e.0 == 1011));
}
