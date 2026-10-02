//! Replays the vanilla combat vectors of `tools/CombatVectors.java` (`KILN_COMBAT_VECTORS`,
//! written by `tools/combat_vectors.py`) through the simulation: each scenario's players are
//! set up as recorded, the attacker sends an Attack packet, and health, absorption, hurt
//! cooldown, exhaustion, fire ticks, item and armor durability, knockback motion packets and
//! death messages must match vanilla's bit for bit. Enchanted scenarios reseed the randoms the
//! way the Java side does (the level random becomes the attacker's level random). Skipped when
//! the vectors are not there.

use crate::testing::{Client, SinkStats, join};
use crate::{Sim, SimConfig};
use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use serde_json::Value;
use std::sync::Arc;

fn f32_of(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

fn vec_of(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
}

fn stack(name: &Value, damage: i64, custom_name: &Value, enchantments: &Value) -> kiln_item::ItemStack {
    let Some(name) = name.as_str() else { return kiln_item::ItemStack::empty() };
    let mut s = kiln_item::ItemStack::of(name, 1).unwrap_or_else(|| panic!("unknown item {name}"));
    if damage > 0 {
        s.insert(kiln_item::keys::DAMAGE, damage as i32);
    }
    if let Some(n) = custom_name.as_str() {
        s.insert(kiln_item::keys::CUSTOM_NAME, kiln_item::Text::literal(n));
    }
    enchant(&mut s, enchantments);
    s
}

/// Adds `{"minecraft:sharpness": 5, ...}` to the stack's `enchantments`, in the listed order.
pub(crate) fn enchant(s: &mut kiln_item::ItemStack, enchantments: &Value) {
    let Some(map) = enchantments.as_object() else { return };
    if map.is_empty() {
        return;
    }
    let mut e = s.get(kiln_item::keys::ENCHANTMENTS).cloned().unwrap_or_default();
    for (id, level) in map {
        let id = kiln_item::registry::ENCHANTMENT.id(id).unwrap_or_else(|| panic!("unknown enchantment {id}"));
        e.set(id, level.as_i64().unwrap() as i32);
    }
    s.insert(kiln_item::keys::ENCHANTMENTS, e);
}

/// The vanilla datapack's loot data (enchantment definitions), loaded once.
pub(crate) fn vanilla_loot() -> Option<Arc<kiln_loot::LootData>> {
    static LOOT: std::sync::OnceLock<Option<Arc<kiln_loot::LootData>>> = std::sync::OnceLock::new();
    LOOT.get_or_init(|| {
        let work = std::env::var_os("KILN_WORK")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        dir.join("data").is_dir().then(|| Arc::new(kiln_loot::LootData::load(&dir).expect("vanilla datapack")))
    })
    .clone()
}

/// Puts a player in the recorded state (`CombatVectors.setup`), `base` being the attacker's
/// position.
fn setup(sim: &mut Sim, conn: u64, side: &Value, base: [f64; 3]) {
    let p = sim.players.get_mut(&conn).unwrap();
    let rel = vec_of(&side["pos"]);
    p.pos = [base[0] + rel[0], base[1] + rel[1], base[2] + rel[2]];
    p.rot = [f32_of(&side["yaw"]), 0.0];
    p.on_ground = side["on_ground"].as_bool().unwrap();
    p.sprinting = side["sprinting"].as_bool().unwrap();
    p.fall_distance = side["fall_distance"].as_f64().unwrap();
    p.attack_ticker = side["ticker"].as_i64().unwrap() as i32;
    p.game_mode = match side["game_mode"].as_str().unwrap() {
        "creative" => 1,
        "adventure" => 2,
        "spectator" => 3,
        _ => 0,
    };
    p.health = f32_of(&side["health"]);
    p.absorption = f32_of(&side["absorption"]);
    p.hurt_cooldown = side["hurt_cooldown"].as_i64().unwrap() as i32;
    p.last_hurt = f32_of(&side["last_hurt"]);
    p.known_movement = vec_of(&side["known_movement"]);
    p.vel = [0.0; 3];
    p.food = 17;
    p.saturation = 0.0;
    p.exhaustion = 0.0;
    p.food_timer = 0;
    p.fire_ticks = 0;
    p.loot = vanilla_loot();
    p.inv = kiln_inventory::PlayerInventory::new();
    p.inv.items[0] = stack(
        &side["main_hand"],
        side["main_hand_damage"].as_i64().unwrap(),
        &side["custom_name"],
        &side["main_hand_enchantments"],
    );
    let armor = side["armor"].as_array().unwrap();
    let armor_damage = side["armor_damage"].as_array().unwrap();
    for i in 0..4 {
        // Equipment order: feet, legs, chest, head.
        p.inv.equipment[i] =
            stack(&armor[i], armor_damage[i].as_i64().unwrap(), &Value::Null, &side["armor_enchantments"][i]);
    }
    // `detectEquipmentUpdates`: attributes follow the equipment.
    for (i, slot) in crate::combat::SLOTS.iter().enumerate() {
        p.equipment_seen[i] = p.inv.equipped(*slot).clone();
    }
}

/// Packets `stats` received with this id.
fn packets_with_id(stats: &SinkStats, id: i32) -> Vec<bytes::Bytes> {
    let log = stats.log.lock().unwrap();
    log.iter()
        .flatten()
        .filter(|p| kiln_proto::codec::Reader::new(p).varint().ok() == Some(id))
        .cloned()
        .collect()
}

/// The text of a component as `Component.getString` would print it.
fn plain(tag: &Tag) -> String {
    match tag {
        Tag::String(s) => s.clone(),
        Tag::Compound(entries) => {
            let get = |k: &str| entries.iter().find(|(n, _)| n == k).map(|(_, v)| v);
            let mut out = String::new();
            if let Some(Tag::String(t)) = get("text") {
                out.push_str(t);
            }
            if let Some(Tag::String(key)) = get("translate") {
                let args: Vec<String> = match get("with") {
                    Some(Tag::List(items)) => items.iter().map(plain).collect(),
                    _ => Vec::new(),
                };
                if key == "chat.square_brackets" {
                    out.push_str(&format!("[{}]", args.join("")));
                } else {
                    out.push_str(key);
                }
            }
            if let Some(Tag::List(extra)) = get("extra") {
                extra.iter().for_each(|e| out.push_str(&plain(e)));
            }
            out
        }
        Tag::List(items) => items.iter().map(plain).collect(),
        _ => String::new(),
    }
}

/// The death message a player's client got: (translation key, plain arguments).
fn death_message(stats: &SinkStats) -> Option<(String, Vec<String>)> {
    let pkt = packets_with_id(stats, kiln_data::packets::play::clientbound::PLAYER_COMBAT_KILL).pop()?;
    let mut r = kiln_proto::codec::Reader::new(&pkt);
    r.varint().unwrap();
    r.varint().unwrap();
    let rest = &pkt[pkt.len() - r.remaining()..];
    let (tag, _) = kiln_proto::nbt::read_network(rest).unwrap();
    let Tag::Compound(entries) = &tag else { return None };
    let key = entries.iter().find_map(|(k, v)| (k == "translate").then(|| v.as_str().unwrap().to_owned()))?;
    let args = match entries.iter().find(|(k, _)| k == "with") {
        Some((_, Tag::List(items))) => items.iter().map(plain).collect(),
        _ => Vec::new(),
    };
    Some((key, args))
}

/// The motion packet a player got for itself, if any.
fn motion_packet(stats: &SinkStats, entity_id: i32) -> Option<bytes::Bytes> {
    packets_with_id(stats, kiln_data::packets::play::clientbound::SET_ENTITY_MOTION).into_iter().find(|p| {
        let mut r = kiln_proto::codec::Reader::new(p);
        r.varint().unwrap();
        r.varint().ok() == Some(entity_id)
    })
}

fn check_side(sim: &Sim, conn: u64, stats: &SinkStats, want: &Value, errors: &mut Vec<String>, who: &str) {
    let p = &sim.players[&conn];
    let mut eq = |what: &str, got: String, expected: String| {
        if got != expected {
            errors.push(format!("{who}.{what}: kiln {got}, vanilla {expected}"));
        }
    };
    eq("health", format!("{:?}", p.health), format!("{:?}", f32_of(&want["health"])));
    eq("absorption", format!("{:?}", p.absorption), format!("{:?}", f32_of(&want["absorption"])));
    eq("exhaustion", format!("{:?}", p.exhaustion), format!("{:?}", f32_of(&want["exhaustion"])));
    eq("last_hurt", format!("{:?}", p.last_hurt), format!("{:?}", f32_of(&want["last_hurt"])));
    // The simulation's tick after the attack counted the cooldown down once.
    let cooldown = want["hurt_cooldown"].as_i64().unwrap() as i32;
    eq("hurt_cooldown", format!("{}", p.hurt_cooldown), format!("{}", (cooldown - 1).max(0)));
    eq("sprinting", format!("{}", p.sprinting), format!("{}", want["sprinting"].as_bool().unwrap()));
    // Like the hurt cooldown, fire counted down once in the tick after the attack; a player
    // that does not burn rests at -20 (`applyEffectsFromBlocks`).
    let fire = want["fire_ticks"].as_i64().unwrap_or(0) as i32;
    let fire = if fire > 0 { fire - 1 } else { -crate::hazards::FIRE_IMMUNE_TICKS };
    eq("fire_ticks", format!("{}", p.fire_ticks), format!("{fire}"));
    let main = p.inv.selected_item();
    let main_name = (!main.is_empty()).then(|| main.item_name().to_owned());
    eq("main_hand", format!("{main_name:?}"), format!("{:?}", want["main_hand"].as_str().map(str::to_owned)));
    eq("main_hand_damage", format!("{}", main.damage()), format!("{}", want["main_hand_damage"].as_i64().unwrap()));
    let armor: Vec<Option<i64>> =
        (0..4).map(|i| (!p.inv.equipment[i].is_empty()).then(|| p.inv.equipment[i].damage() as i64)).collect();
    let want_armor: Vec<Option<i64>> = want["armor_damage"].as_array().unwrap().iter().map(|v| v.as_i64()).collect();
    eq("armor_damage", format!("{armor:?}"), format!("{want_armor:?}"));
    let motion_key = if want.get("pending_motion").is_some() { "pending_motion" } else { "motion" };
    let expected_motion = want[motion_key].as_array().map(|_| kiln_proto::packets::entity::set_entity_motion(p.entity_id, vec_of(&want[motion_key])));
    let got_motion = motion_packet(stats, p.entity_id);
    eq("motion", format!("{got_motion:?}"), format!("{expected_motion:?}"));
    let death = death_message(stats);
    let want_death = want["death"].as_str().map(|k| {
        let args: Vec<String> = want["death_args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect();
        (k.to_owned(), args)
    });
    eq("death", format!("{death:?}"), format!("{want_death:?}"));
}

/// Vanilla names the mock players "Attacker<n>", "Target<n>"; Kiln's replay uses the same.
fn run_scenario(line: &Value) -> Vec<String> {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let names = ["Attacker", "Target", "Bystander"];
    let n = line["name"].as_str().unwrap();
    let with_bystander = !line["bystander"].is_null();
    let count = if with_bystander { 3 } else { 2 };
    let suffix = death_suffix(line);
    let mut clients = Vec::new();
    let mut stats: Vec<Arc<SinkStats>> = Vec::new();
    for (i, name) in names.iter().take(count).enumerate() {
        let conn = i as u64 + 1;
        let (msg, s) = join(conn, &format!("{name}{suffix}"), 2);
        assert!(sim.step([msg]));
        clients.push(Client::new(conn, s.clone()));
        stats.push(s);
    }
    for _ in 0..5 {
        let mut inbox = Vec::new();
        for c in clients.iter_mut() {
            c.tick(None, &mut inbox);
        }
        assert!(sim.step(inbox));
    }
    let mut console = vec![ToSim::Console(format!("difficulty {}", line["difficulty"].as_str().unwrap()))];
    if !line["pvp"].as_bool().unwrap() {
        console.push(ToSim::Console("gamerule minecraft:pvp false".into()));
    }
    assert!(sim.step(console));
    let base = sim.players[&1].pos;
    setup(&mut sim, 1, &line["attacker"], base);
    setup(&mut sim, 2, &line["target"], base);
    if with_bystander {
        setup(&mut sim, 3, &line["bystander"], base);
    }
    // `CombatVectors.run` reseeds the level's random and each player's entity random.
    let seed = line["level_seed"].as_i64().unwrap_or(0);
    for (conn, offset) in [(1u64, 1i64), (2, 2), (3, 3)] {
        if let Some(p) = sim.players.get_mut(&conn) {
            p.entity_rng = kiln_javamath::random::LegacyRandom::new(seed.wrapping_add(offset));
        }
    }
    sim.players.get_mut(&1).unwrap().level_rng = kiln_javamath::random::LegacyRandom::new(seed);
    for s in &stats {
        *s.log.lock().unwrap() = Some(Vec::new());
    }
    // `startAutoSpinAttack(20, 8.0F, main hand)`.
    if line["spin"].as_bool() == Some(true) {
        let p = sim.players.get_mut(&1).unwrap();
        let held = p.inv.selected_item().clone();
        p.start_spin_attack(20, 8.0, held, false);
    }
    let target_id = sim.players[&2].entity_id;
    assert!(sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: target_id })]), "{n}");
    let result = &line["result"];
    let mut errors = Vec::new();
    check_side(&sim, 1, &stats[0], &result["attacker"], &mut errors, "attacker");
    check_side(&sim, 2, &stats[1], &result["target"], &mut errors, "target");
    if with_bystander {
        check_side(&sim, 3, &stats[2], &result["bystander"], &mut errors, "bystander");
    }
    errors
}

/// The number vanilla appended to the players' names (from the recorded death message).
fn death_suffix(line: &Value) -> String {
    // The target's death message, or the attacker's (killed by thorns).
    ["target", "attacker"]
        .iter()
        .find_map(|who| {
            let args = &line["result"][who]["death_args"];
            args.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()).map(|s| {
                s.trim_start_matches(|c: char| c.is_ascii_alphabetic()).to_owned()
            })
        })
        .unwrap_or_default()
}

#[test]
fn combat_parity() {
    let Some(path) = std::env::var_os("KILN_COMBAT_VECTORS") else {
        eprintln!("skipped: set KILN_COMBAT_VECTORS (tools/combat_vectors.py)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        let errors = run_scenario(&v);
        if errors.is_empty() {
            passed += 1;
            println!("ok   {name}");
        } else {
            println!("FAIL {name}\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    println!("combat parity: {passed} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "failed: {failed:?}");
}

// ---------------------------------------------------------------------------------------------
// Riptide: the lift of a release on the ground and the spin's touch check (`riptide.jsonl`,
// written next to the combat vectors by CombatVectors; `KILN_RIPTIDE_VECTORS`).

/// A level with the riptiding player ("Rip", connection 1) and one who watches ("Watcher").
fn riptide_sim() -> (Sim, Vec<Client>, Vec<Arc<SinkStats>>, [f64; 3]) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Rip", 2);
    let (msg2, stats2) = join(2, "Watcher", 2);
    assert!(sim.step([msg, msg2, ToSim::Console("gamemode survival Rip".into()), ToSim::Console("gamemode spectator Watcher".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut clients = vec![Client::new(1, stats.clone()), Client::new(2, stats2.clone())];
    for _ in 0..5 {
        let mut inbox = Vec::new();
        for c in clients.iter_mut() {
            c.tick(None, &mut inbox);
        }
        assert!(sim.step(inbox));
    }
    let base = sim.players[&1].pos;
    (sim, clients, vec![stats, stats2], base)
}

fn see_equipment(p: &mut crate::Player) {
    for (i, slot) in crate::combat::SLOTS.iter().enumerate() {
        p.equipment_seen[i] = p.inv.equipped(*slot).clone();
    }
}

fn console(sim: &mut Sim, command: String) {
    assert!(sim.step([ToSim::Console(command)]));
}

fn run_lift(v: &Value) -> Vec<String> {
    let (mut sim, _client, stats, base) = riptide_sim();
    let (bx, by, bz) = (base[0].floor() as i32, base[1].round() as i32, base[2].floor() as i32);
    for dx in 0..2 {
        for dz in 0..2 {
            console(&mut sim, format!("setblock {} {} {} minecraft:water", bx + dx, by, bz + dz));
        }
    }
    if let Some(spec) = v["ceiling"].as_str().filter(|s| !s.is_empty()) {
        let mut parts = spec.splitn(2, ':');
        let y: i32 = parts.next().unwrap().parse().unwrap();
        let rest = parts.next().unwrap();
        let (block, edge) = match rest.strip_suffix(":edge") {
            Some(b) => (b, true),
            None => (rest, false),
        };
        for x in if edge { 1 } else { -1 }..=1 {
            for z in -1..=1 {
                console(&mut sim, format!("setblock {} {} {} {block}", bx + x, by + y, bz + z));
            }
        }
    }
    let (yaw, pitch) = (v["yaw"].as_f64().unwrap() as f32, v["pitch"].as_f64().unwrap() as f32);
    let level = v["level"].as_i64().unwrap();
    {
        let p = sim.players.get_mut(&1).unwrap();
        p.pos = [bx as f64 + 0.5 + v["dx"].as_f64().unwrap(), base[1], bz as f64 + 0.5];
        p.rot = [yaw, pitch];
        p.on_ground = v["on_ground"].as_bool().unwrap();
        p.sneaking = v["sneak"].as_bool().unwrap();
        p.vel = [0.0; 3];
        p.fall_distance = 0.0;
        p.loot = vanilla_loot();
        p.inv = kiln_inventory::PlayerInventory::new();
        let mut trident = kiln_item::ItemStack::of("minecraft:trident", 1).unwrap();
        enchant(&mut trident, &serde_json::json!({ "minecraft:riptide": level }));
        p.inv.items[0] = trident;
        p.attack_ticker = 100;
        see_equipment(p);
    }
    let mut seq = 0;
    seq += 1;
    assert!(sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: seq, yaw, pitch })]));
    for _ in 0..11 {
        assert!(sim.step([]));
    }
    *stats[0].log.lock().unwrap() = Some(Vec::new());
    *stats[1].log.lock().unwrap() = Some(Vec::new());
    seq += 1;
    assert!(sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 6, pos: [0, 0, 0], face: 0, sequence: seq })]));
    let want = &v["result"];
    let p = &sim.players[&1];
    let mut errors = Vec::new();
    let mut eq = |what: &str, got: String, expected: String| {
        if got != expected {
            errors.push(format!("{what}: kiln {got}, vanilla {expected}"));
        }
    };
    let wp = vec_of(&want["pos"]);
    for (i, name) in ["x", "y", "z"].iter().enumerate() {
        let got = p.pos[i] - [base[0].floor() + 0.5, base[1], base[2].floor() + 0.5][i];
        // Positions are relative to bases that round differently (vanilla's is 100).
        if (got - wp[i]).abs() > 1.0e-9 {
            eq(&format!("pos.{name}"), format!("{got:?}"), format!("{:?}", wp[i]));
        }
    }
    eq("on_ground", p.on_ground.to_string(), want["on_ground"].as_bool().unwrap().to_string());
    eq("spin", (p.spin_ticks > 0).to_string(), want["spin"].as_bool().unwrap().to_string());
    eq("trident_damage", p.inv.items[0].damage().to_string(), want["trident_damage"].as_i64().unwrap().to_string());
    // `Entity.push` only flags `needsSync`: the watcher gets the motion, the player does not.
    let expected_motion = want["needs_sync"].as_bool().unwrap().then(|| kiln_proto::packets::entity::set_entity_motion(p.entity_id, vec_of(&want["delta"])));
    eq("motion (watcher)", format!("{:?}", motion_packet(&stats[1], p.entity_id)), format!("{expected_motion:?}"));
    eq("motion (self)", format!("{:?}", motion_packet(&stats[0], p.entity_id)), "None".to_owned());
    errors
}

fn run_touch(v: &Value) -> Vec<String> {
    let (mut sim, _client, _stats, base) = riptide_sim();
    let around = v["around"].as_str().unwrap();
    let at = [base[0].floor() + 0.5, base[1], base[2].floor() + 0.5];
    if matches!(around, "item" | "pig_and_item") {
        console(&mut sim, format!("summon minecraft:item {} {} {} {{Item:{{id:\"minecraft:stone\",count:1}},PickupDelay:1000s,NoGravity:1b,Motion:[0.0d,0.0d,0.0d]}}", at[0] + 0.2, at[1], at[2]));
    }
    if matches!(around, "pig" | "pig_and_item") {
        console(&mut sim, format!("summon minecraft:pig {} {} {} {{NoAI:1b}}", at[0] - 0.3, at[1], at[2] + 0.2));
    }
    if around == "minecart" {
        console(&mut sim, format!("summon minecraft:minecart {} {} {} {{NoGravity:1b}}", at[0] + 0.1, at[1], at[2]));
    }
    assert!(sim.step([]));
    {
        let p = sim.players.get_mut(&1).unwrap();
        p.pos = at;
        p.rot = [0.0, 0.0];
        p.on_ground = true;
        p.vel = [0.4, 0.1, -0.3];
        p.loot = vanilla_loot();
        p.inv = kiln_inventory::PlayerInventory::new();
        let trident = kiln_item::ItemStack::of("minecraft:trident", 1).unwrap();
        p.inv.items[0] = trident.clone();
        p.attack_ticker = 100;
        see_equipment(p);
        p.horizontal_collision = v["collision"].as_bool().unwrap();
        p.start_spin_attack(v["ticks"].as_i64().unwrap() as i32, 8.0, trident, false);
        p.level_rng = kiln_javamath::random::LegacyRandom::new(77);
        p.entity_rng = kiln_javamath::random::LegacyRandom::new(78);
    }
    assert!(sim.step([]));
    let want = &v["result"];
    let p = &sim.players[&1];
    let mut errors = Vec::new();
    let expected = want["spin_ticks"].as_i64().unwrap().max(0) as i32;
    if p.spin_ticks != expected {
        errors.push(format!("spin_ticks: kiln {}, vanilla {expected}", p.spin_ticks));
    }
    let health: Vec<f32> = sim.mobs().into_iter().filter(|m| m.1 == "minecraft:pig").map(|m| m.3).collect();
    let want_health: Vec<f32> = want["health"].as_array().unwrap().iter().map(f32_of).collect();
    if format!("{health:?}") != format!("{want_health:?}") {
        errors.push(format!("health: kiln {health:?}, vanilla {want_health:?}"));
    }
    errors
}

#[test]
fn riptide_parity() {
    let Some(path) = std::env::var_os("KILN_RIPTIDE_VECTORS") else {
        eprintln!("skipped: set KILN_RIPTIDE_VECTORS (tools/combat_vectors.py)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        assert!(v["kind"].as_str() != Some("error"), "vanilla failed: {}", v["error"]);
        let name = v["name"].as_str().unwrap().to_owned();
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        let errors = match v["kind"].as_str().unwrap() {
            "lift" => run_lift(&v),
            "touch" => run_touch(&v),
            other => panic!("unknown riptide vector {other}"),
        };
        if errors.is_empty() {
            passed += 1;
        } else {
            println!("FAIL {name}\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    println!("riptide parity: {passed} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "failed: {failed:?}");
}
