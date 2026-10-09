//! Display entities, interactions and markers against vanilla 26.3: for every compound the vectors
//! (`tools/EntityNbtVectors.java`, `KILN_ENTITY_NBT_VECTORS`) read, the entity Kiln loads must save the compound vanilla
//! saved, send the entity data vanilla sends (the values that differ from their defaults, from index 8 on) and stand in
//! the box vanilla's stands in.

use kiln_entity::EntityKind;
use kiln_entity::ext_entity::EntityExt as _;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::EntityData;
use serde_json::Value;
use std::path::PathBuf;

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| v.as_str().map_or(f64::NAN, |s| s.parse().unwrap()))
}

/// The typed JSON of `EntityNbtVectors.tagJson`.
fn tag_of(v: &Value) -> Tag {
    let (k, x) = v.as_object().unwrap().iter().next().unwrap();
    match k.as_str() {
        "c" => Tag::Compound(x.as_object().unwrap().iter().map(|(k, v)| (k.clone(), tag_of(v))).collect()),
        "l" => Tag::List(x.as_array().unwrap().iter().map(tag_of).collect()),
        "b" => Tag::Byte(x.as_i64().unwrap() as i8),
        "s" => Tag::Short(x.as_i64().unwrap() as i16),
        "i" => Tag::Int(x.as_i64().unwrap() as i32),
        "L" => Tag::Long(x.as_str().unwrap().parse().unwrap()),
        "f" => Tag::Float(f(x) as f32),
        "d" => Tag::Double(f(x)),
        "str" => Tag::String(x.as_str().unwrap().to_owned()),
        "ia" => Tag::IntArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect()),
        "ba" => Tag::ByteArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i8).collect()),
        "la" => Tag::LongArray(x.as_array().unwrap().iter().map(|v| v.as_str().unwrap().parse().unwrap()).collect()),
        _ => panic!("tag {k}"),
    }
}

/// A tag as text with the keys of compounds sorted (vanilla saves them in an order of its own).
fn canon(t: &Tag) -> String {
    match t {
        Tag::Compound(fields) => {
            let mut v: Vec<String> = fields.iter().map(|(k, v)| format!("{k}:{}", canon(v))).collect();
            v.sort();
            format!("{{{}}}", v.join(","))
        }
        Tag::List(items) => format!("[{}]", items.iter().map(canon).collect::<Vec<_>>().join(",")),
        other => format!("{other:?}"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn no_owner(_: i32) -> Option<u128> {
    None
}

#[test]
fn data_entities_match_vanilla() {
    let work = std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let path = std::env::var_os("KILN_ENTITY_NBT_VECTORS").map(PathBuf::from).unwrap_or_else(|| work.join("wp49/entities/nbt.jsonl"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("entity nbt: no vectors at {} (tools/EntityNbtVectors.java); skipped", path.display());
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let (mut ok, mut failed) = (0, Vec::new());
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap();
        if filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let mut errors = Vec::new();
        let mut input = tag_of(&v["nbt"]);
        let Tag::Compound(fields) = &mut input else { panic!() };
        // (The vectors' entities stand where the compound says; the id is part of it.)
        let _ = fields;
        let loaded = kiln_entity::persist::load(&input, 1, 0);
        let Ok(e) = loaded else {
            errors.push(format!("did not load: {:?}", loaded.err()));
            failed.push((name.to_owned(), errors));
            continue;
        };
        let mut saved = kiln_entity::persist::save(&e, &no_owner);
        if let Tag::Compound(f) = &mut saved {
            f.retain(|(k, _)| k != "UUID");
        }
        let want = tag_of(&v["saved"]);
        if canon(&saved) != canon(&want) {
            errors.push(format!("saved\n    kiln    {}\n    vanilla {}", canon(&saved), canon(&want)));
        }
        // The entity data from index 8 on.
        let mut d = EntityData::new();
        if let EntityKind::Ext(x) = &e.kind {
            x.entity_data(&e, &mut d);
        }
        let got = hex(d.entries());
        let want_meta: String = v["meta"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).filter(|m| u8::from_str_radix(&m[..2], 16).unwrap() >= 8).collect();
        if got != want_meta {
            errors.push(format!("entity data\n    kiln    {got}\n    vanilla {want_meta}"));
        }
        let b = e.bounding_box();
        let bx: Vec<f64> = v["box"].as_array().unwrap().iter().map(f).collect();
        let got_box = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z];
        if got_box.iter().zip(&bx).any(|(a, b)| (a - b).abs() > 1e-9) {
            errors.push(format!("box {got_box:?} vs vanilla {bx:?}"));
        }
        if errors.is_empty() {
            ok += 1;
        } else {
            failed.push((name.to_owned(), errors));
        }
    }
    for (name, errors) in &failed {
        eprintln!("FAIL {name}");
        for e in errors {
            eprintln!("  {e}");
        }
    }
    eprintln!("entity nbt: {ok} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "{} cases differ from vanilla", failed.len());
}
