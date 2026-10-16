//! A value that goes back to its default is sent to the viewers (wp68). The mobs' entity data lists only the values that differ
//! from the default (as a pairing packet does), and the packet for a change carried what was listed now: a baby that grew up
//! simply lacked its `baby` entry, so no client was ever told, and every animal, villager and ghast stayed a baby on screen until
//! its viewer left and came back. Found with a real client: the server said `Age: 0` and the client said `baby: true`.

use bytes::Bytes;
use kiln_data::packets::play::clientbound as id;
use kiln_link::ToSim;
use kiln_proto::codec::Reader;
use kiln_sim::testing::{Client, SinkStats, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::Arc;

fn data_of(log: &[Bytes], entity: i32) -> Vec<Vec<u8>> {
    log.iter()
        .filter(|p| Reader::new(p).varint().unwrap() == id::SET_ENTITY_DATA)
        .filter_map(|p| {
            let mut r = Reader::new(p);
            r.varint().ok()?;
            (r.varint().ok()? == entity).then(|| p[p.len() - r.remaining()..].to_vec())
        })
        .collect()
}

fn has(entries: &[u8], needle: &[u8]) -> bool {
    entries.windows(needle.len()).any(|w| w == needle)
}

struct World {
    sim: Sim,
    client: Client,
    stats: Arc<SinkStats>,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut w = World { sim, client: Client::new(1, stats.clone()), stats };
        w.ticks(8);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn log(&self) -> Vec<Bytes> {
        self.stats.log.lock().unwrap().clone().unwrap()
    }
}

#[test]
fn a_baby_that_grows_up_is_told_to_its_viewers() {
    let (baby, boolean) = (16u8, 8u8);
    for kind in ["cow", "pig", "horse", "happy_ghast", "villager", "sulfur_cube"] {
        let mut w = World::new();
        let p = w.client.pos;
        w.run(&format!("summon minecraft:{kind} {} {} {} {{Age:-30,PersistenceRequired:1b}}", p[0] + 3.0, p[1], p[2]));
        w.ticks(2);
        let id = w.sim.entity_ids_of(&format!("minecraft:{kind}"))[0];
        let paired = data_of(&w.log(), id);
        assert!(paired.iter().any(|d| has(d, &[baby, boolean, 1])), "{kind}: paired as a baby: {paired:?}");
        w.ticks(40);
        let all = data_of(&w.log(), id);
        assert!(all.iter().any(|d| has(d, &[baby, boolean, 0])), "{kind}: told that it is no baby any more: {all:?}");
    }
}
