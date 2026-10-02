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

fn state(e: &kiln_entity::Entity, level: &MemoryLevel) -> (Vec<f64>, String) {
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
    goals.extend(m.brain_trace().into_iter().filter(|t| !t.starts_with("lr:")));
    if m.brain.is_some() {
        let lr = level.random_state();
        goals.push(format!("lr:{lr}"));
    }
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
        // wp28 creaking: the block at pos goes away (`what` = "player": a player breaks it).
        "break_block" => {
            let p = BlockPos::new(pos.x as i32, pos.y as i32, pos.z as i32);
            let source = (what == "player").then(|| {
                let pl = player.expect("a breaking player");
                DamageSource { kind: DamageKind::PlayerAttack, attacker: Some(pl.id), direct: Some(pl.id), pos: None, attacker_is_player: true }
            });
            level.destroy_heart(p, source);
        }
        // wp28 creaking: the player's game mode changes (`what`), or it moves to pos.
        "gamemode" => {
            let mut who = Vec::new();
            for p in level.players.iter_mut() {
                p.creative = what == "creative";
                p.spectator = what == "spectator";
                who.push((p.id, p.creative));
            }
            for (id, creative) in who {
                if let Some(e) = level.entity_mut(id) {
                    e.invulnerable = creative;
                }
            }
        }
        "move" => {
            let mut who = Vec::new();
            for p in level.players.iter_mut() {
                p.pos = pos;
                who.push(p.id);
            }
            for id in who {
                if let Some(e) = level.entity_mut(id) {
                    e.set_pos(pos);
                }
            }
        }
        // wp28 creaking: the player turns (yaw = pos.x, pitch = pos.y).
        "look" => {
            for p in level.players.iter_mut() {
                p.yaw = pos.x as f32;
                p.pitch = pos.y as f32;
            }
        }
        // wp30 llamas: mob `mob` is led by `amp` (an index into the mobs, -2 the player).
        "leash" => {
            let id = ids[a["mob"].as_u64().unwrap() as usize];
            let holder = match a["amp"].as_i64().unwrap() {
                -2 => player.expect("a leading player").id,
                i => ids[i as usize],
            };
            let m = mob::data_mut(level.entity_mut(id).unwrap()).unwrap();
            mob::kinds::llama::set_leash_holder(m, Some(holder));
        }
        "interact" => {
            let id = ids[a["mob"].as_u64().unwrap() as usize];
            let p = player.expect("an interacting player");
            let who = mob::interact::Interactor { id: p.id, creative: p.creative, sneaking: p.sneaking };
            let stack = kiln_item::ItemStack::of(what, 1).unwrap();
            // The harness puts the item in the player's hand, where it stays.
            for p in level.players.iter_mut() {
                p.main_hand = stack.item();
            }
            let e = level.entity_mut(id).unwrap();
            let mut e2 = std::mem::replace(e, kiln_entity::Entity::new("minecraft:marker", -5, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
            let out = mob::interact::interact(&mut e2, level, &who, &stack);
            *level.entity_mut(id).unwrap() = e2;
            // What the interaction did to the held item (a stack of one eaten or filled up).
            let held = match out.held {
                mob::interact::HeldChange::Consume(_) if !who.creative => Some(0),
                mob::interact::HeldChange::Fill(ref f) => Some(f.item()),
                _ => None,
            };
            if let Some(held) = held {
                for p in level.players.iter_mut() {
                    p.main_hand = held;
                }
            }
        }
        // wp32 parrots: the player stands on the ground (a parrot may land on its shoulder).
        "ground" => {
            for p in level.players.iter_mut() {
                p.parrot_can_sit = true;
            }
        }
        // wp32 parrots: a record plays near (or stops near) parrot `mob`.
        "record" => {
            let id = ids[a["mob"].as_u64().unwrap() as usize];
            let at = BlockPos::containing(pos.x, pos.y, pos.z);
            let m = mob::data_mut(level.entity_mut(id).unwrap()).unwrap();
            mob::kinds::parrot::set_record_playing_nearby(m, at, what == "play");
        }
        // wp28: `time set` in the middle of a scenario.
        "daytime" => level.day_time = pos.x as i64,
        // wp28 animals: an item entity (`duration` items, default 1) at rest at the position.
        "drop" => {
            let stack = kiln_item::ItemStack::of(what, a["duration"].as_i64().unwrap_or(0).max(1) as i32).unwrap();
            let id = level.next_entity_id();
            let seed = level.fresh_seed();
            let mut item = kiln_entity::item::new(id, 0, stack, seed);
            item.set_pos(pos);
            item.delta = Vec3::ZERO;
            item.set_old_pos_and_rot();
            level.add_entity(item);
        }
        // wp28 animals: a jukebox starts or stops playing; every allay within 10 blocks hears it.
        "jukebox" => {
            let at = BlockPos::containing(pos.x, pos.y, pos.z);
            let playing = what == "play";
            let allays: Vec<i32> = level.entities().filter(|e| mob::data(e).is_some_and(|m| m.kind == MobKind::Allay)).map(|e| e.id).collect();
            for id in allays {
                let e = level.entity_mut(id).unwrap();
                // The mob data out of the entity, as the mob tick has it.
                let mut k = std::mem::replace(&mut e.kind, EntityKind::MobTicking { gravity: 0.08 });
                if let EntityKind::Mob(m) = &mut k {
                    mob::kinds::allay::hear_jukebox(e, m, playing, at);
                }
                e.kind = k;
            }
        }
        // wp28: `Level.gameEvent(source, event, pos)`: `mob` is the source (an index into the
        // scenario's mobs, -1 nobody, -2 the player); wardens hear it.
        "gameevent" => {
            let event = kiln_entity::vibration::intern(what).expect("game event");
            let m = a["mob"].as_i64().unwrap();
            let source = if m == -2 {
                let p = player.expect("a player source");
                Some(kiln_entity::vibration::EventSource::player(p.id, p.uuid, p.pos, p.sneaking, p.spectator, p.creative))
            } else if m >= 0 {
                let e = level.entity(ids[m as usize]).unwrap();
                Some(kiln_entity::vibration::source_of(e, level))
            } else {
                None
            };
            level.game_event(event, pos, kiln_entity::vibration::Context { source, affected_state: None });
        }
        k => panic!("action {k}"),
    }
}

/// What the harness does to a mob that appeared (`MobVectors.Adopt.adopt`): its random is seeded from
/// the tick and its number, its head and body turn to its yaw (the pinned one in a `pin_yaw`
/// scenario: the constructor's is `Math.random()`'s, where both sides drew one Kiln takes the
/// recording's).
fn pin_fresh(level: &mut MemoryLevel, id: i32, n: i64, tick: i64, pin_yaw: bool, recorded: Option<f32>) {
    let e = level.entity_mut(id).unwrap();
    if let Some(y) = recorded
        && (0.0..6.2832).contains(&y)
        && (0.0..6.2832).contains(&e.y_rot)
    {
        e.y_rot = y;
        e.y_rot_o = y;
    }
    e.random = kiln_javamath::random::LegacyRandom::new(7777 * (tick + 1) + n);
    kiln_entity::mob::brain::pin(e);
    if pin_yaw {
        e.y_rot = 10.0 * (n + 1) as f32;
        e.y_rot_o = e.y_rot;
    }
    let yaw = e.y_rot;
    let m = mob::data_mut(e).unwrap();
    m.y_head_rot = yaw;
    m.y_head_rot_o = yaw;
    m.y_body_rot = yaw;
    m.y_body_rot_o = yaw;
    if let mob::Species::Chicken { egg_time } = &mut m.species {
        *egg_time = 6000 + n as i32;
    }
}

fn replay(s: &Value) -> Result<usize, String> {
    // Diverging (brain-driven) scenarios compare the body only: not the random or the goals.
    let loose = s.get("diverges").and_then(Value::as_bool) == Some(true);
    let pin_yaw = s.get("pin_yaw").and_then(Value::as_bool) == Some(true);
    // Recordings since wp29 pin a rider that appears during its vehicle's tick before it ticks.
    let early_pin = s.get("pin_passengers").and_then(Value::as_bool) == Some(true);
    let mut level = MemoryLevel::new(-64, s["level_seed"].as_i64().unwrap());
    level.share_ai_random = true;
    level.bottom_layer = Some(kiln_data::blocks::default_state::BEDROCK);
    level.sky_darken = s["sky_darken"].as_i64().unwrap() as i32;
    // wp28: the overworld clock (villagers' schedule).
    level.day_time = s.get("day_time").and_then(Value::as_i64).unwrap_or(1000);
    // The recording world is superflat.
    level.sea_level = -63;
    let start = s["game_time"].as_i64().unwrap();
    for b in s["blocks"].as_array().unwrap() {
        let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
        level.blocks.insert(p, b[3].as_u64().unwrap() as u16);
    }
    let mut player = s.get("player").filter(|p| !p.is_null()).map(|p| {
        let mut v = PlayerView::new(p["id"].as_i64().unwrap() as i32, vec3(&p["pos"]));
        v.sneaking = p["sneaking"].as_bool().unwrap_or(false);
        if v.sneaking {
            v.eye_height = 1.27;
        }
        v.creative = p.get("creative").and_then(Value::as_bool).unwrap_or(false);
        // The recording's player is never ticked: it never finds itself in water.
        v.in_water = Some(false);
        // (wp32: a parrot's owner is no spectator, flying or in powder snow; it does not stand on
        // the ground (with a free shoulder) until the scenario says so.)
        v.parrot_may_land = true;
        if let Some(item) = p.get("main_hand").and_then(Value::as_str) {
            v.main_hand = kiln_data::builtin_id("minecraft:item", item).unwrap();
        }
        if let Some(item) = p.get("head").and_then(Value::as_str) {
            v.head = kiln_data::builtin_id("minecraft:item", item).unwrap();
            // (wp28: `PiglinAi.isWearingSafeArmor` reads the armor tag.)
            v.piglin_safe_armor = mob::item_tag(v.head, "minecraft:piglin_safe_armor");
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
        // (wp28: a uuid of its own, for the memories that hold entities by uuid: anger.)
        let mut e = mob::new(kind, id, id as u128 + 0x5eed_0000, 0);
        let yaw = f(&spec["yaw"]) as f32;
        e.set_pos(vec3(&spec["pos"]));
        e.y_rot = yaw;
        e.set_old_pos_and_rot();
        e.random = kiln_javamath::random::LegacyRandom::new(spec["seed"].as_i64().unwrap());
        {
            let m = mob::data_mut(&mut e).unwrap();
            // The harness calls `setYHeadRot` (types like goats clamp it to their body, which is at 0
            // then) before it sets the body.
            m.y_head_rot = m.kind.ext().map_or(yaw, |k| k.set_head_rot(0.0, yaw));
            m.y_body_rot = yaw;
            m.y_head_rot_o = yaw;
            m.y_body_rot_o = yaw;
            if let mob::Species::Chicken { egg_time } = &mut m.species {
                *egg_time = spec["egg_time"].as_i64().unwrap() as i32;
            }
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
        // (wp28: the harness sets the love time after it read the NBT.)
        if let Some(m) = mob::data_mut(&mut e) {
            m.in_love = spec.get("in_love").and_then(Value::as_i64).unwrap_or(0) as i32;
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
        // The harness seeds the mob's random after the NBT and the age were read (the reads may draw
        // from it, a villager's brain being rebuilt).
        e.random = kiln_javamath::random::LegacyRandom::new(spec["seed"].as_i64().unwrap());
        // A brain mob's random for what vanilla draws from the level's: the recording's level
        // random, seeded per scenario; the sensors and gates pinned as `MobVectors.pinBrain` does.
        if let Some(m) = mob::data_mut(&mut e) {
            m.brain_random = kiln_javamath::random::LegacyRandom::new(s["level_seed"].as_i64().unwrap());
        }
        kiln_entity::mob::brain::pin(&mut e);
        ids.push(id);
        level.insert(e);
        // wp28 creaking: the heart holding this creaking (`setCreakingInfo(creaking)`).
        if let Some(h) = spec.get("heart").filter(|v| !v.is_null()) {
            let p = BlockPos::new(h[0].as_i64().unwrap() as i32, h[1].as_i64().unwrap() as i32, h[2].as_i64().unwrap() as i32);
            let mut be = kiln_entity::mob::kinds::creaking_heart::HeartBe::default();
            be.set_creaking(id, 0);
            level.hearts.insert(p, be);
        }
        for fx in spec.get("effects").and_then(Value::as_array).into_iter().flatten() {
            let fx = kiln_entity::effect::Effect::named(fx[0].as_str().unwrap(), fx[1].as_i64().unwrap() as i32, fx[2].as_i64().unwrap() as i32).unwrap();
            level.add_effect_instance(id, fx, None);
        }
    }
    // wp29: riders sit on their mounts (`startRiding`) before the first tick.
    for (i, spec) in s["mobs"].as_array().unwrap().iter().enumerate() {
        let Some(v) = spec.get("vehicle").and_then(Value::as_i64).filter(|&v| v >= 0) else { continue };
        let marker = kiln_entity::Entity::new("minecraft:marker", -5, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0);
        let mut vehicle = std::mem::replace(level.entity_mut(ids[v as usize]).unwrap(), marker);
        assert!(kiln_entity::ride::start_riding(level.entity_mut(ids[i]).unwrap(), &mut vehicle, false), "could not ride");
        *level.entity_mut(ids[v as usize]).unwrap() = vehicle;
    }
    // wp28 creaking: hearts without a creaking yet (the ones with one were made above), and the
    // night attribute.
    level.creaking_active = s.get("creaking_active").and_then(Value::as_bool).unwrap_or(false);
    for h in s.get("hearts").and_then(Value::as_array).into_iter().flatten() {
        let p = BlockPos::new(h[0].as_i64().unwrap() as i32, h[1].as_i64().unwrap() as i32, h[2].as_i64().unwrap() as i32);
        level.hearts.entry(p).or_default();
    }
    // Other entities (end crystals), after the mobs: ticked, not traced.
    let mut other_ids = Vec::new();
    for o in s.get("others").and_then(Value::as_array).into_iter().flatten() {
        let id = o["id"].as_i64().unwrap() as i32;
        let mut fields = vec![
            ("id".into(), kiln_proto::nbt::Tag::String(o["type"].as_str().unwrap().to_owned())),
            ("Pos".into(), kiln_proto::nbt::Tag::List((0..3).map(|i| kiln_proto::nbt::Tag::Double(f(&o["pos"][i]))).collect())),
            ("Rotation".into(), kiln_proto::nbt::Tag::List(vec![kiln_proto::nbt::Tag::Float(f(&o["yaw"]) as f32), kiln_proto::nbt::Tag::Float(0.0)])),
        ];
        // (wp28: an item entity's stack.)
        if let Some(item) = o.get("item").and_then(Value::as_str) {
            fields.push((
                "Item".into(),
                kiln_proto::nbt::Tag::Compound(vec![("id".into(), kiln_proto::nbt::Tag::String(item.to_owned())), ("count".into(), kiln_proto::nbt::Tag::Int(1))]),
            ));
        }
        let tag = kiln_proto::nbt::Tag::Compound(fields);
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
    level.set_next_entity_id(s.get("next_id").and_then(Value::as_i64).map_or_else(|| ids.iter().chain(&other_ids).copied().max().unwrap_or(0) + 1, |v| v as i32));
    level.immediate_adds = true;
    let mut known = level.len();
    let mut compared = 0;
    let window = s.get("compare_ticks").and_then(Value::as_u64).filter(|&n| n > 0).map_or(usize::MAX, |n| n as usize);
    for (tick, expected) in trace.iter().enumerate().take(window) {
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
                // The player may have moved or changed game mode.
                if player.is_some()
                    && let Some(p) = level.players.first()
                {
                    player = Some(*p);
                }
            }
        }
        let before = level.player_hits.len();
        let nearest = player.filter(|p| !p.spectator);
        // The yaw a mob gets from its constructor is `Math.random() * 2 pi` (unseeded): where both
        // sides drew one, Kiln takes the recording's.
        let mut recorded_yaws = s
            .get("spawned")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|sp| sp["tick"].as_i64() == Some(tick) && sp["mob"].as_bool() == Some(true))
            .map(|sp| f(&sp["yaw"]) as f32);
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
                // Riders are ticked with their vehicle (`ServerLevel.tickPassenger`).
                if e.vehicle.is_some() {
                    return;
                }
                e.common_tick();
                e.tick(level);
            });
            // The riders tick right after their vehicle (`ServerLevel.tickPassenger`), with the
            // vehicle in the level as the simulation has it: a copy of it goes to the rider,
            // and what the rider steered it by goes back.
            let Some(v) = level.entity_at(i).filter(|e| !e.is_removed() && e.vehicle.is_none()) else { continue };
            let (vid, riders) = (v.id, v.passengers.clone());
            for pid in riders {
                // (`ServerLevel.tickPassenger` ticks a rider that appeared meanwhile, the skeleton
                // of a trap horse, in this very tick: the harness pins it first.)
                if early_pin && !ids.contains(&pid) {
                    let n = (ids.len() - initial) as i64;
                    pin_fresh(&mut level, pid, n, tick, pin_yaw, recorded_yaws.next());
                    ids.push(pid);
                }
                let Some(mut vehicle) = level.entity(vid).cloned() else { break };
                let Some(slot) = level.entity_mut(pid) else { continue };
                let mut p = std::mem::replace(slot, kiln_entity::Entity::new("minecraft:marker", -5, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
                if p.vehicle == Some(vid) && !p.is_removed() {
                    p.common_tick();
                    if kiln_entity::ride::ride_tick(&mut p, &mut level, &mut vehicle)
                        && let Some(real) = level.entity_mut(vid)
                    {
                        kiln_entity::ride::copy_steering_back(&vehicle, real);
                    }
                }
                *level.entity_mut(pid).unwrap() = p;
            }
        }
        // `Level.tickBlockEntities`: the creaking hearts, after the entities.
        level.tick_hearts();
        let before_flush = known;
        level.flush_spawned();
        known = level.len();
        // wp28: what a breeze shot flies as the recording's did (vanilla draws the shot's spread
        // from the projectile's own random, seeded from the clock, which cannot be pinned).
        {
            // (wp29: skeletons' arrows likewise; wp30: llama spit, and where it starts must be
            // where vanilla's did.)
            for shot in ["minecraft:breeze_wind_charge", "minecraft:arrow", "minecraft:llama_spit"] {
                let recorded: Vec<&Value> = s["spawned"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|x| x["tick"].as_i64() == Some(tick) && x["type"].as_str() == Some(shot))
                    .collect();
                let mut charges: Vec<i32> = (before_flush..level.len()).filter_map(|i| level.entity_at(i)).filter(|e| e.type_name == shot).map(|e| e.id).collect();
                if shot == "minecraft:llama_spit" && charges.len() != recorded.len() {
                    return Err(format!("tick {tick}: {} {shot} (kiln) vs {} (vanilla)", charges.len(), recorded.len()));
                }
                // (Shots of one tick are matched by where they started: the harness finds them in
                // the order of its entity sections.)
                let at = |id: i32| level.entity(id).unwrap().position();
                if shot != "minecraft:llama_spit" {
                    charges.sort_by(|&a, &b| {
                        let key = |id: i32| recorded.iter().position(|r| vec3(&r["pos"]).distance_to_sqr(at(id)) < 1.0e-12).unwrap_or(usize::MAX);
                        key(a).cmp(&key(b))
                    });
                }
                for (k, (id, rec)) in charges.iter().zip(recorded).enumerate() {
                    let e = level.entity_mut(*id).unwrap();
                    if shot == "minecraft:llama_spit" {
                        let want = vec3(&rec["pos"]);
                        let got = e.position();
                        if [got.x, got.y, got.z].map(f64::to_bits) != [want.x, want.y, want.z].map(f64::to_bits) {
                            return Err(format!("tick {tick}: {shot} starts at {got:?} (kiln) vs {want:?} (vanilla)"));
                        }
                    }
                    e.delta = vec3(&rec["motion"]);
                    // An arrow's damage and crit were drawn from its random at the shot, which is
                    // the clock's; its random is pinned for the hit (`MobVectors.Adopt`).
                    // (Vectors recorded before wp29 do not carry them.)
                    if let EntityKind::Arrow(a) = &mut e.kind
                        && rec.get("base_damage").is_some_and(|v| !v.is_null())
                    {
                        a.base_damage = f(&rec["base_damage"]);
                        a.crit = rec["crit"].as_bool().unwrap_or(false);
                        e.random = kiln_javamath::random::LegacyRandom::new(5555 * (tick + 1) + k as i64);
                    }
                }
            }
        }
        // Mobs that appeared get the harness's pinned random and head/body yaw, in the order the
        // harness finds them (`getEntities` over its box: entity sections, then insertion).
        let fresh: Vec<i32> = (before_flush..level.len()).filter_map(|i| level.entity_at(i)).filter(|e| mob::data(e).is_some()).map(|e| e.id).filter(|id| !ids.contains(id)).collect();
        let harness_box = kiln_entity::math::Aabb::new(-60.0, 60.0, -60.0, 60.0, 140.0, 60.0);
        let order = level.entities_in(&harness_box, kiln_entity::EntityFilter::Any, i32::MIN);
        let mut fresh = fresh;
        fresh.sort_by_key(|id| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
        for id in fresh {
            let n = (ids.len() - initial) as i64;
            pin_fresh(&mut level, id, n, tick, pin_yaw, recorded_yaws.next());
            ids.push(id);
        }
        // Explosions hurt the player through events (it is not an entity of the harness).
        for ev in std::mem::take(&mut level.events) {
            if std::env::var_os("KILN_MOB_DEBUG").is_some()
                && let kiln_entity::level::Event::Explosion { pos, power, blocks, .. } = &ev
            {
                eprintln!("dbg tick {tick} explosion at {pos:?} power {power} ({} positions)", blocks.len());
            }
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
            eprintln!("dbg tick {tick} pos {:?} delta {:?} ground {} brain {:?} rnd {} ambient {} noaction {} goals {:?} path {:?}", e.position(), e.delta, e.on_ground, m.brain_trace(), e.random.state(), m.ambient_sound_time, m.no_action_time, m.running_goals(), m.nav.path.as_ref().map(|p| (p.next, p.target, p.nodes.iter().map(|n| (n.x, n.y, n.z)).collect::<Vec<_>>())));
        }
        if std::env::var_os("KILN_MOB_DEBUG_ENTS").is_some() {
            for i in 0..level.len() {
                if let Some(e) = level.entity_at(i).filter(|e| matches!(e.kind, EntityKind::Arrow(_))) {
                    eprintln!("ENT tick={tick} {} id={} removed={} pos={:?} delta={:?}", e.type_name, e.id, e.is_removed(), e.position(), e.delta);
                }
            }
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
    // wp32 parrots: what the scenario drew from the level's random.
    if let Some(want) = s.get("level_random").and_then(Value::as_i64)
        && level.random_state() != want
    {
        return Err(format!("level random {} (kiln) vs {want} (vanilla)", level.random_state()));
    }
    // wp28 creaking: the blocks around the hearts (resin) are the same.
    for b in s.get("end_blocks").and_then(Value::as_array).into_iter().flatten() {
        let p = BlockPos::new(b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32);
        let (got, want) = (level.block(p), b[3].as_u64().unwrap() as u16);
        if got != want {
            return Err(format!("end block {p:?}: {} (kiln) vs {} (vanilla)", kiln_data::blocks_types::block_of(got).name, kiln_data::blocks_types::block_of(want).name));
        }
    }
    // Vanilla arrows draw their damage and spread from their own random, which is seeded from
    // the clock (not pinnable): skeleton scenarios compare the mob, not where arrows land.
    // Shulker bullets likewise steer by their own random.
    // (The skeleton trap's horsemen shoot too.)
    let arrows = s["name"].as_str().is_some_and(|n| n.contains("skeleton_trap")) || s["mobs"].as_array().unwrap().iter().any(|m| matches!(m["main_hand"].as_str(), Some("minecraft:bow" | "minecraft:trident" | "minecraft:crossbow")) || matches!(m["type"].as_str(), Some("minecraft:shulker" | "minecraft:witch")));
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
