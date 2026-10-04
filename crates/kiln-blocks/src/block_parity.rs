//! Replays `tools/BlockTickVectors.java` (what blocks do on their own, recorded in a vanilla
//! server) through kiln-blocks on a [`TestLevel`] and compares, after every op, the area's
//! blocks, its pending scheduled ticks and a draw of the level random.
//!
//! `KILN_BLOCK_VECTORS` names the vectors (`tools/block_vectors.py`); `KILN_PARITY_FILTER` is a
//! regex on scenario names. Without the vectors the test is skipped.

use crate::level::{Level, flags};
use crate::pos::BlockPos;
use crate::state;
use crate::test_level::TestLevel;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;
use std::collections::BTreeMap;

const FLOOR: i32 = 99;
const Y0: i32 = 100;
const HEIGHT: i32 = 12;
/// The window that is built, recorded and ticked (relative x and z, inclusive).
const LO: i32 = -8;
const HI: i32 = 23;

type Blocks = BTreeMap<(i32, i32, i32), u16>;

fn at(e: &serde_json::Value, i: usize) -> i32 {
    e[i].as_i64().unwrap() as i32
}

fn parse_blocks(v: &serde_json::Value) -> Blocks {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let s = e[3].as_str().unwrap();
            ((at(e, 0), at(e, 1), at(e, 2)), state::parse_state(s).unwrap_or_else(|| panic!("state {s}")))
        })
        .collect()
}

fn snapshot(level: &TestLevel, x0: i32, z0: i32) -> Blocks {
    let mut out = BTreeMap::new();
    for y in FLOOR..Y0 + HEIGHT {
        for z in z0 + LO..=z0 + HI {
            for x in x0 + LO..=x0 + HI {
                let s = level.block(BlockPos::new(x, y, z));
                if if y == FLOOR { state::is(s, d::STONE) } else { is_air(s) } {
                    continue;
                }
                out.insert((x - x0, y, z - z0), s);
            }
        }
    }
    out
}

/// The brightness the level reads: 15 above the floor unless the table says otherwise, 0 in it.
fn set_light(level: &mut TestLevel, light: &serde_json::Value, x0: i32, z0: i32) {
    level.brightness.clear();
    for z in z0 + LO..=z0 + HI {
        for x in x0 + LO..=x0 + HI {
            level.brightness.insert(BlockPos::new(x, FLOOR, z), 0);
        }
    }
    for e in light.as_array().unwrap() {
        level.brightness.insert(BlockPos::new(x0 + at(e, 0), at(e, 1), z0 + at(e, 2)), at(e, 3));
    }
}

/// `["b" or "f", x, y, z, type, delay, priority]` of every pending tick of the area, sorted.
fn pending(level: &TestLevel, x0: i32, z0: i32) -> Vec<String> {
    let time = level.game_time;
    let inside = |p: BlockPos| p.x >= x0 + LO && p.x <= x0 + HI && p.z >= z0 + LO && p.z <= z0 + HI;
    let mut out = Vec::new();
    for c in level.block_ticks.chunks().collect::<Vec<_>>() {
        if let Some(t) = level.block_ticks.container(c) {
            for s in t.pack(time).into_iter().filter(|s| inside(s.pos)) {
                out.push(format!("b {} {} {} {} {} {}", s.pos.x, s.pos.y, s.pos.z, s.kind.name(), s.delay, s.priority.value()));
            }
        }
    }
    for c in level.fluid_ticks.chunks().collect::<Vec<_>>() {
        if let Some(t) = level.fluid_ticks.container(c) {
            for s in t.pack(time).into_iter().filter(|s| inside(s.pos)) {
                out.push(format!("f {} {} {} {} {} {}", s.pos.x, s.pos.y, s.pos.z, s.kind.name(), s.delay, s.priority.value()));
            }
        }
    }
    out.sort();
    out
}

fn pending_expected(v: &serde_json::Value, x0: i32, z0: i32) -> Vec<String> {
    let mut out: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|e| format!("{} {} {} {} {} {} {}", e[0].as_str().unwrap(), x0 + at(e, 1), at(e, 2), z0 + at(e, 3), e[4].as_str().unwrap(), at(e, 5), at(e, 6)))
        .collect();
    out.sort();
    out
}

/// The `rt` op: every randomly ticking block of the area in y, z, x order, as it was at the start.
fn random_tick_area(level: &mut TestLevel, x0: i32, z0: i32) {
    let mut ticking = Vec::new();
    for y in FLOOR..Y0 + HEIGHT {
        for z in z0 + LO..=z0 + HI {
            for x in x0 + LO..=x0 + HI {
                let p = BlockPos::new(x, y, z);
                if crate::tick::randomly_ticks(level.block(p)) {
                    ticking.push(p);
                }
            }
        }
    }
    for p in ticking {
        crate::tick::random_tick_at(level, p);
    }
}

fn state_diff(want: &Blocks, got: &Blocks) -> Vec<String> {
    want.iter()
        .filter(|(k, s)| got.get(k) != Some(s))
        .map(|(k, s)| format!("{k:?} want {} got {}", state::state_string(*s), got.get(k).map_or("air".to_owned(), |g| state::state_string(*g))))
        .chain(got.iter().filter(|(k, _)| !want.contains_key(k)).map(|(k, g)| format!("{k:?} want air got {}", state::state_string(*g))))
        .take(6)
        .collect()
}

/// Runs one scenario line; `Err` names the first op that differs.
fn replay(v: &serde_json::Value) -> Result<usize, String> {
    let (x0, z0) = (v["x0"].as_i64().unwrap() as i32, v["z0"].as_i64().unwrap() as i32);
    let mut level = TestLevel::flat(-64, 384, &[d::BEDROCK]);
    level.load_chunks((-4, -4), (4, 4));
    level.difficulty = v["difficulty"].as_i64().unwrap() as i32;
    for x in x0 + LO..=x0 + HI {
        for z in z0 + LO..=z0 + HI {
            level.set_raw(BlockPos::new(x, FLOOR, z), d::STONE, flags::NONE);
        }
    }
    for ((x, y, z), s) in parse_blocks(&v["initial"]) {
        level.set_raw(BlockPos::new(x0 + x, y, z0 + z), s, flags::NONE);
    }
    level.game_time = v["game_time"].as_i64().unwrap();
    set_light(&mut level, &v["initial_light"], x0, z0);
    level.set_random_seed(v["seed"].as_i64().unwrap());
    let ops = v["ops"].as_array().unwrap();
    let results = v["results"].as_array().unwrap();
    // Vanilla rolls a dropped block's loot with the level random; Kiln's drops are the
    // simulation's (own seeding, loot parity is tested elsewhere), so once a block dropped
    // the random draws are no longer comparable. Blocks and ticks still are.
    let mut drops_seen = false;
    for (i, (op, want)) in ops.iter().zip(results).enumerate() {
        let kind = op[0].as_str().unwrap();
        match kind {
            "rt" => random_tick_area(&mut level, x0, z0),
            "tick" => {
                for _ in 0..op[1].as_i64().unwrap() {
                    level.tick(0, &[]);
                }
            }
            "set" => {
                let s = state::parse_state(op[4].as_str().unwrap()).expect("state");
                crate::update::set_block(&mut level, BlockPos::new(x0 + at(op, 1), at(op, 2), z0 + at(op, 3)), s, flags::ALL);
            }
            other => panic!("op {other}"),
        }
        drops_seen |= level.effects.iter().any(|e| matches!(e, crate::Effect::Drop { .. }));
        level.effects.clear();
        let probe = level.random().next_long();
        let (got, want_blocks) = (snapshot(&level, x0, z0), parse_blocks(&want[0]));
        let (got_ticks, want_ticks) = (pending(&level, x0, z0), pending_expected(&want[1], x0, z0));
        let probe_ok = drops_seen || probe == want[3].as_i64().unwrap();
        if got != want_blocks || got_ticks != want_ticks || !probe_ok {
            let ticks_diff = if got_ticks != want_ticks { format!("; ticks want {want_ticks:?} got {got_ticks:?}") } else { String::new() };
            return Err(format!("op {i} ({kind}): random probe ok {probe_ok}; {:?}{ticks_diff}", state_diff(&want_blocks, &got)));
        }
        set_light(&mut level, &want[2], x0, z0);
    }
    Ok(ops.len())
}

#[test]
fn block_parity() {
    let Some(path) = std::env::var_os("KILN_BLOCK_VECTORS") else {
        eprintln!("skipped: set KILN_BLOCK_VECTORS (tools/block_vectors.py)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok().map(|f| regex_lite(&f));
    let text = std::fs::read_to_string(path).unwrap();
    let (mut ok, mut total, mut ops_ok, mut ops_total) = (0, 0, 0, 0);
    let mut failed = Vec::new();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_string();
        assert_ne!(name, "error", "{line}");
        assert!(v.get("error").is_none(), "{name}: {line}");
        if filter.as_ref().is_some_and(|f| !f(&name)) {
            continue;
        }
        total += 1;
        let n = v["ops"].as_array().unwrap().len();
        ops_total += n;
        let result = std::panic::catch_unwind(|| replay(&v));
        match result {
            Ok(Ok(n)) => {
                ok += 1;
                ops_ok += n;
            }
            Ok(Err(e)) => failed.push(format!("{name}: {e}")),
            Err(_) => failed.push(format!("{name}: panicked")),
        }
    }
    eprintln!("block parity: {ok}/{total} scenarios match ({ops_ok}/{ops_total} ops before the first difference)");
    assert!(failed.is_empty(), "{:#?}", failed);
}

/// A substring match is all the filter needs (`a|b` alternatives allowed).
fn regex_lite(pattern: &str) -> impl Fn(&str) -> bool + use<> {
    let alts: Vec<String> = pattern.split('|').map(str::to_owned).collect();
    move |name| alts.iter().any(|a| name.contains(a.as_str()))
}
