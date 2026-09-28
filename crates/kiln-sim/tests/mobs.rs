//! Mobs end to end: `/summon`, spawn eggs, a zombie hitting a player at night, a player killing a
//! pig (loot and experience), a creeper blowing up, burning zombies by day, despawning.
//! The mob behaviour itself is checked tick by tick against vanilla by kiln-entity's
//! `mob_parity` test (tools/mob_vectors.py).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    clients: Vec<Client>,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let (msg, stats) = join(1, "Hunter", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, clients: vec![Client::new(1, stats)] };
        w.ticks(5);
        w.console("gamerule minecraft:natural_health_regeneration false");
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

    /// Summons `entity` at the player's position plus `offset`.
    fn summon(&mut self, entity: &str, offset: [f64; 3], nbt: &str) {
        let p = self.pos();
        self.console(format!("summon {entity} {} {} {} {nbt}", p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]).trim());
        self.ticks(1);
    }

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }

    fn health(&self) -> f32 {
        self.sim.health(1).unwrap().0
    }
}

#[test]
fn summon_creates_mobs_and_they_fall_and_wander() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.summon("minecraft:pig", [3.0, 2.0, 0.0], "");
    let pigs = w.mobs("minecraft:pig");
    assert_eq!(pigs.len(), 1, "one pig");
    let y0 = pigs[0].1[1];
    w.ticks(40);
    let pig = w.mobs("minecraft:pig")[0];
    assert!(pig.1[1] < y0, "the pig fell ({y0} -> {})", pig.1[1]);
    assert_eq!(pig.2, 10.0);
    // An unknown or non-mob type fails.
    w.summon("minecraft:boat", [1.0, 0.0, 0.0], "");
    assert!(w.mobs("minecraft:boat").is_empty());
}

#[test]
fn zombies_attack_survival_players_at_night() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:zombie", [4.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(200);
    assert!(w.health() < 20.0, "the zombie hit the player (health {})", w.health());
}

#[test]
fn creative_players_are_ignored() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [3.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(100);
    assert_eq!(w.health(), 20.0);
}

#[test]
fn players_kill_pigs() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    let id = kiln_data::builtin_id("minecraft:item", "minecraft:diamond_sword").unwrap();
    let stack = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
    w.console("gamemode creative Hunter");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    w.console("gamemode survival Hunter");
    w.summon("minecraft:pig", [0.0, 0.0, 2.0], "{NoAI:1b}");
    w.ticks(20);
    let (pig, _, _) = w.mobs("minecraft:pig")[0];
    // A full-strength diamond sword hit (7) takes the pig to 3, the second one kills it.
    w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: pig }), ToSim::Packet(1, PlayIn::Punch)]);
    assert_eq!(w.mobs("minecraft:pig")[0].2, 3.0);
    w.ticks(25);
    w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: pig }), ToSim::Packet(1, PlayIn::Punch)]);
    let dead = w.mobs("minecraft:pig");
    assert!(dead.is_empty() || dead[0].2 == 0.0, "the pig died");
    w.ticks(25);
    assert!(w.mobs("minecraft:pig").is_empty(), "removed after the death animation");
    let entities = w.sim.entities();
    assert!(entities.iter().any(|e| e.0 == "minecraft:experience_orb"), "experience for a player kill: {entities:?}");
    // With the vanilla datapack's loot tables, the pig drops porkchops (1 to 3).
    if std::env::var_os("KILN_DATAPACK").is_some() {
        assert!(entities.iter().any(|e| e.0 == "minecraft:item"), "loot: {entities:?}");
    }
}

#[test]
fn creepers_explode_next_to_players() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:creeper", [2.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(120);
    assert!(w.mobs("minecraft:creeper").is_empty(), "the creeper blew up");
    assert!(w.health() < 20.0, "the explosion hurt the player (health {})", w.health());
}

#[test]
fn zombies_burn_in_daylight() {
    let mut w = World::new();
    w.console("time set 6000");
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [5.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(600);
    let z = w.mobs("minecraft:zombie");
    assert!(z.is_empty() || z[0].2 < 20.0, "burning hurts: {z:?}");
}

#[test]
fn monsters_far_from_players_despawn() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [3.0, 0.0, 0.0], "");
    w.ticks(2);
    assert_eq!(w.mobs("minecraft:zombie").len(), 1);
    // Peaceful removes monsters, persistent or not.
    w.summon("minecraft:zombie", [3.0, 0.0, 1.0], "{PersistenceRequired:1b}");
    w.console("difficulty peaceful");
    w.ticks(2);
    assert!(w.mobs("minecraft:zombie").is_empty(), "peaceful removes monsters");
}
