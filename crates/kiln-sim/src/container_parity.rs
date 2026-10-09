//! Replays the vanilla container vectors of `tools/ContainerVectors.java`
//! (`KILN_CONTAINER_VECTORS`, written by `tools/container_vectors.py`) through the simulation:
//! each scenario's blocks are placed with the same `/setblock` commands (block entity data
//! included), and after every tick (with the recorded commands before it) the watched
//! containers (items by slot, hopper cooldowns, furnace timers), block states and comparator
//! outputs must be exactly as vanilla had them. Skipped when the vectors are not there.

use crate::testing::{Client, join};
use crate::{OVERWORLD_ID, Sim, SimConfig};
use kiln_blocks::BlockPos;
use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_world::{Blocks, ChunkPos};
use serde_json::{Value, json};

/// `ContainerVectors.BASE`.
const BASE: [i32; 3] = [0, 100, 0];

fn pos_of(v: &Value) -> BlockPos {
    let c = |i: usize| v[i].as_i64().unwrap() as i32;
    BlockPos::new(BASE[0] + c(0), BASE[1] + c(1), BASE[2] + c(2))
}

/// A container's recorded form: items, and the hopper cooldown or furnace timers.
fn container_json(sim: &Sim, pos: BlockPos) -> Value {
    let Some(region) = sim.dims[OVERWORLD_ID].regions.at(ChunkPos::of_block(pos.x, pos.z).cell()) else { return json!({}) };
    let Some(c) = region.part().1.containers.get(pos) else { return json!({}) };
    let items: Vec<Value> =
        c.items.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| json!([i, s.item_name(), s.count()])).collect();
    let mut m = serde_json::Map::new();
    m.insert("items".into(), Value::Array(items));
    match c.kind {
        crate::container::BeKind::Hopper => {
            m.insert("cooldown".into(), json!(c.cooldown));
        }
        crate::container::BeKind::Furnace(_) => {
            m.insert("furnace".into(), json!([c.lit_remaining, c.lit_total, c.cook_timer, c.cook_total]));
        }
        // A crafter: the switched-off slots (a bit each), the crafting countdown, whether it is powered.
        crate::container::BeKind::Crafter => {
            if let Some(cr) = &c.crafter {
                let mask: i32 = (0..9).filter(|&i| cr.disabled[i]).map(|i| 1 << i).sum();
                m.insert("crafter".into(), json!([mask, cr.ticks, i32::from(cr.triggered)]));
            }
        }
        // A command block: the success count, powered, condition met, always active, the last output.
        crate::container::BeKind::CommandBlock => {
            if let Some(d) = &c.command {
                m.insert("cmd".into(), json!([d.success_count, i32::from(d.powered), i32::from(d.condition_met), i32::from(d.auto), d.last_plain.clone().unwrap_or_default()]));
            }
        }
        // A campfire: the four timers, then the four totals.
        crate::container::BeKind::Campfire => {
            let mut v: Vec<i32> = c.cooking.to_vec();
            v.extend(c.cooking_total);
            m.insert("campfire".into(), json!(v));
        }
        // A hive: [ticks in the hive, least ticks, nectar] of each bee.
        crate::container::BeKind::Beehive => {
            let bees: Vec<Value> = c
                .hive
                .as_ref()
                .map(|h| h.occupants.iter().map(|o| json!([o.ticks, o.min_ticks, i32::from(o.data.get("HasNectar").and_then(Tag::as_i64).unwrap_or(0) != 0)])).collect())
                .unwrap_or_default();
            m.insert("hive".into(), Value::Array(bees));
        }
        // The song player: playing, ticks since the song started.
        crate::container::BeKind::Jukebox => {
            m.insert("jukebox".into(), json!([i32::from(c.song.is_some()), c.song.map_or(0, |s| s.1)]));
        }
        _ => {}
    }
    Value::Object(m)
}

/// The slots of the container minecart standing in the block at `pos` (`{}` when none does),
/// as `ContainerVectors.cartState` records them.
fn cart_json(sim: &Sim, pos: BlockPos) -> Value {
    let lo = [pos.x as f64 + 0.01, pos.y as f64 + 0.01, pos.z as f64 + 0.01];
    let hi = [pos.x as f64 + 0.99, pos.y as f64 + 0.99, pos.z as f64 + 0.99];
    for region in sim.dims[OVERWORLD_ID].regions.iter() {
        for e in region.part().0.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            let bb = phys.bounding_box();
            if !(bb.min_x < hi[0] && bb.max_x > lo[0] && bb.min_y < hi[1] && bb.max_y > lo[1] && bb.min_z < hi[2] && bb.max_z > lo[2]) {
                continue;
            }
            let Some(c) = kiln_entity::ext_entity::container(phys) else {
                continue;
            };
            let items: Vec<Value> =
                c.items.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| json!([i, s.item_name(), s.count()])).collect();
            return json!({ "items": items });
        }
    }
    json!({})
}

/// The item entities lying in the scenario's area: [item, total count], sorted by item
/// (`ContainerVectors.dropsState`).
fn drops_json(sim: &Sim) -> Value {
    let mut sums: std::collections::BTreeMap<String, i64> = Default::default();
    let (lo, hi) = ([BASE[0] as f64 - 3.0, BASE[1] as f64 - 3.0, BASE[2] as f64 - 3.0], [BASE[0] as f64 + 7.0, BASE[1] as f64 + 7.0, BASE[2] as f64 + 7.0]);
    for region in sim.dims[OVERWORLD_ID].regions.iter() {
        for e in region.part().0.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            let kiln_entity::EntityKind::Item(d) = &phys.kind else { continue };
            let bb = phys.bounding_box();
            if bb.min_x < hi[0] && bb.max_x > lo[0] && bb.min_y < hi[1] && bb.max_y > lo[1] && bb.min_z < hi[2] && bb.max_z > lo[2] {
                *sums.entry(d.stack.item_name().to_owned()).or_default() += d.stack.count() as i64;
            }
        }
    }
    Value::Array(sums.into_iter().map(|(k, v)| json!([k, v])).collect())
}

/// A comparator block entity's `OutputSignal` (-1 without one).
fn comparator_output(sim: &Sim, pos: BlockPos) -> i64 {
    let Some(chunk) = sim.dims[OVERWORLD_ID].regions.chunk(ChunkPos::of_block(pos.x, pos.z)) else { return -1 };
    match chunk.block_entity((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize) {
        Some(be) if kiln_world::block_entity::type_name(be.kind) == "minecraft:comparator" => {
            be.nbt.get("OutputSignal").and_then(Tag::as_i64).unwrap_or(0)
        }
        _ => -1,
    }
}

/// `ContainerVectors.absolute`: `~d` coordinates relative to the scenario base.
fn absolute(cmd: &str) -> String {
    let mut axis = 0;
    cmd.split(' ')
        .map(|part| match part.strip_prefix('~') {
            Some(d) => {
                let v = BASE[axis % 3] as f64 + if d.is_empty() { 0.0 } else { d.parse::<f64>().unwrap() };
                axis += 1;
                // (Java's `Double.toString`/`Long.toString`: whole values print without a point.)
                if v == v.round() { (v as i64).to_string() } else { v.to_string() }
            }
            None => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replays one scenario; returns (values compared, mismatches).
fn run_scenario(line: &Value) -> (usize, Vec<String>) {
    let mut config = SimConfig::new(2, 3, None);
    config.enable_command_block = true;
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Hoppers", 3);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator Hoppers".into()), ToSim::Console("difficulty easy".into()), ToSim::Console(format!("tp Hoppers {} {} {}", BASE[0], BASE[1] + 8, BASE[2]))]));
    let mut client = Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    // Vanilla's `/summon` adds the entity at once; here entities summoned by a command join the
    // level in the next tick, so the summons of tick `t` are sent with tick `t - 1` (tick 1's in
    // a step of their own before it).
    let actions = |t: usize, summons: bool| -> Vec<ToSim> {
        line["actions"].get(t.to_string()).and_then(Value::as_array).map_or(Vec::new(), |a| {
            a.iter()
                .filter(|c| c.as_str().unwrap().starts_with("summon ") == summons)
                .map(|c| ToSim::Console(absolute(c.as_str().unwrap())))
                .collect()
        })
    };
    let first_summons = actions(1, true);
    if !first_summons.is_empty() {
        let mut inbox = first_summons;
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    // The game time is a multiple of 20 as the scenario begins (daylight detectors work on it).
    if line["align20"].as_bool() == Some(true) {
        // (The harness ticks the level alone, which leaves the world clock where `/time` put it.)
        let mut inbox = vec![ToSim::Console("gamerule advance_time false".into())];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        while sim.game_time() % 20 != 0 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    }
    let containers: Vec<BlockPos> = line["containers"].as_array().unwrap().iter().map(pos_of).collect();
    let states: Vec<BlockPos> = line["states"].as_array().unwrap().iter().map(pos_of).collect();
    let comparators: Vec<BlockPos> = line["comparators"].as_array().unwrap().iter().map(pos_of).collect();
    let carts: Vec<BlockPos> = line["carts"].as_array().map_or(Vec::new(), |a| a.iter().map(pos_of).collect());
    let (mut compared, mut errors) = (0, Vec::new());
    let mut kill_items = false;
    let mut previous_drops = serde_json::json!([]);
    let mut seen_bees: std::collections::HashSet<i32> = Default::default();
    let mut seen_entities: std::collections::HashSet<i32> = Default::default();
    let mut pending_entities: Vec<i32> = Vec::new();
    let mut previous_entities = serde_json::json!([]);
    for (t, want) in line["result"].as_array().unwrap().iter().enumerate() {
        let tick = t + 1;
        let mut inbox = Vec::new();
        if std::mem::take(&mut kill_items) {
            inbox.push(ToSim::Console("kill @e[type=item]".into()));
        }
        if tick == 1 {
            for b in line["blocks"].as_array().unwrap() {
                let p = pos_of(b);
                inbox.push(ToSim::Console(format!("setblock {} {} {} {}", p.x, p.y, p.z, b[3].as_str().unwrap())));
            }
        }
        inbox.extend(actions(tick, false));
        inbox.extend(actions(tick + 1, true));
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        for (i, &p) in containers.iter().enumerate() {
            let got = container_json(&sim, p);
            compared += 1;
            if got != want["containers"][i] {
                errors.push(format!("tick {tick} container {i} {p:?}: kiln {got}, vanilla {}", want["containers"][i]));
            }
        }
        for (i, &p) in states.iter().enumerate() {
            let want_state = kiln_blocks::state::parse_state(want["states"][i].as_str().unwrap());
            let got = sim.block_at(p.x, p.y, p.z);
            compared += 1;
            if got != want_state {
                let name = |s: Option<u16>| s.map_or("?".to_owned(), kiln_blocks::state::state_string);
                errors.push(format!("tick {tick} state {p:?}: kiln {}, vanilla {}", name(got), want["states"][i]));
            }
        }
        for (i, &p) in carts.iter().enumerate() {
            let got = cart_json(&sim, p);
            compared += 1;
            if got != want["carts"][i] {
                errors.push(format!("tick {tick} cart {i} {p:?}: kiln {got}, vanilla {}", want["carts"][i]));
            }
        }
        if line["bees"].as_bool() == Some(true) {
            // The bees that appeared this tick (where they were put), sorted, and how many there are.
            let mut fresh: Vec<[f64; 3]> = Vec::new();
            let mut count = 0;
            for region in sim.dims[OVERWORLD_ID].regions.iter() {
                for e in region.part().0.list.iter().filter(|e| !e.removed && e.kind.name == "minecraft:bee") {
                    count += 1;
                    if seen_bees.insert(e.id) {
                        fresh.push(e.pos);
                    }
                }
            }
            fresh.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let got = json!(fresh);
            compared += 2;
            if got != want["bees_new"] {
                errors.push(format!("tick {tick} new bees: kiln {got}, vanilla {}", want["bees_new"]));
            }
            if Some(count) != want["bee_count"].as_i64() {
                errors.push(format!("tick {tick} bee count: kiln {count}, vanilla {}", want["bee_count"]));
            }
        }
        if line["entities"].as_bool() == Some(true) {
            // The entities that appeared: vanilla ticks them in the tick they are made, Kiln's join the level after
            // the tick's work, so those that came last tick are reported now and compared with last tick's vanilla.
            let mut reported: Vec<(String, f64, f64, f64)> = Vec::new();
            let mut now_new: Vec<i32> = Vec::new();
            for region in sim.dims[OVERWORLD_ID].regions.iter() {
                for e in region.part().0.list.iter().filter(|e| !e.removed && e.kind.name != "minecraft:item" && e.kind.name != "minecraft:player") {
                    if pending_entities.contains(&e.id) {
                        let r = |v: f64| (v * 10000.0).round() / 10000.0;
                        // (A primed TNT hops a random way at its making: only where it is to a tenth is compared.)
                        let t = |v: f64| if e.kind.name == "minecraft:tnt" || e.kind.name == "minecraft:sulfur_cube" { (v * 10.0).round() / 10.0 } else { r(v) };
                        let shot = matches!(
                            e.kind.name,
                            "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:egg" | "minecraft:snowball" | "minecraft:splash_potion" | "minecraft:lingering_potion" | "minecraft:experience_bottle" | "minecraft:small_fireball" | "minecraft:wind_charge" | "minecraft:firework_rocket"
                        );
                        if shot {
                            reported.push((e.kind.name.to_owned(), 0.0, 0.0, 0.0));
                        } else {
                            reported.push((e.kind.name.to_owned(), t(e.pos[0]), r(e.pos[1]), t(e.pos[2])));
                        }
                    }
                    if seen_entities.insert(e.id) {
                        now_new.push(e.id);
                    }
                }
            }
            pending_entities = now_new;
            reported.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)).then(a.3.total_cmp(&b.3)));
            let got = json!(reported.iter().map(|r| json!([r.0, r.1, r.2, r.3])).collect::<Vec<_>>());
            let expected = std::mem::replace(&mut previous_entities, want["entities_new"].clone());
            compared += 1;
            if tick > 1 && got != expected {
                errors.push(format!("tick {tick} new entities: kiln {got}, vanilla {expected}"));
            }
        }
        if line["track"].as_bool() == Some(true) {
            // `ContainerVectors`: every entity but items and players: type, position, motion and health (-1: not living).
            let mut got: Vec<(String, [f64; 3], [f64; 3], f64)> = Vec::new();
            for t in sim.entity_nbt() {
                let id = t.get("id").and_then(Tag::as_str).unwrap_or("");
                if matches!(id, "minecraft:item" | "minecraft:player") {
                    continue;
                }
                let triple = |key: &str| -> [f64; 3] {
                    match t.get(key) {
                        Some(Tag::List(l)) if l.len() == 3 => [l[0].as_f64().unwrap_or(0.0), l[1].as_f64().unwrap_or(0.0), l[2].as_f64().unwrap_or(0.0)],
                        _ => [0.0; 3],
                    }
                };
                let pos = triple("Pos");
                // (Only what the harness's box, 8 below to 12 above `BASE`, holds: by the entity's bounding box.)
                let (w, h) = kiln_data::entities::by_name(id).map_or((0.0, 0.0), |k| (k.width as f64, k.height as f64));
                let inside = |lo: f64, hi: f64, base: i32| lo < (base + 12) as f64 && hi > (base - 8) as f64;
                if !(inside(pos[0] - w / 2.0, pos[0] + w / 2.0, BASE[0]) && inside(pos[1], pos[1] + h, BASE[1]) && inside(pos[2] - w / 2.0, pos[2] + w / 2.0, BASE[2])) {
                    continue;
                }
                got.push((id.to_owned(), pos, triple("Motion"), t.get("Health").and_then(Tag::as_f64).unwrap_or(-1.0)));
            }
            got.sort_by(|a, b| a.0.cmp(&b.0).then(a.1[0].total_cmp(&b.1[0])).then(a.1[2].total_cmp(&b.1[2])).then(a.1[1].total_cmp(&b.1[1])));
            let got = json!(got.iter().map(|g| json!([g.0, g.1[0], g.1[1], g.1[2], g.2[0], g.2[1], g.2[2], g.3])).collect::<Vec<_>>());
            compared += 1;
            if got != want["track"] {
                errors.push(format!("tick {tick} tracked entities: kiln {got}, vanilla {}", want["track"]));
            }
        }
        if line["mobs"].as_bool() == Some(true) {
            // `ContainerVectors`: each living thing's type, what it wears (by slot) and whether it carries a chest.
            let mut got: Vec<String> = Vec::new();
            for t in sim.entity_nbt() {
                let id = t.get("id").and_then(Tag::as_str).unwrap_or("");
                if matches!(id, "minecraft:item" | "minecraft:player") || t.get("Health").is_none() && id != "minecraft:armor_stand" {
                    continue;
                }
                let mut parts: Vec<String> = Vec::new();
                if let Some(Tag::Compound(eq)) = t.get("equipment") {
                    for (slot, item) in eq {
                        let name = item.get("id").and_then(Tag::as_str).unwrap_or("");
                        let count = item.get("count").and_then(Tag::as_i64).unwrap_or(1);
                        parts.push(format!("{slot}={name}*{count}"));
                    }
                }
                parts.sort();
                let mut sig = id.to_owned();
                for p in parts {
                    sig.push('|');
                    sig.push_str(&p);
                }
                if t.get("ChestedHorse").and_then(Tag::as_i64) == Some(1) {
                    sig.push_str("|chest");
                }
                got.push(sig);
            }
            got.sort();
            compared += 1;
            if json!(got) != want["mobs"] {
                errors.push(format!("tick {tick} mobs: kiln {got:?}, vanilla {}", want["mobs"]));
            }
        }
        if line["drops"].as_bool() == Some(true) {
            let got = drops_json(&sim);
            compared += 1;
            // (Items made by the entity phase join the level a tick later: `drops_lag` compares
            // against the vanilla items of the tick before.)
            let expected = if line["drops_lag"].as_bool() == Some(true) { std::mem::replace(&mut previous_drops, want["drops"].clone()) } else { want["drops"].clone() };
            if got != expected {
                errors.push(format!("tick {tick} dropped items: kiln {got}, vanilla {expected}"));
            }
            // (Only what each tick makes is compared: the items are removed before the next.)
            kill_items = line["drops_lag"].as_bool() != Some(true) || got.as_array().is_some_and(|a| !a.is_empty());
        }
        for (i, &p) in comparators.iter().enumerate() {
            let got = comparator_output(&sim, p);
            compared += 1;
            if Some(got) != want["comparators"][i].as_i64() {
                errors.push(format!("tick {tick} comparator {p:?}: kiln {got}, vanilla {}", want["comparators"][i]));
            }
        }
        if errors.len() > 8 {
            break;
        }
    }
    (compared, errors)
}

#[test]
fn container_parity() {
    let Some(path) = std::env::var_os("KILN_CONTAINER_VECTORS") else {
        eprintln!("skipped: set KILN_CONTAINER_VECTORS (tools/container_vectors.py)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    let mut values = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let (n, errors) = run_scenario(&v);
        values += n;
        if errors.is_empty() {
            passed += 1;
        } else {
            eprintln!("FAIL {name}:\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    eprintln!("container parity: {passed} scenarios pass, {} fail; {values} values compared", failed.len());
    assert!(failed.is_empty(), "failing: {failed:?}");
}
