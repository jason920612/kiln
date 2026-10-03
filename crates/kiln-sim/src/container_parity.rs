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
                let v = BASE[axis % 3] + if d.is_empty() { 0 } else { d.parse::<i32>().unwrap() };
                axis += 1;
                v.to_string()
            }
            None => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replays one scenario; returns (values compared, mismatches).
fn run_scenario(line: &Value) -> (usize, Vec<String>) {
    let mut sim = Sim::new(SimConfig::new(2, 3, None));
    let (msg, stats) = join(1, "Hoppers", 3);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator Hoppers".into()), ToSim::Console(format!("tp Hoppers {} {} {}", BASE[0], BASE[1] + 8, BASE[2]))]));
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
    let containers: Vec<BlockPos> = line["containers"].as_array().unwrap().iter().map(pos_of).collect();
    let states: Vec<BlockPos> = line["states"].as_array().unwrap().iter().map(pos_of).collect();
    let comparators: Vec<BlockPos> = line["comparators"].as_array().unwrap().iter().map(pos_of).collect();
    let carts: Vec<BlockPos> = line["carts"].as_array().map_or(Vec::new(), |a| a.iter().map(pos_of).collect());
    let (mut compared, mut errors) = (0, Vec::new());
    for (t, want) in line["result"].as_array().unwrap().iter().enumerate() {
        let tick = t + 1;
        let mut inbox = Vec::new();
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
