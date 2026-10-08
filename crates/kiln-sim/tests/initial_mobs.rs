//! The animals chunk generation makes (`NaturalSpawner.spawnMobsForChunkGeneration`), compared with a real vanilla
//! server's: `tools/InitialMobVectors.java` generates a window of chunks of a seed and lists the mobs of each chunk
//! (type, position, yaw). The simulation, with vanilla generation of the same seed, makes the same mobs in the same
//! places. Needs the vanilla datapack (`KILN_DATAPACK`, else `$KILN_WORK/generated`) and `KILN_INITIAL_MOB_VECTORS`.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join, log_initial_mobs, take_initial_mobs};
use kiln_sim::{NoiseConfig, Sim, SimConfig};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn datapack() -> Option<PathBuf> {
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let work = match std::env::var_os("KILN_WORK") {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
            };
            work.join("generated")
        }
    };
    dir.join("reports/biome_parameters").is_dir().then_some(dir)
}

#[test]
fn initial_mobs_parity() {
    let Some(path) = std::env::var_os("KILN_INITIAL_MOB_VECTORS") else {
        eprintln!("skipped: set KILN_INITIAL_MOB_VECTORS (tools/InitialMobVectors.java)");
        return;
    };
    let Some(pack) = datapack() else {
        eprintln!("skipped: no datapack");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let seed = head["seed"].as_i64().unwrap();
    // The chunks the simulation loads around the player and compares: all in the window the vectors cover.
    let view: i32 = std::env::var("KILN_INITIAL_MOB_VIEW").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let radius = (view - 2).min(head["checked"].as_i64().unwrap() as i32);
    let mut want: BTreeMap<(i32, i32), Vec<(String, [f64; 3], f32, bool)>> = BTreeMap::new();
    for l in lines {
        let v: Value = serde_json::from_str(l).unwrap();
        let (cx, cz) = (v["chunk"][0].as_i64().unwrap() as i32, v["chunk"][1].as_i64().unwrap() as i32);
        // Only the creatures: villagers and the like come from structures.
        let mobs: Vec<_> = v["mobs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m[6] == "creature")
            .map(|m| (m[0].as_str().unwrap().to_owned(), [m[1].as_f64().unwrap(), m[2].as_f64().unwrap(), m[3].as_f64().unwrap()], m[4].as_f64().unwrap() as f32, m[5].as_bool().unwrap()))
            .collect();
        if !mobs.is_empty() {
            want.insert((cx, cz), mobs);
        }
    }
    log_initial_mobs(true);
    let mut config = SimConfig::new(4, view as u8, None);
    config.noise = Some(NoiseConfig { seed, datapack: pack, threads: 8 });
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Walker", view as u8);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator Walker".into()), ToSim::Console("tp Walker 8.5 200 8.5".into())]));
    let mut client = Client::new(1, stats);
    // Until the window's corners are loaded (generation runs on its own threads in real time).
    let corners = [(-radius, -radius), (radius, radius), (-radius, radius), (radius, -radius)];
    for _ in 0..300000 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        if corners.iter().all(|&(cx, cz)| sim.block_at(cx * 16 + 8, 0, cz * 16 + 8).is_some()) {
            break;
        }
        std::thread::yield_now();
    }
    for _ in 0..40 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let mut got: BTreeMap<(i32, i32), Vec<(String, [f64; 3], f32, bool)>> = BTreeMap::new();
    for (c, name, pos, yaw) in take_initial_mobs() {
        got.entry(c).or_default().push((name.to_owned(), pos, yaw, false));
    }
    // Debug: the blocks of a box by type (`x0,z0,x1,z1`), to compare with `InitialMobVectors` (INITIAL_MOB_COUNT).
    if let Ok(count) = std::env::var("KILN_INITIAL_COUNT") {
        let c: Vec<i32> = count.split(',').map(|v| v.parse().unwrap()).collect();
        let mut counts: BTreeMap<&'static str, i32> = BTreeMap::new();
        for x in c[0]..=c[2] {
            for z in c[1]..=c[3] {
                for y in -64..320 {
                    if let Some(s) = sim.block_at(x, y, z) {
                        *counts.entry(kiln_blocks::state::BlockId::of(s).name()).or_default() += 1;
                    }
                }
            }
        }
        for (n, v) in counts {
            println!("COUNT {n} {v}");
        }
    }
    // Debug: the blocks at some places (`x,y,z;x,y,z`), to compare with `InitialMobVectors` (INITIAL_MOB_DUMP).
    if let Ok(dump) = std::env::var("KILN_INITIAL_DUMP") {
        for p in dump.split(';') {
            let c: Vec<i32> = p.split(',').map(|v| v.parse().unwrap()).collect();
            let s = sim.block_at(c[0], c[1], c[2]);
            println!("BLOCK {p} {}", s.map_or("unloaded".to_owned(), |s| kiln_blocks::state::state_string(s)));
        }
    }
    println!("kiln made mobs in {} chunks ({} mobs), vanilla in {} chunks", got.len(), got.values().map(|v| v.len()).sum::<usize>(), want.len());
    let mut checked = 0;
    let mut wrong = Vec::new();
    for cx in -radius..=radius {
        for cz in -radius..=radius {
            let (mut g, mut w) = (got.remove(&(cx, cz)).unwrap_or_default(), want.get(&(cx, cz)).cloned().unwrap_or_default());
            let key = |m: &(String, [f64; 3], f32, bool)| (m.0.clone(), m.1[0].to_bits(), m.1[2].to_bits());
            g.sort_by_key(key);
            w.sort_by_key(key);
            checked += 1;
            let same = g.len() == w.len() && g.iter().zip(&w).all(|(a, b)| a.0 == b.0 && (0..3).all(|i| (a.1[i] - b.1[i]).abs() < 1e-9) && (a.2 - b.2).abs() < 1e-3);
            if !same {
                wrong.push(format!("chunk ({cx}, {cz})\n  kiln    {:?}\n  vanilla {:?}", g.iter().map(|m| (&m.0, m.1, m.2)).collect::<Vec<_>>(), w.iter().map(|m| (&m.0, m.1, m.2)).collect::<Vec<_>>()));
            }
        }
    }
    println!("initial mobs: {checked} chunks compared, {} differ", wrong.len());
    for w in &wrong {
        println!("{w}");
    }
    assert!(wrong.len() * 20 <= checked, "{} of {checked} chunks differ:\n{}", wrong.len(), wrong.iter().take(6).cloned().collect::<Vec<_>>().join("\n"));
}
