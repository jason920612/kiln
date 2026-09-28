//! Spawn protection: non-operators cannot break or place blocks within `radius` blocks of the
//! world spawn (the square vanilla's `spawn-protection` uses). Subscriptions are fail-closed:
//! a trap or a timeout denies.
//!
//! The claim is cell-scoped data: the first event in a cell near spawn records the claim
//! (centre and radius) in that cell, and later decisions read the cell's claim, so a claim
//! stays as recorded even if the configured radius changes. Each cell also counts denials.
//!
//! `chaos` (`trap` or `spin`) with `chaos_y` makes handlers misbehave at that height, after
//! their writes, for the host's failure-policy tests.

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{BlockPos, InitInfo, Plugin, Verdict, colored, config, export_plugin};
use std::sync::Mutex;

#[derive(Clone, Copy)]
struct Settings {
    spawn: (i32, i32),
    radius: i32,
    chaos: Chaos,
    chaos_y: i32,
}

#[derive(Clone, Copy, PartialEq)]
enum Chaos {
    None,
    Trap,
    Spin,
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

fn settings() -> Settings {
    SETTINGS.lock().unwrap().expect("init_region ran")
}

/// A claim as stored in a cell: centre x, centre z, radius (little-endian i32s).
fn encode_claim(x: i32, z: i32, r: i32) -> Vec<u8> {
    [x, z, r].iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn decode_claim(b: &[u8]) -> Option<(i32, i32, i32)> {
    let v: Vec<i32> = b.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
    (v.len() == 3).then(|| (v[0], v[1], v[2]))
}

fn within(pos: &BlockPos, (x, z, r): (i32, i32, i32)) -> bool {
    (pos.x - x).abs().max((pos.z - z).abs()) <= r
}

/// The shared decision of break and place.
fn decide(operator: bool, level: &str, pos: &BlockPos, cell: u64) -> Verdict {
    let s = settings();
    let mut deny = false;
    if !operator && level == "minecraft:overworld" {
        let configured = (s.spawn.0, s.spawn.1, s.radius);
        let scope = || Scope::Cell(cell);
        let claim = match state::get(scope(), "claim").as_deref().and_then(decode_claim) {
            Some(c) => c,
            None if within(pos, configured) => {
                state::put(scope(), "claim", Some(&encode_claim(configured.0, configured.1, configured.2)));
                configured
            }
            None => configured,
        };
        if within(pos, claim) {
            deny = true;
            let n = state::get_i64(scope(), "denied");
            state::put_i64(scope(), "denied", n + 1);
        }
    }
    if s.chaos != Chaos::None && pos.y == s.chaos_y {
        match s.chaos {
            Chaos::Trap => panic!("chaos: trap"),
            _ => {
                let mut i = 0u64;
                loop {
                    i = std::hint::black_box(i.wrapping_add(1));
                }
            }
        }
    }
    if deny { Verdict::Deny(Some(vec![colored("This area is protected (spawn).", "red")])) } else { Verdict::Allow }
}

struct SpawnProtection;

impl Plugin for SpawnProtection {
    fn init_region(info: InitInfo) {
        let chaos = match config(&info, "chaos") {
            Some("trap") => Chaos::Trap,
            Some("spin") => Chaos::Spin,
            _ => Chaos::None,
        };
        let int = |k: &str, d: i32| config(&info, k).and_then(|v| v.parse().ok()).unwrap_or(d);
        *SETTINGS.lock().unwrap() = Some(Settings {
            spawn: (info.spawn.x, info.spawn.z),
            radius: int("radius", 16),
            chaos,
            chaos_y: int("chaos_y", i32::MIN),
        });
    }

    fn on_block_break(ev: kiln_plugin_sdk::BlockEvent) -> Verdict {
        decide(ev.player.operator, &ev.level, &ev.pos, ev.cell)
    }

    fn on_block_place(ev: kiln_plugin_sdk::PlaceEvent) -> Verdict {
        decide(ev.player.operator, &ev.level, &ev.pos, ev.cell)
    }
}

export_plugin!(SpawnProtection);
