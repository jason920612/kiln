//! What a block entity loads from saved data it cannot fully read, against vanilla 26.3 (`tools/LoadDropVectors.java`, run by
//! `tools/parity_suites.py` with `KILN_LOAD_VECTORS`): the item entries of `Items`, `item`, `RecordItem` and `Book` that vanilla
//! drops (an unknown id, a slot outside the container, a missing id) go; counts out of range become 1; a component that cannot be
//! read is left off; the live block entity then saves what vanilla saved. Skipped without the vectors.

use crate::container::{BeKind, ContainerBe};
use kiln_proto::nbt::Tag;
use serde_json::Value;

/// The typed JSON of `LoadDropVectors.tagJson`.
fn tag_of(v: &Value) -> Tag {
    let (k, x) = v.as_object().unwrap().iter().next().unwrap();
    match k.as_str() {
        "c" => Tag::Compound(x.as_object().unwrap().iter().map(|(k, v)| (k.clone(), tag_of(v))).collect()),
        "l" => Tag::List(x.as_array().unwrap().iter().map(tag_of).collect()),
        "b" => Tag::Byte(x.as_i64().unwrap() as i8),
        "s" => Tag::Short(x.as_i64().unwrap() as i16),
        "i" => Tag::Int(x.as_i64().unwrap() as i32),
        "L" => Tag::Long(x.as_str().unwrap().parse().unwrap()),
        "f" => Tag::Float(x.as_f64().unwrap_or_else(|| x.as_str().unwrap().parse().unwrap()) as f32),
        "d" => Tag::Double(x.as_f64().unwrap_or_else(|| x.as_str().unwrap().parse().unwrap())),
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
        Tag::List(items) => format!("[{}]", items.iter().map(|t| canon(t.unwrap_list_element())).collect::<Vec<_>>().join(",")),
        other => format!("{other:?}"),
    }
}

/// The item-bearing fields of a saved compound.
fn items_of(t: &Tag) -> String {
    let Tag::Compound(fields) = t else { return String::new() };
    let mut v: Vec<String> = fields
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "Items" | "item" | "RecordItem" | "Book"))
        .map(|(k, v)| format!("{k}:{}", canon(v)))
        .collect();
    v.sort();
    v.join(",")
}

#[test]
fn block_entities_drop_what_vanilla_drops() {
    let Some(path) = std::env::var_os("KILN_LOAD_VECTORS") else {
        eprintln!("skipped: set KILN_LOAD_VECTORS (tools/LoadDropVectors.java)");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let (mut ok, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["kind"].as_str() != Some("block") {
            continue;
        }
        let name = v["name"].as_str().unwrap();
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        let ty = v["type"].as_str().unwrap();
        let Some(kind) = BeKind::by_type(ty.split('[').next().unwrap()) else { continue };
        let want = tag_of(&v["saved"]);
        let input = tag_of(&v["nbt"]);
        let Some(type_id) = kiln_world::block_entity::type_id(&format!("minecraft:{}", ty.split('[').next().unwrap())) else { continue };
        let c = ContainerBe::load(kind, type_id, &input);
        let got = c.save();
        if items_of(&got) == items_of(&want) {
            ok += 1;
        } else {
            failed.push(format!("{name}\n    kiln    {}\n    vanilla {}", items_of(&got), items_of(&want)));
        }
    }
    for f in &failed {
        eprintln!("FAIL {f}");
    }
    eprintln!("block entity loads: {ok} match vanilla, {} differ", failed.len());
    assert!(failed.is_empty(), "{} cases differ from vanilla", failed.len());
}
