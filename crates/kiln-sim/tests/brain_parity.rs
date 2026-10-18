//! `Brain.memories` of loaded entities against vanilla 26.3 (`tools/BrainRoundTrip.java`, `KILN_BRAIN_VECTORS`): every living entity type is
//! summoned with all the serializable memories in its `Brain` tag, and the brain-driven types with each memory alone (with and without a
//! `ttl`), with invalid values, unknown memories and odd lifetimes. What the entity saves one and three ticks later (which memories its brain
//! registers and keeps, their values and their remaining time) must be what vanilla saves. Skipped without the vectors.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use serde_json::Value;

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

/// A tag as text with the keys of compounds sorted.
fn canon(t: &Tag) -> String {
    match t {
        Tag::Compound(fields) => {
            let mut v: Vec<String> = fields.iter().map(|(k, v)| format!("{k}:{}", canon(v))).collect();
            v.sort();
            format!("{{{}}}", v.join(","))
        }
        Tag::List(items) => format!("[{}]", items.iter().map(|t| canon(t.unwrap_list_element())).collect::<Vec<_>>().join(",")),
        other => format!("{other:?}"),
    }
}

/// The saved memories as text: the entries the case gave, with their values and lifetimes. The others (the brain made them itself, by
/// random draws and at moments that depend on them: a goat's ram cooldown) are left out; `saved_parity` has the defaults.
fn memories(brain: Option<&Tag>, nbt: &str) -> String {
    let Some(b) = brain else { return "(no Brain)".into() };
    let Some(Tag::Compound(mems)) = b.get("memories") else { return "(no memories)".into() };
    let mut v: Vec<String> = mems
        .iter()
        .filter(|(k, _)| nbt.contains(&format!("\"{k}\"")))
        .map(|(k, t)| format!("{k}:{}", canon(t)))
        .collect();
    v.sort();
    v.join(",")
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
fn loaded_brain_memories_match_vanilla() {
    let Some(path) = std::env::var_os("KILN_BRAIN_VECTORS") else {
        eprintln!("skipped: set KILN_BRAIN_VECTORS (tools/BrainRoundTrip.java)");
        return;
    };
    let only = std::env::var("KILN_BRAIN_ONLY").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut sim, mut client) = world();
    let step = |sim: &mut Sim, client: &mut Client, cmds: Vec<String>| {
        let mut inbox: Vec<ToSim> = cmds.into_iter().map(ToSim::Console).collect();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    };
    let (mut ok, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["loaded"].as_bool() != Some(true) {
            continue;
        }
        let name = v["name"].as_str().unwrap();
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let (ty, nbt) = (v["type"].as_str().unwrap(), v["nbt"].as_str().unwrap_or(""));
        step(&mut sim, &mut client, vec![format!("summon {ty} 8.5 80 8.5 {nbt}")]);
        let mut done = 1;
        if std::env::var_os("KILN_BRAIN_TRACE").is_some() {
            for t in 1..=20 {
                if let Some(s) = sim.entity_nbt().into_iter().rfind(|n| n.get("id").and_then(Tag::as_str) == Some(ty)) {
                    eprintln!("TRACE {name} t={t}: {}", memories(s.get("Brain"), nbt));
                }
                step(&mut sim, &mut client, vec![]);
            }
            done = 21;
        }
        for s in v["samples"].as_array().unwrap() {
            let t = s["t"].as_i64().unwrap() as usize;
            if t == 0 {
                continue;
            }
            while done < t {
                step(&mut sim, &mut client, vec![]);
                done += 1;
            }
            let Some(saved) = sim.entity_nbt().into_iter().rfind(|n| n.get("id").and_then(Tag::as_str) == Some(ty)) else {
                failed.push(format!("{name} t={t}: gone in Kiln"));
                break;
            };
            let vb = (!s["saved"]["brain"].is_null()).then(|| tag_of(&s["saved"]["brain"]));
            let (k, w) = (memories(saved.get("Brain"), nbt), memories(vb.as_ref(), nbt));
            if k == w {
                ok += 1;
            } else {
                failed.push(format!("{name} t={t}:\n    Kiln    {k}\n    vanilla {w}"));
            }
        }
        step(&mut sim, &mut client, vec!["kill @e[type=!player]".into()]);
        step(&mut sim, &mut client, vec![]);
    }
    for f in &failed {
        eprintln!("FAIL {f}");
    }
    eprintln!("loaded brains: {ok} samples match vanilla, {} differ", failed.len());
    assert!(failed.is_empty(), "{} samples differ from vanilla", failed.len());
}
