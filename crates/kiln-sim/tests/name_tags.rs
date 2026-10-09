//! Name tags (`NameTagItem.interactLivingEntity`): a tag with a name names a living mob and keeps it from despawning;
//! one without a name does nothing. The name goes to viewers as entity data.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode survival User".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        Self { sim, client }
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

    fn summon(&mut self, name: &str) -> i32 {
        let before = self.sim.entity_ids_of(name);
        let p = self.client.pos;
        self.run(&format!("summon {name} {} {} {} {{NoAI:1b}}", p[0] + 1.5, p[1], p[2]));
        self.ticks(1);
        *self.sim.entity_ids_of(name).iter().find(|id| !before.contains(id)).expect("summoned")
    }

    fn interact(&mut self, id: i32) {
        let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn nbt(&self, name: &str) -> Tag {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
    }

    fn tags(&self) -> i32 {
        self.sim.inventory(1).unwrap().iter().flatten().filter(|(id, _)| kiln_data::builtin_entries("minecraft:item").unwrap()[*id as usize] == "minecraft:name_tag").map(|(_, n)| *n).sum()
    }
}

#[test]
fn a_named_tag_names_the_mob() {
    let mut w = World::new();
    w.run("give User minecraft:name_tag[custom_name='\"Bessie\"']");
    w.ticks(1);
    assert_eq!(w.tags(), 1);
    let cow = w.summon("minecraft:cow");
    assert_eq!(w.nbt("minecraft:cow").get("PersistenceRequired").and_then(Tag::as_i64), Some(0));
    w.interact(cow);
    w.ticks(1);
    let nbt = w.nbt("minecraft:cow");
    assert_eq!(nbt.get("CustomName").and_then(Tag::as_str), Some("Bessie"), "{nbt:?}");
    assert_eq!(nbt.get("PersistenceRequired").and_then(Tag::as_i64), Some(1));
    assert_eq!(w.tags(), 0, "the tag is used up");
}

#[test]
fn a_tag_without_a_name_does_nothing() {
    let mut w = World::new();
    w.run("give User minecraft:name_tag");
    w.ticks(1);
    let cow = w.summon("minecraft:cow");
    w.interact(cow);
    w.ticks(1);
    let nbt = w.nbt("minecraft:cow");
    assert!(nbt.get("CustomName").is_none());
    assert_eq!(w.tags(), 1);
}

#[test]
fn creative_players_keep_their_tags() {
    let mut w = World::new();
    w.run("gamemode creative User");
    w.run("give User minecraft:name_tag[custom_name='\"Rex\"']");
    w.ticks(1);
    let wolf = w.summon("minecraft:wolf");
    w.interact(wolf);
    w.ticks(1);
    assert_eq!(w.nbt("minecraft:wolf").get("CustomName").and_then(Tag::as_str), Some("Rex"));
    assert_eq!(w.tags(), 1);
}
