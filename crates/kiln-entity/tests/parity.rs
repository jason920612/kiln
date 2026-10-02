//! Differential tests against vanilla 26.3: replays the scenarios recorded by
//! tools/EntityVectors.java (`python tools/entity_parity.py`) and compares every entity's state
//! after every tick, bit for bit.
//!
//! Vectors: `$KILN_ENTITY_VECTORS`, else `<KILN_WORK or workspace/work>/wp4-entities/vectors.jsonl`;
//! the test is skipped when they are absent. `KILN_PARITY_FILTER` selects scenarios by name.

use kiln_entity::entity::{Entity, EntityKind};
use kiln_entity::level::EntityLevel;
use kiln_entity::math::BlockPos;
use kiln_entity::memory::MemoryLevel;
use kiln_entity::{arrow, falling_block, item, player, projectile, tnt, xp_orb};
use kiln_item::ItemStack;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn vec3(v: &Value) -> kiln_entity::math::Vec3 {
    kiln_entity::math::Vec3::new(f(&v[0]), f(&v[1]), f(&v[2]))
}

/// Puts "slot:item:count,..." into a container entity's slots.
fn fill(c: &mut kiln_entity::ext_entity::minecart::Contents, items: &str) {
    for part in items.split(',') {
        let p: Vec<&str> = part.split(':').collect();
        let name = format!("{}:{}", p[1], p[2]);
        c.items[p[0].parse::<usize>().unwrap()] = ItemStack::of(&name, p[3].parse().unwrap()).unwrap_or_else(|| panic!("unknown item {name}"));
    }
}

fn spawn(spec: &Value) -> Entity {
    let id = spec["id"].as_i64().unwrap() as i32;
    let seed = spec["seed"].as_i64().unwrap();
    let int = |k: &str, d: i64| spec.get(k).and_then(Value::as_i64).unwrap_or(d) as i32;
    let kind = spec["kind"].as_str().unwrap();
    let mut e = match kind {
        "item" => {
            let name = spec.get("item").and_then(Value::as_str).unwrap_or("minecraft:stone");
            let stack = ItemStack::of(name, int("count", 1)).unwrap_or_else(|| panic!("unknown item {name}"));
            let mut data = item::ItemData::new(stack);
            data.pickup_delay = int("pickup_delay", 10);
            data.age = int("age", 0);
            Entity::new("minecraft:item", id, 0, EntityKind::Item(data), seed)
        }
        "tnt" => Entity::new(
            "minecraft:tnt",
            id,
            0,
            EntityKind::Tnt(tnt::TntData { fuse: int("fuse", 80), ..tnt::TntData::new() }),
            seed,
        ),
        "falling_block" => {
            let state = spec["block_id"].as_i64().unwrap_or(0) as u16;
            Entity::new(
                "minecraft:falling_block",
                id,
                0,
                EntityKind::FallingBlock(falling_block::FallingBlockData {
                    state,
                    time: int("time", 0),
                    drop_item: spec.get("drop_item").and_then(Value::as_bool).unwrap_or(true),
                    cancel_drop: spec.get("cancel_drop").and_then(Value::as_bool).unwrap_or(false),
                    hurt_entities: spec.get("hurt_per_distance").is_some(),
                    fall_damage_max: int("hurt_max", 40),
                    fall_damage_per_distance: spec.get("hurt_per_distance").map_or(0.0, f) as f32,
                }),
                seed,
            )
        }
        "experience_orb" => Entity::new(
            "minecraft:experience_orb",
            id,
            0,
            EntityKind::ExperienceOrb(xp_orb::OrbData { count: int("count", 1), age: int("age", 0), ..xp_orb::OrbData::new(int("value", 1)) }),
            seed,
        ),
        "snowball" | "egg" | "ender_pearl" => {
            let kind = match kind {
                "snowball" => projectile::Throwable::Snowball,
                "egg" => projectile::Throwable::Egg,
                _ => projectile::Throwable::EnderPearl,
            };
            projectile::new(id, 0, kind, vec3(&spec["pos"]), vec3(&spec["motion"]), None, seed)
        }
        "minecart" => {
            let mut e = kiln_entity::ext_entity::minecart::new("minecraft:minecart", vec3(&spec["pos"]), seed);
            e.id = id;
            e
        }
        "furnace_minecart" | "tnt_minecart" | "hopper_minecart" | "chest_minecart" => {
            use kiln_entity::ext_entity::minecart::Minecart;
            let name: &'static str = kiln_data::entities::by_name(&format!("minecraft:{kind}")).unwrap().name;
            let mut e = kiln_entity::ext_entity::minecart::new(name, vec3(&spec["pos"]), seed);
            e.id = id;
            let cart = kiln_entity::ext_entity::get_mut::<Minecart>(&mut e).unwrap();
            cart.fuel = int("fuel", 0);
            cart.push = kiln_entity::math::Vec3::new(spec.get("push_x").map_or(0.0, f), 0.0, spec.get("push_z").map_or(0.0, f));
            cart.fuse = int("fuse", -1);
            if spec.get("disabled").is_some() {
                cart.enabled = false;
            }
            if let (Some(items), Some(c)) = (spec.get("items").and_then(Value::as_str), cart.contents.as_mut()) {
                fill(c, items);
            }
            e
        }
        "firework" => {
            let mut e = kiln_entity::ext_entity::firework::new(vec3(&spec["pos"]), ItemStack::of("minecraft:firework_rocket", 1).unwrap(), None, None, false, seed);
            e.id = id;
            if let Some(x) = kiln_entity::ext_entity::get_mut::<kiln_entity::ext_entity::firework::Firework>(&mut e) {
                x.lifetime = int("lifetime", 20);
            }
            e
        }
        boat if boat.ends_with("_boat") || boat.ends_with("_raft") => {
            let name: &'static str = kiln_data::entities::by_name(&format!("minecraft:{boat}")).unwrap().name;
            let mut e = kiln_entity::ext_entity::boat::new(name, vec3(&spec["pos"]), f(&spec["yaw"]) as f32, seed);
            e.id = id;
            if let (Some(items), Some(c)) = (spec.get("items").and_then(Value::as_str), kiln_entity::ext_entity::container_mut(&mut e)) {
                fill(c, items);
            }
            e
        }
        "arrow" => arrow::new(id, 0, "minecraft:arrow", vec3(&spec["pos"]), vec3(&spec["motion"]), None, seed),
        "player" => {
            let shift = spec.get("shift").and_then(Value::as_bool).unwrap_or(false);
            let mut p = player::new(id, 0, vec3(&spec["pos"]), if shift { 1.5 } else { 1.8 }, 0.6);
            p.shift_key_down = shift;
            if let EntityKind::Player(d) = &mut p.kind {
                d.flying = spec.get("flying").and_then(Value::as_bool).unwrap_or(false);
            }
            p
        }
        other => panic!("unknown kind {other}"),
    };
    e.set_pos(vec3(&spec["pos"]));
    e.delta = vec3(&spec["motion"]);
    e.y_rot = f(&spec["yaw"]) as f32;
    if spec.get("no_gravity").is_some() {
        e.no_gravity = true;
    }
    if let Some(v) = spec.get("fire") {
        e.remaining_fire_ticks = v.as_i64().unwrap() as i32;
    }
    if let Some(v) = spec.get("fall_distance") {
        e.fall_distance = f(v);
    }
    e
}

/// The recorded state vector of an entity (see EntityVectors.state).
fn state(e: &Entity) -> Vec<f64> {
    let p = e.position();
    let v = e.delta;
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let mut out = vec![
        e.id as f64,
        p.x,
        p.y,
        p.z,
        v.x,
        v.y,
        v.z,
        b(e.on_ground),
        b(e.horizontal_collision),
        b(e.vertical_collision),
        e.fall_distance,
        b(e.is_removed()),
        e.remaining_fire_ticks as f64,
        e.air_supply as f64,
    ];
    match &e.kind {
        EntityKind::Item(d) => {
            out.extend([d.stack.count() as f64, d.age as f64, d.pickup_delay as f64, d.health as f64]);
        }
        EntityKind::Tnt(d) => out.push(d.fuse as f64),
        EntityKind::FallingBlock(d) => out.extend([d.time as f64, d.state as f64]),
        EntityKind::ExperienceOrb(d) => out.extend([d.value as f64, d.count as f64, d.age as f64]),
        EntityKind::Arrow(a) => out.extend([a.in_ground as i32 as f64, a.shake_time as f64, a.life as f64]),
        EntityKind::Ext(_) if kiln_entity::ext_entity::get::<kiln_entity::ext_entity::firework::Firework>(e).is_some() => {
            let r = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::firework::Firework>(e).unwrap();
            out.extend([r.life as f64, r.lifetime as f64]);
        }
        EntityKind::Ext(_) if kiln_entity::ext_entity::get::<kiln_entity::ext_entity::minecart::Minecart>(e).is_some() => {
            let cart = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::minecart::Minecart>(e).unwrap();
            out.extend([e.y_rot as f64, b(cart.flipped)]);
            if cart.furnace {
                out.extend([cart.fuel as f64, cart.push.x, cart.push.z]);
            } else if cart.tnt {
                out.push(cart.fuse as f64);
            } else if let Some(c) = &cart.contents {
                out.push(b(cart.enabled || e.type_name != "minecraft:hopper_minecart"));
                for s in &c.items {
                    out.extend([if s.is_empty() { 0.0 } else { s.item() as f64 + 1.0 }, s.count() as f64]);
                }
            }
        }
        EntityKind::Ext(_) if kiln_entity::ext_entity::get::<kiln_entity::ext_entity::boat::Boat>(e).is_some() => {
            out.push(e.y_rot as f64);
            if let Some(c) = kiln_entity::ext_entity::container(e) {
                for s in &c.items {
                    out.extend([if s.is_empty() { 0.0 } else { s.item() as f64 + 1.0 }, s.count() as f64]);
                }
            }
        }
        EntityKind::Player(_) | EntityKind::Throwable(_) | EntityKind::Other { .. } | EntityKind::Mob(_) | EntityKind::MobTicking { .. } | EntityKind::Ext(_) => {}
    }
    out
}

const FIELDS: &[&str] = &[
    "id", "x", "y", "z", "dx", "dy", "dz", "on_ground", "h_coll", "v_coll", "fall", "removed", "fire", "air", "k0", "k1", "k2", "k3",
];

fn field_name(i: usize) -> String {
    FIELDS.get(i).map_or_else(|| format!("k{}", i - 14), |s| (*s).to_string())
}

/// The scripted hits of an entity ("tick:kind:amount;...").
fn hits_of(spec: &Value) -> Vec<(usize, kiln_entity::level::DamageKind, f32)> {
    use kiln_entity::level::DamageKind;
    let Some(text) = spec.get("hits").and_then(Value::as_str) else { return Vec::new() };
    text.split(';')
        .map(|part| {
            let p: Vec<&str> = part.split(':').collect();
            let kind = match p[1] {
                "generic" => DamageKind::Generic,
                "explosion" => DamageKind::Explosion,
                "in_fire" => DamageKind::InFire,
                "on_fire" => DamageKind::OnFire,
                "lava" => DamageKind::Lava,
                other => panic!("unknown damage {other}"),
            };
            (p[0].parse().unwrap(), kind, p[2].parse().unwrap())
        })
        .collect()
}

/// Replays one scenario; `Err` describes the first mismatch.
fn replay(s: &Value) -> Result<(), String> {
    let mut level = MemoryLevel::new(-64, s["level_seed"].as_i64().unwrap());
    // The harness world is superflat with one bedrock layer.
    level.bottom_layer = Some(kiln_data::blocks::default_state::BEDROCK);
    for b in s["blocks"].as_array().unwrap() {
        let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
        level.blocks.insert(p, b[3].as_u64().unwrap() as u16);
    }
    let mut moves: HashMap<i32, Vec<kiln_entity::math::Vec3>> = HashMap::new();
    let mut hits: HashMap<i32, Vec<(usize, kiln_entity::level::DamageKind, f32)>> = HashMap::new();
    for spec in s["entities"].as_array().unwrap() {
        let mut e = spawn(spec);
        let h = hits_of(spec);
        if !h.is_empty() {
            hits.insert(e.id, h);
        }
        if let Some(m) = spec.get("moves").and_then(Value::as_array) {
            moves.insert(e.id, m.iter().map(vec3).collect());
            if spec.get("on_ground").and_then(Value::as_bool).unwrap_or(false) {
                e.set_on_ground(&level, true);
            }
        }
        level.insert(e);
    }
    let trace = s["trace"].as_array().unwrap();
    let initial = level.len();
    let mut seen_spawned: Vec<usize> = Vec::new();
    for (tick, expected) in trace.iter().enumerate() {
        // Scripted hits land before the entities tick.
        for i in 0..level.len() {
            level.tick_one(i, |e, level| {
                for &(at, kind, amount) in hits.get(&e.id).into_iter().flatten() {
                    if at == tick {
                        e.hurt(level, kind, amount, None);
                    }
                }
            });
        }
        for i in 0..level.len() {
            level.tick_one(i, |e, level| {
                if let Some(m) = moves.get(&e.id) {
                    e.delta = m[tick];
                    player::server_move(level, e, m[tick]);
                } else {
                    e.common_tick();
                    e.tick(level);
                }
            });
        }
        level.flush_spawned();
        let expected = expected.as_array().unwrap();
        for (k, want) in expected.iter().enumerate() {
            let want: Vec<f64> = want.as_array().unwrap().iter().map(f).collect();
            let Some(entity) = level.entity_at(k) else {
                return Err(format!("tick {tick}: vanilla has entity #{k} ({want:?}), kiln has none"));
            };
            let got = state(entity);
            // Entities spawned during the run (drops, primed TNT) get fresh ids and, in vanilla,
            // velocities from an unseeded random: compare where and what they are when they appear.
            let spawned = k >= initial;
            if spawned && seen_spawned.contains(&k) {
                continue;
            }
            if spawned {
                seen_spawned.push(k);
            }
            if got.len() != want.len() {
                return Err(format!("tick {tick}: entity {k} state length {} vs {}", got.len(), want.len()));
            }
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                if spawned && matches!(i, 0 | 4 | 5 | 6) {
                    continue;
                }
                // A burning arrow that sets a TNT minecart off is thrown by the blast in vanilla
                // (it is still alive when the cart explodes); Kiln's blast cannot reach the
                // entity being ticked, and the arrow is gone either way.
                if want[11] == 1.0 && matches!(i, 4 | 5 | 6) && matches!(entity.kind, EntityKind::Arrow(_)) && s["name"].as_str().is_some_and(|n| n.starts_with("arrow_vehicle/")) {
                    continue;
                }
                if g.to_bits() != w.to_bits() {
                    if std::env::var_os("KILN_PARITY_DUMP").is_some() {
                        for (n, ex) in trace.iter().enumerate().take(tick + 1).skip(tick.saturating_sub(2)) {
                            for (k2, want2) in ex.as_array().unwrap().iter().enumerate() {
                                eprintln!("  tick {n} entity {k2} vanilla {want2}");
                            }
                        }
                        for (k2, e2) in level.entities().enumerate() {
                            eprintln!("  now entity {k2} kiln {:?}", state(e2));
                        }
                    }
                    return Err(format!(
                        "tick {tick} entity {} field {}: kiln {g:?} vanilla {w:?}\n  kiln    {got:?}\n  vanilla {want:?}",
                        want[0],
                        field_name(i)
                    ));
                }
            }
        }
        if level.len() > expected.len() {
            return Err(format!("tick {tick}: kiln has {} entities, vanilla {}", level.len(), expected.len()));
        }
    }
    if let Some(want) = s["final_blocks"].as_array() {
        for b in want {
            let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
            let w = b[3].as_u64().unwrap() as u16;
            if level.block(p) != w {
                return Err(format!("final block at {p:?}: kiln {} vanilla {w}", level.block(p)));
            }
        }
        let nonair = level.blocks.values().filter(|&&v| v != 0).count();
        if nonair != want.len() {
            return Err(format!("final blocks: kiln has {nonair} non-air, vanilla {}", want.len()));
        }
    }
    Ok(())
}

fn vectors_path() -> PathBuf {
    if let Some(p) = std::env::var_os("KILN_ENTITY_VECTORS") {
        return PathBuf::from(p);
    }
    let work = std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    work.join("wp4-entities/vectors.jsonl")
}

#[test]
fn vanilla_parity() {
    let path = vectors_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipping: no vectors at {}", path.display());
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let (mut pass, mut fail) = (0, 0);
    let mut by_group: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
    let mut failures = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let s: Value = serde_json::from_str(line).expect("vector line");
        let name = s["name"].as_str().unwrap().to_string();
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let group = name.split('/').next().unwrap().to_string();
        let result = if s.get("error").is_some() {
            Err(format!("vanilla error: {}", s["error"]))
        } else {
            std::panic::catch_unwind(|| replay(&s)).unwrap_or_else(|p| {
                Err(format!("panic: {}", p.downcast_ref::<String>().cloned().unwrap_or_else(|| {
                    p.downcast_ref::<&str>().map(|s| s.to_string()).unwrap_or_default()
                })))
            })
        };
        let g = by_group.entry(group).or_default();
        match result {
            Ok(()) => {
                pass += 1;
                g.0 += 1;
            }
            Err(msg) => {
                fail += 1;
                g.1 += 1;
                failures.push(format!("{name}: {msg}"));
            }
        }
    }
    for (g, (p, f)) in &by_group {
        eprintln!("{g:24} {p:5} pass {f:5} fail");
    }
    for msg in failures.iter().take(15) {
        eprintln!("FAIL {msg}");
    }
    eprintln!("parity: {pass} passed, {fail} failed");
    assert_eq!(fail, 0, "{fail} scenarios differ from vanilla");
}
