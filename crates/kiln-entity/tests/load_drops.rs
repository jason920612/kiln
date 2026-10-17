//! What an entity loads from saved data it cannot fully read, against vanilla 26.3 (`tools/LoadDropVectors.java`,
//! `KILN_LOAD_VECTORS`): the item-bearing fields vanilla saves again after loading (an unknown item is dropped from a container, an
//! inventory, an equipment slot or a villager offer; an item frame is empty; a thrown item is its default; a dropped item with no valid
//! stack is removed). Skipped without the vectors.

use kiln_proto::nbt::Tag;
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
        "f" => Tag::Float(x.as_f64().unwrap_or_else(|| x.as_str().unwrap().parse().unwrap()) as f32),
        "d" => Tag::Double(x.as_f64().unwrap_or_else(|| x.as_str().unwrap().parse().unwrap())),
        "str" => Tag::String(x.as_str().unwrap().to_owned()),
        "ia" => Tag::IntArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect()),
        "ba" => Tag::ByteArray(x.as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i8).collect()),
        "la" => Tag::LongArray(x.as_array().unwrap().iter().map(|v| v.as_str().unwrap().parse().unwrap()).collect()),
        _ => panic!("tag {k}"),
    }
}

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

/// The fields that hold items.
fn items_of(t: &Tag) -> String {
    let Tag::Compound(fields) = t else { return String::new() };
    let mut v: Vec<String> = fields
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "Items" | "Item" | "item" | "equipment" | "Inventory" | "Offers" | "FireworksItem" | "weapon" | "drop_chances"))
        .map(|(k, v)| format!("{k}:{}", canon(v)))
        .collect();
    v.sort();
    v.join(",")
}

#[test]
fn entities_drop_what_vanilla_drops() {
    let Some(path) = std::env::var_os("KILN_LOAD_VECTORS") else {
        eprintln!("skipped: set KILN_LOAD_VECTORS (tools/LoadDropVectors.java)");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let (mut ok, mut skipped, mut failed) = (0, 0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["kind"].as_str() != Some("entity") {
            continue;
        }
        let name = v["name"].as_str().unwrap();
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        let mut input = tag_of(&v["nbt"]);
        if let Tag::Compound(f) = &mut input {
            f.retain(|(k, _)| k != "UUID");
        }
        let vanilla_removed = v["removed"].as_bool() == Some(true) || v["loaded"].as_bool() != Some(true);
        let loaded = kiln_entity::persist::load(&input, 1, 0);
        match loaded {
            Err(kiln_entity::persist::LoadError::NotSimulated) => {
                skipped += 1;
            }
            Err(e) if vanilla_removed => {
                let _ = e;
                ok += 1;
            }
            Err(e) => failed.push(format!("{name}: did not load ({e:?}), vanilla kept it")),
            Ok(e) => {
                if vanilla_removed {
                    failed.push(format!("{name}: loaded, vanilla removed it"));
                    continue;
                }
                let mut saved = kiln_entity::persist::save(&e, &|_| None);
                if let Tag::Compound(f) = &mut saved {
                    f.retain(|(k, _)| k != "UUID");
                }
                let want = tag_of(&v["saved"]);
                if items_of(&saved) == items_of(&want) {
                    ok += 1;
                } else {
                    failed.push(format!("{name}\n    kiln    {}\n    vanilla {}", items_of(&saved), items_of(&want)));
                }
            }
        }
    }
    for f in &failed {
        eprintln!("FAIL {f}");
    }
    eprintln!("entity loads: {ok} match vanilla, {} differ, {skipped} left to the simulation", failed.len());
    assert!(failed.is_empty(), "{} cases differ from vanilla", failed.len());
}
