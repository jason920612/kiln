//! wp33: `/kill` of mobs through their death code, the curse of vanishing and `keepInventory`
//! on a player's death, natural jockeys, the undead mounts and projectile deflection, end to end.
//! The mobs' own behaviour is compared with vanilla by kiln-entity's `mob_parity`
//! (`work/wp33/vectors.jsonl`, tools/mob_vectors.py).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

/// The datapack (the repository's `work/generated` unless `KILN_DATAPACK` says otherwise); loot
/// tables and enchantment effects come from it. False when there is none.
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
        self.console(format!("summon {entity} {} {} {} {nbt}", p[0] + dx, p[1], p[2]).trim());
    }

    fn nbt_of(&self, id: &str) -> Vec<Tag> {
        self.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(|i| i.as_str()) == Some(id)).collect()
    }

    /// The dropped items on the ground (item id, count).
    fn dropped(&self) -> Vec<(String, i64)> {
        self.nbt_of("minecraft:item")
            .iter()
            .filter_map(|t| {
                let item = t.get("Item")?;
                Some((item.get("id")?.as_str()?.to_string(), item.get("count").and_then(|c| c.as_i64()).unwrap_or(1)))
            })
            .collect()
    }
}

#[test]
fn kill_runs_a_mobs_death_loot() {
    if !datapack() {
        return;
    }
    let mut w = World::new("creative");
    w.summon_at("minecraft:pig", 3.0, "{PersistenceRequired:1b}");
    w.ticks(3);
    assert_eq!(w.nbt_of("minecraft:pig").len(), 1);
    w.console("kill @e[type=minecraft:pig]");
    w.ticks(30);
    assert!(w.nbt_of("minecraft:pig").is_empty(), "the pig died and was removed");
    let drops = w.dropped();
    assert!(drops.iter().any(|(id, n)| id == "minecraft:porkchop" && *n >= 1), "a pig killed by /kill drops its loot: {drops:?}");
}

#[test]
fn kill_drops_the_equipment_a_mob_keeps_for_sure() {
    let mut w = World::new("creative");
    // A drop chance above 1.0 is a preserved item (`Mob.dropCustomDeathLoot`): it drops even
    // without a player kill.
    w.summon_at("minecraft:zombie", 3.0, "{PersistenceRequired:1b,equipment:{mainhand:{id:\"minecraft:iron_sword\",count:1}},drop_chances:{mainhand:2.0f}}");
    w.ticks(3);
    assert_eq!(w.nbt_of("minecraft:zombie").len(), 1);
    w.console("kill @e[type=minecraft:zombie]");
    w.ticks(30);
    assert!(w.nbt_of("minecraft:zombie").is_empty());
    assert!(w.dropped().iter().any(|(id, _)| id == "minecraft:iron_sword"), "{:?}", w.dropped());
}

fn give_cursed(w: &mut World) {
    w.console("give Bait minecraft:diamond_sword[enchantments={\"minecraft:vanishing_curse\":1}]");
    w.console("give Bait minecraft:dirt 5");
    w.console("give Bait minecraft:iron_helmet[enchantments={\"minecraft:vanishing_curse\":1}]");
    w.console("give Bait minecraft:cobblestone 3");
    w.ticks(2);
}

#[test]
fn items_with_the_curse_of_vanishing_vanish_when_the_player_dies() {
    datapack();
    let mut w = World::new("survival");
    give_cursed(&mut w);
    w.console("kill Bait");
    w.ticks(5);
    let drops = w.dropped();
    assert!(drops.iter().any(|(id, n)| id == "minecraft:dirt" && *n == 5), "{drops:?}");
    assert!(drops.iter().any(|(id, n)| id == "minecraft:cobblestone" && *n == 3), "{drops:?}");
    assert!(!drops.iter().any(|(id, _)| id == "minecraft:diamond_sword" || id == "minecraft:iron_helmet"), "cursed items vanish: {drops:?}");
}

#[test]
fn keep_inventory_keeps_everything_cursed_or_not() {
    datapack();
    let mut w = World::new("survival");
    w.console("gamerule minecraft:keep_inventory true");
    give_cursed(&mut w);
    w.console("kill Bait");
    w.ticks(5);
    assert!(w.dropped().is_empty(), "nothing dropped: {:?}", w.dropped());
    let inv = w.sim.inventory(1).unwrap();
    let held: i32 = inv.iter().flatten().map(|(_, n)| *n).sum();
    assert!(held >= 1 + 5 + 3, "the inventory is intact ({held} items)");
}

#[test]
fn a_horse_and_a_donkey_have_a_mule() {
    let mut w = World::new("creative");
    w.summon_at("minecraft:horse", 3.0, "{Tame:1b,InLove:600,PersistenceRequired:1b}");
    w.summon_at("minecraft:donkey", 4.0, "{Tame:1b,InLove:600,PersistenceRequired:1b}");
    w.ticks(400);
    let mules = w.nbt_of("minecraft:mule");
    assert_eq!(mules.len(), 1, "a mule foal: {:?}", w.sim.mobs());
    assert!(mules[0].get("Age").and_then(|a| a.as_i64()).is_some_and(|a| a < 0), "a baby");
    assert_eq!(w.nbt_of("minecraft:horse").len(), 1);
    assert_eq!(w.nbt_of("minecraft:donkey").len(), 1);
    // Two donkeys have a donkey.
    w.summon_at("minecraft:donkey", -3.0, "{Tame:1b,InLove:600,PersistenceRequired:1b}");
    w.summon_at("minecraft:donkey", -4.0, "{Tame:1b,InLove:600,PersistenceRequired:1b}");
    w.ticks(400);
    assert_eq!(w.nbt_of("minecraft:donkey").len(), 4, "two donkeys, a donkey foal and the first one");
    assert_eq!(w.nbt_of("minecraft:mule").len(), 1, "no more mules");
}

/// Summons `count` of `entity` along a line (each gets its own `finalizeSpawn` seed), then lets
/// them settle.
fn summon_many(w: &mut World, entity: &str, count: usize) {
    for i in 0..count {
        w.summon_at(entity, 4.0 + i as f64 * 0.05, "");
        w.ticks(1);
    }
    w.ticks(3);
}

/// Every rider of `riders_type` sits on a vehicle of `vehicle_type` that lists it; returns how
/// many there are.
fn seated_riders(w: &World, riders_type: &str, vehicle_type: &str) -> usize {
    let mobs = w.sim.mobs();
    let riding = w.sim.riding();
    let mut n = 0;
    for r in mobs.iter().filter(|m| m.1 == riders_type) {
        let Some((_, Some(v), _)) = riding.iter().find(|x| x.0 == r.0) else { continue };
        let vt = mobs.iter().find(|m| m.0 == *v).map(|m| m.1);
        assert_eq!(vt, Some(vehicle_type), "the vehicle of {riders_type} {}", r.0);
        let (_, _, passengers) = riding.iter().find(|x| x.0 == *v).expect("the vehicle is simulated");
        assert!(passengers.contains(&r.0), "{vehicle_type} {v} lists {:?}", passengers);
        n += 1;
    }
    n
}

#[test]
fn some_spiders_spawn_with_a_skeleton_rider() {
    let mut w = World::new("creative");
    summon_many(&mut w, "minecraft:spider", 500);
    let riders = seated_riders(&w, "minecraft:skeleton", "minecraft:spider");
    assert!((1..=20).contains(&riders), "{riders} spider jockeys among 500 spiders");
    assert_eq!(w.sim.mobs().iter().filter(|m| m.1 == "minecraft:skeleton").count(), riders, "every skeleton is a rider");
}

#[test]
fn some_striders_spawn_with_a_piglin_or_a_baby_rider() {
    let mut w = World::new("creative");
    summon_many(&mut w, "minecraft:strider", 400);
    let piglins = seated_riders(&w, "minecraft:zombified_piglin", "minecraft:strider");
    assert!(piglins >= 1, "{piglins} zombified piglin jockeys among 400 striders");
    // The jockey's strider is saddled, and the rider holds a warped fungus on a stick.
    let striders = w.nbt_of("minecraft:strider");
    assert!(striders.iter().filter(|s| s.get("equipment").and_then(|e| e.get("saddle")).is_some()).count() >= piglins);
    for p in w.nbt_of("minecraft:zombified_piglin") {
        let hand = p.get("equipment").and_then(|e| e.get("mainhand")).and_then(|h| h.get("id")).and_then(|i| i.as_str());
        assert_eq!(hand, Some("minecraft:warped_fungus_on_a_stick"));
    }
}
