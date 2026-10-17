//! What a living entity saves as `attributes` and `Brain`, against vanilla 26.3 (`tools/SavedStateVectors.java`, `KILN_SAVED_VECTORS`): for
//! every living entity type (adult and baby) the sets of attribute instances (`AttributeMap.pack` lists every instance that exists, not
//! only the ones that were changed) and of memories (`Brain.Packed`: the memories with a codec) right after `/summon` and after 1, 2, 5, 20,
//! 100 and 300 ticks. Skipped without the vectors.
//!
//! A memory whose presence is a matter of chance in both servers (a piglin's `hunted_recently`, a vex's `home`, the cooldowns that start at a
//! random length) is compared by name only where the vector at that time and Kiln's both have it or both lack it; the report lists the rest.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use serde_json::Value;
use std::collections::BTreeSet;

fn tag_of(v: &Value) -> Tag {
    let (k, x) = v.as_object().unwrap().iter().next().unwrap();
    match k.as_str() {
        "c" => Tag::Compound(x.as_object().unwrap().iter().map(|(k, v)| (k.clone(), tag_of(v))).collect()),
        "l" => Tag::List(x.as_array().unwrap().iter().map(tag_of).collect()),
        "b" => Tag::Byte(x.as_i64().unwrap() as i8),
        "s" => Tag::Short(x.as_i64().unwrap() as i16),
        "i" => Tag::Int(x.as_i64().unwrap() as i32),
        "L" => Tag::Long(x.as_str().unwrap().parse().unwrap()),
        "f" => Tag::Float(x.as_f64().unwrap() as f32),
        "d" => Tag::Double(x.as_f64().unwrap()),
        "str" => Tag::String(x.as_str().unwrap().to_owned()),
        "ia" => Tag::IntArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect()),
        "ba" => Tag::ByteArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i8).collect()),
        "la" => Tag::LongArray(x.as_array().unwrap().iter().map(|v| v.as_str().unwrap().parse().unwrap()).collect()),
        _ => panic!("tag {k}"),
    }
}

/// The attributes saved, each as `id base [modifier ids]` (the amounts of random modifiers differ from world to world).
fn attr_set(t: Option<&Tag>) -> BTreeSet<String> {
    match t {
        Some(Tag::List(l)) => l
            .iter()
            .filter_map(|a| {
                let a = a.unwrap_list_element();
                let id = a.get("id").and_then(Tag::as_str)?;
                // (Sizes, horse stats and the reinforcement chance are drawn at random.)
                let random = matches!(id, "minecraft:spawn_reinforcements" | "minecraft:max_health" | "minecraft:movement_speed" | "minecraft:jump_strength" | "minecraft:armor" | "minecraft:attack_damage")
                    && std::env::var("KILN_SAVED_TYPE").is_ok_and(|t| matches!(t.as_str(), "horse" | "donkey" | "mule" | "llama" | "trader_llama" | "skeleton_horse" | "zombie_horse" | "slime" | "magma_cube" | "sulfur_cube" | "zombie" | "husk" | "drowned" | "zombie_villager" | "zombified_piglin"))
                    || id == "minecraft:spawn_reinforcements";
                let base = if random { "~".to_owned() } else { a.get("base").and_then(Tag::as_f64).map_or("-".to_owned(), |b| format!("{b}")) };
                let mut mods: Vec<String> = match a.get("modifiers") {
                    Some(Tag::List(m)) => m.iter().filter_map(|m| m.unwrap_list_element().get("id").and_then(Tag::as_str).map(str::to_owned)).collect(),
                    _ => Vec::new(),
                };
                mods.sort();
                Some(format!("{id} {base} {mods:?}"))
            })
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn memory_set(t: Option<&Tag>) -> BTreeSet<String> {
    match t.and_then(|b| b.get("memories")) {
        Some(Tag::Compound(m)) => m.iter().map(|(k, _)| k.clone()).collect(),
        _ => BTreeSet::new(),
    }
}

/// The types and keys of a tag with the numbers taken out (the values of timers and positions come from random draws).
fn shape(t: &Tag) -> String {
    match t {
        Tag::Compound(fields) => {
            let mut v: Vec<String> = fields.iter().map(|(k, v)| format!("{k}:{}", shape(v))).collect();
            v.sort();
            format!("{{{}}}", v.join(","))
        }
        Tag::List(items) => format!("[{}]", items.first().map(|t| shape(t.unwrap_list_element())).unwrap_or_default()),
        Tag::Byte(_) => "b".into(),
        Tag::Short(_) => "s".into(),
        Tag::Int(_) => "i".into(),
        Tag::Long(_) => "L".into(),
        Tag::Float(_) => "f".into(),
        Tag::Double(_) => "d".into(),
        Tag::String(_) => "str".into(),
        Tag::IntArray(_) => "ia".into(),
        other => format!("{other:?}"),
    }
}

fn world() -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(2, 4, None));
    let (msg, stats) = join(1, "Probe", 3);
    assert!(sim.step([
        msg,
        ToSim::Console("gamemode spectator Probe".into()),
        ToSim::Console("difficulty normal".into()),
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console("tp Probe 8 90 8".into()),
        ToSim::Console("fill 0 79 0 15 79 15 stone".into()),
        ToSim::Console("fill 0 80 0 15 85 15 air".into()),
    ]));
    let mut client = Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    (sim, client)
}

#[test]
fn saved_attributes_and_brain_match_vanilla() {
    let Some(path) = std::env::var_os("KILN_SAVED_VECTORS") else {
        eprintln!("skipped: set KILN_SAVED_VECTORS (tools/SavedStateVectors.java)");
        return;
    };
    let only = std::env::var("KILN_SAVED_ONLY").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut sim, mut client) = world();
    let mut step = |sim: &mut Sim, client: &mut Client, cmds: Vec<String>| {
        let mut inbox: Vec<ToSim> = cmds.into_iter().map(ToSim::Console).collect();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    };
    let (mut ok, mut failed): (i32, Vec<(String, String, usize)>) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["loaded"].as_bool() != Some(true) {
            continue;
        }
        let name = v["name"].as_str().unwrap();
        // SAFETY: the test is single-threaded.
        unsafe { std::env::set_var("KILN_SAVED_TYPE", v["type"].as_str().unwrap().trim_start_matches("minecraft:")) };
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let ty = v["type"].as_str().unwrap();
        let baby = name.ends_with("_baby");
        let tail = if baby { " {Age:-24000,IsBaby:1b}" } else { "" };
        step(&mut sim, &mut client, vec![format!("summon {ty} 8.5 80 8.5{tail}")]);
        let mut done = 1;
        for s in v["samples"].as_array().unwrap() {
            let t = s["t"].as_i64().unwrap() as usize;
            if t == 0 {
                continue;
            }
            while done < t {
                step(&mut sim, &mut client, vec![]);
                done += 1;
            }
            let saved = sim.entity_nbt().into_iter().rfind(|n| n.get("id").and_then(Tag::as_str) == Some(ty));
            let Some(saved) = saved else {
                failed.push((name.to_owned(), "gone in Kiln".to_owned(), t));
                break;
            };
            let vs = &s["saved"];
            let vattrs = (!vs["attributes"].is_null()).then(|| tag_of(&vs["attributes"]));
            let vbrain = (!vs["brain"].is_null()).then(|| tag_of(&vs["brain"]));
            let (ka, va) = (attr_set(saved.get("attributes")), attr_set(vattrs.as_ref()));
            let (km, vm) = (memory_set(saved.get("Brain")), memory_set(vbrain.as_ref()));
            let mut bad = Vec::new();
            if ka != va {
                bad.push(format!("attributes: Kiln-only {:?}, vanilla-only {:?}", ka.difference(&va).collect::<Vec<_>>(), va.difference(&ka).collect::<Vec<_>>()));
            }
            if km != vm {
                bad.push(format!("memories: Kiln-only {:?}, vanilla-only {:?}", km.difference(&vm).collect::<Vec<_>>(), vm.difference(&km).collect::<Vec<_>>()));
            }
            for k in km.intersection(&vm) {
                let (a, b) = (saved.get("Brain").and_then(|b| b.get("memories")).and_then(|m| m.get(k)), vbrain.as_ref().and_then(|b| b.get("memories")).and_then(|m| m.get(k)));
                if let (Some(a), Some(b)) = (a, b)
                    && shape(a) != shape(b)
                {
                    bad.push(format!("memory {k}: Kiln {}, vanilla {}", shape(a), shape(b)));
                }
            }
            if saved.get("Brain").is_some() != vbrain.is_some() {
                bad.push(format!("Brain tag: Kiln {}, vanilla {}", saved.get("Brain").is_some(), vbrain.is_some()));
            }
            if bad.is_empty() {
                ok += 1;
            } else {
                failed.push((name.to_owned(), bad.join("; "), t));
            }
        }
        step(&mut sim, &mut client, vec![format!("kill @e[type={ty}]")]);
        step(&mut sim, &mut client, vec!["kill @e[type=!player]".into()]);
    }
    let mut grouped: Vec<(String, String, Vec<usize>)> = Vec::new();
    for (n, m, t) in &failed {
        match grouped.iter_mut().find(|g| g.0 == *n && g.1 == *m) {
            Some(g) => g.2.push(*t),
            None => grouped.push((n.clone(), m.clone(), vec![*t])),
        }
    }
    for (n, m, ts) in &grouped {
        eprintln!("FAIL {n} t={ts:?}: {m}");
    }
    eprintln!("saved attributes and brain: {ok} samples match vanilla, {} differ", failed.len());
    assert!(failed.is_empty(), "{} samples differ from vanilla", failed.len());
}
