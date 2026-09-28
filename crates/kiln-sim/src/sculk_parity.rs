//! Replays the vanilla sculk vectors of `tools/SculkVectors.java` (`KILN_SCULK_VECTORS`,
//! written by `tools/sculk_vectors.py`) through the simulation: each scenario's blocks are
//! placed with the same `/setblock` commands, and after every tick (with the recorded commands
//! before it) the watched block states and the watched sculk sensors' vibration state (last
//! frequency, travelling vibration, selector candidate) must be exactly as vanilla had them.
//! Skipped when the vectors are not there.

use crate::testing::{Client, join};
use crate::{Sim, SimConfig};
use kiln_blocks::BlockPos;
use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use serde_json::{Value, json};

/// `SculkVectors.BASE`.
const BASE: [i32; 3] = [0, 100, 0];

fn pos_of(v: &Value) -> BlockPos {
    let c = |i: usize| v[i].as_i64().unwrap() as i32;
    BlockPos::new(BASE[0] + c(0), BASE[1] + c(1), BASE[2] + c(2))
}

/// `SculkVectors.absolute`: `~d` coordinates relative to the scenario base.
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

/// A sensor's recorded vibration state: [last frequency, travelling event, its distance, ticks
/// left, candidate event, candidate tick counted from the scenario's start (-1: none)].
fn vibration(sim: &Sim, p: BlockPos, start: i64) -> Value {
    let Some(t) = sim.block_entity_nbt(p.x, p.y, p.z) else { return json!([null, null, null, null, null, null]) };
    let l = t.get("listener");
    let event = l.and_then(|l| l.get("event"));
    let name = |e: Option<&Tag>| e.and_then(|e| e.get("game_event")).and_then(Tag::as_str).map(str::to_owned);
    let sel = l.and_then(|l| l.get("selector"));
    json!([
        t.get("last_vibration_frequency").and_then(Tag::as_i64),
        name(event),
        event.and_then(|e| e.get("distance")).and_then(Tag::as_f64),
        l.and_then(|l| l.get("event_delay")).and_then(Tag::as_i64),
        name(sel.and_then(|s| s.get("event"))),
        sel.and_then(|s| s.get("tick")).and_then(Tag::as_i64).map(|t| if t < 0 { t } else { t - start }),
    ])
}

/// Vanilla's floats come as the doubles of floats: compare as f32 bits.
fn same(got: &Value, want: &Value) -> bool {
    match (got, want) {
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y)),
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            a.as_f64().map(|v| v as f32) == b.as_f64().map(|v| v as f32)
        }
        _ => got == want,
    }
}

/// Replays one scenario; returns (values compared, mismatches).
fn run_scenario(line: &Value) -> (usize, Vec<String>) {
    let mut sim = Sim::new(SimConfig::new(2, 4, None));
    let (msg, stats) = join(1, "Sculk", 4);
    assert!(sim.step([
        msg,
        ToSim::Console("gamemode spectator Sculk".into()),
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console(format!("tp Sculk {} {} {}", BASE[0], BASE[1] + 30, BASE[2])),
    ]));
    let mut client = Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let actions = |t: usize| -> Vec<ToSim> {
        line["actions"].get(t.to_string()).and_then(Value::as_array).map_or(Vec::new(), |a| {
            a.iter().map(|c| ToSim::Console(absolute(c.as_str().unwrap()))).collect()
        })
    };
    let sensors: Vec<BlockPos> = line["sensors"].as_array().unwrap().iter().map(pos_of).collect();
    let states: Vec<BlockPos> = line["states"].as_array().unwrap().iter().map(pos_of).collect();
    let (mut compared, mut errors) = (0, Vec::new());
    // Vanilla's commands run before its tick; Kiln's in the tick they arrive with.
    let start = sim.game_time();
    for (t, want) in line["result"].as_array().unwrap().iter().enumerate() {
        let tick = t + 1;
        let mut inbox = Vec::new();
        if tick == 1 {
            for b in line["blocks"].as_array().unwrap() {
                let p = pos_of(b);
                inbox.push(ToSim::Console(format!("setblock {} {} {} {}", p.x, p.y, p.z, b[3].as_str().unwrap())));
            }
        }
        inbox.extend(actions(tick));
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        for (i, &p) in states.iter().enumerate() {
            let want_state = kiln_blocks::state::parse_state(want["states"][i].as_str().unwrap());
            let got = sim.block_at(p.x, p.y, p.z);
            compared += 1;
            if got != want_state {
                let name = |s: Option<u16>| s.map_or("?".to_owned(), kiln_blocks::state::state_string);
                errors.push(format!("tick {tick} state {p:?}: kiln {}, vanilla {}", name(got), want["states"][i]));
            }
        }
        for (i, &p) in sensors.iter().enumerate() {
            let got = vibration(&sim, p, start);
            compared += 1;
            if !same(&got, &want["sensors"][i]) {
                errors.push(format!("tick {tick} sensor {p:?}: kiln {got}, vanilla {}", want["sensors"][i]));
            }
        }
        if errors.len() > 8 {
            break;
        }
    }
    (compared, errors)
}

#[test]
fn sculk_parity() {
    let Some(path) = std::env::var_os("KILN_SCULK_VECTORS") else {
        eprintln!("skipped: set KILN_SCULK_VECTORS (tools/sculk_vectors.py)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    let (mut values, mut values_ok) = (0, 0);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let (n, errors) = run_scenario(&v);
        values += n;
        values_ok += n.saturating_sub(errors.len());
        if errors.is_empty() {
            passed += 1;
        } else {
            eprintln!("FAIL {name}:\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    eprintln!("sculk parity: {passed} scenarios pass, {} fail; {values_ok}/{values} values match", failed.len());
    assert!(failed.is_empty(), "failing: {failed:?}");
}
