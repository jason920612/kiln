//! The skeleton trap end to end: a trap horse (what a thunderstorm spawns) springs when a living
//! player comes within 10 blocks: a visual bolt, a skeleton on the horse and three more
//! horsemen, riders linked to their horses by network ids, gear enchanted from the datapack.
//! The trap goal itself is compared tick by tick with vanilla by kiln-entity's `mob_parity`
//! (`skeleton_trap_*`, tools/mob_vectors.py).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new(gamemode: &str) -> World {
        let mut sim = Sim::new(SimConfig::new(6, 4, None));
        let (msg, stats) = join(1, "Bait", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        // Only what the test summons.
        w.console("gamerule minecraft:spawn_mobs false");
        w.console(&format!("gamemode {gamemode} Bait"));
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

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn summon_at(&mut self, entity: &str, dx: f64, nbt: &str) {
        let p = self.client.pos;
        self.console(&format!("summon {entity} {} {} {} {nbt}", p[0] + dx, p[1], p[2]));
    }

    fn nbt_of(&self, id: &str) -> Vec<Tag> {
        self.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(|i| i.as_str()) == Some(id)).collect()
    }
}

/// The datapack's enchantment data (the repository's `work/generated` unless `KILN_DATAPACK`
/// says otherwise); false when there is none.
fn datapack() -> bool {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let work = std::env::var_os("KILN_WORK").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        if !dir.join("data/minecraft/enchantment").is_dir() {
            return false;
        }
        unsafe { std::env::set_var("KILN_DATAPACK", dir) };
    }
    true
}

fn enchantment_count(item: Option<&Tag>) -> Option<usize> {
    // A saved stack: `components` -> `minecraft:enchantments` (a map of levels).
    let item = item?;
    let comps = item.get("components")?;
    match comps.get("minecraft:enchantments")? {
        Tag::Compound(levels) => Some(levels.len()),
        Tag::Int(_) | _ => None,
    }
}

#[test]
fn a_player_within_ten_blocks_springs_the_trap() {
    let enchanting = datapack();
    let mut w = World::new("creative");
    // Out of reach first: the trap waits.
    w.summon_at("minecraft:skeleton_horse", 12.0, "{SkeletonTrap:1b}");
    w.ticks(30);
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton_horse").count(), 1, "nothing yet");
    assert!(w.sim.mobs().iter().all(|m| m.1 != "minecraft:skeleton"));
    let horse = w.nbt_of("minecraft:skeleton_horse");
    assert_eq!(horse[0].get("SkeletonTrap").and_then(|t| t.as_i64()), Some(1));

    // A second one within reach springs it.
    w.summon_at("minecraft:skeleton_horse", 6.0, "{SkeletonTrap:1b}");
    w.ticks(3);
    let mobs = w.sim.mobs();
    let horses: Vec<i32> = mobs.iter().filter(|m| m.1 == "minecraft:skeleton_horse").map(|m| m.0).collect();
    let skeletons: Vec<i32> = mobs.iter().filter(|m| m.1 == "minecraft:skeleton").map(|m| m.0).collect();
    // The waiting trap, the sprung one and its three companions; four riders.
    assert_eq!(horses.len(), 5, "{mobs:?}");
    assert_eq!(skeletons.len(), 4, "{mobs:?}");
    assert!(w.sim.entity_ids_of("minecraft:lightning_bolt").len() <= 1);

    // Every rider sits on a skeleton horse (real ids on both sides), one to a horse.
    let riding = w.sim.riding();
    let mut seated = Vec::new();
    for &s in &skeletons {
        let (_, vehicle, _) = riding.iter().find(|r| r.0 == s).expect("the skeleton is simulated");
        let v = vehicle.expect("a rider");
        assert!(v > 0 && horses.contains(&v), "vehicle {v} of skeleton {s}");
        let (_, _, passengers) = riding.iter().find(|r| r.0 == v).unwrap();
        assert_eq!(passengers, &vec![s], "passengers of horse {v}");
        seated.push(v);
    }
    seated.sort_unstable();
    seated.dedup();
    assert_eq!(seated.len(), 4, "four different horses");

    // The sprung horse is tame and no longer a trap; the waiting one still is.
    let states: Vec<(i64, i64)> = w.nbt_of("minecraft:skeleton_horse").iter().map(|t| (t.get("SkeletonTrap").and_then(|x| x.as_i64()).unwrap_or(-1), t.get("Tame").and_then(|x| x.as_i64()).unwrap_or(-1))).collect();
    assert_eq!(states.iter().filter(|s| **s == (0, 1)).count(), 4, "tamed horses: {states:?}");
    assert_eq!(states.iter().filter(|s| **s == (1, 0)).count(), 1, "the waiting trap: {states:?}");

    // The horsemen are persistent and wear an iron helmet unless they got another head item;
    // their gear is enchanted from `minecraft:mob_spawn_equipment`.
    for s in w.nbt_of("minecraft:skeleton") {
        assert_eq!(s.get("PersistenceRequired").and_then(|t| t.as_i64()), Some(1));
        let equipment = s.get("equipment").expect("equipment");
        let head = equipment.get("head").expect("a head item");
        assert_eq!(head.get("id").and_then(|i| i.as_str()), Some("minecraft:iron_helmet"), "{head:?}");
        let bow = equipment.get("mainhand").expect("a weapon");
        assert_eq!(bow.get("id").and_then(|i| i.as_str()), Some("minecraft:bow"));
        if enchanting {
            assert!(enchantment_count(Some(bow)).is_some_and(|n| n > 0), "bow: {bow:?}");
            assert!(enchantment_count(Some(head)).is_some_and(|n| n > 0), "helmet: {head:?}");
        }
    }
}

#[test]
fn a_trap_nobody_springs_expires() {
    let mut w = World::new("creative");
    w.summon_at("minecraft:skeleton_horse", 30.0, "{SkeletonTrap:1b,SkeletonTrapTime:17990}");
    w.ticks(5);
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton_horse").count(), 1);
    w.ticks(20);
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton_horse").count(), 0, "gone after 18000 ticks");
    // A persistent trap stays.
    w.summon_at("minecraft:skeleton_horse", 30.0, "{SkeletonTrap:1b,SkeletonTrapTime:17990,PersistenceRequired:1b}");
    w.ticks(40);
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton_horse").count(), 1);
}

#[test]
fn spectators_and_the_far_do_not_spring_it() {
    let mut w = World::new("spectator");
    w.summon_at("minecraft:skeleton_horse", 3.0, "{SkeletonTrap:1b}");
    w.ticks(30);
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton_horse").count(), 1, "a spectator does not count");
    w.console("gamemode survival Bait");
    w.ticks(5);
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton_horse").count(), 4, "a survival player does");
}

/// The skeleton on the trap horse ticks in the very tick the trap springs (`tickPassenger` ticks
/// what joined its vehicle meanwhile): it already sits on the horse when that tick is over, not
/// where it was made.
#[test]
fn the_rider_of_the_trap_horse_ticks_in_the_tick_the_trap_springs() {
    let mut w = World::new("creative");
    w.summon_at("minecraft:skeleton_horse", 12.0, "{SkeletonTrap:1b}");
    w.ticks(10);
    // Steps into reach, then one tick at a time: the first tick with a skeleton is the spring's.
    w.console("tp Bait ~7 ~ ~");
    for _ in 0..5 {
        w.ticks(1);
        let mobs = w.sim.mobs();
        let Some(rider) = mobs.iter().find(|m| m.1 == "minecraft:skeleton") else { continue };
        let riding = w.sim.riding();
        let (_, vehicle, _) = riding.iter().find(|r| r.0 == rider.0).expect("simulated");
        let horse = mobs.iter().find(|m| Some(m.0) == *vehicle).expect("its horse");
        // A skeleton sits 0.7 below the horse's seat (a skeleton horse's is 1.31875 up).
        let dy = rider.2[1] - horse.2[1];
        assert!((dy - (1.318_750_023_841_858 - 0.699_999_988_079_071)).abs() < 1e-9, "the rider sits on its horse in the spring's own tick: {dy}");
        return;
    }
    panic!("the trap did not spring");
}

/// The horsemen of a sprung trap charge a survival player: the rider's goals steer the horse, so
/// even the trap horse itself (which wanders nowhere on its own once it has a rider) walks.
#[test]
fn the_horsemen_steer_their_horses_toward_a_survival_player() {
    let mut w = World::new("survival");
    w.summon_at("minecraft:skeleton_horse", 8.0, "{SkeletonTrap:1b}");
    let trap_horse = w.sim.mobs().iter().find(|m| m.1 == "minecraft:skeleton_horse").map(|m| m.0).expect("the trap horse");
    // (The spring's tick, then a few more for the riders to settle.)
    let mut sprung = false;
    for _ in 0..10 {
        w.ticks(1);
        if w.sim.mobs().iter().any(|m| m.1 == "minecraft:skeleton") {
            sprung = true;
            break;
        }
    }
    assert!(sprung, "the trap sprang");
    w.ticks(5);
    let start = w.sim.mobs().iter().find(|m| m.0 == trap_horse).map(|m| m.2).expect("still there");
    let player = w.client.pos;
    w.ticks(40);
    let mobs = w.sim.mobs();
    let now = mobs.iter().find(|m| m.0 == trap_horse).map(|m| m.2).expect("still there");
    let moved = ((now[0] - start[0]).powi(2) + (now[2] - start[2]).powi(2)).sqrt();
    assert!(moved > 1.0, "the trap horse was carried {moved} blocks by its rider (player at {player:?}, horse {start:?} -> {now:?})");
}
