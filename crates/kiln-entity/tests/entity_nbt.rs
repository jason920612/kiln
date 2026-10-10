//! Display entities, interactions and markers against vanilla 26.3: for every compound the vectors
//! (`tools/EntityNbtVectors.java`, `KILN_ENTITY_NBT_VECTORS`) read, the entity Kiln loads must save the compound vanilla
//! saved, send the entity data vanilla sends (the values that differ from their defaults, from index 8 on) and stand in
//! the box vanilla's stands in.

use kiln_entity::EntityKind;
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
        Tag::List(items) => format!("[{}]", items.iter().map(|t| canon(t.unwrap_list_element())).collect::<Vec<_>>().join(",")),
        other => format!("{other:?}"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The entries of an entity data list (index, serializer, value), the values that hold NBT (text components, item
/// stacks) with the order of their keys taken out: the game writes compounds in the order of its hash maps and item
/// components in the order of an identity hash.
fn entries(mut bytes: &[u8]) -> Vec<(u8, i32, String)> {
    use kiln_proto::codec::Reader;
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let mut r = Reader::new(bytes);
        let index = r.u8().unwrap();
        let serializer = r.varint().unwrap();
        let before = r.remaining();
        let value = match serializer {
            0 | 8 => hex(r.bytes(1).unwrap()),
            1 | 14 => hex(&{
                let v = r.varint().unwrap();
                v.to_le_bytes()
            }),
            3 => hex(r.bytes(4).unwrap()),
            39 => hex(r.bytes(12).unwrap()),
            40 => hex(r.bytes(16).unwrap()),
            5 => {
                let rest = r.rest();
                let (tag, used) = kiln_proto::nbt::read_network(rest).unwrap();
                r = Reader::new(&rest[used..]);
                canon(&tag)
            }
            7 => {
                let stack = kiln_item::ItemStack::read_optional(&mut r).unwrap();
                canon(&stack.to_nbt())
            }
            // OPTIONAL_COMPONENT.
            6 => {
                let present = r.bool().unwrap();
                if present {
                    let rest = r.rest();
                    let (tag, used) = kiln_proto::nbt::read_network(rest).unwrap();
                    r = Reader::new(&rest[used..]);
                    canon(&tag)
                } else {
                    "none".into()
                }
            }
            // POSE and HUMANOID_ARM.
            20 | 42 => hex(&r.varint().unwrap().to_le_bytes()),
            // RESOLVABLE_PROFILE: read through to find where it ends.
            41 => {
                let start = r.remaining();
                let all = r.rest();
                let mut p = Reader::new(all);
                let props = |p: &mut Reader| {
                    for _ in 0..p.varint().unwrap() {
                        p.string(32767).unwrap();
                        p.string(32767).unwrap();
                        if p.bool().unwrap() {
                            p.string(32767).unwrap();
                        }
                    }
                };
                if p.bool().unwrap() {
                    p.bytes(16).unwrap();
                    p.string(16).unwrap();
                    props(&mut p);
                } else {
                    if p.bool().unwrap() {
                        p.string(16).unwrap();
                    }
                    if p.bool().unwrap() {
                        p.bytes(16).unwrap();
                    }
                    props(&mut p);
                }
                for _ in 0..3 {
                    if p.bool().unwrap() {
                        p.string(32767).unwrap();
                    }
                }
                if p.bool().unwrap() {
                    p.varint().unwrap();
                }
                let used = start - p.remaining();
                r = Reader::new(&all[used..]);
                hex(&all[..used])
            }
            other => panic!("serializer {other}"),
        };
        let used = bytes.len() - r.remaining();
        let _ = before;
        out.push((index, serializer, value));
        bytes = &bytes[used..];
    }
    out
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
    let (mut ok, mut failed, mut skipped) = (0, Vec::new(), 0);
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
        // A text that needs the level (selectors, scores) is resolved by the simulation: `kiln-sim`'s display_text test.
        if kiln_entity::ext_entity::display::get(&e).is_some_and(|d| d.unresolved.is_some()) {
            skipped += 1;
            continue;
        }
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
        // (A mob type: its own fields, from the avatar's on; the living entity part is the simulation's `mobs::metadata`.)
        let mut from = 8;
        match &e.kind {
            EntityKind::Ext(x) => x.entity_data(&e, &mut d),
            EntityKind::Mob(m) => {
                from = 15;
                if let Some(k) = m.kind.ext() {
                    k.entity_data(&e, m, &mut d);
                }
            }
            _ => {}
        }
        let got: Vec<_> = entries(d.entries()).into_iter().filter(|g| g.0 >= from).collect();
        let want_bytes: Vec<u8> = v["meta"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap())
            .filter(|m| u8::from_str_radix(&m[..2], 16).unwrap() >= from)
            .flat_map(|m| (0..m.len()).step_by(2).map(|i| u8::from_str_radix(&m[i..i + 2], 16).unwrap()).collect::<Vec<_>>())
            .collect();
        let want_meta = entries(&want_bytes);
        if got != want_meta {
            errors.push(format!("entity data\n    kiln    {got:?}\n    vanilla {want_meta:?}"));
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
    eprintln!("entity nbt: {ok} passed, {} failed, {skipped} left to the simulation", failed.len());
    assert!(failed.is_empty(), "{} cases differ from vanilla", failed.len());
}
