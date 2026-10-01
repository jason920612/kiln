//! Sniffers and camels in the running simulation (wp28): a sniffer digs up a seed (the vanilla
//! datapack's `gameplay/sniffer_digging`: `KILN_DATAPACK`), bred sniffers leave an egg item, camels
//! stroll about. Their brains against vanilla are checked by kiln-entity's
//! `mob_parity`.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Listener", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative Listener".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
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

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }

    fn items(&self) -> usize {
        self.sim.entities().into_iter().filter(|e| e.0 == "minecraft:item").count()
    }
}

#[test]
fn bred_sniffers_leave_an_egg_item_and_no_baby() {
    let mut w = World::new();
    let p = w.client.pos;
    for dx in [2.0, 3.5] {
        w.run(&format!("summon minecraft:sniffer {} {} {} {{InLove:600,PersistenceRequired:1b}}", p[0] + dx, p[1], p[2] + 2.0));
    }
    w.ticks(2);
    assert_eq!(w.mobs("minecraft:sniffer").len(), 2);
    for _ in 0..400 {
        w.ticks(1);
        if w.items() > 0 {
            break;
        }
    }
    let eggs: Vec<_> = w.sim.entities().into_iter().filter(|e| e.0 == "minecraft:item").collect();
    assert_eq!(eggs.len(), 1, "one egg item: {eggs:?}");
    assert_eq!(w.mobs("minecraft:sniffer").len(), 2, "no baby is born, only an egg");
}

#[test]
fn a_sniffer_digs_up_a_seed() {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        eprintln!("no vanilla datapack (KILN_DATAPACK): the sniffer_digging loot table is missing; skipped");
        return;
    }
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!("summon minecraft:sniffer {} {} {} {{PersistenceRequired:1b}}", p[0] + 3.0, p[1], p[2] + 3.0));
    // Scenting, sniffing, searching and digging take a couple of minutes at most.
    let mut seen = false;
    for _ in 0..60 {
        w.ticks(100);
        let seeds = w
            .sim
            .entities()
            .into_iter()
            .filter(|e| e.0 == "minecraft:item")
            .count();
        if seeds > 0 {
            seen = true;
            break;
        }
    }
    assert!(seen, "the sniffer dug up an item within five minutes");
    assert_eq!(w.mobs("minecraft:sniffer").len(), 1);
}

#[test]
fn camels_stroll_about_and_live_on() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!("summon minecraft:camel {} {} {} {{PersistenceRequired:1b}}", p[0] + 3.0, p[1], p[2] + 3.0));
    w.ticks(2);
    let start = w.mobs("minecraft:camel")[0].1;
    let mut moved = 0.0f64;
    for _ in 0..30 {
        w.ticks(100);
        let c = w.mobs("minecraft:camel");
        assert_eq!(c.len(), 1);
        moved = moved.max(((c[0].1[0] - start[0]).powi(2) + (c[0].1[2] - start[2]).powi(2)).sqrt());
    }
    assert!(moved > 1.0, "the camel strolled about ({moved})");
}
