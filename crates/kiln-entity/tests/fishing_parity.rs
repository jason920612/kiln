//! Fishing bobbers against a vanilla 26.3 server, bit for bit (`tools/FishingVectors.java`, `KILN_FISHING_VECTORS`): a bobber cast from a rod with
//! Luck of the Sea and Lure falls into a patch of random blocks around its water; after every tick the state, the nibble and wait counters, the fish angle,
//! the biting flag, "open water" and the position and motion must be vanilla's (Lure's wait times, the open-water test of the 5x4x5 area, the bob).
//! Skipped without the vectors.

use kiln_entity::ext_entity::fishing_hook;
use kiln_entity::level::{EntityLevel, PlayerView};
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use serde_json::Value;

const X: i32 = 8;
const Z: i32 = 8;
const H: i32 = 80;

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| v.as_str().map_or(f64::NAN, |s| s.parse().unwrap()))
}

#[test]
fn bobbers_match_vanilla_tick_by_tick() {
    let Some(path) = std::env::var_os("KILN_FISHING_VECTORS") else {
        eprintln!("skipped: set KILN_FISHING_VECTORS (tools/FishingVectors.java)");
        return;
    };
    let only = std::env::var("KILN_FISHING_ONLY").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let rod = kiln_data::builtin_id("minecraft:item", "minecraft:fishing_rod").unwrap();
    let mut failures = Vec::new();
    let (mut cases, mut values) = (0, 0usize);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap();
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        cases += 1;
        let mut level = MemoryLevel::new(-64, 1);
        level.game_time = v["game_time"].as_i64().unwrap();
        // The patch: x, z in -2..=2 and six layers from H-2, in the order the harness wrote them.
        let mut blocks = v["blocks"].as_array().unwrap().iter();
        for dx in -2..=2 {
            for dz in -2..=2 {
                for i in 0..6 {
                    let id = blocks.next().unwrap().as_i64().unwrap() as u16;
                    level.blocks.insert(BlockPos::new(X + dx, H + i - 2, Z + dz), id);
                }
            }
        }
        let mut owner = PlayerView::new(1, Vec3::new(X as f64 + 0.5, H as f64, Z as f64 + 5.5));
        owner.main_hand = rod;
        level.players.push(owner);
        let luck = v["luck"].as_i64().unwrap() as i32;
        let lure = v["lure_speed"].as_i64().unwrap() as i32;
        let uuid_lsb: i64 = v["uuid_lsb"].as_str().unwrap().parse().unwrap();
        let seed: i64 = v["seed"].as_str().unwrap().parse().unwrap();
        let mut hook = fishing_hook::cast(5, uuid_lsb as u64 as u128, 1, Vec3::new(X as f64 + 0.5, H as f64, Z as f64 + 5.5), 1.62, 180.0, -30.0, luck, lure, seed);
        hook.set_pos(Vec3::new(X as f64 + 0.5, H as f64 + 0.9, Z as f64 + 0.5));
        hook.delta = Vec3::ZERO;
        hook.random = kiln_javamath::random::LegacyRandom::new(seed);
        level.insert(hook);
        for (t, want) in v["ticks"].as_array().unwrap().iter().enumerate() {
            if want.is_null() {
                break;
            }
            // The light the bobber's block sees (the level here has no light engine): what the vector says of the cell above.
            let row = want.as_array().unwrap();
            let sees = row[13].as_bool().unwrap();
            level.lights.insert(BlockPos::new(row[14].as_i64().unwrap() as i32, row[15].as_i64().unwrap() as i32, row[16].as_i64().unwrap() as i32), (if sees { 15 } else { 14 }, 0));
            level.tick();
            let Some(e) = level.entities().find(|e| e.id == 5) else {
                failures.push(format!("{name} t={}: the bobber is gone", t + 1));
                break;
            };
            let h = fishing_hook::get(e).unwrap();
            let state = match h.state {
                fishing_hook::State::Flying => 0.0,
                fishing_hook::State::HookedInEntity => 1.0,
                fishing_hook::State::Bobbing => 2.0,
            };
            let got = [
                state, h.nibble as f64, h.time_until_lured as f64, h.time_until_hooked as f64, h.fish_angle as f64, h.biting as u8 as f64, h.open_water as u8 as f64,
                e.position().x, e.position().y, e.position().z, e.delta.x, e.delta.y, e.delta.z,
            ];
            let w: Vec<f64> = want.as_array().unwrap()[..13].iter().map(|x| x.as_bool().map_or_else(|| num(x), |b| b as u8 as f64)).collect();
            values += got.len();
            // (The fish angle is a float: the vector has its shortest decimal form.)
            let same = |i: usize, a: f64, b: f64| if i == 4 { a as f32 == b as f32 } else { a == b || (a.is_nan() && b.is_nan()) };
            if let Some(first) = got.iter().zip(&w).enumerate().position(|(i, (a, b))| !same(i, *a, *b)) {
                let mut text = format!("{name} (luck {} lure {}) t={} first field {first}: Kiln {got:?}
    vanilla {w:?}", v["luck_level"], v["lure_level"], t + 1);
                if std::env::var_os("KILN_FISHING_TRACE").is_some() {
                    for k in t.saturating_sub(3)..t {
                        let before: Vec<f64> = v["ticks"][k].as_array().unwrap().iter().map(|x| x.as_bool().map_or_else(|| num(x), |b| b as u8 as f64)).collect();
                        text.push_str(&format!("
    vanilla t={} {before:?}", k + 1));
                    }
                }
                failures.push(text);
                break;
            }
        }
    }
    for f in failures.iter().take(std::env::var("KILN_FISHING_SHOW").ok().and_then(|n| n.parse().ok()).unwrap_or(12)) {
        eprintln!("FAIL {f}");
    }
    eprintln!("fishing: {cases} cases, {values} values, {} differ", failures.len());
    assert!(failures.is_empty(), "{} fishing cases differ", failures.len());
}
