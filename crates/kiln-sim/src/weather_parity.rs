//! Differential test against vanilla 26.3 vectors from `tools/WeatherVectors.java` (run by
//! `tools/weather_vectors.py`, which sets `KILN_WEATHER_VECTORS`): the weather cycle tick by
//! tick, sky darkening, precipitation over biome areas, the night skip's clock and weather
//! reset, and where players stand up from beds. Skipped without the vectors.

use crate::weather::{Climates, LevelWeather, WeatherData, marker_move, sky_darken};
use crate::OVERWORLD_ID;
use kiln_blocks::{BlockPos, Direction, Level, TestLevel};
use kiln_javamath::random::LegacyRandom;
use serde_json::Value;

fn vectors() -> Option<Vec<Value>> {
    let path = std::env::var_os("KILN_WEATHER_VECTORS")?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str(l).expect("vector json")).collect())
}

fn f32_of(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

/// (clear, rain, thunder, raining, thundering, rain level, thunder level, isRaining, isThundering).
fn state(w: &WeatherData, l: &LevelWeather) -> Vec<Value> {
    vec![
        w.clear_weather_time.into(),
        w.rain_time.into(),
        w.thunder_time.into(),
        w.raining.into(),
        w.thundering.into(),
        (l.rain as f64).into(),
        (l.thunder as f64).into(),
        l.is_raining().into(),
        l.is_thundering().into(),
    ]
}

fn same_state(ours: &[Value], theirs: &Value) -> bool {
    let t = theirs.as_array().unwrap();
    ours.iter().zip(t).all(|(a, b)| match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_f64() || b.is_f64() => x as f32 == y as f32,
        _ => a == b,
    })
}

fn datapack() -> std::path::PathBuf {
    crate::datapack_dir(None)
}

#[test]
fn weather_parity() {
    let Some(vs) = vectors() else {
        eprintln!("KILN_WEATHER_VECTORS not set: skipped");
        return;
    };
    let (mut total, mut failed) = (0, 0);
    let mut report = |name: &str, ok: bool, detail: String| {
        total += 1;
        if !ok {
            failed += 1;
            eprintln!("MISMATCH {name}: {detail}");
        }
    };
    let info = vs.iter().find(|v| v["name"] == "info").expect("info line");
    assert_eq!(info["random"], "net.minecraft.world.level.levelgen.LegacyRandomSource");
    let sea_level = info["sea_level"].as_i64().unwrap() as i32;
    let zoom_seed = kiln_worldgen::generator::obfuscate_seed(info["seed"].as_i64().unwrap());
    for v in &vs {
        let name = v["name"].as_str().unwrap();
        if name.starts_with("cycle") {
            let st = v["start"].as_array().unwrap();
            let mut w = WeatherData {
                clear_weather_time: st[0].as_i64().unwrap() as i32,
                rain_time: st[1].as_i64().unwrap() as i32,
                thunder_time: st[2].as_i64().unwrap() as i32,
                raining: st[3].as_bool().unwrap(),
                thundering: st[4].as_bool().unwrap(),
            };
            let (r, t) = (f32_of(&st[5]), f32_of(&st[6]));
            let mut l = LevelWeather { rain: r, o_rain: r, thunder: t, o_thunder: t };
            let mut random = LegacyRandom::new(st[7].as_i64().unwrap());
            for (i, tick) in v["ticks"].as_array().unwrap().iter().enumerate() {
                w.advance(&mut random);
                l.step(&w);
                let ours = state(&w, &l);
                let ok = same_state(&ours, tick);
                report(&format!("{name} tick {i}"), ok, format!("ours {ours:?} vanilla {tick}"));
                if !ok {
                    break;
                }
            }
        } else if name == "sky" {
            for s in v["samples"].as_array().unwrap() {
                let s = s.as_array().unwrap();
                let (t, r, th) = (s[0].as_i64().unwrap(), f32_of(&s[1]), f32_of(&s[2]));
                let l = LevelWeather { rain: r, o_rain: r, thunder: th, o_thunder: th };
                let ours = sky_darken(OVERWORLD_ID, t, &l);
                let theirs = s[3].as_i64().unwrap() as i32;
                report(&format!("sky {t} {r} {th}"), ours == theirs, format!("ours {ours} vanilla {theirs}"));
            }
        } else if name.starts_with("precipitation") {
            precipitation(v, sea_level, zoom_seed, &mut report);
        } else if name == "wake" {
            for (n, s) in v["samples"].as_array().unwrap().iter().enumerate() {
                let s = s.as_array().unwrap();
                let t = s[0].as_i64().unwrap();
                let moved = marker_move(t, 0);
                let after = moved.unwrap_or(t);
                let result = if moved.is_some() { "MOVED" } else { "NOT_MOVED" };
                report(&format!("wake {t}"), after == s[2].as_i64().unwrap() && result == s[1], format!("ours {result} {after} vanilla {} {}", s[1], s[2]));
                let mut w = WeatherData { raining: true, thundering: true, rain_time: 500, thunder_time: 300, ..Default::default() };
                let mut l = LevelWeather { rain: 1.0, o_rain: 1.0, thunder: 1.0, o_thunder: 1.0 };
                w.reset_cycle();
                let mut random = LegacyRandom::new(77 + n as i64);
                w.advance(&mut random);
                l.step(&w);
                let ours = state(&w, &l);
                report(&format!("wake {t} weather"), same_state(&ours, &s[3]), format!("ours {ours:?} vanilla {}", s[3]));
            }
        } else if name == "standup" {
            standup(v, &mut report);
        }
    }
    eprintln!("weather parity: {}/{} match", total - failed, total);
    assert_eq!(failed, 0, "{failed} of {total} weather vectors differ");
}

fn state_id(s: &str) -> u16 {
    kiln_blocks::state::parse_state(s).unwrap_or_else(|| panic!("unknown state {s}"))
}

/// The biome areas WeatherVectors fills (16x16 columns each, from y 96 to 112, in this
/// order; `filled` of them so far), the void elsewhere.
fn area_biome(qx: i32, qy: i32, qz: i32, filled: usize) -> &'static str {
    const AREAS: [((i32, i32), &str); 4] = [
        ((0, 0), "minecraft:snowy_plains"),
        ((1, 0), "minecraft:plains"),
        ((0, 1), "minecraft:frozen_ocean"),
        ((1, 1), "minecraft:ice_spikes"),
    ];
    if !(24..=28).contains(&qy) {
        return "minecraft:the_void";
    }
    let at = (qx.div_euclid(4), qz.div_euclid(4));
    AREAS[..filled].iter().find(|(a, _)| *a == at).map_or("minecraft:the_void", |(_, b)| b)
}

fn precipitation(v: &Value, sea_level: i32, zoom_seed: i64, report: &mut impl FnMut(&str, bool, String)) {
    let name = v["name"].as_str().unwrap();
    let (x0, z0) = (v["x0"].as_i64().unwrap() as i32, v["z0"].as_i64().unwrap() as i32);
    let biome = v["biome"].as_str().unwrap().to_owned();
    let climates = Climates::load(&datapack()).expect("datapack biomes");
    let index: usize = name.trim_start_matches("precipitation").parse().unwrap();
    let filled = (index + 1).min(4);
    let mut level = TestLevel::void();
    level.climate = Some(Box::new(move |at: BlockPos, pos| {
        let b = kiln_worldgen::generator::zoomed_biome(zoom_seed, at.x, at.y, at.z, &mut |qx, qy, qz| {
            kiln_data::synced_id("minecraft:worldgen/biome", area_biome(qx, qy, qz, filled)).unwrap() as u16
        });
        climates.climate(b, sea_level, pos)
    }));
    level.weather = kiln_blocks::weather::Weather { raining: true, thundering: false, max_snow_height: v["max_snow"].as_i64().unwrap() as i32 };
    let set = |level: &mut TestLevel, x: i32, y: i32, z: i32, s: &str| {
        level.set_raw(BlockPos::new(x0 + x, y, z0 + z), state_id(s), 0);
    };
    let fill = |level: &mut TestLevel, (xa, za): (i32, i32), (xb, zb): (i32, i32), y: i32, s: &str| {
        for x in xa..=xb {
            for z in za..=zb {
                level.set_raw(BlockPos::new(x0 + x, y, z0 + z), state_id(s), 0);
            }
        }
    };
    // The layout WeatherVectors builds.
    fill(&mut level, (0, 0), (15, 15), 99, "minecraft:stone");
    fill(&mut level, (1, 1), (5, 5), 99, "minecraft:water");
    set(&mut level, 8, 100, 1, "minecraft:cauldron");
    set(&mut level, 9, 100, 1, "minecraft:water_cauldron[level=1]");
    set(&mut level, 10, 100, 1, "minecraft:water_cauldron[level=2]");
    set(&mut level, 11, 100, 1, "minecraft:powder_snow_cauldron[level=1]");
    set(&mut level, 12, 100, 1, "minecraft:water_cauldron[level=3]");
    fill(&mut level, (8, 4), (12, 5), 100, "minecraft:snow[layers=2]");
    fill(&mut level, (8, 8), (12, 9), 100, "minecraft:stone_slab[type=bottom]");
    fill(&mut level, (1, 10), (3, 12), 100, "minecraft:glass");
    fill(&mut level, (5, 10), (6, 12), 100, "minecraft:ice");
    level.set_random_seed(v["seed"].as_i64().unwrap());
    for _ in 0..v["rounds"].as_i64().unwrap() {
        for dz in 0..16 {
            for dx in 0..16 {
                kiln_blocks::weather::tick_precipitation(&mut level, BlockPos::new(x0 + dx, 0, z0 + dz));
            }
        }
    }
    let cols = v["columns"].as_array().unwrap();
    let mut bad = Vec::new();
    for dz in 0..16 {
        for dx in 0..16 {
            let col = cols[(dz * 16 + dx) as usize].as_array().unwrap();
            for (i, y) in (99..=101).enumerate() {
                let theirs = state_id(col[i].as_str().unwrap());
                let ours = level.block(BlockPos::new(x0 + dx, y, z0 + dz));
                if ours != theirs {
                    bad.push(format!("({dx},{y},{dz}) ours {ours} vanilla {}", col[i]));
                }
            }
        }
    }
    report(name, bad.is_empty(), format!("{biome}: {} blocks differ: {:?}", bad.len(), &bad[..bad.len().min(6)]));
}

fn standup(v: &Value, report: &mut impl FnMut(&str, bool, String)) {
    let yaws: Vec<f32> = v["yaws"].as_array().unwrap().iter().map(f32_of).collect();
    for (i, sample) in v["samples"].as_array().unwrap().iter().enumerate() {
        let label = sample[0].as_str().unwrap();
        let (hx, hz) = (40 + i as i32 * 10, 0);
        let mut level = TestLevel::void();
        let fill = |level: &mut TestLevel, a: [i32; 3], b: [i32; 3], s: &str| {
            let id = state_id(s);
            for x in a[0].min(b[0])..=a[0].max(b[0]) {
                for y in a[1].min(b[1])..=a[1].max(b[1]) {
                    for z in a[2].min(b[2])..=a[2].max(b[2]) {
                        level.set_raw(BlockPos::new(x, y, z), id, 0);
                    }
                }
            }
        };
        fill(&mut level, [hx - 5, 99, hz - 5], [hx + 5, 99, hz + 5], "minecraft:stone");
        let bed = |level: &mut TestLevel| {
            level.set_raw(BlockPos::new(hx, 100, hz), state_id("minecraft:red_bed[part=head,facing=east]"), 0);
            level.set_raw(BlockPos::new(hx - 1, 100, hz), state_id("minecraft:red_bed[part=foot,facing=east]"), 0);
        };
        bed(&mut level);
        let rel = |dx: i32, dy: i32, dz: i32| [hx + dx, 100 + dy, hz + dz];
        match label {
            "wall_south" => fill(&mut level, rel(-3, 0, 1), rel(3, 1, 1), "minecraft:stone"),
            "boxed" => {
                fill(&mut level, rel(-3, 0, -1), rel(3, 1, -1), "minecraft:stone");
                fill(&mut level, rel(-3, 0, 1), rel(3, 1, 1), "minecraft:stone");
            }
            "lava_side" => fill(&mut level, rel(-3, -1, 1), rel(3, -1, 1), "minecraft:lava"),
            "closed" => {
                fill(&mut level, rel(-3, 0, -1), rel(3, 1, 1), "minecraft:stone");
                bed(&mut level);
            }
            _ => {}
        }
        for (j, &yaw) in yaws.iter().enumerate() {
            let ours = crate::sleep::bed_stand_up(&level, BlockPos::new(hx, 100, hz), Direction::East, yaw).map(|p| [p[0] - hx as f64, p[1], p[2] - hz as f64]);
            let theirs = sample[1][j].as_array().map(|a| [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]);
            report(&format!("standup {label} yaw {yaw}"), ours == theirs, format!("ours {ours:?} vanilla {theirs:?}"));
        }
    }
}
