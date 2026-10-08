//! Replays the vanilla effect, fire and air vectors of `tools/EffectVectors.java`
//! (`KILN_EFFECT_VECTORS`, written by `tools/effect_vectors.py`) through the simulation: each
//! scenario's blocks are placed, the player is set up as recorded, and every tick (with the
//! recorded actions before it) must leave health, absorption, food, fire ticks, the on-fire
//! flag, air, the hurt cooldown, the active effects with their hidden ones, attribute values,
//! the destroy speed and the effect and entity event packets exactly as vanilla had them.
//! The first line's registry dump checks Kiln's effect and potion tables. Skipped when the
//! vectors are not there.

use crate::combat_parity::{enchant, vanilla_loot};
use crate::effects::{self, Effect};
use crate::health::{Cause, DamageCtx};
use crate::testing::{Client, SinkStats, join};
use crate::{Sim, SimConfig};
use kiln_entity::math::BlockPos;
use kiln_item::ItemStack;
use kiln_link::{PlayIn, ToSim};
use serde_json::Value;
use std::collections::HashMap;

fn f32_of(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

fn i32_of(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}

/// Checks the effect and potion tables against the registry dump.
fn check_registries(line: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    let table = effects::effect_table();
    let effects = line["effects"].as_array().unwrap();
    if effects.len() != table.len() {
        errors.push(format!("{} effects in vanilla, {} in Kiln", effects.len(), table.len()));
    }
    for (e, t) in effects.iter().zip(table) {
        let name = e["id"].as_str().unwrap();
        let mut eq = |what: &str, got: String, want: String| {
            if got != want {
                errors.push(format!("{name}.{what}: kiln {got}, vanilla {want}"));
            }
        };
        eq("name", t.name.to_owned(), name.to_owned());
        eq("id", format!("{:?}", effects::effect_id(name)), format!("{:?}", e["raw"].as_i64().map(|v| v as i32)));
        eq("color", t.color.to_string(), e["color"].to_string());
        eq("instantaneous", t.kind.instantaneous().to_string(), e["instantaneous"].to_string());
        let kind = match t.kind {
            effects::Kind::Plain => "MobEffect",
            effects::Kind::Regeneration => "RegenerationMobEffect",
            effects::Kind::Poison => "PoisonMobEffect",
            effects::Kind::Wither => "WitherMobEffect",
            effects::Kind::Hunger => "HungerMobEffect",
            effects::Kind::Saturation => "SaturationMobEffect",
            effects::Kind::Absorption => "AbsorptionMobEffect",
            effects::Kind::HealOrHarm { .. } => "HealOrHarmMobEffect",
            effects::Kind::BadOmen => "BadOmenMobEffect",
            effects::Kind::RaidOmen => "RaidOmenMobEffect",
            effects::Kind::Infested => "InfestedMobEffect",
            effects::Kind::Oozing => "OozingMobEffect",
            effects::Kind::Weaving => "WeavingMobEffect",
            effects::Kind::WindCharged => "WindChargedMobEffect",
        };
        let class = e["class"].as_str().unwrap();
        eq("class", kind.to_owned(), class.to_owned());
        let mods: Vec<String> = e["modifiers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| format!("{} {} {:?} {}", m[0].as_str().unwrap(), m[1].as_str().unwrap(), m[2].as_f64().unwrap(), m[3].as_str().unwrap()))
            .collect();
        let op = |o: kiln_item::component::AttributeOperation| match o {
            kiln_item::component::AttributeOperation::AddValue => "ADD_VALUE",
            kiln_item::component::AttributeOperation::AddMultipliedBase => "ADD_MULTIPLIED_BASE",
            kiln_item::component::AttributeOperation::AddMultipliedTotal => "ADD_MULTIPLIED_TOTAL",
        };
        let got: Vec<String> =
            t.modifier.iter().map(|m| format!("{} {} {:?} {}", m.attr, m.id, m.amount, op(m.op))).collect();
        eq("modifiers", format!("{got:?}"), format!("{mods:?}"));
    }
    let potions = line["potions"].as_array().unwrap();
    let table = effects::potion_table();
    if potions.len() != table.len() {
        errors.push(format!("{} potions in vanilla, {} in Kiln", potions.len(), table.len()));
    }
    for (p, (name, list)) in potions.iter().zip(table) {
        let want: Vec<String> = p["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("{} {} {} {} {} {}", e[0].as_str().unwrap(), e[1], e[2], e[3], e[4], e[5]))
            .collect();
        let got: Vec<String> = list.iter().map(|(e, d, a)| format!("minecraft:{e} {d} {a} false true true")).collect();
        let id = format!("minecraft:{name}");
        if id != p["id"].as_str().unwrap() || got != want {
            errors.push(format!("potion {id}: kiln {got:?}, vanilla {} {want:?}", p["id"]));
        }
    }
    errors
}

fn stack(name: &str) -> ItemStack {
    ItemStack::of(name, 1).unwrap_or_else(|| panic!("unknown item {name}"))
}

/// The item a `hold` action puts in the main hand.
fn held(a: &Value) -> ItemStack {
    let mut s = stack(a["item"].as_str().unwrap());
    if let Some(potion) = a["potion"].as_str() {
        let id = kiln_item::registry::POTION.id(potion).unwrap_or_else(|| panic!("unknown potion {potion}"));
        s.insert(kiln_item::keys::POTION_CONTENTS, kiln_item::component::PotionContents { potion: Some(id), ..Default::default() });
    }
    if let Some(kv) = a["stew"].as_array() {
        let entries = kv
            .chunks(2)
            .map(|c| kiln_item::component::StewEffect {
                effect: effects::effect_id(c[0].as_str().unwrap()).unwrap(),
                duration: i32_of(&c[1]),
            })
            .collect();
        s.insert(kiln_item::keys::SUSPICIOUS_STEW_EFFECTS, kiln_item::component::SuspiciousStewEffects(entries));
    }
    s
}

/// The effects as the vectors print them.
fn effect_json(e: &Effect) -> Value {
    serde_json::json!({
        "id": kiln_item::registry::MOB_EFFECT.name(e.id).unwrap(),
        "amp": e.amplifier,
        "duration": e.duration,
        "ambient": e.ambient,
        "visible": e.visible,
        "icon": e.show_icon,
        "hidden": e.hidden.as_deref().map(effect_json),
    })
}

/// Packets of these ids the player got since the last call.
fn take_packets(stats: &SinkStats, ids: &[i32]) -> Vec<bytes::Bytes> {
    let mut log = stats.log.lock().unwrap();
    let all = std::mem::take(log.as_mut().unwrap());
    all.into_iter().filter(|p| kiln_proto::codec::Reader::new(p).varint().ok().is_some_and(|id| ids.contains(&id))).collect()
}

/// Sets attribute base values the way `/attribute base set` keeps them (`[[id, value], ..]`).
fn set_attributes(p: &mut crate::Player, attrs: &Value) {
    for a in attrs.as_array().map(Vec::as_slice).unwrap_or_default() {
        let name = a[0].as_str().unwrap();
        let attr = crate::command_data::PLAYER_ATTRIBUTES
            .iter()
            .find(|x| x.name() == name)
            .unwrap_or_else(|| panic!("attribute {name}"));
        let c = &mut p.command_attributes;
        c.bases.retain(|(n, _)| *n != attr.name());
        c.bases.push((attr.name(), a[1].as_f64().unwrap()));
    }
}

fn run_scenario(line: &Value) -> Vec<String> {
    use kiln_data::packets::play::clientbound as ids;
    let mut sim = Sim::new(SimConfig::new(2, 2, None));
    let (msg, stats) = join(1, "Effects", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats.clone());
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let mut console = vec![ToSim::Console(format!("difficulty {}", line["difficulty"].as_str().unwrap()))];
    for b in line["blocks"].as_array().unwrap() {
        console.push(ToSim::Console(format!("setblock {} {} {} {}", b[0], b[1], b[2], b[3].as_str().unwrap())));
    }
    assert!(sim.step(console));
    let pos: Vec<f64> = line["pos"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let base = pos.iter().map(|c| c.floor() as i32).collect::<Vec<_>>();
    // The blocks around the player, for effects that read the world outside the tick.
    let snapshot = |sim: &Sim| {
        let mut m = HashMap::new();
        for x in -3..=3 {
            for y in (base[1].min(100) - 8)..=(base[1] + 6) {
                for z in -3..=3 {
                    let p = [base[0] + x, y, base[2] + z];
                    if let Some(s) = sim.block_at(p[0], p[1], p[2]) {
                        m.insert(p, s);
                    }
                }
            }
        }
        m
    };
    let mut blocks = snapshot(&sim);
    let rules = sim.damage_rules();
    let seed = line["seed"].as_i64().unwrap();
    {
        let p = sim.players.get_mut(&1).unwrap();
        p.pos = [pos[0], pos[1], pos[2]];
        // The join teleport is long confirmed on a real connection; the client moves from here.
        p.awaiting_teleport = None;
        p.first_good = p.pos;
        p.rot = [0.0, 0.0];
        p.on_ground = line["on_ground"].as_bool().unwrap();
        p.sneaking = line["sneaking"].as_bool().unwrap();
        p.fall_distance = 0.0;
        p.main_supporting_block = None;
        // The body of vanilla's fresh mock player has not moved yet (this one stood on the ground).
        p.server_delta = [0.0; 3];
        p.on_ground_no_blocks = false;
        p.was_touching_water = false;
        p.starting_to_fall = None;
        p.ticks_frozen = 0;
        p.is_in_powder_snow = false;
        p.frost_speed = None;
        p.game_mode = match line["game_mode"].as_str().unwrap() {
            "creative" => 1,
            "adventure" => 2,
            "spectator" => 3,
            _ => 0,
        };
        p.loot = vanilla_loot();
        p.inv = kiln_inventory::PlayerInventory::new();
        if let Some(item) = line["main_hand"].as_str() {
            p.inv.items[0] = stack(item);
        }
        let armor = line["armor"].as_array().unwrap();
        for i in 0..4 {
            if let Some(item) = armor[i].as_str() {
                let mut s = stack(item);
                enchant(&mut s, &line["armor_enchantments"][i]);
                p.inv.equipment[i] = s;
            }
        }
        for (i, slot) in crate::combat::SLOTS.iter().enumerate() {
            p.equipment_seen[i] = p.inv.equipped(*slot).clone();
        }
        p.effects.clear();
        p.health = f32_of(&line["health"]);
        p.absorption = f32_of(&line["absorption"]);
        p.hurt_cooldown = 0;
        p.last_hurt = 0.0;
        p.food = i32_of(&line["food"]);
        p.saturation = f32_of(&line["saturation"]);
        p.exhaustion = 0.0;
        p.food_timer = 0;
        p.air = i32_of(&line["air"]);
        p.set_fire_ticks(i32_of(&line["fire"]));
        p.tick_count = 0;
        p.entity_rng = kiln_javamath::random::LegacyRandom::new(seed.wrapping_add(1));
        p.level_rng = kiln_javamath::random::LegacyRandom::new(seed);
        // Attribute base values the scenario set (the way `/attribute base set` keeps them).
        set_attributes(p, &line["attrs"]);
    }
    let client = line["client"].as_bool().unwrap_or(false);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let entity_id = sim.players[&1].entity_id;
    let stone = kiln_data::blocks_types::block_by_name("minecraft:stone").unwrap().default;
    let mut errors = Vec::new();
    let result = line["result"].as_array().unwrap();
    for (i, want) in result.iter().enumerate() {
        let t = i + 1;
        let actions = line["actions"].get(t.to_string()).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut inbox = Vec::new();
        let mut finished = false;
        for a in &actions {
            let (mut spawns, mut deaths) = (Vec::new(), Vec::new());
            let game_time = sim.game_time;
            let reader = blocks.clone();
            let block = move |p: BlockPos| reader.get(&[p.x, p.y, p.z]).copied().unwrap_or(0);
            let p = sim.players.get_mut(&1).unwrap();
            let mut ctx = DamageCtx { rules, game_time, spawns: &mut spawns, deaths: &mut deaths, level_rng: None };
            let id = |a: &Value| effects::effect_id(a["id"].as_str().unwrap()).unwrap();
            match a["op"].as_str().unwrap() {
                "effect" => {
                    p.add_effect(Effect::new(
                        id(a),
                        i32_of(&a["duration"]),
                        i32_of(&a["amp"]),
                        a["ambient"].as_bool().unwrap(),
                        a["visible"].as_bool().unwrap(),
                        a["icon"].as_bool().unwrap(),
                    ));
                }
                "remove" => {
                    p.remove_effect(id(a));
                }
                "clear" => {
                    p.remove_all_effects();
                }
                "hurt" => {
                    let cause = Cause::Other(crate::health::static_damage_type(a["type"].as_str().unwrap()));
                    p.hurt(f32_of(&a["amount"]), &cause.into(), &mut ctx);
                }
                "hold" => {
                    p.inv.items[0] = held(a);
                    p.inv.times_changed += 1;
                }
                "offhand" => {
                    let index = kiln_inventory::inventory::equipment_index(kiln_item::component::EquipmentSlot::OffHand, p.inv.selected);
                    *kiln_inventory::Container::item_mut(&mut p.inv, index) = stack(a["item"].as_str().unwrap());
                    p.inv.times_changed += 1;
                }
                "finish" => {
                    finished = true;
                    p.finish_using(false, &block, &mut ctx);
                }
                "use" => p.use_item(false, &block, &mut ctx),
                // (`jump` is the shadow client's: its moves arrive as packets.)
                "jump" | "velocity" => {}
                "attribute" => set_attributes(p, &serde_json::json!([[a["id"], a["value"]]])),
                "sneak" => p.sneaking = a["on"].as_bool().unwrap(),
                "gamerule" => inbox.push(ToSim::Console(format!("gamerule {} {}", a["name"].as_str().unwrap(), a["value"]))),
                "setblock" => {
                    let at = a["pos"].as_array().unwrap();
                    inbox.push(ToSim::Console(format!("setblock {} {} {} {}", at[0], at[1], at[2], a["state"].as_str().unwrap())));
                }
                other => panic!("unknown op {other}"),
            }
        }
        if client {
            // What the shadow client sent before this tick (`LocalPlayer.sendPosition`).
            let mv = &line["moves"][i];
            let pos = mv["pos"].as_array().map(|c| [c[0].as_f64().unwrap(), c[1].as_f64().unwrap(), c[2].as_f64().unwrap()]);
            inbox.push(ToSim::Packet(
                1,
                PlayIn::Move {
                    pos,
                    rot: None,
                    on_ground: mv["on_ground"].as_bool().unwrap(),
                    horizontal_collision: mv["hcol"].as_bool().unwrap(),
                },
            ));
            inbox.push(ToSim::Packet(1, PlayIn::ClientTickEnd));
        }
        assert!(sim.step(inbox));
        if actions.iter().any(|a| a["op"] == "setblock") {
            blocks = snapshot(&sim);
        }
        let reader = blocks.clone();
        let block = move |p: BlockPos| reader.get(&[p.x, p.y, p.z]).copied().unwrap_or(0);
        // Vanilla moves a player it gets no movement from (levitation lifts it off the
        // ground); a client would report that, so the destroy speed uses vanilla's on_ground.
        let speed = {
            let p = sim.players.get_mut(&1).unwrap();
            let eye_in_water = p.fluids(&block).eye_in_water;
            let on_ground = std::mem::replace(&mut p.on_ground, want["on_ground"].as_bool().unwrap());
            let speed = p.destroy_speed(stone, eye_in_water);
            p.on_ground = on_ground;
            speed
        };
        let p = &sim.players[&1];
        let mut eq = |what: &str, got: String, expected: String| {
            if got != expected {
                errors.push(format!("tick {t} {what}: kiln {got}, vanilla {expected}"));
            }
        };
        eq("health", format!("{:?}", p.health), format!("{:?}", f32_of(&want["health"])));
        eq("absorption", format!("{:?}", p.absorption), format!("{:?}", f32_of(&want["absorption"])));
        eq("food", p.food.to_string(), want["food"].to_string());
        eq("saturation", format!("{:?}", p.saturation), format!("{:?}", f32_of(&want["saturation"])));
        eq("exhaustion", format!("{:?}", p.exhaustion), format!("{:?}", f32_of(&want["exhaustion"])));
        eq("fire", p.fire_ticks.to_string(), want["fire"].to_string());
        eq("on_fire", p.on_fire_flag.to_string(), want["on_fire"].to_string());
        eq("air", p.air.to_string(), want["air"].to_string());
        eq("hurt_cooldown", p.hurt_cooldown.to_string(), want["hurt_cooldown"].to_string());
        let dead = p.dead || p.health <= 0.0;
        eq("dead", dead.to_string(), want["dead"].to_string());
        let got_effects: Vec<Value> = p.effects.values().map(effect_json).collect();
        eq("effects", Value::Array(got_effects).to_string(), want["effects"].to_string());
        for (name, value) in want["attributes"].as_object().unwrap() {
            let attr = [
                crate::combat::MOVEMENT_SPEED,
                crate::combat::ATTACK_DAMAGE,
                crate::combat::ATTACK_SPEED,
                crate::combat::MAX_HEALTH,
                crate::combat::MAX_ABSORPTION,
                crate::combat::LUCK,
                crate::combat::SAFE_FALL_DISTANCE,
                crate::combat::OXYGEN_BONUS,
                crate::combat::BURNING_TIME,
                crate::combat::WAYPOINT_TRANSMIT_RANGE,
            ]
            .into_iter()
            .find(|a| a.name == name.as_str())
            .unwrap_or_else(|| panic!("attribute {name}"));
            eq(name, format!("{:?}", p.attribute(attr)), format!("{:?}", value.as_f64().unwrap()));
        }
        eq("destroy_speed", format!("{speed:?}"), format!("{:?}", f32_of(&want["destroy_speed"])));
        if let Some(fz) = want["frozen"].as_i64() {
            eq("frozen", p.ticks_frozen.to_string(), fz.to_string());
        }
        if let Some(fd) = want["fall_distance"].as_f64() {
            eq("fall_distance", format!("{:?}", p.fall_distance), format!("{fd:?}"));
        }
        let got_packets: Vec<bytes::Bytes> =
            take_packets(&stats, &[ids::UPDATE_MOB_EFFECT, ids::REMOVE_MOB_EFFECT, ids::ENTITY_EVENT])
                .into_iter()
                .filter(|pkt| {
                    let mut r = kiln_proto::codec::Reader::new(pkt);
                    let id = r.varint().unwrap();
                    let eid = if id == ids::ENTITY_EVENT { r.i32().unwrap() } else { r.varint().unwrap() };
                    // `finish` completes the use directly, without ServerPlayer's "use complete"
                    // event.
                    let use_complete = id == ids::ENTITY_EVENT && r.u8().unwrap() == 9;
                    eid == entity_id && !(finished && use_complete)
                })
                .collect();
        let want_packets: Vec<bytes::Bytes> = want["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| match w["t"].as_str().unwrap() {
                "effect" => {
                    let e = kiln_proto::packets::entity::MobEffect {
                        effect: effects::effect_id(w["id"].as_str().unwrap()).unwrap(),
                        amplifier: i32_of(&w["amp"]),
                        duration: i32_of(&w["duration"]),
                        flags: i32_of(&w["flags"]) as u8,
                    };
                    kiln_proto::packets::entity::update_mob_effect(entity_id, &e)
                }
                "remove" => kiln_proto::packets::entity::remove_mob_effect(entity_id, effects::effect_id(w["id"].as_str().unwrap()).unwrap()),
                _ => kiln_proto::packets::entity::entity_event(entity_id, i32_of(&w["event"]) as u8),
            })
            .collect();
        // `removeAllEffects` goes through a hash map copy in identity-hash order: compare runs
        // of removals as sets.
        let (got_packets, want_packets) = (sort_removal_runs(got_packets), sort_removal_runs(want_packets));
        eq("packets", format!("{got_packets:?}"), format!("{want_packets:?}"));
        if want["dead"].as_bool().unwrap() || errors.len() > 12 {
            break;
        }
    }
    errors
}

fn sort_removal_runs(mut packets: Vec<bytes::Bytes>) -> Vec<bytes::Bytes> {
    let is_removal = |p: &bytes::Bytes| {
        kiln_proto::codec::Reader::new(p).varint().ok() == Some(kiln_data::packets::play::clientbound::REMOVE_MOB_EFFECT)
    };
    let mut i = 0;
    while i < packets.len() {
        let start = i;
        while i < packets.len() && is_removal(&packets[i]) {
            i += 1;
        }
        packets[start..i].sort();
        i = i.max(start + 1);
    }
    packets
}

/// Scenarios Kiln does not match yet, with what differs (wp45: kept in the vectors so that a fix
/// shows, and so that nothing else slips in: the test fails on any other difference, and on a
/// listed scenario that now passes).
const KNOWN_GAPS: &[(&str, &str)] = &[
    ("fall_powder_snow_4", "entering powder snow in a fall freezes one tick more than vanilla (two steps of the tick's path)"),
    ("fall_powder_snow_40", "as fall_powder_snow_4"),
    ("haz_snow_lava_clears", "a burning player in powder snow next to lava: the fire is put out one tick early"),
    ("fall_bubble_6", "bubble column: the drag changes the exhaustion of the first ticks of the rise"),
    ("fall_bubble_10", "as fall_bubble_6"),
    ("fall_bubble_20", "as fall_bubble_6"),
    ("fall_bubble_40", "as fall_bubble_6"),
    ("fall_bed_bounce_12", "the jump exhaustion after a bed's bounce"),
    ("haz_wall_head_only", "only the head in a block: vanilla stops suffocating after the first hit (the body moves out), Kiln keeps the player in place"),
    ("haz_wall_head_only_sneaking", "as haz_wall_head_only"),
    ("haz_wall_placed_over_player", "as haz_wall_head_only"),
    ("haz_wall_ceiling_slab_top", "a low ceiling forces the crouching pose (and with it the locator bar attribute); poses are not tracked"),
];

#[test]
fn effect_parity() {
    let Some(path) = std::env::var_os("KILN_EFFECT_VECTORS") else {
        eprintln!("skipped: set KILN_EFFECT_VECTORS (tools/effect_vectors.py)");
        return;
    };
    // Several name parts may be given, separated by commas (a scenario matching any runs).
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let exclude = std::env::var("KILN_PARITY_EXCLUDE").ok();
    let matches = |name: &str| {
        filter.as_ref().is_none_or(|f| f.split(',').any(|part| name.contains(part)))
            && !exclude.as_ref().is_some_and(|x| x.split(',').any(|part| name.contains(part)))
    };
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    let mut seen = Vec::new();
    let mut ticks = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        if name == "@registries" {
            let errors = check_registries(&v);
            assert!(errors.is_empty(), "registries differ:\n  {}", errors.join("\n  "));
            println!("ok   registries");
            continue;
        }
        if !matches(&name) {
            continue;
        }
        let errors = run_scenario(&v);
        seen.push(name.clone());
        if errors.is_empty() {
            passed += 1;
            ticks += v["result"].as_array().unwrap().len();
            println!("ok   {name}");
        } else {
            println!("FAIL {name}\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    let known = |n: &String| KNOWN_GAPS.iter().any(|(k, _)| k == n);
    let (gaps, new): (Vec<_>, Vec<_>) = failed.iter().cloned().partition(|n| known(n));
    println!("effect parity: {passed} passed ({ticks} ticks), {} known gaps, {} failed", gaps.len(), new.len());
    assert!(new.is_empty(), "failed: {new:?}");
    // (Only when the whole file ran.)
    if filter.is_none() && exclude.is_none() {
        // (Only the ones this file has: older recordings do not hold the hazard scenarios.)
        let fixed: Vec<&str> = KNOWN_GAPS.iter().map(|(k, _)| *k).filter(|k| seen.iter().any(|n| n == k) && !failed.iter().any(|f| f == k)).collect();
        assert!(fixed.is_empty(), "known gaps that now pass (take them off the list): {fixed:?}");
    }
}
