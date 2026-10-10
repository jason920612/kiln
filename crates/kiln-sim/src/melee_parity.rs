//! Replays the melee vectors of `tools/CombatVectors.java` (`melee.jsonl` beside the combat vectors,
//! `KILN_MELEE_VECTORS`; `tools/combat_vectors.py --filter melee`) through the simulation: a player
//! attacks a mob or a player with every wrinkle (sweeping against both kinds, critical hits,
//! effects, enchanted weapons, armored and enchanted victims, riding, water) and the mace (smash
//! attack, density, breach, wind burst, the knockback blast). Each scenario sets the attacker, the
//! victims (players, or `/summon`ed frozen mobs) and the blocks as recorded, sends Attack packets
//! (an attack strength ticker set before each), and compares after each hit: the attacker and the
//! players (health, absorption, exhaustion, fire, durability, motion packets), the mobs (health,
//! motion, fire, hurt time, effects, equipment wear) and the world packets the attacker got
//! (sounds, particles, level events, animations, explosions, motions). Skipped when the vectors
//! are not there.

use crate::combat_parity::{f32_of, setup, vec_of};
use crate::testing::{Client, SinkStats, join};
use crate::{Sim, SimConfig};
use kiln_javamath::random::LegacyRandom;
use kiln_link::{PlayIn, ToSim};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

struct World {
    sim: Sim,
    stats: Vec<Arc<SinkStats>>,
    base: [f64; 3],
}

impl World {
    fn new(names: &[String]) -> World {
        let t0 = std::time::Instant::now();
        let timing = std::env::var_os("KILN_PARITY_TIMING").is_some();
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        if timing {
            eprintln!("Sim::new {:?}", t0.elapsed());
        }
        let (mut clients, mut stats) = (Vec::new(), Vec::new());
        for (i, name) in names.iter().enumerate() {
            let conn = i as u64 + 1;
            let (msg, s) = join(conn, name, 2);
            // (No natural spawns from the first tick: a slime chunk of the flat world would put one in
            // reach of the mobs the scenario counts.)
            assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
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
        if timing {
            eprintln!("joined {:?}", t0.elapsed());
        }
        let base = [0.5, 100.0, 0.5];
        let mut w = World { sim, stats, base };
        for c in ["gamerule minecraft:spawn_mobs false", "tick freeze"] {
            w.console(c);
        }
        if timing {
            eprintln!("rules {:?}", t0.elapsed());
        }
        // Vanilla's level is a void: nothing but what a scenario places is in the way.
        let (lo, hi) = (w.at(-4, -3, -4), w.at(4, 6, 14));
        w.console(&format!("fill {} {} {} {} {} {} minecraft:air", lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]));
        w
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    /// Block coordinates of an offset of the attacker's own block (`MeleeVectors.blockCommand`: y is
    /// relative to the attacker's feet).
    fn at(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        [self.base[0].floor() as i32 + x, self.base[1].round() as i32 + y, self.base[2].floor() as i32 + z]
    }

    fn player(&mut self, conn: u64) -> &mut crate::Player {
        self.sim.players.get_mut(&conn).unwrap()
    }
}

/// `Victim.command` of the Java side.
fn summon_command(v: &Value, base: [f64; 3]) -> String {
    let ty = v["type"].as_str().unwrap();
    let pos = vec_of(&v["pos"]);
    let (x, y, z) = (base[0] + pos[0], base[1] + pos[1], base[2] + pos[2]);
    let yaw = v["yaw"].as_f64().unwrap() as f32;
    let nbt = v["nbt"].as_str().unwrap();
    let inner = if nbt.is_empty() { "NoAI:1b,PersistenceRequired:1b".to_owned() } else { format!("NoAI:1b,PersistenceRequired:1b,{nbt}") };
    match v["vehicle"].as_str() {
        None => format!("summon {ty} {x:?} {y:?} {z:?} {{{inner},Rotation:[{yaw}f,0f]}}"),
        Some(vehicle) => format!("summon {vehicle} {x:?} {y:?} {z:?} {{Rotation:[{yaw}f,0f],Passengers:[{{id:\"{ty}\",{inner}}}]}}"),
    }
}

fn effect(sim_player: &mut crate::Player, e: &Value) {
    let name = e[0].as_str().unwrap();
    let id = crate::effects::effect_id(name).unwrap_or_else(|| panic!("effect {name}"));
    sim_player.add_effect(crate::effects::Effect::simple(id, e[2].as_i64().unwrap() as i32, e[1].as_i64().unwrap() as i32));
}

fn eq(errors: &mut Vec<String>, what: &str, got: String, want: String) {
    if got != want {
        errors.push(format!("{what}: kiln {got}, vanilla {want}"));
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1.0e-6 * (1.0 + a.abs().max(b.abs()))
}

/// The world packets of a player's log, decoded: kind -> canonical strings.
struct Decoded {
    sounds: Vec<(String, f32, f32, [f64; 3])>,
    particles: Vec<(String, i32, [f64; 3], [f32; 3], [f32; 3])>,
    events: Vec<(i32, [i32; 3], i32)>,
    animates: Vec<(i32, u8)>,
    motions: Vec<(i32, bytes::Bytes)>,
    explodes: Vec<Explode>,
}

struct Explode {
    center: [f64; 3],
    radius: f32,
    block_count: i32,
    knockback: Option<[f64; 3]>,
    particle: String,
    sound: String,
}

fn decode(stats: &SinkStats) -> Decoded {
    use kiln_data::packets::play::clientbound as c;
    let mut d = Decoded { sounds: vec![], particles: vec![], events: vec![], animates: vec![], motions: vec![], explodes: vec![] };
    let log: Vec<bytes::Bytes> = stats.log.lock().unwrap().clone().unwrap_or_default();
    let particle_names = kiln_data::builtin_entries("minecraft:particle_type").unwrap();
    for p in &log {
        let mut r = kiln_proto::codec::Reader::new(p);
        let Ok(id) = r.varint() else { continue };
        if id == c::SOUND {
            let holder = r.varint().unwrap();
            let name = kiln_item::registry::SOUND_EVENT.name(holder - 1).unwrap_or("?").to_owned();
            let _source = r.varint().unwrap();
            let pos = [r.i32().unwrap() as f64 / 8.0, r.i32().unwrap() as f64 / 8.0, r.i32().unwrap() as f64 / 8.0];
            let (volume, pitch) = (r.f32().unwrap(), r.f32().unwrap());
            d.sounds.push((name, volume, pitch, pos));
        } else if id == c::LEVEL_PARTICLES {
            let kind = r.varint().unwrap();
            let name = particle_names.get(kind as usize).copied().unwrap_or("?").to_owned();
            // (Only options-free particles are looked at, and item particles (their template: item, count, empty patch); the rest decode wrongly and mismatch.)
            if name == "minecraft:item" {
                for _ in 0..4 {
                    r.varint().unwrap();
                }
            }
            let (_override, _always) = (r.bool().unwrap(), r.bool().unwrap());
            let pos = [r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()];
            let offset = [r.f32().unwrap(), r.f32().unwrap(), r.f32().unwrap()];
            let speed = [r.f32().unwrap(), r.f32().unwrap(), r.f32().unwrap()];
            let count = r.varint().unwrap();
            d.particles.push((name, count, pos, offset, speed));
        } else if id == c::LEVEL_EVENT {
            let ty = r.i32().unwrap();
            let v = r.i64().unwrap();
            let (x, y, z) = ((v >> 38) as i32, (v << 52 >> 52) as i32, (v << 26 >> 38) as i32);
            let data = r.i32().unwrap();
            d.events.push((ty, [x, y, z], data));
        } else if id == c::ANIMATE {
            let who = r.varint().unwrap();
            let action = r.u8().unwrap();
            d.animates.push((who, action));
        } else if id == c::SET_ENTITY_MOTION {
            let who = r.varint().unwrap();
            d.motions.push((who, p.clone()));
        } else if id == c::EXPLODE {
            if let Some(e) = decode_explode(&mut r, &particle_names) {
                d.explodes.push(e);
            }
        }
    }
    d
}

fn decode_explode(r: &mut kiln_proto::codec::Reader, particles: &[&'static str]) -> Option<Explode> {
    let center = [r.f64().ok()?, r.f64().ok()?, r.f64().ok()?];
    let radius = r.f32().ok()?;
    let block_count = r.i32().ok()?;
    let knockback = if r.bool().ok()? { Some([r.f64().ok()?, r.f64().ok()?, r.f64().ok()?]) } else { None };
    let kind = r.varint().ok()?;
    let particle = particles.get(kind as usize).copied().unwrap_or("?").to_owned();
    let sound = String::new();
    Some(Explode { center, radius, block_count, knockback, particle, sound })
}

struct Ctx<'a> {
    base: [f64; 3],
    ids: &'a HashMap<String, i32>,
    /// Positions are not compared (something rides: Kiln seats the rider where vanilla has not yet).
    skip_pos: bool,
}

impl Ctx<'_> {
    fn who(&self, id: i32) -> String {
        self.ids.iter().find(|(_, v)| **v == id).map_or("other".to_owned(), |(k, _)| k.clone())
    }

    fn block_base(&self) -> [i32; 3] {
        [self.base[0].floor() as i32, self.base[1].round() as i32, self.base[2].floor() as i32]
    }
}

/// Compares what the attacker's client got of the world with the recording.
fn check_world(stats: &SinkStats, want: &Value, cx: &Ctx, errors: &mut Vec<String>, who: &str) {
    let got = decode(stats);
    // Sounds: as a multiset (the order across phases is not a sound's business); the pitch of a
    // mob's voice is its own random's.
    let want_sounds: Vec<(String, f32, f32, [f64; 3])> = want["sounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["sound"].as_str().unwrap().to_owned(), f32_of(&s["volume"]), f32_of(&s["pitch"]), vec_of(&s["pos"])))
        .collect();
    let pitch_counts = |n: &str| n.starts_with("minecraft:item.") || n.starts_with("minecraft:entity.player.attack");
    let skip = cx.skip_pos;
    let fmt = |l: &[(String, f32, f32, [f64; 3])], base: [f64; 3], rel: bool| {
        let mut v: Vec<String> = l
            .iter()
            .map(|(n, vol, pitch, pos)| {
                let pos = if skip { [0.0; 3] } else if rel { [pos[0] - base[0], pos[1] - base[1], pos[2] - base[2]] } else { *pos };
                let pitch = if pitch_counts(n) { pitch.to_bits() } else { 0 };
                format!("{n} vol {vol} pitch {pitch} at [{:.1}, {:.1}, {:.1}]", pos[0], pos[1], pos[2])
            })
            .collect();
        v.sort();
        v
    };
    let (g, w) = (fmt(&got.sounds, cx.base, true), fmt(&want_sounds, cx.base, false));
    if g != w {
        errors.push(format!("{who}.sounds: kiln {g:?}\n    vanilla {w:?}"));
    }
    // Particles.
    let want_particles: Vec<&Value> = want["particles"].as_array().unwrap().iter().collect();
    if got.particles.len() != want_particles.len() {
        errors.push(format!(
            "{who}.particles: kiln {:?}, vanilla {:?}",
            got.particles.iter().map(|p| (&p.0, p.1)).collect::<Vec<_>>(),
            want_particles.iter().map(|p| (p["particle"].as_str().unwrap(), p["count"].as_i64().unwrap())).collect::<Vec<_>>()
        ));
    } else {
        for (g, w) in got.particles.iter().zip(&want_particles) {
            let wp = vec_of(&w["pos"]);
            let rel = [g.2[0] - cx.base[0], g.2[1] - cx.base[1], g.2[2] - cx.base[2]];
            let same = g.0 == w["particle"].as_str().unwrap()
                && g.1 == w["count"].as_i64().unwrap() as i32
                && (cx.skip_pos || (0..3).all(|i| close(rel[i], wp[i])))
                && (0..3).all(|i| g.3[i].to_bits() == (w["offset"][i].as_f64().unwrap() as f32).to_bits() && g.4[i].to_bits() == (w["speed"][i].as_f64().unwrap() as f32).to_bits());
            if !same {
                errors.push(format!("{who}.particle: kiln {g:?} (rel {rel:?}), vanilla {w}"));
            }
        }
    }
    // Level events.
    let bb = cx.block_base();
    let g: Vec<String> = got.events.iter().map(|e| format!("{} at {:?} data {}", e.0, [e.1[0] - bb[0], e.1[1] - bb[1], e.1[2] - bb[2]], e.2)).collect();
    let w: Vec<String> = want["level_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{} at {:?} data {}", e["type"], [e["pos"][0].as_i64().unwrap() as i32, e["pos"][1].as_i64().unwrap() as i32, e["pos"][2].as_i64().unwrap() as i32], e["data"]))
        .collect();
    eq(errors, &format!("{who}.level_events"), format!("{g:?}"), format!("{w:?}"));
    // Animations (critical hits).
    let g: Vec<String> = got.animates.iter().map(|(id, a)| format!("{}:{a}", cx.who(*id))).collect();
    let w: Vec<String> = want["animates"].as_array().unwrap().iter().map(|a| format!("{}:{}", a["who"].as_str().unwrap(), a["action"])).collect();
    eq(errors, &format!("{who}.animates"), format!("{g:?}"), format!("{w:?}"));
    // Explosions.
    let w = want["explodes"].as_array().unwrap();
    if got.explodes.len() != w.len() {
        errors.push(format!("{who}.explodes: kiln {} vanilla {}", got.explodes.len(), w.len()));
    } else {
        for (g, w) in got.explodes.iter().zip(w) {
            let wc = vec_of(&w["center"]);
            let rel = [g.center[0] - cx.base[0], g.center[1] - cx.base[1], g.center[2] - cx.base[2]];
            let wk = w["knockback"].as_array().map(|_| vec_of(&w["knockback"]));
            let same_k = match (g.knockback, wk) {
                (None, None) => true,
                (Some(a), Some(b)) => (0..3).all(|i| a[i].to_bits() == b[i].to_bits()),
                _ => false,
            };
            if !((0..3).all(|i| close(rel[i], wc[i])) && g.radius.to_bits() == f32_of(&w["radius"]).to_bits() && g.block_count == w["block_count"].as_i64().unwrap() as i32 && same_k && g.particle == w["particle"].as_str().unwrap()) {
                errors.push(format!("{who}.explode: kiln center {rel:?} r {} blocks {} knockback {:?} {}; vanilla {w}", g.radius, g.block_count, g.knockback, g.particle));
            }
        }
    }
    let _ = &got.sounds;
    let _ = got.explodes.iter().map(|e| &e.sound).count();
}

/// The motion packets a player received for itself, in order (decoded) against the vanilla list of
/// immediate packets then its pending motion.
fn check_motions(stats: &SinkStats, entity_id: i32, want: &Value, errors: &mut Vec<String>, who: &str, self_name: &str) {
    let got: Vec<bytes::Bytes> = decode(stats).motions.into_iter().filter(|m| m.0 == entity_id).map(|m| m.1).collect();
    let mut expected: Vec<[f64; 3]> = want["packets"]["motions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["who"].as_str() == Some(self_name))
        .map(|m| vec_of(&m["motion"]))
        .collect();
    if let Some(p) = want.get("pending_motion").filter(|p| p.is_array()) {
        expected.push(vec_of(p));
    }
    let expected: Vec<bytes::Bytes> = expected.iter().map(|v| kiln_proto::packets::entity::set_entity_motion(entity_id, *v)).collect();
    // The end of the tick may repeat the last motion (what a hurt player is sent again).
    let mut got_trim = got.clone();
    if got_trim.len() == expected.len() + 1 && got_trim.last() == got_trim.get(got_trim.len().wrapping_sub(2)) {
        got_trim.pop();
    }
    eq(errors, &format!("{who}.motions"), format!("{got_trim:?}"), format!("{expected:?}"));
}

fn check_player(w: &World, conn: u64, want: &Value, errors: &mut Vec<String>, who: &str, self_name: &str, primary_target: bool) {
    let p = &w.sim.players[&conn];
    let stats = &w.stats[conn as usize - 1];
    let mut e = |what: &str, got: String, expected: String| eq(errors, &format!("{who}.{what}"), got, expected);
    e("health", format!("{:?}", p.health), format!("{:?}", f32_of(&want["health"])));
    e("absorption", format!("{:?}", p.absorption), format!("{:?}", f32_of(&want["absorption"])));
    // The simulation's tick after the attack counted the cooldowns, fire and exhaustion once.
    let exhaustion = f32_of(&want["exhaustion"]);
    e("exhaustion", format!("{:?}", p.exhaustion), format!("{:?}", if exhaustion > 4.0 { exhaustion - 4.0 } else { exhaustion }));
    let cooldown = want["hurt_cooldown"].as_i64().unwrap() as i32;
    e("hurt_cooldown", format!("{}", p.hurt_cooldown), format!("{}", (cooldown - 1).max(0)));
    e("last_hurt", format!("{:?}", p.last_hurt), format!("{:?}", f32_of(&want["last_hurt"])));
    e("sprinting", format!("{}", p.sprinting), format!("{}", want["sprinting"].as_bool().unwrap()));
    let fire = want["fire_ticks"].as_i64().unwrap_or(0) as i32;
    let fire = if fire > 0 { fire - 1 } else { -crate::hazards::FIRE_IMMUNE_TICKS };
    // (Spectators do not rest at -20.)
    if p.game_mode != 3 {
        e("fire_ticks", format!("{}", p.fire_ticks), format!("{fire}"));
    }
    let main = p.inv.selected_item();
    let main_name = (!main.is_empty()).then(|| main.item_name().to_owned());
    e("main_hand", format!("{main_name:?}"), format!("{:?}", want["main_hand"].as_str().map(str::to_owned)));
    e("main_hand_damage", format!("{}", main.damage()), format!("{}", want["main_hand_damage"].as_i64().unwrap()));
    let armor: Vec<Option<i64>> = (0..4).map(|i| (!p.inv.equipment[i].is_empty()).then(|| p.inv.equipment[i].damage() as i64)).collect();
    let want_armor: Vec<Option<i64>> = want["armor_damage"].as_array().unwrap().iter().map(|v| v.as_i64()).collect();
    e("armor_damage", format!("{armor:?}"), format!("{want_armor:?}"));
    let _ = primary_target;
    check_motions(stats, p.entity_id, want, errors, who, self_name);
    let death = death_message(stats);
    let want_death = want["death"].as_str().map(|k| {
        let args: Vec<String> = want["death_args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect();
        (k.to_owned(), args)
    });
    eq(errors, &format!("{who}.death"), format!("{death:?}"), format!("{want_death:?}"));
}

fn death_message(stats: &SinkStats) -> Option<(String, Vec<String>)> {
    crate::combat_parity::death_message(stats)
}

fn check_mob(w: &World, id: i32, want: &Value, errors: &mut Vec<String>, who: &str) {
    let Some(s) = w.sim.mob_state(id) else {
        eq(errors, &format!("{who}.exists"), "gone".into(), "present".into());
        return;
    };
    let mut e = |what: &str, got: String, expected: String| eq(errors, &format!("{who}.{what}"), got, expected);
    e("health", format!("{:?}", s.health), format!("{:?}", f32_of(&want["health"])));
    e("alive", format!("{}", s.alive), format!("{}", want["alive"].as_bool().unwrap()));
    if !want["alive"].as_bool().unwrap() {
        return;
    }
    let v = vec_of(&want["velocity"]);
    e("velocity", format!("{:?}", s.delta), format!("{v:?}"));
    e("fire_ticks", format!("{}", s.fire_ticks), format!("{}", want["fire_ticks"].as_i64().unwrap().max(0)));
    e("hurt_time", format!("{}", s.hurt_time), format!("{}", want["hurt_time"]));
    e("damage_cooldown", format!("{}", s.damage_cooldown), format!("{}", want["damage_cooldown"]));
    e("last_hurt", format!("{:?}", s.last_hurt), format!("{:?}", f32_of(&want["last_hurt"])));
    e("absorption", format!("{:?}", s.absorption), format!("{:?}", f32_of(&want["absorption"])));
    let want_eq: Vec<Option<i64>> = want["equipment_damage"].as_array().unwrap().iter().map(|v| v.as_i64()).collect();
    let got_eq: Vec<Option<i64>> = s.equipment_damage.iter().map(|d| d.map(|d| d as i64)).collect();
    e("equipment_damage", format!("{:?}", &got_eq[..6]), format!("{:?}", &want_eq[..6]));
    let mut want_fx: Vec<String> = want["effects"].as_array().unwrap().iter().map(|f| format!("{} {} {}", f[0].as_str().unwrap(), f[1], f[2])).collect();
    let mut got_fx: Vec<String> = s.effects.iter().map(|f| format!("{} {} {}", f.0, f.1, f.2)).collect();
    want_fx.sort();
    got_fx.sort();
    e("effects", format!("{got_fx:?}"), format!("{want_fx:?}"));
    e("on_ground", format!("{}", s.on_ground), format!("{}", want["on_ground"].as_bool().unwrap()));
    e("vehicle", format!("{:?}", s.vehicle), format!("{:?}", want["vehicle"].as_str()));
}

fn run_case(line: &Value) -> Vec<String> {
    let victims = line["victims"].as_array().unwrap();
    let attacker_name = line["attacker_name"].as_str().unwrap().to_owned();
    let mut names = vec![attacker_name.clone()];
    let mut conn_of: Vec<Option<u64>> = Vec::new();
    for v in victims {
        if v["kind"].as_str() == Some("player") {
            names.push(v["name"].as_str().unwrap().to_owned());
            conn_of.push(Some(names.len() as u64));
        } else {
            conn_of.push(None);
        }
    }
    let mut w = World::new(&names);
    w.console(&format!("difficulty {}", line["difficulty"].as_str().unwrap()));
    w.console(&format!("gamerule minecraft:pvp {}", line["pvp"].as_bool().unwrap()));
    // The mobs, then the blocks (the commands tick the world; the players are put back after).
    let base = w.base;
    let mut mob_ids: Vec<Option<i32>> = Vec::new();
    let mut seen: Vec<i32> = Vec::new();
    for v in victims {
        if v["kind"].as_str() == Some("mob") {
            let cmd = summon_command(v, base);
            w.console(&cmd);
            assert!(w.sim.step([]));
            let ty = v["type"].as_str().unwrap();
            let fresh: Vec<i32> = w.sim.entity_ids_of(ty).into_iter().filter(|i| !seen.contains(i)).collect();
            // (Everything new is marked seen, the vehicle too.)
            let mut now = w.sim.entity_ids_of(ty);
            if let Some(vehicle) = v["vehicle"].as_str() {
                now.extend(w.sim.entity_ids_of(vehicle));
            }
            for i in now {
                if !seen.contains(&i) {
                    seen.push(i);
                }
            }
            mob_ids.push(fresh.first().copied());
        } else {
            mob_ids.push(None);
        }
    }
    for b in line["blocks"].as_array().unwrap() {
        // `setblock x y z state` with y relative to 100 (the attacker's feet level).
        let parts: Vec<&str> = b.as_str().unwrap().splitn(5, ' ').collect();
        let p = w.at(parts[1].parse().unwrap(), parts[2].parse::<i32>().unwrap() - 100, parts[3].parse().unwrap());
        w.console(&format!("setblock {} {} {} {}", p[0], p[1], p[2], parts[4]));
    }
    // The players as recorded (after everything that ticked), the seeds, the effects.
    let seed = line["level_seed"].as_i64().unwrap();
    let setup_all = |w: &mut World| {
        let base = w.base;
        setup(&mut w.sim, 1, &line["attacker"], base);
        for (i, v) in victims.iter().enumerate() {
            if let Some(conn) = conn_of[i] {
                setup(&mut w.sim, conn, &v["side"], base);
            }
        }
        {
            let p = w.player(1);
            p.rot[1] = f32_of(&line["attacker_pitch"]);
            p.entity_rng = LegacyRandom::new(seed.wrapping_add(1));
            p.level_rng = LegacyRandom::new(seed);
            for e in line["attacker_effects"].as_array().unwrap() {
                effect(p, e);
            }
            p.fall_flying = line["fall_flying"].as_bool().unwrap();
        }
        for (i, v) in victims.iter().enumerate() {
            if let Some(conn) = conn_of[i] {
                let p = w.player(conn);
                p.entity_rng = LegacyRandom::new(seed.wrapping_add(2 + i as i64));
                for e in v["effects"].as_array().unwrap() {
                    effect(p, e);
                }
            }
        }
    };
    setup_all(&mut w);
    if line["mounted"].as_bool().unwrap() {
        w.console("summon minecraft:pig ~ ~ ~ {NoAI:1b,PersistenceRequired:1b}");
    }
    setup_all(&mut w);
    if line["mounted"].as_bool().unwrap() {
        let pig = w.sim.entity_ids_of("minecraft:pig").into_iter().find(|i| !seen.contains(i));
        w.player(1).vehicle = pig;
        if let Some(pig) = pig {
            let pid = w.sim.players[&1].entity_id;
            for dim in w.sim.dims.iter_mut() {
                for r in dim.regions.iter_mut() {
                    if let Some(phys) = r.part_mut().0.list.iter_mut().find(|e| e.id == pig).and_then(|e| e.phys.as_deref_mut()) {
                        kiln_entity::ride::add_passenger(phys, pid, true, false);
                    }
                }
            }
        }
    }
    // What the attacker's state came to on the Java side (the fluids reset a fall, riding lifts).
    {
        let at = &line["attacker_at_attack"];
        let rel = vec_of(&at["pos"]);
        let p = w.player(1);
        p.pos = [base[0] + rel[0], base[1] + rel[1], base[2] + rel[2]];
        p.fall_distance = at["fall_distance"].as_f64().unwrap();
        p.on_ground = at["on_ground"].as_bool().unwrap();
    }
    // The mobs' own randoms are seeded like the Java side's.
    for (i, v) in victims.iter().enumerate() {
        let (Some(id), Some(mob_seed)) = (mob_ids[i], v["seed"].as_i64()) else { continue };
        for dim in w.sim.dims.iter_mut() {
            for r in dim.regions.iter_mut() {
                if let Some(phys) = r.part_mut().0.list.iter_mut().find(|e| e.id == id).and_then(|e| e.phys.as_deref_mut()) {
                    phys.random = LegacyRandom::new(mob_seed);
                }
            }
        }
    }
    // Who is who (the ids the animations name).
    let mut ids: HashMap<String, i32> = HashMap::new();
    ids.insert("attacker".into(), w.sim.players[&1].entity_id);
    for (i, _) in victims.iter().enumerate() {
        if let Some(conn) = conn_of[i] {
            ids.insert(format!("victim{i}"), w.sim.players[&conn].entity_id);
        } else if let Some(id) = mob_ids[i] {
            ids.insert(format!("victim{i}"), id);
        }
    }
    let skip_pos = line["mounted"].as_bool().unwrap() || victims.iter().any(|v| !v["vehicle"].is_null());
    let target_index = line["target"].as_u64().unwrap() as usize;
    let target_id = ids[&format!("victim{target_index}")];
    let mut errors = Vec::new();
    let steps = line["steps"].as_array().unwrap();
    let later: Vec<i64> = line["later"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    for (n, step) in steps.iter().enumerate() {
        if n > 0 {
            w.player(1).attack_ticker = later[n - 1] as i32;
            for conn in 1..=names.len() {
                *w.stats[conn - 1].log.lock().unwrap() = Some(Vec::new());
            }
        } else {
            for s in &w.stats {
                *s.log.lock().unwrap() = Some(Vec::new());
            }
        }
        assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: target_id })]));
        let prefix = if steps.len() > 1 { format!("hit{n}.") } else { String::new() };
        let cx = Ctx { base: w.base, ids: &ids, skip_pos };
        check_player(&w, 1, &step["attacker"], &mut errors, &format!("{prefix}attacker"), "attacker", false);
        check_world(&w.stats[0], &step["attacker"]["packets"], &cx, &mut errors, &format!("{prefix}attacker"));
        for (i, v) in step["victims"].as_array().unwrap().iter().enumerate() {
            match v["kind"].as_str().unwrap() {
                "player" => check_player(&w, conn_of[i].unwrap(), v, &mut errors, &format!("{prefix}victim{i}"), &format!("victim{i}"), i == target_index),
                "mob" => {
                    if let Some(id) = mob_ids[i] {
                        check_mob(&w, id, v, &mut errors, &format!("{prefix}victim{i}"));
                    }
                }
                _ => {}
            }
        }
    }
    errors
}

#[test]
fn melee_parity() {
    let Some(path) = std::env::var_os("KILN_MELEE_VECTORS") else {
        eprintln!("skipped: set KILN_MELEE_VECTORS (tools/combat_vectors.py --filter melee)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let mut cases: Vec<Value> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        cases.push(v);
    }
    // The scenarios are independent worlds: several run side by side.
    let threads = std::env::var("KILN_PARITY_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4usize).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<Vec<(usize, Vec<String>)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(case) = cases.get(i) else { break };
                    let errors = std::panic::catch_unwind(|| run_case(case)).unwrap_or_else(|e| {
                        let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                        vec![format!("panic: {msg}")]
                    });
                    results.lock().unwrap().push((i, errors));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    let (mut passed, mut failed) = (0, Vec::new());
    for (i, errors) in results {
        let name = cases[i]["name"].as_str().unwrap().to_owned();
        if errors.is_empty() {
            passed += 1;
        } else {
            println!("FAIL {name}\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    println!("melee parity: {passed} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "{} failed", failed.len());
}
