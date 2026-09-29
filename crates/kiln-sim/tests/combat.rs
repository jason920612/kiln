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
    let turn = PlayIn::Move { pos: None, rot: Some([180.0, 0.0]), on_ground: true };
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
