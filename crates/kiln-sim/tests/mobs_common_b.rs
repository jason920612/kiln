//! Common mobs B end to end: golems built from blocks, fish scooped into buckets, mooshrooms
//! milked for stew and sheared, snow golems throwing snowballs. The behaviour itself is checked
//! tick by tick against vanilla by kiln-entity's `mob_parity` test (tools/mob_vectors.py).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    clients: Vec<Client>,
    sequence: i32,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let (msg, stats) = join(1, "Builder", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, clients: vec![Client::new(1, stats)], sequence: 0 };
        w.console("gamerule minecraft:spawn_mobs false");
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            for c in self.clients.iter_mut() {
                c.tick(None, &mut inbox);
            }
            assert!(self.sim.step(inbox));
        }
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn pos(&self) -> [f64; 3] {
        self.clients[0].pos
    }

    fn ground(&self) -> [i32; 3] {
        let p = self.pos();
        [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32]
    }

    fn summon(&mut self, entity: &str, offset: [f64; 3], nbt: &str) {
        let p = self.pos();
        self.console(format!("summon {entity} {} {} {} {nbt}", p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]).trim());
        self.ticks(1);
    }

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }

    fn hold(&mut self, item: &str, count: i32) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    fn held(&self) -> Option<(i32, i32)> {
        self.sim.inventory(1).unwrap()[36]
    }

    fn interact(&mut self, entity_id: i32) {
        let pkt = PlayIn::Interact { entity_id, hand: kiln_proto::packets::serverbound::Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    /// Right-clicks the top face of `pos` with the held item.
    fn use_on_top(&mut self, pos: [i32; 3]) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn item_id(name: &str) -> i32 {
        kiln_data::builtin_id("minecraft:item", name).unwrap()
    }
}

#[test]
fn pumpkins_on_snow_and_iron_make_golems() {
    let mut w = World::new();
    w.console("gamemode creative Builder");
    let g = w.ground();
    let (x, y, z) = (g[0] + 3, g[1] + 1, g[2]);
    w.console(&format!("setblock {x} {y} {z} minecraft:snow_block"));
    w.console(&format!("setblock {x} {} {z} minecraft:snow_block", y + 1));
    w.hold("minecraft:carved_pumpkin", 4);
    w.use_on_top([x, y + 1, z]);
    w.ticks(2);
    let golems = w.mobs("minecraft:snow_golem");
    assert_eq!(golems.len(), 1, "a snow golem");
    // The snow blocks are gone (the golem's trail may have put a snow layer where it stands).
    let at = |w: &World, y: i32| kiln_data::blocks_types::block_of(w.sim.block_at(x, y, z).unwrap()).name;
    assert_ne!(at(&w, y), "minecraft:snow_block", "the snow is gone");
    assert_eq!(at(&w, y + 2), "minecraft:air", "the pumpkin is gone");
    assert!((golems[0].1[1] - (y as f64 + 0.05)).abs() < 0.2, "at the bottom block ({:?})", golems[0].1);
    // An iron golem: a T of iron blocks, the pumpkin on top of the middle.
    let (x, z) = (g[0] - 4, g[2] + 3);
    w.console(&format!("setblock {x} {y} {z} minecraft:iron_block"));
    for dx in -1..=1 {
        w.console(&format!("setblock {} {} {z} minecraft:iron_block", x + dx, y + 1));
    }
    w.use_on_top([x, y + 1, z]);
    w.ticks(2);
    assert_eq!(w.mobs("minecraft:iron_golem").len(), 1, "an iron golem");
    // Two snow blocks under a pumpkin placed sideways do not stand up as a golem by accident:
    // only a finished pattern makes one.
    w.console(&format!("setblock {} {y} {} minecraft:snow_block", g[0] + 6, g[2] + 6));
    w.use_on_top([g[0] + 6, y, g[2] + 6]);
    w.ticks(2);
    assert_eq!(w.mobs("minecraft:snow_golem").len(), 1);
}

#[test]
fn water_buckets_scoop_up_fish() {
    let mut w = World::new();
    w.console("gamemode creative Builder");
    w.hold("minecraft:water_bucket", 1);
    w.console("gamemode survival Builder");
    w.summon("minecraft:salmon", [1.5, 0.0, 0.0], "{NoAI:1b,type:\"large\"}");
    let salmon = w.mobs("minecraft:salmon")[0].0;
    w.interact(salmon);
    w.ticks(1);
    assert!(w.mobs("minecraft:salmon").is_empty(), "the salmon went into the bucket");
    assert_eq!(w.held().map(|h| h.0), Some(World::item_id("minecraft:salmon_bucket")));
}

#[test]
fn mooshrooms_give_stew_and_shear_into_cows() {
    let mut w = World::new();
    w.console("gamemode creative Builder");
    w.hold("minecraft:bowl", 1);
    w.console("gamemode survival Builder");
    w.summon("minecraft:mooshroom", [1.5, 0.0, 0.0], "{NoAI:1b}");
    let moo = w.mobs("minecraft:mooshroom")[0].0;
    w.interact(moo);
    assert_eq!(w.held().map(|h| h.0), Some(World::item_id("minecraft:mushroom_stew")));
    w.console("gamemode creative Builder");
    w.hold("minecraft:shears", 1);
    w.console("gamemode survival Builder");
    w.interact(moo);
    w.ticks(2);
    assert!(w.mobs("minecraft:mooshroom").is_empty(), "the mooshroom is gone");
    assert_eq!(w.mobs("minecraft:cow").len(), 1, "a cow in its place");
    if std::env::var_os("KILN_DATAPACK").is_some() {
        let items = w.sim.entities().iter().filter(|e| e.0 == "minecraft:item").count();
        assert!(items >= 1, "mushrooms dropped");
    }
}

#[test]
fn snow_golems_pelt_monsters() {
    let mut w = World::new();
    w.console("gamemode creative Builder");
    w.console("time set 18000");
    w.summon("minecraft:snow_golem", [2.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.summon("minecraft:zombie", [8.0, 0.0, 0.0], "{PersistenceRequired:1b,NoAI:1b}");
    let mut snowballs = 0;
    for _ in 0..100 {
        w.ticks(1);
        snowballs = snowballs.max(w.sim.entities().iter().filter(|e| e.0 == "minecraft:snowball").count());
    }
    assert!(snowballs >= 1, "the golem threw snowballs");
}
