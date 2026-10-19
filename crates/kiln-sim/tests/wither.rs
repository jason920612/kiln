//! The wither end to end: building it from soul sand and wither skeleton skulls, its
//! invulnerable start and the explosion that ends it. Guardians swim in a pool. The behaviour
//! itself is checked tick by tick against vanilla by kiln-entity's `mob_parity` test.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Builder", 2);
        assert!(sim.step([
            msg,
            ToSim::Console("gamemode creative Builder".into()),
            ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
            ToSim::Console("difficulty normal".into())
        ]));
        let mut w = World { sim, client: Client::new(1, stats), ground: [0; 3] };
        w.ticks(5);
        let p = w.client.pos;
        w.ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
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

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn set(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn name(&self, p: [i32; 3]) -> &'static str {
        kiln_entity::blocks::block_name(self.sim.block_at(p[0], p[1], p[2]).expect("loaded"))
    }

    fn hold(&mut self, item: &str) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count: 64, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    fn use_on_top(&mut self, pos: [i32; 3]) {
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }
}

#[test]
fn wither_is_built_from_soul_sand_and_skulls() {
    let mut w = World::new();
    // The T of soul sand along z, two skulls already on it.
    let (x, z) = (4, 0);
    for p in [w.at(x, 1, z), w.at(x, 2, z - 1), w.at(x, 2, z), w.at(x, 2, z + 1)] {
        w.set(p, "minecraft:soul_sand");
    }
    w.set(w.at(x, 3, z - 1), "minecraft:wither_skeleton_skull");
    w.set(w.at(x, 3, z + 1), "minecraft:wither_skeleton_skull");
    w.ticks(1);
    assert!(w.mobs("minecraft:wither").is_empty());
    // The third skull completes it.
    w.hold("minecraft:wither_skeleton_skull");
    w.use_on_top(w.at(x, 2, z));
    w.ticks(1);
    let withers = w.mobs("minecraft:wither");
    assert_eq!(withers.len(), 1, "a wither appears");
    let (_, pos, health) = withers[0];
    assert!(health == 100.0 || health == 110.0, "a third of its health, maybe healed once ({health})");
    let base = w.at(x, 1, z);
    assert_eq!([pos[0], pos[2]], [base[0] as f64 + 0.5, base[2] as f64 + 0.5]);
    for p in [w.at(x, 1, z), w.at(x, 2, z - 1), w.at(x, 2, z), w.at(x, 3, z), w.at(x, 3, z + 1)] {
        assert_eq!(w.name(p), "minecraft:air", "the pattern is cleared");
    }
    // Invulnerable, it heals 10 every 10 ticks; after 220 ticks it explodes and is free.
    w.ticks(100);
    let h = w.mobs("minecraft:wither")[0].2;
    assert!(h > 180.0, "healing while invulnerable ({h})");
    let ground_before = w.name(w.at(x, 0, z));
    w.ticks(130);
    assert_eq!(w.mobs("minecraft:wither")[0].2, 300.0);
    assert_ne!(ground_before, "minecraft:air");
    assert_eq!(w.name(w.at(x, 0, z)), "minecraft:air", "the explosion broke the ground under it");
}

#[test]
fn an_incomplete_pattern_or_peaceful_builds_nothing() {
    let mut w = World::new();
    let (x, z) = (4, 0);
    for p in [w.at(x, 2, z - 1), w.at(x, 2, z), w.at(x, 2, z + 1)] {
        w.set(p, "minecraft:soul_soil");
    }
    w.set(w.at(x, 3, z - 1), "minecraft:wither_skeleton_skull");
    w.set(w.at(x, 3, z + 1), "minecraft:wither_skeleton_skull");
    w.hold("minecraft:wither_skeleton_skull");
    // No stem under the middle: not a T.
    w.use_on_top(w.at(x, 2, z));
    w.ticks(2);
    assert!(w.mobs("minecraft:wither").is_empty());
    // With the stem, but peaceful.
    w.set(w.at(x, 3, z), "minecraft:air");
    w.set(w.at(x, 1, z), "minecraft:soul_soil");
    w.run("difficulty peaceful");
    w.use_on_top(w.at(x, 2, z));
    w.ticks(2);
    assert!(w.mobs("minecraft:wither").is_empty());
}

#[test]
fn guardians_swim_in_water() {
    let mut w = World::new();
    let [gx, gy, gz] = w.at(0, 1, 6);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:stone", gx - 4, gy, gz - 4, gx + 4, gy + 4, gz + 4));
    w.run(&format!("fill {} {} {} {} {} {} minecraft:water", gx - 3, gy, gz - 3, gx + 3, gy + 4, gz + 3));
    w.run(&format!("summon minecraft:guardian {} {} {}", gx as f64 + 0.5, gy as f64 + 1.0, gz as f64 + 0.5));
    w.ticks(1);
    let start = w.mobs("minecraft:guardian")[0].1;
    let mut moved = 0.0f64;
    for _ in 0..20 {
        w.ticks(20);
        let g = w.mobs("minecraft:guardian");
        assert_eq!(g.len(), 1, "the guardian stays");
        let p = g[0].1;
        moved = moved.max((p[0] - start[0]).abs() + (p[2] - start[2]).abs());
        assert!(p[1] >= gy as f64 && p[1] < (gy + 6) as f64, "in the pool or at its surface ({p:?})");
    }
    assert!(moved > 0.5, "it swam about ({moved})");
}

/// wp78 (a real client side by side): cows the wither's skulls killed left wither roses in vanilla (`LivingEntity.createWitherRose` after the
/// death loot, for a victim whose kill credit is a wither); Kiln's mobs died without one.
fn kill_cow_by(w: &mut World, killer: &str, griefing: bool) -> ([i32; 3], Vec<(&'static str, [f64; 3])>) {
    w.run(&format!("gamerule minecraft:mob_griefing {griefing}"));
    let spot = w.at(6, 1, 3);
    w.set(w.at(6, 0, 3), "minecraft:dirt");
    w.set(spot, "minecraft:air");
    w.run("summon minecraft:wither 20.5 200 20.5 {NoAI:1b,Invul:0}");
    w.run(&format!("summon minecraft:cow {}.5 {} {}.5 {{NoAI:1b,PersistenceRequired:1b}}", spot[0], spot[1], spot[2]));
    w.ticks(2);
    if killer == "wither" {
        w.run("damage @e[type=cow,limit=1] 100 minecraft:mob_attack by @e[type=wither,limit=1]");
    } else {
        w.run("kill @e[type=cow,limit=1]");
    }
    w.ticks(3);
    (spot, w.sim.entities().into_iter().map(|(k, p)| (k, [p[0], p[1], p[2]])).collect())
}

#[test]
fn a_cow_killed_by_a_wither_leaves_a_wither_rose() {
    let mut w = World::new();
    let (spot, _) = kill_cow_by(&mut w, "wither", true);
    assert_eq!(w.name(spot), "minecraft:wither_rose", "the rose grows where the cow died");
}

#[test]
fn without_mob_griefing_the_wither_rose_drops_as_an_item() {
    let mut w = World::new();
    let (spot, ents) = kill_cow_by(&mut w, "wither", false);
    assert_eq!(w.name(spot), "minecraft:air");
    assert!(ents.iter().any(|(k, _)| *k == "minecraft:item"), "a dropped flower: {ents:?}");
}

#[test]
fn a_cow_killed_by_anything_else_leaves_no_wither_rose() {
    let mut w = World::new();
    let (spot, _) = kill_cow_by(&mut w, "kill", true);
    assert_eq!(w.name(spot), "minecraft:air");
}

/// Boss event actions the client was sent about the wither's bar since the log started: 0 add, 1 remove.
fn boss_actions(w: &World) -> Vec<i32> {
    let log = w.client.stats.log.lock().unwrap().replace(Vec::new()).unwrap_or_default();
    let mut out = Vec::new();
    for p in log {
        let mut r = kiln_proto::codec::Reader::new(&p);
        if r.varint().ok() == Some(kiln_data::packets::play::clientbound::BOSS_EVENT) {
            let _ = r.uuid();
            if let Ok(a) = r.varint() {
                out.push(a);
            }
        }
    }
    out
}

/// wp78 (real client side by side): a wither's bar stayed on the screen of a player who left its level or went out of its range (vanilla's
/// `ServerBossEvent.removePlayer` follows the entity ceasing to be seen). Kiln only removed it when the wither itself went.
#[test]
fn the_withers_bar_goes_when_the_player_leaves_its_range() {
    let mut w = World::new();
    *w.client.stats.log.lock().unwrap() = Some(Vec::new());
    let p = w.client.pos;
    w.run(&format!("summon minecraft:wither {} {} {} {{NoAI:1b,Invul:0}}", p[0] + 5.0, p[1] + 3.0, p[2]));
    w.ticks(5);
    assert!(boss_actions(&w).contains(&0), "the bar appeared");
    // To another dimension: the wither is not seen any more.
    w.run("execute in minecraft:the_nether run tp Builder 0 100 0");
    w.ticks(10);
    assert!(boss_actions(&w).contains(&1), "the bar was taken away with the level");
}

/// wp78 (real client side by side): a summoned `NoAI` wither never ticks, and vanilla's bar for it is full; Kiln sent an empty one.
#[test]
fn a_wither_that_never_ticks_shows_a_full_bar() {
    let mut w = World::new();
    *w.client.stats.log.lock().unwrap() = Some(Vec::new());
    let p = w.client.pos;
    w.run(&format!("summon minecraft:wither {} {} {} {{NoAI:1b,Invul:0}}", p[0] + 5.0, p[1] + 3.0, p[2]));
    w.ticks(3);
    let log = w.client.stats.log.lock().unwrap().replace(Vec::new()).unwrap_or_default();
    let mut progress = None;
    for pkt in log {
        let mut r = kiln_proto::codec::Reader::new(&pkt);
        if r.varint().ok() == Some(kiln_data::packets::play::clientbound::BOSS_EVENT) {
            let _ = r.uuid();
            if r.varint().ok() == Some(0) {
                // Add: name (a text component, nbt) then the progress: read it as the last f32 of the fixed tail (progress, color, overlay, flags)
                let tail = &pkt[pkt.len() - 7..];
                progress = Some(f32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]));
            }
        }
    }
    assert_eq!(progress, Some(1.0), "the bar a never-ticking wither shows");
}

/// wp78: a player the wither kills leaves a wither rose too (`LivingEntity.createWitherRose(getKillCredit())` is for every living entity).
#[test]
fn a_player_killed_by_a_wither_leaves_a_wither_rose() {
    let mut w = World::new();
    w.run("gamerule minecraft:mob_griefing true");
    w.run("gamemode survival Builder");
    let p = w.client.pos;
    let spot = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    w.set([spot[0], spot[1] - 1, spot[2]], "minecraft:dirt");
    w.ticks(2);
    w.run(&format!("summon minecraft:wither {} {} {} {{NoAI:1b,Invul:0}}", p[0] + 20.0, p[1] + 5.0, p[2]));
    w.ticks(2);
    w.run("damage Builder 1000 minecraft:mob_attack by @e[type=wither,limit=1]");
    w.ticks(4);
    let items = w.sim.entities().into_iter().filter(|(k, _)| *k == "minecraft:item").count();
    let grew = (-1..=1).any(|dx| (-1..=1).any(|dz| (-1..=2).any(|dy| w.name([spot[0] + dx, spot[1] + dy, spot[2] + dz]) == "minecraft:wither_rose")));
    assert!(grew || items > 0, "a rose block or a dropped flower where the player died");
}

fn wither_health(w: &World) -> f32 {
    w.mobs("minecraft:wither").first().expect("a wither").2
}

/// wp81: the half-health shield of a wither keys on the direct entity being an arrow (`getDirectEntity() instanceof AbstractArrow`), not on the
/// damage type: `/damage … minecraft:arrow` without a projectile hurts it (wp78 lock-step: 5.76 and 7.744 taken), and the same damage caused by
/// an arrow entity does not.
#[test]
fn a_powered_wither_takes_arrow_type_damage_without_an_arrow_entity() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!("summon minecraft:wither {} {} {} {{NoAI:1b,Invul:0,Health:100f}}", p[0] + 5.0, p[1] + 3.0, p[2]));
    w.ticks(2);
    let before = wither_health(&w);
    assert!(before <= 150.0, "below half health ({before})");
    w.run("damage @e[type=minecraft:wither,limit=1] 8 minecraft:arrow");
    w.ticks(2);
    let hurt = wither_health(&w);
    assert!(hurt < before - 7.0, "arrow-type damage with no projectile hurts: {before} -> {hurt}");
    // With an arrow as the direct entity it is shrugged off.
    w.run(&format!("summon minecraft:arrow {} {} {} {{NoGravity:1b,Motion:[0.0,0.0,0.0]}}", p[0], p[1] + 3.0, p[2] + 3.0));
    w.ticks(1);
    w.run("damage @e[type=minecraft:wither,limit=1] 8 minecraft:arrow by @e[type=minecraft:arrow,limit=1]");
    w.ticks(2);
    let after = wither_health(&w);
    assert!(after >= hurt, "no damage from an arrow entity: {hurt} -> {after}");
}
