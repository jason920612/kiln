//! Player combat end to end: attack packets, the attack strength cooldown, the hurt cooldown,
//! armor, sweeping, PvP rules, death messages and durability. The expected numbers are
//! vanilla 26.3's (tools/combat_vectors.py replays many more scenarios against the real
//! server).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    clients: Vec<Client>,
}

impl World {
    /// Players named `names` (connections 1, 2, ...), loaded and standing at the spawn.
    fn new(names: &[&str]) -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let mut clients = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let (msg, stats) = join(i as u64 + 1, name, 2);
            assert!(sim.step([msg]));
            clients.push(Client::new(i as u64 + 1, stats));
        }
        let mut w = World { sim, clients };
        w.ticks(5);
        // Health only changes by the hits under test.
        w.console("gamerule minecraft:natural_health_regeneration false");
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            self.step(Vec::new());
        }
    }

    /// One tick: `extra` packets, then every client's own tick.
    fn step(&mut self, extra: Vec<ToSim>) {
        let mut inbox = extra;
        for c in self.clients.iter_mut() {
            c.tick(None, &mut inbox);
        }
        assert!(self.sim.step(inbox));
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    /// Puts an item in a slot of a (creative) player's inventory menu.
    fn give(&mut self, conn: u64, slot: i16, item: &str) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(conn, PlayIn::SetCreativeSlot { slot, item: Some(stack) })]));
    }

    /// Moves `name` to the first player's position plus `offset`.
    fn place(&mut self, name: &str, offset: [f64; 3]) {
        let base = self.clients[0].pos;
        self.console(&format!("tp {name} {} {} {}", base[0] + offset[0], base[1] + offset[1], base[2] + offset[2]));
        self.ticks(2);
    }

    fn attack(&mut self, attacker: u64, target: u64) {
        let entity_id = self.sim.entity_id(target).unwrap();
        self.step(vec![
            ToSim::Packet(attacker, PlayIn::Attack { entity_id }),
            ToSim::Packet(attacker, PlayIn::Punch),
        ]);
    }

    fn health(&self, conn: u64) -> f32 {
        self.sim.health(conn).unwrap().0
    }
}

/// An attacker with a diamond sword and a survival target two blocks in front of it.
fn duel() -> World {
    let mut w = World::new(&["Attacker", "Target"]);
    w.give(1, 36, "minecraft:diamond_sword");
    w.console("gamemode survival Target");
    w.place("Target", [0.0, 0.0, 2.0]);
    // Past the sword's 12.5-tick attack strength delay.
    w.ticks(20);
    w
}

#[test]
fn sword_hits_follow_attack_strength_and_the_hurt_cooldown() {
    let mut w = duel();
    // Full strength: the sword's 7 attack damage.
    w.attack(1, 2);
    assert_eq!(w.health(2), 13.0);
    // Right away again: a weak hit (the swing reset the attack strength) does not exceed the
    // last one while the target's hurt cooldown is above half.
    w.attack(1, 2);
    assert_eq!(w.health(2), 13.0);
    // 13 ticks later: full strength again, and the cooldown is down to 5.
    w.ticks(13);
    w.attack(1, 2);
    assert_eq!(w.health(2), 6.0);
}

#[test]
fn a_weak_hit_scales_with_the_square_of_attack_strength() {
    let mut w = duel();
    w.attack(1, 2);
    assert_eq!(w.health(2), 13.0);
    // Wait out the hurt cooldown (20 ticks), with a swing 5 ticks before the next attack:
    // vanilla's `diamond_sword_ticker_5` vector.
    w.ticks(20);
    w.step(vec![ToSim::Packet(1, PlayIn::Punch)]);
    w.ticks(4);
    w.attack(1, 2);
    assert_eq!(w.health(2), 13.0 - 2.48416);
}

#[test]
fn out_of_reach_players_and_pvp_off_take_no_damage() {
    let mut w = duel();
    w.place("Target", [0.0, 0.0, 9.0]);
    w.ticks(20);
    w.attack(1, 2);
    assert_eq!(w.health(2), 20.0, "out of reach");
    w.place("Target", [0.0, 0.0, 2.0]);
    w.console("gamerule minecraft:pvp false");
    w.ticks(20);
    w.attack(1, 2);
    assert_eq!(w.health(2), 20.0, "pvp off");
    w.console("gamerule minecraft:pvp true");
    w.console("gamemode creative Target");
    w.ticks(20);
    w.attack(1, 2);
    assert_eq!(w.health(2), 20.0, "creative players are invulnerable");
}

#[test]
fn armor_reduces_damage_and_wears() {
    let mut w = World::new(&["Attacker", "Target"]);
    w.give(1, 36, "minecraft:diamond_sword");
    for (slot, item) in [(5, "iron_helmet"), (6, "iron_chestplate"), (7, "iron_leggings"), (8, "iron_boots")] {
        w.give(2, slot, &format!("minecraft:{item}"));
    }
    w.console("gamemode survival Target");
    w.place("Target", [0.0, 0.0, 2.0]);
    w.ticks(20);
    w.attack(1, 2);
    // 15 armor against 7 damage: vanilla's `armor_iron` vector.
    assert_eq!(w.health(2), 16.220001);
    for slot in 5..=8 {
        assert_eq!(w.sim.item_damage(2, slot), Some(1), "armor slot {slot}");
    }
}

#[test]
fn a_sword_sweep_hits_players_next_to_the_target() {
    let mut w = World::new(&["Attacker", "Target", "Bystander"]);
    w.give(1, 36, "minecraft:diamond_sword");
    w.console("gamemode survival Target");
    w.console("gamemode survival Bystander");
    w.place("Target", [0.0, 0.0, 2.0]);
    w.place("Bystander", [1.0, 0.0, 2.2]);
    w.ticks(20);
    w.attack(1, 2);
    assert_eq!(w.health(2), 13.0);
    // Sweeping: 1 damage (no sweeping edge).
    assert_eq!(w.health(3), 19.0);
}

#[test]
fn a_killing_blow_names_the_attacker() {
    let mut w = duel();
    let log = w.clients[1].stats.clone();
    *log.log.lock().unwrap() = Some(Vec::new());
    for _ in 0..3 {
        w.attack(1, 2);
        w.ticks(20);
    }
    assert_eq!(w.sim.health(2), Some((0.0, true)));
    let packets = log.log.lock().unwrap().take().unwrap();
    let kill = kiln_data::packets::play::clientbound::PLAYER_COMBAT_KILL;
    let death = packets.iter().find(|p| kiln_proto::codec::Reader::new(p).varint().ok() == Some(kill)).expect("death screen");
    let text = String::from_utf8_lossy(death);
    assert!(text.contains("death.attack.player") && text.contains("Attacker") && text.contains("Target"), "{text}");
}

#[test]
fn attacking_yourself_disconnects() {
    let mut w = duel();
    w.attack(1, 1);
    assert!(w.clients[0].stats.disconnected.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn survival_hits_and_mining_wear_tools() {
    let mut w = World::new(&["Miner", "Target"]);
    w.give(1, 36, "minecraft:diamond_pickaxe");
    w.console("gamemode survival Miner");
    w.console("gamemode survival Target");
    w.place("Target", [0.0, 0.0, 2.0]);
    w.ticks(20);
    w.attack(1, 2);
    // Pickaxes are weapons that lose 2 durability per hit; 1 + 4 attack damage.
    assert_eq!(w.health(2), 15.0);
    assert_eq!(w.sim.item_damage(1, 36), Some(2));
    // A stone block beside the miner: 1 more durability when it breaks.
    let at = w.clients[0].pos.map(|c| c.floor() as i32);
    let stone = [at[0] + 1, at[1], at[2]];
    w.console(&format!("setblock {} {} {} minecraft:stone", stone[0], stone[1], stone[2]));
    w.step(vec![ToSim::Packet(1, PlayIn::PlayerAction { action: 0, pos: stone, face: 4, sequence: 1 })]);
    w.ticks(8);
    w.step(vec![ToSim::Packet(1, PlayIn::PlayerAction { action: 3, pos: stone, face: 4, sequence: 2 })]);
    assert_eq!(w.sim.block_at(stone[0], stone[1], stone[2]), Some(kiln_data::blocks::default_state::AIR));
    assert_eq!(w.sim.item_damage(1, 36), Some(3));
}

/// Mending: orb points repair a damaged mending item two durability per point before the
/// player gets any; what the repair does not use goes to the player.
#[test]
fn mending_spends_orb_points_on_repairs() {
    // Enchantment effects come from the vanilla datapack: the repository's when KILN_DATAPACK
    // is not set (the simulation looks relative to the working directory, the crate here).
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        if !dir.join("data/minecraft/enchantment").is_dir() {
            eprintln!("skipped: no datapack (KILN_DATAPACK)");
            return;
        }
        unsafe { std::env::set_var("KILN_DATAPACK", dir) };
    }
    let mut w = World::new(&["Mender"]);
    w.console("gamemode survival Mender");
    w.console("give Mender minecraft:diamond_pickaxe[minecraft:damage=25,minecraft:enchantments={\"minecraft:mending\":1}]");
    w.ticks(2);
    if w.sim.item_damage(1, 36) != Some(25) {
        eprintln!("skipped: no datapack (KILN_DATAPACK)");
        return;
    }
    let orb = |w: &mut World, value: i32| {
        let p = w.clients[0].pos;
        w.console(&format!("summon minecraft:experience_orb {} {} {} {{Value:{value}}}", p[0], p[1], p[2]));
        w.ticks(4);
    };
    orb(&mut w, 10);
    // 10 points repair 20; nothing left for the player.
    assert_eq!(w.sim.item_damage(1, 36), Some(5));
    assert_eq!(w.sim.experience(1).unwrap().2, 0);
    orb(&mut w, 10);
    // 5 repaired use 10 - 5 * 10 / 20 = 8 points less: 8 go to the player.
    assert_eq!(w.sim.item_damage(1, 36), Some(0));
    assert_eq!(w.sim.experience(1).unwrap().2, 8);
}

#[test]
fn viewers_see_held_items_and_armor() {
    let mut w = World::new(&["Holder", "Watcher"]);
    w.console("gamemode creative Holder");
    w.ticks(2);
    let equipment = kiln_data::packets::play::clientbound::SET_EQUIPMENT;
    let watcher = w.clients[1].stats.clone();
    watcher.count_ids.store(true, std::sync::atomic::Ordering::Relaxed);
    let count = || watcher.by_id.lock().unwrap().get(&equipment).map_or(0, |e| e.0);
    let before = count();
    w.give(1, 36, "minecraft:diamond_sword");
    w.give(1, 5, "minecraft:iron_helmet");
    w.ticks(2);
    assert!(count() > before, "the watcher got Set Equipment");
    // Nothing changes, nothing more is sent.
    let settled = count();
    w.ticks(5);
    assert_eq!(count(), settled);
}

#[test]
fn raised_shields_block_until_an_axe_disables_them() {
    use kiln_proto::packets::serverbound::Hand;
    let mut w = World::new(&["Attacker", "Target"]);
    w.give(1, 36, "minecraft:diamond_sword");
    w.give(1, 37, "minecraft:diamond_axe");
    w.give(2, 36, "minecraft:shield");
    w.console("gamemode survival Target");
    w.place("Target", [0.0, 0.0, 2.0]);
    // The target faces the attacker (north) and raises the shield.
    let turn = PlayIn::Move { pos: None, rot: Some([180.0, 0.0]), on_ground: true, horizontal_collision: false };
    let raise = PlayIn::UseItem { hand: Hand::Main, sequence: 1, yaw: 180.0, pitch: 0.0 };
    w.step(vec![ToSim::Packet(2, turn), ToSim::Packet(2, raise)]);
    // Blocking starts a quarter second into the use.
    w.ticks(20);
    w.attack(1, 2);
    assert_eq!(w.health(2), 20.0, "the shield took the sword's hit");
    assert_eq!(w.sim.item_damage(2, 36), Some(8), "1 + the 7 blocked");
    // An axe knocks the shield down for 5 seconds; the next hit lands.
    w.step(vec![ToSim::Packet(1, PlayIn::SetCarriedItem { slot: 1 })]);
    w.ticks(25);
    w.attack(1, 2);
    assert_eq!(w.health(2), 20.0, "blocked, but the shield is disabled");
    w.ticks(20);
    w.attack(1, 2);
    assert!(w.health(2) < 20.0, "the lowered shield blocks nothing: {}", w.health(2));
}

/// What the last `update_attributes` packet to a client says about `attribute`:
/// (base, the modifier amounts), if it lists it.
fn told_attribute(packets: &[bytes::Bytes], entity_id: i32, attribute: &str) -> Option<(f64, Vec<f64>)> {
    use kiln_proto::codec::Reader;
    let want = kiln_data::builtin_id("minecraft:attribute", attribute).unwrap();
    let mut found = None;
    for p in packets {
        let mut r = Reader::new(p);
        if r.varint().ok() != Some(kiln_data::packets::play::clientbound::UPDATE_ATTRIBUTES) || r.varint().ok() != Some(entity_id) {
            continue;
        }
        for _ in 0..r.varint().unwrap() {
            let (id, base) = (r.varint().unwrap(), r.f64().unwrap());
            let mods: Vec<f64> = (0..r.varint().unwrap())
                .map(|_| {
                    r.string(256).unwrap();
                    let amount = r.f64().unwrap();
                    r.varint().unwrap();
                    amount
                })
                .collect();
            if id == want {
                found = Some((base, mods));
            }
        }
    }
    found
}

/// wp59: the real client's armor bar stayed empty: the server computed the armor points but
/// only told clients six attributes (the ones effects change) and never when the equipment did.
#[test]
fn armor_and_tool_attributes_reach_the_client_when_equipment_changes() {
    let mut w = World::new(&["Wearer"]);
    w.ticks(5);
    let stats = w.clients[0].stats.clone();
    *stats.log.lock().unwrap() = Some(Vec::new());
    let me = w.sim.entity_id(1).unwrap();
    for (slot, item) in [(5, "diamond_helmet"), (6, "diamond_chestplate"), (7, "diamond_leggings"), (8, "diamond_boots")] {
        w.give(1, slot, &format!("minecraft:{item}"));
    }
    w.ticks(5);
    w.console("gamemode survival Wearer");
    w.ticks(5);
    let packets = stats.log.lock().unwrap().clone().unwrap();
    let armor = told_attribute(&packets, me, "minecraft:armor").expect("the armor attribute is sent when armor is put on");
    assert_eq!(armor.1.len(), 4, "one modifier per piece: {armor:?}");
    assert_eq!(armor.1.iter().sum::<f64>(), 20.0, "diamond armor is 3+8+6+3 points");
    let toughness = told_attribute(&packets, me, "minecraft:armor_toughness").expect("armor toughness too");
    assert_eq!(toughness.1.iter().sum::<f64>(), 8.0, "4 x 2");
    // And when it comes off again the client hears the modifiers are gone.
    *stats.log.lock().unwrap() = Some(Vec::new());
    w.console("item replace entity Wearer armor.chest with minecraft:air");
    w.ticks(5);
    let packets = stats.log.lock().unwrap().clone().unwrap();
    let armor = told_attribute(&packets, me, "minecraft:armor").expect("the change is sent");
    assert_eq!(armor.1.len(), 3, "{armor:?}");
}

/// wp60: the client predicts its own movement from gravity, step height, jump strength and
/// sneaking speed, draws other players at their scale and checks reach with the interaction
/// ranges, but the server only told it 12 of vanilla's 30 syncable attributes.
#[test]
fn every_syncable_attribute_reaches_the_client() {
    let mut w = World::new(&["Tweaker"]);
    w.ticks(5);
    let stats = w.clients[0].stats.clone();
    *stats.log.lock().unwrap() = Some(Vec::new());
    let me = w.sim.entity_id(1).unwrap();
    for (attr, v) in [
        ("scale", 2.0),
        ("gravity", 0.04),
        ("step_height", 1.5),
        ("jump_strength", 0.8),
        ("sneaking_speed", 0.5),
        ("block_interaction_range", 7.0),
        ("entity_interaction_range", 5.0),
        ("movement_efficiency", 0.5),
        ("fall_damage_multiplier", 0.25),
        ("burning_time", 0.5),
    ] {
        w.console(&format!("attribute Tweaker minecraft:{attr} base set {v}"));
    }
    w.ticks(3);
    let packets = stats.log.lock().unwrap().clone().unwrap();
    for (attr, v) in [
        ("scale", 2.0),
        ("gravity", 0.04),
        ("step_height", 1.5),
        ("jump_strength", 0.8),
        ("sneaking_speed", 0.5),
        ("block_interaction_range", 7.0),
        ("entity_interaction_range", 5.0),
        ("movement_efficiency", 0.5),
        ("fall_damage_multiplier", 0.25),
        ("burning_time", 0.5),
    ] {
        let told = told_attribute(&packets, me, &format!("minecraft:{attr}")).unwrap_or_else(|| panic!("{attr} is sent"));
        assert_eq!(told.0, v, "{attr}: {told:?}");
    }
    // The server-only attributes stay home.
    assert!(told_attribute(&packets, me, "minecraft:attack_damage").is_none());
    assert!(told_attribute(&packets, me, "minecraft:knockback_resistance").is_none());
    // Creative mode's reach modifiers are sent when the mode changes.
    w.console("gamemode survival Tweaker");
    w.ticks(3);
    *stats.log.lock().unwrap() = Some(Vec::new());
    w.console("gamemode creative Tweaker");
    w.ticks(3);
    let packets = stats.log.lock().unwrap().clone().unwrap();
    assert_eq!(told_attribute(&packets, me, "minecraft:block_interaction_range").expect("block reach").1, vec![0.5]);
    assert_eq!(told_attribute(&packets, me, "minecraft:entity_interaction_range").expect("entity reach").1, vec![2.0]);
}

/// wp60: clients never heard a mob's attributes, so a horse the player rode used the client's
/// default speed and jump strength instead of its own rolled ones.
#[test]
fn a_mobs_attributes_are_sent_when_it_appears_and_when_they_change() {
    let mut w = World::new(&["Rider"]);
    w.ticks(5);
    let stats = w.clients[0].stats.clone();
    *stats.log.lock().unwrap() = Some(Vec::new());
    let me = w.sim.entity_id(1).unwrap();
    w.console("summon minecraft:horse ~2 ~ ~");
    w.ticks(5);
    let packets = stats.log.lock().unwrap().clone().unwrap();
    let horse = horse_id(&packets, me).expect("an attribute packet for the horse");
    let speed = told_attribute(&packets, horse, "minecraft:movement_speed").expect("speed");
    let jump = told_attribute(&packets, horse, "minecraft:jump_strength").expect("jump");
    assert!((0.1125..=0.3375).contains(&speed.0), "a rolled horse speed, not the default 0.7: {speed:?}");
    assert!((0.4..=1.0).contains(&jump.0), "a rolled jump strength: {jump:?}");
    assert!(told_attribute(&packets, horse, "minecraft:follow_range").is_none(), "follow range is server-only");
    // A change after the viewers have it is sent again.
    *stats.log.lock().unwrap() = Some(Vec::new());
    w.console("attribute @e[type=minecraft:horse,limit=1] minecraft:scale base set 2");
    w.console("effect give @e[type=minecraft:horse,limit=1] minecraft:speed 30 1");
    w.ticks(3);
    let packets = stats.log.lock().unwrap().clone().unwrap();
    assert_eq!(told_attribute(&packets, horse, "minecraft:scale").expect("scale").0, 2.0);
    assert_eq!(told_attribute(&packets, horse, "minecraft:movement_speed").expect("speed modifier").1.len(), 1);
}

/// The entity id of the one non-player entity an Update Attributes packet in `packets` is about.
fn horse_id(packets: &[bytes::Bytes], me: i32) -> Option<i32> {
    use kiln_proto::codec::Reader;
    packets.iter().find_map(|p| {
        let mut r = Reader::new(p);
        if r.varint().ok()? != kiln_data::packets::play::clientbound::UPDATE_ATTRIBUTES {
            return None;
        }
        let id = r.varint().ok()?;
        (id != me).then_some(id)
    })
}

/// wp61 (a real client next to a vanilla server): a successful use of a block (a lever here) moves
/// the user's arm for the players who watch (`swingAndResetAttackStrength`), and a click on nothing
/// does not.
#[test]
fn using_a_block_makes_the_arm_swing_for_the_others() {
    let mut w = World::new(&["Actor", "Watcher"]);
    w.console("gamemode survival Actor");
    w.clients[1].stats.count_ids.store(true, std::sync::atomic::Ordering::Relaxed);
    let base = w.clients[0].pos;
    let at = [base[0].floor() as i32 + 1, base[1].floor() as i32, base[2].floor() as i32];
    w.console(&format!("setblock {} {} {} minecraft:lever[face=floor,facing=east]", at[0], at[1], at[2]));
    w.ticks(3);
    let swings = |w: &World| w.clients[1].stats.by_id.lock().unwrap().get(&kiln_data::packets::play::clientbound::SWING_ANIMATION).map_or(0, |a| a.0);
    let before = swings(&w);
    w.step(vec![ToSim::Packet(1, PlayIn::UseItemOn { hand: 0, pos: at, face: 1, cursor: [0.5, 0.2, 0.5], inside: false, sequence: 1 })]);
    w.ticks(2);
    assert_eq!(swings(&w) - before, 1, "the lever click swung the arm once for the watcher");
    // A click on air (nothing to use) swings nothing.
    let air = [at[0] + 3, at[1] + 5, at[2]];
    w.step(vec![ToSim::Packet(1, PlayIn::UseItemOn { hand: 0, pos: air, face: 1, cursor: [0.5, 0.5, 0.5], inside: false, sequence: 2 })]);
    w.ticks(10);
    assert_eq!(swings(&w) - before, 1);
}
