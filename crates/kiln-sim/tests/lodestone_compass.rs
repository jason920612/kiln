//! wp80 (found with a real client beside vanilla: a compass used on a lodestone did nothing in Kiln; vanilla plays the lock sound, makes one compass of the stack a
//! lodestone compass, and a tracked compass forgets its target once the lodestone is gone): `CompassItem.useOn` and `LodestoneTracker.tick`.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    seq: i32,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode survival Keeper".into())]));
        let mut w = World { sim, client: Client::new(1, stats), seq: 0 };
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, c: &str) {
        assert!(self.sim.step([ToSim::Console(c.into())]));
    }

    fn compasses(&mut self) -> Vec<Tag> {
        let nbt = self.sim.saved_player_nbt(1).expect("player data");
        match nbt.get("Inventory") {
            Some(Tag::List(items)) => items.iter().filter(|i| i.get("id").and_then(Tag::as_str) == Some("minecraft:compass")).cloned().collect(),
            _ => Vec::new(),
        }
    }
}

#[test]
fn a_compass_used_on_a_lodestone_locks_onto_it_and_forgets_it_when_the_lodestone_goes() {
    let mut w = World::new();
    let p = w.client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:air", x - 1, y - 1, z - 1, x + 1, y + 2, z + 1));
    w.run(&format!("setblock {x} {} {z} minecraft:stone", y - 1));
    w.run(&format!("setblock {x} {y} {z} minecraft:lodestone"));
    w.run("give Keeper minecraft:compass 2");
    w.ticks(2);
    w.seq += 1;
    let pkt = PlayIn::UseItemOn { hand: 0, pos: [x, y, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: w.seq };
    assert!(w.sim.step([ToSim::Packet(1, pkt)]));
    w.ticks(2);
    let compasses = w.compasses();
    let tracked: Vec<_> = compasses.iter().filter(|c| c.get("components").and_then(|k| k.get("minecraft:lodestone_tracker")).is_some()).collect();
    assert_eq!(compasses.len(), 2, "one compass split off the stack of two: {compasses:?}");
    assert_eq!(tracked.len(), 1, "exactly one of them tracks: {compasses:?}");
    let tracker = tracked[0].get("components").and_then(|k| k.get("minecraft:lodestone_tracker")).unwrap();
    assert!(tracker.get("target").is_some(), "it points at the lodestone: {tracker:?}");
    // The lodestone goes: the compass lets go of its target.
    w.run(&format!("setblock {x} {y} {z} minecraft:air"));
    w.ticks(3);
    let compasses = w.compasses();
    let tracker = compasses.iter().find_map(|c| c.get("components").and_then(|k| k.get("minecraft:lodestone_tracker"))).expect("still a lodestone compass");
    assert!(tracker.get("target").is_none(), "no target without the lodestone: {tracker:?}");
}
