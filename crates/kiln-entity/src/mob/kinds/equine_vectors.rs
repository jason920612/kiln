//! Horse, donkey, mule and llama breeding against vanilla 26.3 (wp63): replays the breedings that
//! tools/EquineBreedVectors.java recorded (`python tools/equine_breed.py`): parents with varied attributes and a
//! pinned random, and compares the child's kind, attribute bases, strength and coat bit for bit, and the draw
//! the random makes next (so it was used as often as vanilla's).
//!
//! Vectors: `$KILN_EQUINE_VECTORS`, else `<KILN_WORK or workspace/work>/equine-breed/vectors.jsonl`; the test is
//! skipped when they are absent.

use super::horse::State;
use super::llama;
use crate::mob::attributes::Attr;
use crate::mob::breed::breed_offspring;
use crate::mob::{self, MobData, MobKind};
use crate::{entity::Entity, memory::MemoryLevel};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::path::PathBuf;

fn kind_of(name: &str) -> MobKind {
    match name {
        "minecraft:horse" => MobKind::Horse,
        "minecraft:donkey" => MobKind::Donkey,
        "minecraft:mule" => MobKind::Mule,
        "minecraft:llama" => MobKind::Llama,
        "minecraft:trader_llama" => MobKind::TraderLlama,
        other => panic!("unexpected kind {other}"),
    }
}

fn bits(v: &Value) -> f64 {
    f64::from_bits(v.as_i64().expect("attribute bits") as u64)
}

/// A tame parent as the harness set it.
fn parent(spec: &Value, id: i32, seed: i64) -> (Entity, Box<MobData>) {
    let kind = kind_of(spec["kind"].as_str().unwrap());
    let mut e = mob::new(kind, id, id as u128, seed);
    e.random = LegacyRandom::new(seed);
    let mut m = mob::take(&mut e);
    for (a, key) in [(Attr::MaxHealth, "max_health"), (Attr::MovementSpeed, "speed"), (Attr::JumpStrength, "jump")] {
        m.attrs.get_mut(a).expect("attribute").base = bits(&spec[key]);
    }
    let s = crate::mob::ext::state_mut::<State>(&mut m).expect("horse state");
    s.tamed = true;
    if let Some(v) = spec["type_variant"].as_i64() {
        s.type_variant = v as i32;
    }
    if let Some(v) = spec["variant"].as_i64() {
        s.llama_variant = v as i32;
    }
    if let Some(v) = spec["strength"].as_i64() {
        llama::set_strength(&mut m, v as i32);
    }
    (e, m)
}

#[test]
fn breeding_matches_vanilla() {
    let path = std::env::var_os("KILN_EQUINE_VECTORS").map(PathBuf::from).unwrap_or_else(|| {
        let work = std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        work.join("equine-breed/vectors.jsonl")
    });
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("no equine breeding vectors at {} (python tools/equine_breed.py makes them): skipped", path.display());
        return;
    };
    let mut per_case: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let mut failures = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let (case, seed) = (v["case"].as_str().unwrap().to_owned(), v["seed"].as_i64().unwrap());
        let (mut e, mut m) = parent(&v["a"], 1, seed);
        let (_, pm) = parent(&v["b"], 2, seed + 1);
        let mut level = MemoryLevel::new(0, seed);
        let child = breed_offspring(&mut e, &mut m, &pm, &mut level);
        let next = e.random.next_long();
        let cm = mob::data(&child).expect("the child is a mob");
        let want = &v["child"];
        let mut diffs = Vec::new();
        if cm.kind != kind_of(want["kind"].as_str().unwrap()) {
            diffs.push(format!("kind {:?} != {}", cm.kind, want["kind"]));
        }
        for (a, key) in [(Attr::MaxHealth, "max_health"), (Attr::MovementSpeed, "speed"), (Attr::JumpStrength, "jump")] {
            let (got, w) = (cm.attrs.base(a), bits(&want[key]));
            if got.to_bits() != w.to_bits() {
                diffs.push(format!("{key} {got:?} != {w:?}"));
            }
        }
        let s = crate::mob::ext::state::<State>(cm).expect("horse state");
        for (key, got) in [("strength", s.strength), ("variant", s.llama_variant), ("type_variant", s.type_variant)] {
            if let Some(w) = want[key].as_i64()
                && got != w as i32
            {
                diffs.push(format!("{key} {got} != {w}"));
            }
        }
        if next != v["next"].as_i64().unwrap() {
            diffs.push(format!("the random's next draw {next} != {}", v["next"]));
        }
        let c = per_case.entry(case.clone()).or_default();
        c.0 += 1;
        if !diffs.is_empty() {
            c.1 += 1;
            if failures.len() < 12 {
                failures.push(format!("{case} seed {seed}: {}", diffs.join(", ")));
            }
        }
    }
    for (case, (n, bad)) in &per_case {
        eprintln!("{case}: {n} breedings, {bad} differ");
    }
    assert!(failures.is_empty(), "breeding differs from vanilla:\n{}", failures.join("\n"));
    assert!(per_case.len() >= 8, "all eight parent pairs are recorded");
}
