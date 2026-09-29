//! Differential tests of mobs against vanilla 26.3: replays the scenarios recorded by
//! tools/MobVectors.java (`python tools/mob_vectors.py`) and compares every mob's state after
//! every tick bit for bit (position, velocity, rotations, health, hurt time, target, running
//! goals), and the ticks at which the player was hit.
//!
//! Vectors: `$KILN_MOB_VECTORS`, else `<KILN_WORK or workspace/work>/m6-mobs2/vectors.jsonl`; the
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
fn goal_class(name: &'static str, kind: MobKind) -> &'static str {
    match name {
        "float" => "FloatGoal",
        "panic" => "PanicGoal",
        "tempt" => "TemptGoal",
        "breed" => "BreedGoal",
        "follow_parent" => "FollowParentGoal",
        "stroll" => {
            if matches!(kind, MobKind::Drowned | MobKind::Pillager | MobKind::Vindicator | MobKind::Evoker | MobKind::Illusioner) {
                "RandomStrollGoal"
            } else {
                "WaterAvoidingRandomStrollGoal"
            }
        }
        "look_at_player" => "LookAtPlayerGoal",
        "look_around" => "RandomLookAroundGoal",
        "eat_block" => "EatBlockGoal",
        "melee" => match kind {
            k if k.is_zombie() => "ZombieAttackGoal",
            MobKind::Spider => "SpiderAttackGoal",
            // `AbstractSkeleton$1` (an anonymous class: no simple name).
            k if k.is_skeleton() => "",
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
        // Extension goals are named after their vanilla class.
        n if n.starts_with(|c: char| c.is_ascii_uppercase()) => n,
        _ => "?",
    }
}

/// Typed JSON NBT (see `MobVectors.tagJson`).
fn tag_of(v: &Value) -> kiln_proto::nbt::Tag {
    use kiln_proto::nbt::Tag;
    let (k, x) = v.as_object().unwrap().iter().next().unwrap();
    match k.as_str() {
        "c" => Tag::Compound(x.as_object().unwrap().iter().map(|(k, v)| (k.clone(), tag_of(v))).collect()),
        "l" => Tag::List(x.as_array().unwrap().iter().map(tag_of).collect()),
        "b" => Tag::Byte(x.as_i64().unwrap() as i8),
        "s" => Tag::Short(x.as_i64().unwrap() as i16),
        "i" => Tag::Int(x.as_i64().unwrap() as i32),
        "L" => Tag::Long(x.as_str().unwrap().parse().unwrap()),
        "f" => Tag::Float(f(x) as f32),
        "d" => Tag::Double(f(x)),
        "str" => Tag::String(x.as_str().unwrap().to_owned()),
        "ia" => Tag::IntArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect()),
        "ba" => Tag::ByteArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i8).collect()),
        "la" => Tag::LongArray(x.as_array().unwrap().iter().map(|v| v.as_str().unwrap().parse().unwrap()).collect()),
        _ => panic!("tag {k}"),
    }
}

fn state(e: &kiln_entity::Entity, level: &dyn EntityLevel) -> (Vec<f64>, String) {
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
        // `Mob.getTarget`: the target while it can be attacked (not once it died).
        mob::goals::target(m, level).map_or(-1.0, |t| t.id as f64),
        e.random.state() as f64,
        effects_sig(m) as f64,
        m.absorption as f64,
    ];
    let mut goals: Vec<String> = m.running_goals().into_iter().map(|g| goal_class(g, m.kind).to_owned()).collect();
    goals.retain(|g| !g.is_empty());
    // The dragon's phase (`EnderDragonPhase` id).
    if let Some(d) = mob::kinds::ender_dragon::state_of(e) {
        goals.push(format!("DragonPhase{}", d.phase.id()));
    }
    (nums, goals.join(" "))
}

const FIELDS: &[&str] = &[
    "id", "x", "y", "z", "dx", "dy", "dz", "yaw", "pitch", "head", "body", "on_ground", "health", "hurt_time", "removed", "fire", "target", "random",
    "effects", "absorption",
];

/// `MobVectors.effectsSig`: the sum of (id + 1) * 100000 + duration * 10 + amplifier.
fn effects_sig(m: &mob::MobData) -> i64 {
    m.effects.values().map(|e| (e.id as i64 + 1) * 100000 + e.duration as i64 * 10 + e.amplifier as i64).sum()
}

/// A scenario action (`MobVectors.Action`), run before the entity ticks of its tick.
fn act(level: &mut MemoryLevel, ids: &[i32], player: Option<PlayerView>, a: &Value) {
    let kind = a["kind"].as_str().unwrap();
    let what = a["what"].as_str().unwrap_or("");
    let pos = vec3(&a["pos"]);
    match kind {
        "effect" => {
            let id = ids[a["mob"].as_u64().unwrap() as usize];
            let fx = kiln_entity::effect::Effect::named(what, a["duration"].as_i64().unwrap() as i32, a["amp"].as_i64().unwrap() as i32).unwrap();
            level.add_effect_instance(id, fx, None);
        }
        "splash" | "linger" => {
            // A potion entity at the spot (it takes an id, as vanilla's constructor does), broken
            // on a block hit there.
            let item = if kind == "splash" { "minecraft:splash_potion" } else { "minecraft:lingering_potion" };
            let mut stack = kiln_item::ItemStack::of(item, 1).unwrap();
            stack.insert(kiln_item::keys::POTION_CONTENTS, kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id(what), ..Default::default() });
            let pid = level.next_entity_id();
            let throwable = if kind == "splash" { kiln_entity::projectile::Throwable::SplashPotion } else { kiln_entity::projectile::Throwable::LingeringPotion };
            let mut p = kiln_entity::projectile::new(pid, 0, throwable, pos, Vec3::ZERO, None, 0);
            let hit = kiln_entity::projectile::Hit::Block { pos: BlockPos::containing(pos.x, pos.y, pos.z), face: kiln_entity::math::Direction::Up, location: pos };
            if kind == "splash" {
                mob::kinds::witch::splash(&mut p, level, hit, &stack, None);
            } else {
                mob::kinds::witch::linger(&mut p, level, hit, &stack, None);
            }
        }
        "interact" => {
            let id = ids[a["mob"].as_u64().unwrap() as usize];
            let p = player.expect("an interacting player");
            let who = mob::interact::Interactor { id: p.id, creative: p.creative, sneaking: p.sneaking };
            let stack = kiln_item::ItemStack::of(what, 1).unwrap();
            let e = level.entity_mut(id).unwrap();
            let mut e2 = std::mem::replace(e, kiln_entity::Entity::new("minecraft:marker", -5, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
            mob::interact::interact(&mut e2, level, &who, &stack);
            *level.entity_mut(id).unwrap() = e2;
        }
        k => panic!("action {k}"),
    }
}

fn replay(s: &Value) -> Result<usize, String> {
    // Diverging (brain-driven) scenarios compare the body only: not the random or the goals.
    let loose = s.get("diverges").and_then(Value::as_bool) == Some(true);
    let mut level = MemoryLevel::new(-64, s["level_seed"].as_i64().unwrap());
    level.bottom_layer = Some(kiln_data::blocks::default_state::BEDROCK);
    level.sky_darken = s["sky_darken"].as_i64().unwrap() as i32;
    // The recording world is superflat.
    level.sea_level = -63;
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
        // The recording's player is never ticked: it never finds itself in water.
        v.in_water = Some(false);
        if let Some(item) = p.get("main_hand").and_then(Value::as_str) {
            v.main_hand = kiln_data::builtin_id("minecraft:item", item).unwrap();
        }
        if let Some(item) = p.get("head").and_then(Value::as_str) {
            v.head = kiln_data::builtin_id("minecraft:item", item).unwrap();
        }
        v.yaw = p.get("yaw").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        // The recording's player is never ticked: its clock and hurt stamp as they were.
        v.tick_count = p.get("tick_count").and_then(Value::as_i64).unwrap_or(0) as i32;
        v.last_hurt_by_mob_time = p.get("last_hurt_by_mob_time").and_then(Value::as_i64).unwrap_or(0) as i32;
        v.pitch = p.get("pitch").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        if let Some(u) = p.get("uuid").and_then(Value::as_array) {
            v.uuid = u.iter().fold(0u128, |acc, x| (acc << 32) | (x.as_i64().unwrap() as u32 as u128));
        }
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
            m.in_love = spec.get("in_love").and_then(Value::as_i64).unwrap_or(0) as i32;
            // The harness equips the main hand before it reads the NBT (which replaces the
            // equipment).
            if let Some(item) = spec["main_hand"].as_str() {
                m.equipment[mob::MAINHAND] = kiln_item::ItemStack::of(item, 1).unwrap();
                mob::reassess_weapon_goal(m, false);
            }
        }
        if let Some(nbt) = spec.get("nbt").filter(|v| !v.is_null()) {
            mob::persist::apply_nbt(&mut e, &tag_of(nbt));
        }
        {
            let age = spec.get("age").and_then(Value::as_i64).unwrap_or(0) as i32;
            if age != 0 {
                let mut m = std::mem::replace(&mut e.kind, EntityKind::MobTicking { gravity: 0.08 });
                if let EntityKind::Mob(md) = &mut m {
                    mob::set_age(&mut e, md, age);
                }
                e.kind = m;
            }
        }
        ids.push(id);
        level.insert(e);
        for fx in spec.get("effects").and_then(Value::as_array).into_iter().flatten() {
            let fx = kiln_entity::effect::Effect::named(fx[0].as_str().unwrap(), fx[1].as_i64().unwrap() as i32, fx[2].as_i64().unwrap() as i32).unwrap();
            level.add_effect_instance(id, fx, None);
        }
    }
    // Other entities (end crystals), after the mobs: ticked, not traced.
    let mut other_ids = Vec::new();
    for o in s.get("others").and_then(Value::as_array).into_iter().flatten() {
        let id = o["id"].as_i64().unwrap() as i32;
        let tag = kiln_proto::nbt::Tag::Compound(vec![
            ("id".into(), kiln_proto::nbt::Tag::String(o["type"].as_str().unwrap().to_owned())),
            ("Pos".into(), kiln_proto::nbt::Tag::List((0..3).map(|i| kiln_proto::nbt::Tag::Double(f(&o["pos"][i]))).collect())),
            ("Rotation".into(), kiln_proto::nbt::Tag::List(vec![kiln_proto::nbt::Tag::Float(f(&o["yaw"]) as f32), kiln_proto::nbt::Tag::Float(0.0)])),
        ]);
        let e = kiln_entity::persist::load(&tag, id, 0).expect("other entity");
        other_ids.push(id);
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
    let initial = ids.len();
    // Vanilla numbers new entities on from the scenario's mobs (id parity paces the AI).
    level.set_next_entity_id(ids.iter().chain(&other_ids).copied().max().unwrap_or(0) + 1);
    level.immediate_adds = true;
    let mut known = level.len();
    let mut compared = 0;
    for (tick, expected) in trace.iter().enumerate() {
        let tick = tick as i64;
        level.game_time = start + 1 + tick;
        level.tick_players();
        // What the hurts and actions spawn is in the level at once but joins the harness's
        // ticked entities at the end of the tick (vanilla's harness ticks what it tracks).
        let ticked = level.len();
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
        for a in s.get("actions").and_then(Value::as_array).into_iter().flatten() {
            if a["tick"].as_i64() == Some(tick) {
                act(&mut level, &ids, player, a);
            }
        }
        let before = level.player_hits.len();
        let nearest = player.filter(|p| !p.spectator);
        for i in 0..ticked {
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
        let before_flush = known;
        level.flush_spawned();
        known = level.len();
        // Mobs that appeared get the harness's pinned random and head/body yaw, in the order the
        // harness finds them (`getEntities` over its box: entity sections, then insertion).
        let fresh: Vec<i32> = (before_flush..level.len()).filter_map(|i| level.entity_at(i)).filter(|e| mob::data(e).is_some()).map(|e| e.id).collect();
        let harness_box = kiln_entity::math::Aabb::new(-60.0, 60.0, -60.0, 60.0, 140.0, 60.0);
        let order = level.entities_in(&harness_box, kiln_entity::EntityFilter::Any, i32::MIN);
        let mut fresh = fresh;
        fresh.sort_by_key(|id| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
        for id in fresh {
            let n = (ids.len() - initial) as i64;
            let e = level.entity_mut(id).unwrap();
            e.random = kiln_javamath::random::LegacyRandom::new(7777 * (tick + 1) + n);
            let yaw = e.y_rot;
            let m = mob::data_mut(e).unwrap();
            m.y_head_rot = yaw;
            m.y_head_rot_o = yaw;
            m.y_body_rot = yaw;
            m.y_body_rot_o = yaw;
            if let mob::Species::Chicken { egg_time } = &mut m.species {
                *egg_time = 6000 + n as i32;
            }
            ids.push(id);
        }
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
        if let Some(k) = std::env::var("KILN_MOB_DEBUG").ok().map(|v| v.parse::<usize>().unwrap_or(0))
            && let Some(e) = ids.get(k).and_then(|&id| level.entity(id))
        {
            let m = mob::data(e).unwrap();
            eprintln!("dbg tick {tick} rnd {} ambient {} noaction {} goals {:?} path {:?}", e.random.state(), m.ambient_sound_time, m.no_action_time, m.running_goals(), m.nav.path.as_ref().map(|p| (p.next, p.target, p.nodes.iter().map(|n| (n.x, n.y, n.z)).collect::<Vec<_>>())));
        }
        for (k, want) in expected.as_array().unwrap().iter().enumerate() {
            let want = want.as_array().unwrap();
            let want_goals = want.last().unwrap().as_str().unwrap();
            let want: Vec<f64> = want[..want.len() - 1].iter().map(f).collect();
            let e = level.entity(ids[k]).ok_or_else(|| format!("tick {tick}: mob {k} missing"))?;
            let (got, goals) = state(e, &level);
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                // Mobs that appeared have ids of their own on each side.
                if (k >= initial && i == 0) || (loose && i == 17) {
                    continue;
                }
                // Rotations and health are floats, printed by Java's `Float.toString`.
                let float = matches!(i, 7..=10 | 12 | 19);
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
            if a != b && !loose {
                return Err(format!("tick {tick} mob {k}: goals [{goals}] (kiln) vs [{want_goals}] (vanilla)"));
            }
            compared += 1;
        }
    }
    // Vanilla arrows draw their damage and spread from their own random, which is seeded from
    // the clock (not pinnable): skeleton scenarios compare the mob, not where arrows land.
    // Shulker bullets likewise steer by their own random.
    let arrows = s["mobs"].as_array().unwrap().iter().any(|m| matches!(m["main_hand"].as_str(), Some("minecraft:bow" | "minecraft:trident" | "minecraft:crossbow")) || matches!(m["type"].as_str(), Some("minecraft:shulker" | "minecraft:witch")));
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
    let p = work.join("m6-mobs2/vectors.jsonl");
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
        if filter.as_deref().is_some_and(|f| !f.split('|').any(|f| name.contains(f))) {
            continue;
        }
        // Raid wave compositions are checked by kiln-sim's raid tests.
        if s.get("raid_waves").is_some() {
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
            // Brain-driven mobs Kiln approximates with goals: where they part is reported.
            Err(e) if s.get("diverges").and_then(Value::as_bool) == Some(true) => {
                eprintln!("DIVERGES {name} (brain vs goals): {}", e.lines().next().unwrap_or(""));
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
