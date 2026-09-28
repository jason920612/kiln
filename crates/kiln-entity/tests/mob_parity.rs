//! Differential tests of mobs against vanilla 26.3: replays the scenarios recorded by
//! tools/MobVectors.java (`python tools/mob_vectors.py`) and compares every mob's state after
//! every tick bit for bit (position, velocity, rotations, health, hurt time, target, running
//! goals), and the ticks at which the player was hit.
//!
//! Vectors: `$KILN_MOB_VECTORS`, else `<KILN_WORK or workspace/work>/m6-mobs/vectors.jsonl`; the
//! test is skipped when they are absent. `KILN_PARITY_FILTER` selects scenarios by name.

use kiln_entity::entity::EntityKind;
use kiln_entity::level::{DamageKind, EntityLevel, PlayerView};
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::{self, DamageSource, MobKind};
use serde_json::Value;
use std::path::PathBuf;

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> Vec3 {
    Vec3::new(f(&v[0]), f(&v[1]), f(&v[2]))
}

/// Vanilla goal class names for Kiln's goals.
fn goal_class(name: &str, kind: MobKind) -> &'static str {
    match name {
        "float" => "FloatGoal",
        "panic" => "PanicGoal",
        "tempt" => "TemptGoal",
        "stroll" => "WaterAvoidingRandomStrollGoal",
        "look_at_player" => "LookAtPlayerGoal",
        "look_around" => "RandomLookAroundGoal",
        "eat_block" => "EatBlockGoal",
        "melee" => match kind {
            MobKind::Zombie => "ZombieAttackGoal",
            MobKind::Spider => "SpiderAttackGoal",
            MobKind::Skeleton => "",
            _ => "MeleeAttackGoal",
        },
        "bow" => "RangedBowAttackGoal",
        "swell" => "SwellGoal",
        "leap" => "LeapAtTargetGoal",
        "restrict_sun" => "RestrictSunGoal",
        "flee_sun" => "FleeSunGoal",
        "turtle_egg" => "ZombieAttackTurtleEggGoal",
        "hurt_by" => "HurtByTargetGoal",
        "nearest_attackable" => {
            if kind == MobKind::Spider {
                "SpiderTargetGoal"
            } else {
                "NearestAttackableTargetGoal"
            }
        }
        _ => "?",
    }
}

fn state(e: &kiln_entity::Entity) -> (Vec<f64>, String) {
    let m = mob::data(e).expect("a mob");
    let p = e.position();
    let v = e.delta;
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let nums = vec![
        e.id as f64,
        p.x,
        p.y,
        p.z,
        v.x,
        v.y,
        v.z,
        e.y_rot as f64,
        e.x_rot as f64,
        m.y_head_rot as f64,
        m.y_body_rot as f64,
        b(e.on_ground),
        m.health as f64,
        m.hurt_time as f64,
        b(e.is_removed()),
        e.remaining_fire_ticks as f64,
        m.target.map_or(-1.0, |t| t as f64),
        e.random.state() as f64,
    ];
    let mut goals: Vec<&str> = m.running_goals().into_iter().map(|g| goal_class(g, m.kind)).collect();
    goals.retain(|g| !g.is_empty());
    (nums, goals.join(" "))
}

const FIELDS: &[&str] =
    &["id", "x", "y", "z", "dx", "dy", "dz", "yaw", "pitch", "head", "body", "on_ground", "health", "hurt_time", "removed", "fire", "target", "random"];

fn replay(s: &Value) -> Result<usize, String> {
    let mut level = MemoryLevel::new(-64, s["level_seed"].as_i64().unwrap());
    level.bottom_layer = Some(kiln_data::blocks::default_state::BEDROCK);
    level.sky_darken = s["sky_darken"].as_i64().unwrap() as i32;
    let start = s["game_time"].as_i64().unwrap();
    for b in s["blocks"].as_array().unwrap() {
        let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
        level.blocks.insert(p, b[3].as_u64().unwrap() as u16);
    }
    let player = s.get("player").filter(|p| !p.is_null()).map(|p| {
        let mut v = PlayerView::new(p["id"].as_i64().unwrap() as i32, vec3(&p["pos"]));
        v.sneaking = p["sneaking"].as_bool().unwrap_or(false);
        if v.sneaking {
            v.eye_height = 1.27;
        }
        v.creative = p.get("creative").and_then(Value::as_bool).unwrap_or(false);
        v
    });
    if let Some(p) = player {
        level.players.push(p);
        // A stand-in the explosion can see and hurt.
        let mut proxy = kiln_entity::Entity::new("minecraft:player", p.id, 0, EntityKind::Other { type_name: "minecraft:player" }, 0);
        if p.sneaking {
            proxy.height = 1.5;
            proxy.eye_height = 1.27;
        }
        proxy.set_pos(p.pos);
        proxy.invulnerable = p.creative;
        level.insert(proxy);
    }
    let mut ids = Vec::new();
    for spec in s["mobs"].as_array().unwrap() {
        let kind = MobKind::by_name(spec["type"].as_str().unwrap()).expect("mob type");
        let id = spec["id"].as_i64().unwrap() as i32;
        let mut e = mob::new(kind, id, 0, 0);
        let yaw = f(&spec["yaw"]) as f32;
        e.set_pos(vec3(&spec["pos"]));
        e.y_rot = yaw;
        e.set_old_pos_and_rot();
        e.random = kiln_javamath::random::LegacyRandom::new(spec["seed"].as_i64().unwrap());
        {
            let m = mob::data_mut(&mut e).unwrap();
            m.y_head_rot = yaw;
            m.y_body_rot = yaw;
            m.y_head_rot_o = yaw;
            m.y_body_rot_o = yaw;
            if let mob::Species::Chicken { egg_time } = &mut m.species {
                *egg_time = spec["egg_time"].as_i64().unwrap() as i32;
            }
            if let Some(item) = spec["main_hand"].as_str() {
                m.equipment[mob::MAINHAND] = kiln_item::ItemStack::of(item, 1).unwrap();
                mob::reassess_weapon_goal(m, false);
            }
        }
        ids.push(id);
        level.insert(e);
    }
    let hurts: Vec<(i64, usize, f32)> = s["hurts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| (h[0].as_i64().unwrap(), h[1].as_u64().unwrap() as usize, f(&h[2]) as f32))
        .collect();
    let want_hits: Vec<(i64, f64)> = s["hits"].as_array().unwrap().iter().map(|h| (h[0].as_i64().unwrap(), f(&h[1]))).collect();
    let mut got_hits: Vec<(i64, f64)> = Vec::new();
    let trace = s["trace"].as_array().unwrap();
    let mut compared = 0;
    for (tick, expected) in trace.iter().enumerate() {
        let tick = tick as i64;
        level.game_time = start + 1 + tick;
        level.tick_players();
        for &(t, i, amount) in &hurts {
            if t == tick {
                let p = player.expect("a hurting player");
                let source = DamageSource {
                    kind: DamageKind::PlayerAttack,
                    attacker: Some(p.id),
                    direct: Some(p.id),
                    pos: Some(p.pos),
                    attacker_is_player: true,
                };
                let e = level.entity_mut(ids[i]).unwrap();
                let mut e2 = std::mem::replace(e, kiln_entity::Entity::new("minecraft:marker", -5, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
                mob::hurt_entity(&mut e2, &mut level, source, amount);
                *level.entity_mut(ids[i]).unwrap() = e2;
            }
        }
        let before = level.player_hits.len();
        let nearest = player.filter(|p| !p.spectator);
        for i in 0..level.len() {
            level.tick_one(i, |e, level| {
                if let (Some(p), EntityKind::Mob(_)) = (nearest, &e.kind) {
                    let d = e.position().distance_to_sqr(p.pos);
                    mob::check_despawn(e, level, Some(d));
                    if e.is_removed() {
                        return;
                    }
                }
                if matches!(e.kind, EntityKind::Other { .. }) {
                    return;
                }
                e.common_tick();
                e.tick(level);
            });
        }
        level.flush_spawned();
        // Explosions hurt the player through events (it is not an entity of the harness).
        for ev in std::mem::take(&mut level.events) {
            if let kiln_entity::level::Event::Hurt { target, amount, kind, attacker } = ev
                && Some(target) == player.map(|p| p.id)
            {
                let source = DamageSource { kind, attacker, direct: attacker, pos: None, attacker_is_player: false };
                level.hurt_player(target, source, amount);
            }
        }
        let dealt: f32 = level.player_hits[before..].iter().map(|h| h.1).sum();
        if dealt > 0.0 {
            got_hits.push((tick, dealt as f64));
        }
        if std::env::var_os("KILN_MOB_DEBUG").is_some() {
            let e = level.entity(ids[0]).unwrap();
            let m = mob::data(e).unwrap();
            eprintln!("dbg tick {tick} rnd {} ambient {} noaction {} goals {:?}", e.random.state(), m.ambient_sound_time, m.no_action_time, m.running_goals());
        }
        for (k, want) in expected.as_array().unwrap().iter().enumerate() {
            let want = want.as_array().unwrap();
            let want_goals = want.last().unwrap().as_str().unwrap();
            let want: Vec<f64> = want[..want.len() - 1].iter().map(f).collect();
            let e = level.entity(ids[k]).ok_or_else(|| format!("tick {tick}: mob {k} missing"))?;
            let (got, goals) = state(e);
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                // Rotations and health are floats, printed by Java's `Float.toString`.
                let float = matches!(i, 7..=10 | 12);
                let same = if float { (*g as f32).to_bits() == (*w as f32).to_bits() } else { g.to_bits() == w.to_bits() };
                if !same {
                    return Err(format!(
                        "tick {tick} mob {k}: {} = {g} (kiln) vs {w} (vanilla)\n  kiln    {got:?} [{goals}]\n  vanilla {want:?} [{want_goals}]",
                        FIELDS[i]
                    ));
                }
            }
            let mut a: Vec<&str> = goals.split_whitespace().collect();
            let mut b: Vec<&str> = want_goals.split_whitespace().collect();
            a.sort_unstable();
            b.sort_unstable();
            if a != b {
                return Err(format!("tick {tick} mob {k}: goals [{goals}] (kiln) vs [{want_goals}] (vanilla)"));
            }
            compared += 1;
        }
    }
    // Vanilla arrows draw their damage and spread from their own random, which is seeded from
    // the clock (not pinnable): skeleton scenarios compare the mob, not where arrows land.
    let arrows = s["mobs"].as_array().unwrap().iter().any(|m| m["main_hand"].as_str() == Some("minecraft:bow"));
    let f32s = |v: &[(i64, f64)]| v.iter().map(|&(t, a)| (t, (a as f32).to_bits())).collect::<Vec<_>>();
    if !arrows && f32s(&got_hits) != f32s(&want_hits) {
        return Err(format!("player hits {got_hits:?} (kiln) vs {want_hits:?} (vanilla)"));
    }
    Ok(compared)
}

fn vectors() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KILN_MOB_VECTORS") {
        return Some(PathBuf::from(p));
    }
    let work = std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let p = work.join("m6-mobs/vectors.jsonl");
    p.exists().then_some(p)
}

#[test]
fn mobs_match_vanilla() {
    let Some(path) = vectors() else {
        eprintln!("mob parity: no vectors (run tools/mob_vectors.py); skipped");
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap();
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let (mut pass, mut fail, mut states) = (0, 0, 0);
    for line in text.lines() {
        let s: Value = serde_json::from_str(line).unwrap();
        let name = s["name"].as_str().unwrap().to_owned();
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        if s.get("error").is_some() {
            eprintln!("{name}: vanilla error {}", s["error"]);
            continue;
        }
        match replay(&s) {
            Ok(n) => {
                pass += 1;
                states += n;
                eprintln!("ok   {name} ({n} mob states)");
            }
            Err(e) => {
                fail += 1;
                eprintln!("FAIL {name}: {e}");
            }
        }
    }
    eprintln!("mob parity: {pass} passed, {fail} failed, {states} mob states identical");
    assert_eq!(fail, 0, "{fail} scenarios differ from vanilla");
}
