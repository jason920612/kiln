//! Replays the spear vectors of `tools/CombatVectors.java` (`spear.jsonl` beside the combat
//! vectors, `KILN_SPEAR_VECTORS`; `tools/combat_vectors.py --filter spear`) through the
//! simulation: a stab is a Player Action STAB packet from an attacker holding the recorded
//! spear; a charge is the use of the spear for a number of ticks with the attacker's position and
//! known movement set each tick. Health, absorption, durability, exhaustion, the knockback and
//! lunge motion packets, the sounds heard, and what happened to the mobs must match vanilla's.
//! Entities do not tick (`/tick freeze`), so a mob is looked at as the attack left it. Skipped
//! when the vectors are not there.

use crate::combat_parity::{f32_of, motion_packet, packets_with_id, setup, vanilla_loot, vec_of};
use crate::testing::{Client, SinkStats, join};
use crate::{Sim, SimConfig};
use kiln_javamath::random::LegacyRandom;
use kiln_link::{PlayIn, ToSim};
use serde_json::Value;
use std::sync::Arc;

/// The sounds a player was sent: (name, volume bits, pitch bits), in order.
fn sounds(stats: &SinkStats) -> Vec<(String, u32, u32)> {
    packets_with_id(stats, kiln_data::packets::play::clientbound::SOUND)
        .iter()
        .map(|p| {
            let mut r = kiln_proto::codec::Reader::new(p);
            r.varint().unwrap();
            let holder = r.varint().unwrap();
            let name = kiln_item::registry::SOUND_EVENT.name(holder - 1).unwrap_or("?").to_owned();
            r.varint().unwrap();
            for _ in 0..3 {
                r.i32().unwrap();
            }
            let (volume, pitch) = (r.f32().unwrap(), r.f32().unwrap());
            (name, volume.to_bits(), pitch.to_bits())
        })
        .filter(heard_in_both)
        .map(normalized)
        .collect()
}

/// A player's own hurt and death sounds (other players hear them from the server in vanilla;
/// Kiln leaves them to the damage event) are not part of what a spear does; a mob's pitch is
/// drawn from its own random, which a fresh mob of the vectors does not share.
fn heard_in_both(s: &(String, u32, u32)) -> bool {
    !s.0.starts_with("minecraft:entity.player.")
}

/// The pitch only counts for the item's own sounds.
fn normalized(mut s: (String, u32, u32)) -> (String, u32, u32) {
    if !s.0.starts_with("minecraft:item.") {
        s.2 = 0;
    }
    s
}

fn want_sounds(v: &Value) -> Vec<(String, u32, u32)> {
    v.as_array()
        .map(|l| l.iter().map(|s| (s["sound"].as_str().unwrap().to_owned(), f32_of(&s["volume"]).to_bits(), f32_of(&s["pitch"]).to_bits())).filter(heard_in_both).map(normalized).collect())
        .unwrap_or_default()
}

/// What a tick of the food data does to the exhaustion vanilla read right after the attack.
fn after_tick(exhaustion: f32) -> f32 {
    if exhaustion > 4.0 { exhaustion - 4.0 } else { exhaustion }
}

struct World {
    sim: Sim,
    stats: Vec<Arc<SinkStats>>,
    base: [f64; 3],
}

impl World {
    fn new(names: &[String]) -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (mut clients, mut stats) = (Vec::new(), Vec::new());
        for (i, name) in names.iter().enumerate() {
            let conn = i as u64 + 1;
            let (msg, s) = join(conn, name, 2);
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
        let base = sim.players[&1].pos;
        let mut w = World { sim, stats, base };
        for c in ["difficulty normal", "gamerule minecraft:spawn_mobs false", "gamerule minecraft:pvp true", "tick freeze"] {
            w.console(c);
        }
        // Vanilla's level is a void: nothing but what a scenario places is in the way.
        let (lo, hi) = (w.at(-4, -3, -4), w.at(4, 6, 14));
        w.console(&format!("fill {} {} {} {} {} {} minecraft:air", lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]));
        w
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    /// The block coordinates of the attacker's own block and an offset of it (`SpearVectors.at`).
    fn at(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        [self.base[0].floor() as i32 + x, self.base[1].round() as i32 + y, self.base[2].floor() as i32 + z]
    }

    fn setblock(&mut self, x: i32, y: i32, z: i32, block: &str) {
        let p = self.at(x, y, z);
        self.console(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    /// `/summon` of a frozen mob at an offset of the attacker's position.
    fn summon(&mut self, spec: &[Value]) {
        let ty = spec[0].as_str().unwrap();
        let off = |i: usize| spec[i].as_str().unwrap().parse::<f64>().unwrap();
        let (x, y, z) = (self.base[0] + off(1), self.base[1] + off(2), self.base[2] + off(3));
        let mut equipment = Vec::new();
        if let Some(list) = spec.get(4).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            for piece in list.split(',') {
                let (slot, item) = piece.split_once('=').unwrap();
                let slot = match slot {
                    "HEAD" => "head",
                    "CHEST" => "chest",
                    "LEGS" => "legs",
                    "FEET" => "feet",
                    other => panic!("slot {other}"),
                };
                equipment.push(format!("{slot}:{{id:\"{item}\",count:1}}"));
            }
        }
        let nbt = format!("{{NoAI:1b,PersistenceRequired:1b,Rotation:[180f,0f],equipment:{{{}}}}}", equipment.join(","));
        self.console(&format!("summon {ty} {x} {y} {z} {nbt}"));
        assert!(self.sim.step([]));
    }

    /// The saved data of the entities of a type, in id order.
    fn entities_of(&self, ty: &str) -> Vec<kiln_proto::nbt::Tag> {
        self.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(|i| i.as_str()) == Some(ty)).collect()
    }

    fn start_logs(&self) {
        for s in &self.stats {
            *s.log.lock().unwrap() = Some(Vec::new());
        }
    }

    fn player(&mut self, conn: u64) -> &mut crate::Player {
        self.sim.players.get_mut(&conn).unwrap()
    }
}

/// The attacker and target players are named like vanilla's mock players (names only matter for
/// the messages).
fn names(line: &Value, target: bool) -> Vec<String> {
    let n = line["name"].as_str().unwrap().len();
    let mut v = vec![format!("Stab{n}")];
    if target {
        v.push(format!("Mark{n}"));
    }
    v
}

fn eq(errors: &mut Vec<String>, what: &str, got: String, want: String) {
    if got != want {
        errors.push(format!("{what}: kiln {got}, vanilla {want}"));
    }
}

fn stab(line: &Value) -> Vec<String> {
    let has_target = !line["target"].is_null();
    let mut w = World::new(&names(line, has_target));
    let (attacker, target) = (&line["attacker"], &line["target"]);
    let base = w.base;
    setup(&mut w.sim, 1, attacker, base);
    if has_target {
        setup(&mut w.sim, 2, target, base);
    }
    let seed = line["level_seed"].as_i64().unwrap();
    {
        let p = w.player(1);
        p.rot[1] = f32_of(&line["attacker_pitch"]);
        p.food = line["food"].as_i64().unwrap() as i32;
        p.entity_rng = LegacyRandom::new(seed.wrapping_add(1));
        p.level_rng = LegacyRandom::new(seed);
    }
    if has_target {
        w.player(2).entity_rng = LegacyRandom::new(seed.wrapping_add(2));
        // The target in vanilla stands where `setup` put it; a player on the ground that is not
        // told otherwise keeps its position.
    }
    for mob in line["mobs"].as_array().unwrap() {
        w.summon(mob.as_array().unwrap());
    }
    match line["twist"].as_str().unwrap() {
        "wall" | "wall_close" => w.setblock(0, 1, 1, "minecraft:stone"),
        "mounted" => {
            w.summon(&[Value::from("minecraft:pig"), Value::from("0.0"), Value::from("0.0"), Value::from("0.0")]);
        }
        "water" => {
            w.setblock(0, 0, 0, "minecraft:water");
            w.setblock(0, 1, 0, "minecraft:water");
        }
        "glide" => w.player(1).fall_flying = true,
        "shield" => {
            let shield = kiln_item::ItemStack::of("minecraft:shield", 1).unwrap();
            let p = w.player(2);
            p.set_in_hand(true, shield.clone());
            p.start_using(true, &shield, 72000);
        }
        _ => {}
    }
    // The setup of the players is repeated after the blocks and mobs changed (the ticks that
    // made them must not have moved anything).
    setup(&mut w.sim, 1, attacker, base);
    let mount = (line["twist"].as_str() == Some("mounted")).then(|| w.sim.entity_ids_of("minecraft:pig").last().copied()).flatten();
    {
        let p = w.player(1);
        p.rot[1] = f32_of(&line["attacker_pitch"]);
        p.food = line["food"].as_i64().unwrap() as i32;
        p.entity_rng = LegacyRandom::new(seed.wrapping_add(1));
        p.level_rng = LegacyRandom::new(seed);
        if line["twist"].as_str() == Some("glide") {
            p.fall_flying = true;
        }
        p.vehicle = mount;
    }
    if let Some(pig) = mount {
        let pid = w.sim.players[&1].entity_id;
        for dim in w.sim.dims.iter_mut() {
            for r in dim.regions.iter_mut() {
                if let Some(phys) = r.part_mut().0.list.iter_mut().find(|e| e.id == pig).and_then(|e| e.phys.as_deref_mut()) {
                    kiln_entity::ride::add_passenger(phys, pid, true, false);
                }
            }
        }
    }
    if has_target {
        setup(&mut w.sim, 2, target, base);
        w.player(2).entity_rng = LegacyRandom::new(seed.wrapping_add(2));
        if line["twist"].as_str() == Some("shield") {
            let shield = kiln_item::ItemStack::of("minecraft:shield", 1).unwrap();
            let p = w.player(2);
            p.set_in_hand(true, shield.clone());
            p.start_using(true, &shield, 72000);
        }
    }
    w.start_logs();
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 8, pos: [0, 0, 0], face: 0, sequence: 0 })]));
    let want = &line["result"];
    let mut errors = Vec::new();
    check_player(&mut w, 1, &want["attacker"], &mut errors, "attacker");
    if has_target {
        check_player(&mut w, 2, &want["target"], &mut errors, "target");
    }
    let mobs: Vec<&Value> = want["mobs"].as_array().unwrap().iter().filter(|m| m["type"].as_str() != Some("minecraft:pig") || line["twist"].as_str() != Some("mounted")).collect();
    for (i, m) in mobs.iter().enumerate() {
        let ty = m["type"].as_str().unwrap();
        // The i-th mob of that type among those the vectors list.
        let index = mobs[..i].iter().filter(|o| o["type"].as_str() == Some(ty)).count();
        let got = w.entities_of(ty);
        check_mob(got.get(index), m, &mut errors, &format!("mob {i} ({ty})"));
    }
    errors
}

fn check_player(w: &mut World, conn: u64, want: &Value, errors: &mut Vec<String>, who: &str) {
    let stats = w.stats[conn as usize - 1].clone();
    let p = &w.sim.players[&conn];
    eq(errors, &format!("{who}.health"), format!("{:?}", p.health), format!("{:?}", f32_of(&want["health"])));
    eq(errors, &format!("{who}.absorption"), format!("{:?}", p.absorption), format!("{:?}", f32_of(&want["absorption"])));
    eq(errors, &format!("{who}.exhaustion"), format!("{:?}", p.exhaustion), format!("{:?}", after_tick(f32_of(&want["exhaustion"]))));
    eq(errors, &format!("{who}.ticker"), format!("{}", p.attack_ticker), format!("{}", want["ticker"].as_i64().unwrap() + 1));
    let main = p.inv.selected_item();
    eq(errors, &format!("{who}.main_hand_damage"), format!("{}", main.damage()), format!("{}", want["main_hand_damage"].as_i64().unwrap()));
    let armor: Vec<Option<i64>> = (0..4).map(|i| (!p.inv.equipment[i].is_empty()).then(|| p.inv.equipment[i].damage() as i64)).collect();
    let want_armor: Vec<Option<i64>> = want["armor_damage"].as_array().unwrap().iter().map(|v| v.as_i64()).collect();
    eq(errors, &format!("{who}.armor_damage"), format!("{armor:?}"), format!("{want_armor:?}"));
    let key = if want["pending_motion"].is_null() { "motion" } else { "pending_motion" };
    // (A hurt player is sent its motion again at the end of the tick, which vanilla's vectors,
    // read right after the attack, do not have: only the recorded packets are compared.)
    if let Some(_) = want[key].as_array() {
        let expected = kiln_proto::packets::entity::set_entity_motion(p.entity_id, vec_of(&want[key]));
        eq(errors, &format!("{who}.motion"), format!("{:?}", motion_packet(&stats, p.entity_id)), format!("{:?}", Some(expected)));
    }
    eq(errors, &format!("{who}.sounds"), format!("{:?}", sounds(&stats)), format!("{:?}", want_sounds(&want["sounds"])));
}

fn check_mob(got: Option<&kiln_proto::nbt::Tag>, want: &Value, errors: &mut Vec<String>, who: &str) {
    let health = got.and_then(|t| t.get("Health")).and_then(|h| h.as_f64()).map_or(0.0, |h| h as f32);
    eq(errors, &format!("{who}.health"), format!("{health:?}"), format!("{:?}", f32_of(&want["health"])));
    if want["alive"].as_bool().unwrap() {
        let motion = got.and_then(|t| t.get("Motion")).and_then(|m| m.as_list()).map(|l| [0, 1, 2].map(|i| l[i].as_f64().unwrap()));
        eq(errors, &format!("{who}.velocity"), format!("{motion:?}"), format!("{:?}", Some(vec_of(&want["velocity"]))));
        let fire = got.and_then(|t| t.get("Fire")).and_then(|f| f.as_i64()).unwrap_or(0);
        let want_fire = want["fire_ticks"].as_i64().unwrap().max(0);
        eq(errors, &format!("{who}.fire"), format!("{}", fire > 0), format!("{}", want_fire > 0));
    }
}

fn charge(line: &Value) -> Vec<String> {
    let kind = line["target_kind"].as_str().unwrap();
    let mut w = World::new(&names(line, kind == "player"));
    let base = w.base;
    setup(&mut w.sim, 1, &line["attacker"], base);
    let seed = line["level_seed"].as_i64().unwrap();
    let z0 = 7.0;
    if kind == "player" {
        let mut side = line["attacker"].clone();
        side["main_hand"] = Value::Null;
        side["pos"] = serde_json::json!([0.0, 0.0, z0]);
        side["yaw"] = serde_json::json!(180.0);
        side["armor"] = serde_json::json!([null, null, null, null]);
        side["main_hand_enchantments"] = serde_json::json!({});
        side["main_hand_damage"] = serde_json::json!(0);
        setup(&mut w.sim, 2, &side, base);
    }
    let mount_ty = "minecraft:pig";
    match kind {
        "pig" => w.summon(&[Value::from("minecraft:pig"), Value::from("0.0"), Value::from("0.0"), Value::from(format!("{z0}"))]),
        "mounted_zombie" => {
            w.summon(&[Value::from(mount_ty), Value::from("0.0"), Value::from("0.0"), Value::from(format!("{z0}"))]);
            w.summon(&[Value::from("minecraft:zombie"), Value::from("0.0"), Value::from("0.0"), Value::from(format!("{z0}"))]);
            let (pig, zombie) = (w.sim.entity_ids_of(mount_ty)[0], w.sim.entity_ids_of("minecraft:zombie")[0]);
            for dim in w.sim.dims.iter_mut() {
                for r in dim.regions.iter_mut() {
                    let list = &mut r.part_mut().0.list;
                    if let (Ok(z), Ok(p)) = (list.binary_search_by_key(&zombie, |e| e.id), list.binary_search_by_key(&pig, |e| e.id)) {
                        let mut rider = list[z].phys.take().unwrap();
                        if let Some(vp) = list[p].phys.as_deref_mut() {
                            kiln_entity::ride::start_riding(&mut rider, vp, false);
                        }
                        list[z].phys = Some(rider);
                    }
                }
            }
        }
        _ => {}
    }
    {
        let p = w.player(1);
        p.entity_rng = LegacyRandom::new(seed.wrapping_add(1));
        p.level_rng = LegacyRandom::new(seed);
    }
    let (yaw, pitch) = (f32_of(&line["attacker"]["yaw"]), 0.0f32);
    let speed = line["speed"].as_f64().unwrap();
    let step = line["target_step"].as_f64().unwrap();
    let start = line["start_time"].as_i64().unwrap();
    let mut errors = Vec::new();
    let states = line["states"].as_array().unwrap();
    // Vanilla's target never ticks, so its hurt cooldown stays where a hit left it.
    let mut hurt_before = false;
    for (t, state) in states.iter().enumerate() {
        if kind == "player" && hurt_before {
            w.player(2).hurt_cooldown = 21;
        }
        let a = w.player(1);
        a.pos[2] += speed;
        a.known_movement = [0.0, 0.0, speed];
        if kind == "player" {
            let tp = w.player(2);
            tp.pos[2] += step;
            tp.known_movement = [0.0, 0.0, step];
        }
        move_mobs(&mut w, kind, step);
        for s in &w.stats {
            *s.log.lock().unwrap() = Some(Vec::new());
        }
        // (A frozen game does not count its time: the use of the weapon is counted by it.)
        w.sim.game_time = start + t as i64;
        // The first tick starts the use (`Item.use`), as the vectors do.
        let inbox = if t == 0 {
            vec![ToSim::Packet(1, PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: 1, yaw, pitch })]
        } else {
            Vec::new()
        };
        assert!(w.sim.step(inbox));
        let who = format!("tick {t}");
        if kind == "player" {
            hurt_before |= w.sim.players[&2].health < 20.0;
        }
        let a = &w.sim.players[&1];
        eq(&mut errors, &format!("{who} a_damage"), format!("{}", a.inv.selected_item().damage()), format!("{}", state["a_damage"].as_i64().unwrap()));
        if kind == "player" {
            let tp = &w.sim.players[&2];
            eq(&mut errors, &format!("{who} t_health"), format!("{:?}", tp.health), format!("{:?}", f32_of(&state["t_health"])));
            if state["t_motion"].as_array().is_some() {
                let expected = kiln_proto::packets::entity::set_entity_motion(tp.entity_id, vec_of(&state["t_motion"]));
                eq(&mut errors, &format!("{who} t_motion"), format!("{:?}", motion_packet(&w.stats[1], tp.entity_id)), format!("{:?}", Some(expected)));
            }
            eq(&mut errors, &format!("{who} t_sounds"), format!("{:?}", sounds(&w.stats[1])), format!("{:?}", want_sounds(&state["t_sounds"])));
        } else {
            let ty = if kind == "pig" { "minecraft:pig" } else { "minecraft:zombie" };
            let got = w.entities_of(ty);
            let health = got.first().and_then(|t| t.get("Health")).and_then(|h| h.as_f64()).map_or(0.0, |h| h as f32);
            eq(&mut errors, &format!("{who} m_health"), format!("{health:?}"), format!("{:?}", f32_of(&state["m_health"])));
            let motion = got.first().and_then(|t| t.get("Motion")).and_then(|m| m.as_list()).map(|l| [0, 1, 2].map(|i| l[i].as_f64().unwrap()));
            if state["m_health"].as_f64().unwrap() > 0.0 {
                eq(&mut errors, &format!("{who} m_delta"), format!("{motion:?}"), format!("{:?}", Some(vec_of(&state["m_delta"]))));
            }
            if kind == "mounted_zombie" {
                let riding = got.first().and_then(|t| t.get("id")).is_some() && w.sim.riding().iter().any(|(id, v, _)| Some(*id) == w.sim.entity_ids_of(ty).first().copied() && v.is_some());
                eq(&mut errors, &format!("{who} m_vehicle"), format!("{riding}"), format!("{}", state["m_vehicle"].as_bool().unwrap()));
            }
        }
        if !errors.is_empty() {
            break;
        }
    }
    errors
}

/// Moves a target mob (and what it rides) along z by `step` and tells it how fast.
fn move_mobs(w: &mut World, kind: &str, step: f64) {
    if kind == "player" || kind == "none" {
        return;
    }
    for dim in w.sim.dims.iter_mut() {
        for r in dim.regions.iter_mut() {
            for e in r.part_mut().0.list.iter_mut() {
                let Some(phys) = e.phys.as_deref_mut() else { continue };
                let is_mount = phys.type_name == "minecraft:pig";
                let is_rider = phys.type_name == "minecraft:zombie";
                if !(is_mount || is_rider) {
                    continue;
                }
                if is_mount || kind == "pig" {
                    let p = phys.position();
                    phys.set_pos(kiln_entity::math::Vec3::new(p.x, p.y, p.z + step));
                    phys.last_known_speed = kiln_entity::math::Vec3::new(0.0, 0.0, step);
                    e.sync();
                } else if is_rider {
                    // The rider sits on its mount.
                    let p = phys.position();
                    phys.set_pos(kiln_entity::math::Vec3::new(p.x, p.y, p.z + step));
                    e.sync();
                }
            }
        }
    }
}

#[test]
fn spear_parity() {
    let Some(path) = std::env::var_os("KILN_SPEAR_VECTORS") else {
        eprintln!("skipped: set KILN_SPEAR_VECTORS (tools/combat_vectors.py --filter spear)");
        return;
    };
    let _ = vanilla_loot();
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap_or("?").to_owned();
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        let errors = match v["kind"].as_str().unwrap() {
            "stab" => stab(&v),
            "charge" => charge(&v),
            other => panic!("kind {other}"),
        };
        if errors.is_empty() {
            passed += 1;
            println!("ok   {name}");
        } else {
            println!("FAIL {name}\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    println!("spear parity: {passed} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "failed: {failed:?}");
}
