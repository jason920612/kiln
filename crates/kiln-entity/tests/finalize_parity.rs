//! `finalizeSpawn` of natural spawns against vanilla: for each seed of the level random, the random
//! state afterwards, and the riding stack the mob makes (jockeys), each entity with its baby flag,
//! equipment and the entity it rides. The vectors come from `tools/mob_vectors.py --filter finalize
//! --out <work>/wp33/finalize.jsonl` (MobVectors.finalizeVectors); the test is skipped without them
//! (`KILN_FINALIZE_VECTORS` names another file).

use kiln_entity::mob::{self, GroupData, MobKind, Seat, SpawnContext};
use kiln_javamath::random::LegacyRandom;
use serde_json::Value;
use std::path::PathBuf;

const SLOTS: [&str; 8] = ["mainhand", "offhand", "head", "chest", "legs", "feet", "body", "saddle"];

fn vectors() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KILN_FINALIZE_VECTORS") {
        return Some(PathBuf::from(p));
    }
    let work = std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let p = work.join("wp33/finalize.jsonl");
    p.exists().then_some(p)
}

/// The item ids of `e`'s equipment slots, in [`SLOTS`] order ("" for an empty one).
fn items(e: &kiln_entity::Entity) -> Vec<String> {
    let tag = kiln_entity::persist::save(e, &|_| None);
    let eq = tag.get("equipment");
    SLOTS.iter().map(|s| eq.and_then(|q| q.get(s)).and_then(|i| i.get("id")).and_then(|i| i.as_str()).unwrap_or("").to_owned()).collect()
}

fn check(rec: &Value) -> Result<(), String> {
    let name = rec["type"].as_str().unwrap();
    let seed = rec["seed"].as_i64().unwrap();
    let kind = MobKind::by_name(name).ok_or_else(|| format!("no kind {name}"))?;
    let plains = kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains");
    let ctx = SpawnContext {
        biome: plains,
        moon_brightness: 1.0,
        special_multiplier: rec["special"].as_f64().unwrap() as f32,
        effective_difficulty: rec["eff"].as_f64().unwrap() as f32,
        hard: false,
        halloween: false,
    };
    let mut e = mob::new(kind, 1, 0, seed);
    let mut group = GroupData { camel_space: true, ..Default::default() };
    let mut rng = LegacyRandom::new(seed);
    mob::finalize_spawn(&mut e, &mut rng, &ctx, &mut group, true);
    let mut got: Vec<(String, bool, i64, Vec<String>)> = vec![(name.to_owned(), mob::data(&e).unwrap().baby(), -1, items(&e))];
    let mut vehicle = vec![-1i64; group.companions.len() + 1];
    for (j, c) in group.companions.iter().enumerate() {
        match c.seat {
            Seat::OnMob => vehicle[j + 1] = 0,
            Seat::UnderMob => vehicle[0] = j as i64 + 1,
            Seat::OnCompanion(k) => vehicle[j + 1] = k as i64 + 1,
            Seat::Loose => {}
        }
        got.push((c.entity.type_name.to_owned(), mob::data(&c.entity).unwrap().baby(), -1, items(&c.entity)));
    }
    // Only the riding stack of the mob is recorded by vanilla: a chicken a baby husk first sat
    // on is left behind when it takes the camel husk.
    let mut keep = vec![false; got.len()];
    keep[0] = true;
    loop {
        let mut changed = false;
        for i in 0..got.len() {
            if keep[i] && vehicle[i] >= 0 && !keep[vehicle[i] as usize] {
                keep[vehicle[i] as usize] = true;
                changed = true;
            }
            if !keep[i] && vehicle[i] >= 0 && keep[vehicle[i] as usize] {
                keep[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let new_index: Vec<i64> = keep.iter().scan(0i64, |n, &k| Some(if k { *n += 1; *n - 1 } else { -1 })).collect();
    for (g, v) in got.iter_mut().zip(vehicle) {
        g.2 = if v >= 0 { new_index[v as usize] } else { -1 };
    }
    let mut kept = keep.iter();
    got.retain(|_| *kept.next().unwrap());
    let want: Vec<(String, bool, i64, Vec<String>)> = rec["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|en| {
            let it = &en["items"];
            (
                en["type"].as_str().unwrap().to_owned(),
                en["baby"].as_bool().unwrap(),
                en["vehicle"].as_i64().unwrap(),
                SLOTS.iter().map(|s| it.get(s).and_then(Value::as_str).unwrap_or("").to_owned()).collect(),
            )
        })
        .collect();
    if got != want {
        return Err(format!("stack {got:?} (kiln) vs {want:?} (vanilla)"));
    }
    let lr = rec["lr"].as_i64().unwrap();
    if rng.state() != lr {
        return Err(format!("level random {} (kiln) vs {lr} (vanilla)", rng.state()));
    }
    Ok(())
}

#[test]
fn finalize_spawn_matches_vanilla() {
    let Some(path) = vectors() else {
        eprintln!("finalize parity: no vectors (run tools/mob_vectors.py --filter finalize); skipped");
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap();
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let mut per: std::collections::BTreeMap<String, (u32, u32, Option<String>)> = Default::default();
    for line in text.lines() {
        let rec: Value = serde_json::from_str(line).unwrap();
        let name = rec["type"].as_str().unwrap().to_owned();
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let entry = per.entry(name).or_default();
        match check(&rec) {
            Ok(()) => entry.0 += 1,
            Err(e) => {
                entry.1 += 1;
                entry.2.get_or_insert(format!("seed {}: {e}", rec["seed"]));
            }
        }
    }
    let mut failed = 0;
    for (name, (pass, fail, first)) in &per {
        eprintln!("{name}: {pass} identical, {fail} differ{}", first.as_ref().map(|f| format!(" ({f})")).unwrap_or_default());
        failed += fail;
    }
    assert_eq!(failed, 0, "{failed} finalizeSpawn records differ from vanilla");
}
