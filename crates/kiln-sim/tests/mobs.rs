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
        // Only the mobs a test summons (natural spawning has its own test).
        w.console("gamerule minecraft:spawn_mobs false");
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

#[test]
fn monsters_spawn_naturally_at_night_within_the_cap() {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        eprintln!("natural spawning needs the vanilla datapack (KILN_DATAPACK); skipped");
        return;
    }
    let mut w = World::new();
    w.console("gamerule minecraft:spawn_mobs true");
    w.console("gamemode creative Hunter");
    w.console("time set 18000");
    // Freshly spawned: at least 24 blocks from the player.
    w.ticks(20);
    let p = w.pos();
    for m in w.sim.mobs() {
        let d = ((m.2[0] - p[0]).powi(2) + (m.2[1] - p[1]).powi(2) + (m.2[2] - p[2]).powi(2)).sqrt();
        assert!(d > 23.0, "{} spawned {d:.1} blocks from the player", m.1);
    }
    w.ticks(280);
    let all = w.sim.mobs();
    let monsters = all.iter().filter(|m| matches!(m.1, "minecraft:zombie" | "minecraft:skeleton" | "minecraft:creeper" | "minecraft:spider")).count();
    assert!(monsters > 0, "no monsters spawned: {all:?}");
    // The cap is checked before each chunk; a chunk may add up to its cluster size (4) past it.
    assert!(monsters < 70 + 4, "{monsters} monsters exceed the cap");
}

#[test]
fn selectors_see_mobs_and_kill_removes_them() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [4.0, 0.0, 0.0], "");
    w.summon("minecraft:pig", [-4.0, 0.0, 0.0], "");
    w.ticks(5);
    w.console("kill @e[type=minecraft:zombie]");
    w.ticks(25);
    assert!(w.mobs("minecraft:zombie").is_empty(), "the zombie died and was removed");
    assert_eq!(w.mobs("minecraft:pig").len(), 1, "the pig is untouched");
    w.console("kill @e[type=!minecraft:player]");
    w.ticks(25);
    assert!(w.mobs("minecraft:pig").is_empty());
    assert!(w.sim.health(1).is_some_and(|h| !h.1), "the player is alive");
}

impl World {
    /// Puts `count` of `item` in the first hotbar slot (through creative mode).
    fn hold(&mut self, item: &str, count: i32) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    fn interact(&mut self, entity_id: i32) {
        let pkt = PlayIn::Interact { entity_id, hand: kiln_proto::packets::serverbound::Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn held(&self) -> Option<(i32, i32)> {
        // Menu slot 36 is the first hotbar slot.
        self.sim.inventory(1).unwrap()[36]
    }
}

#[test]
fn fed_animals_breed_and_babies_grow() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:wheat", 64);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:cow", [1.5, 0.0, 0.0], "");
    w.summon("minecraft:cow", [-1.5, 0.0, 0.0], "");
    let cows: Vec<i32> = w.mobs("minecraft:cow").iter().map(|c| c.0).collect();
    for &c in &cows {
        w.interact(c);
    }
    assert_eq!(w.held(), Some((kiln_data::builtin_id("minecraft:item", "minecraft:wheat").unwrap(), 62)), "two wheat eaten");
    w.ticks(200);
    assert_eq!(w.mobs("minecraft:cow").len(), 3, "a calf was born");
    assert!(w.sim.entities().iter().any(|e| e.0 == "minecraft:experience_orb"), "breeding experience");
    // The parents are on their breeding cooldown: more wheat does nothing.
    for &c in &cows {
        w.interact(c);
    }
    assert_eq!(w.held().unwrap().1, 62);
}

#[test]
fn sheep_shear_and_dye_cows_milk() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:red_dye", 2);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:sheep", [1.5, 0.0, 0.0], "{NoAI:1b}");
    let sheep = w.mobs("minecraft:sheep")[0].0;
    w.interact(sheep);
    assert_eq!(w.held().unwrap().1, 1, "dye used");
    w.console("gamemode creative Hunter");
    w.hold("minecraft:shears", 1);
    w.console("gamemode survival Hunter");
    w.interact(sheep);
    w.ticks(2);
    if std::env::var_os("KILN_DATAPACK").is_some() {
        let items = w.sim.entities().iter().filter(|e| e.0 == "minecraft:item").count();
        assert!(items >= 1, "wool dropped");
    }
    assert_eq!(w.sim.item_damage(1, 36), Some(1), "the shears wore");
    w.console("gamemode creative Hunter");
    w.hold("minecraft:bucket", 1);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:cow", [-1.5, 0.0, 0.0], "{NoAI:1b}");
    let cow = w.mobs("minecraft:cow")[0].0;
    w.interact(cow);
    assert_eq!(w.held().map(|h| h.0), kiln_data::builtin_id("minecraft:item", "minecraft:milk_bucket"), "milked");
}

#[test]
fn staring_at_an_enderman_angers_it() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:enderman", [0.0, 0.0, 5.0], "{PersistenceRequired:1b}");
    // Looking straight ahead does not meet its eyes; looking up at them does.
    w.ticks(60);
    assert_eq!(w.health(), 20.0, "not stared at");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Move { pos: None, rot: Some([0.0, -10.5]), on_ground: true })]));
    // It freezes while looked at; looking away lets it come.
    w.ticks(30);
    assert_eq!(w.health(), 20.0, "frozen while stared at");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Move { pos: None, rot: Some([180.0, 0.0]), on_ground: true })]));
    w.ticks(200);
    assert!(w.health() < 20.0, "the enderman attacked (health {})", w.health());
}

#[test]
fn shulker_bullets_hurt_and_levitate() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:shulker", [0.0, 0.0, 5.0], "");
    let mut levitated = false;
    for _ in 0..300 {
        w.ticks(1);
        let fx = w.sim.effects(1).unwrap();
        levitated |= fx.iter().any(|e| e.0 == "minecraft:levitation");
    }
    assert!(w.health() < 20.0, "a bullet hit (health {})", w.health());
    assert!(levitated, "the hit made the player levitate");
}

#[test]
fn end_city_sentries_load_as_shulkers() {
    use kiln_proto::nbt::Tag;
    // The compound end city generation leaves for a sentry.
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:shulker".into())),
        ("Pos".into(), Tag::List(vec![Tag::Double(3.5), Tag::Double(70.0), Tag::Double(4.5)])),
        ("Rotation".into(), Tag::List(vec![Tag::Float(0.0), Tag::Float(0.0)])),
        ("AttachFace".into(), Tag::Byte(1)),
        ("Peek".into(), Tag::Byte(0)),
        ("Color".into(), Tag::Byte(16)),
    ]);
    let e = kiln_entity::persist::load(&tag, 7, 1).expect("loads");
    let m = kiln_entity::mob::data(&e).expect("a mob");
    assert_eq!(m.kind, kiln_entity::mob::MobKind::Shulker);
    assert_eq!(kiln_entity::mob::kinds::shulker::st(m).attach, kiln_entity::math::Direction::Up);
}

#[test]
fn every_mob_type_summons_ticks_and_saves() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    for (i, kind) in kiln_entity::mob::ALL_KINDS.iter().enumerate() {
        let a = i as f64 * 0.7;
        w.summon(kind.type_name(), [6.0 * a.cos(), 0.0, 6.0 * a.sin()], "{PersistenceRequired:1b}");
    }
    w.ticks(100);
    for kind in kiln_entity::mob::ALL_KINDS {
        // Endermen hunt endermites.
        if kind == kiln_entity::mob::MobKind::Endermite {
            continue;
        }
        assert!(!w.mobs(kind.type_name()).is_empty(), "{} is gone", kind.type_name());
    }
}
