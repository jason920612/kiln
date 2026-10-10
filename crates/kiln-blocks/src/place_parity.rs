//! Replays `tools/PlaceVectors.java` (every block item placed in many surroundings, recorded in a
//! vanilla server) through [`placement::place`] on a [`TestLevel`] and compares the blocks that
//! changed around the placed one.
//!
//! `KILN_PLACE_VECTORS` names the vector files (`:` separated); `KILN_PLACE_REPORT` a file that
//! gets the mismatches grouped by item; `KILN_PLACE_EXPECTED` a file of items (one per line,
//! `#` comments) whose mismatches are known and documented. Without the vectors the test is
//! skipped.

use crate::level::{Level, flags};
use crate::placement::{self, BlockItem, PlaceContext};
use crate::pos::{BlockPos, Direction};
use crate::state;
use crate::test_level::TestLevel;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const O: BlockPos = BlockPos::new(8, 150, 8);
const R: i32 = 3;

type Changes = BTreeMap<(i32, i32, i32), u16>;

fn int(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}

fn parse_changes(v: &Value) -> Changes {
    v.as_array().unwrap().iter().map(|e| ((int(&e[0]), int(&e[1]), int(&e[2])), state::parse_state(e[3].as_str().unwrap()).unwrap_or_else(|| panic!("state {}", e[3])))).collect()
}

fn describe(c: &Changes) -> String {
    c.iter().map(|((x, y, z), s)| format!("{x},{y},{z}={}", state::state_string(*s))).collect::<Vec<_>>().join(" ")
}

fn window(level: &TestLevel, p: BlockPos) -> Vec<u16> {
    let mut out = Vec::new();
    for x in -R..=R {
        for y in -R..=R {
            for z in -R..=R {
                out.push(level.block(p.offset(x, y, z)));
            }
        }
    }
    out
}

fn clear(level: &mut TestLevel, around: BlockPos) {
    for x in -R - 1..=R + 1 {
        for y in -R - 1..=R + 1 {
            for z in -R - 1..=R + 1 {
                let p = around.offset(x, y, z);
                if !kiln_data::blocks_types::is_air(level.block(p)) {
                    level.set_raw(p, kiln_data::blocks::default_state::AIR, flags::SKIP_ALL_SIDEEFFECTS);
                }
            }
        }
    }
}

/// What mismatched: item, kind, one example.
struct Miss {
    kind: &'static str,
    example: String,
}

#[test]
fn place_parity() {
    let Some(paths) = std::env::var_os("KILN_PLACE_VECTORS") else {
        eprintln!("skipped: set KILN_PLACE_VECTORS (tools/PlaceVectors.java)");
        return;
    };
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (2, 2));
    let (mut contexts, mut placements, mut agree) = (0usize, 0usize, 0usize);
    let mut misses: BTreeMap<String, Vec<Miss>> = BTreeMap::new();
    let mut counts: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    for path in std::env::split_paths(&paths) {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lines = text.lines();
        let header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        let items: Vec<String> = header["items"].as_array().unwrap().iter().map(|i| i.as_str().unwrap().to_owned()).collect();
        let kiln_items: Vec<Option<BlockItem>> = items.iter().map(|i| BlockItem::of_item(i)).collect();
        for line in lines {
            let v: Value = serde_json::from_str(line).unwrap();
            assert!(v.get("error").is_none(), "{line}");
            contexts += 1;
            let face = Direction::from_index(int(&v["face"]) as usize);
            let p = O.relative(face);
            clear(&mut level, O);
            clear(&mut level, p);
            level.brightness.clear();
            level.block_brightness.clear();
            for e in v["env"].as_array().unwrap() {
                let s = state::parse_state(e[3].as_str().unwrap()).unwrap_or_else(|| panic!("state {}", e[3]));
                level.set_raw(O.offset(int(&e[0]), int(&e[1]), int(&e[2])), s, flags::SKIP_ALL_SIDEEFFECTS);
            }
            for e in v["light"].as_array().unwrap() {
                let q = O.offset(int(&e[0]), int(&e[1]), int(&e[2]));
                level.brightness.insert(q, int(&e[3]));
                level.block_brightness.insert(q, int(&e[4]));
            }
            let cursor = [v["cursor"][0].as_f64().unwrap(), v["cursor"][1].as_f64().unwrap(), v["cursor"][2].as_f64().unwrap()];
            let ctx = PlaceContext {
                hit: O,
                face,
                click: [O.x as f64 + cursor[0], O.y as f64 + cursor[1], O.z as f64 + cursor[2]],
                yaw: v["yaw"].as_f64().unwrap() as f32,
                pitch: v["pitch"].as_f64().unwrap() as f32,
                sneaking: v["sneak"].as_bool().unwrap(),
            };
            let only = v.get("only").and_then(|o| o.as_str());
            let results = v["r"].as_object().unwrap();
            let before = window(&level, p);
            for (name, item) in items.iter().zip(&kiln_items) {
                if only.is_some_and(|o| o != name) {
                    continue;
                }
                placements += 1;
                let want: Changes = match results.get(name) {
                    None => Changes::new(),
                    Some(Value::Number(_)) => {
                        let b = BlockItem::of_item(name).map(|i| i.block).or_else(|| crate::state::BlockId::by_name(name));
                        let s = b.expect("block of the item").default_state();
                        BTreeMap::from([((p.x - O.x, p.y - O.y, p.z - O.z), s)])
                    }
                    Some(c) => parse_changes(c),
                };
                let got: Changes = match item {
                    None => Changes::new(),
                    Some(item) => {
                        placement::place(&mut level, item, &ctx);
                        level.effects.clear();
                        let after = window(&level, p);
                        let mut c = Changes::new();
                        let mut i = 0;
                        for x in -R..=R {
                            for y in -R..=R {
                                for z in -R..=R {
                                    if after[i] != before[i] {
                                        let q = p.offset(x, y, z);
                                        c.insert((q.x - O.x, q.y - O.y, q.z - O.z), after[i]);
                                        level.set_raw(q, before[i], flags::SKIP_ALL_SIDEEFFECTS);
                                    }
                                    i += 1;
                                }
                            }
                        }
                        c
                    }
                };
                if got == want {
                    agree += 1;
                    continue;
                }
                let kind = if item.is_none() {
                    "not a block item in kiln"
                } else if want.is_empty() {
                    "kiln places, vanilla does not"
                } else if got.is_empty() {
                    "vanilla places, kiln does not"
                } else {
                    "both place, different blocks"
                };
                *counts.entry((name.clone(), kind)).or_default() += 1;
                let list = misses.entry(name.clone()).or_default();
                if list.iter().filter(|m| m.kind == kind).count() < 3 {
                    let env: Vec<String> = v["env"].as_array().unwrap().iter().map(|e| format!("{},{},{}={}", e[0], e[1], e[2], e[3].as_str().unwrap())).collect();
                    list.push(Miss {
                        kind,
                        example: format!(
                            "ctx {} face {:?} yaw {} pitch {} env [{}] light {} | vanilla: {} | kiln: {}",
                            v["id"], face, v["yaw"], v["pitch"], env.join(" "), v["light"], describe(&want), describe(&got)
                        ),
                    });
                }
            }
        }
    }
    let expected: BTreeSet<String> = std::env::var_os("KILN_PLACE_EXPECTED")
        .map(|f| std::fs::read_to_string(f).unwrap().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_owned).collect())
        .unwrap_or_default();
    let mut report = String::new();
    for (name, list) in &misses {
        let known = if expected.contains(name) { " (known)" } else { "" };
        report.push_str(&format!("{name}{known}\n"));
        for ((n, kind), c) in counts.iter().filter(|((n, _), _)| n == name) {
            let _ = n;
            report.push_str(&format!("  {c} x {kind}\n"));
        }
        for m in list {
            report.push_str(&format!("    [{}] {}\n", m.kind, m.example));
        }
    }
    if let Some(f) = std::env::var_os("KILN_PLACE_REPORT") {
        std::fs::write(f, &report).unwrap();
    }
    let unexpected: Vec<&String> = misses.keys().filter(|n| !expected.contains(*n)).collect();
    eprintln!("place parity: {contexts} contexts, {placements} placements, {agree} agree, {} items differ, {} not documented", misses.len(), unexpected.len());
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
