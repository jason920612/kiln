//! Checks every codec against item stacks that tools/ItemVectors.java encoded with vanilla's own
//! codecs: the committed `vectors.jsonl` (a subset covering every component type), and the full
//! corpus from `python tools/item_vectors.py corpus` (`<work>/wp2-items/corpus.jsonl`, or
//! `KILN_ITEM_CORPUS`; skipped when absent). `KILN_ITEM_ONLY=<component name>[,<name>...]`
//! restricts the corpus run to those component types.
//!
//! Per component value: network decode -> encode is byte-identical, the persistent form (NBT)
//! and hash equal vanilla's, and NBT -> typed -> network gives vanilla's bytes. Per stack: the
//! three network codecs round-trip, and the NBT form matches vanilla's both ways.

use bytes::BytesMut;
use kiln_item::{Component, ComponentId, HashedStack, ItemStack, Value, component, hash};
use kiln_proto::Reader;
use kiln_proto::nbt::{self, Tag};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn corpus() -> Option<String> {
    let path = match std::env::var_os("KILN_ITEM_CORPUS") {
        Some(p) => PathBuf::from(p),
        None => {
            let work = std::env::var_os("KILN_WORK")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
            work.join("wp2-items/corpus.jsonl")
        }
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(_) => {
            eprintln!("no item corpus at {}; run `python tools/item_vectors.py corpus`", path.display());
            None
        }
    }
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn read_nbt(hexstr: &str) -> Tag {
    let bytes = unhex(hexstr);
    let (tag, used) = nbt::read_network(&bytes).unwrap();
    assert_eq!(used, bytes.len());
    tag
}

/// Vanilla sends the hashed components in `IdentityHashMap` order; compare as sets.
fn sorted(h: &HashedStack) -> HashedStack {
    match h.clone() {
        HashedStack::Item { item, count, mut added, mut removed } => {
            added.sort_unstable();
            removed.sort_unstable();
            HashedStack::Item { item, count, added, removed }
        }
        empty => empty,
    }
}

#[derive(Default)]
struct Tally {
    ok: usize,
    fail: BTreeMap<&'static str, (usize, String)>,
}

impl Tally {
    fn check(&mut self, what: &'static str, ok: bool, detail: impl FnOnce() -> String) {
        if ok {
            self.ok += 1;
        } else {
            let e = self.fail.entry(what).or_insert_with(|| (0, detail()));
            e.0 += 1;
        }
    }
}

#[test]
fn committed_vectors() {
    let failed = check(include_str!("vectors.jsonl"), None);
    assert_eq!(failed, 0, "{failed} vector checks failed (see the table above)");
}

#[test]
fn vanilla_item_corpus() {
    let Some(text) = corpus() else { return };
    let only: Option<Vec<ComponentId>> = std::env::var("KILN_ITEM_ONLY")
        .ok()
        .map(|list| list.split(',').map(|n| component::by_name(n.trim()).expect("KILN_ITEM_ONLY")).collect());
    let failed = check(&text, only);
    assert_eq!(failed, 0, "{failed} corpus checks failed (see the table above)");
}

/// Runs every check over `text` (JSON lines), prints a per-component table and returns the
/// number of failed checks.
fn check(text: &str, only: Option<Vec<ComponentId>>) -> usize {
    let mut per_type: BTreeMap<ComponentId, Tally> = BTreeMap::new();
    let mut stacks = Tally::default();
    let mut records = 0;
    let mut values = 0;
    let (mut reordered_values, mut reordered_stacks, mut unsaveable) = (0, 0, 0);
    for line in text.lines() {
        let rec: serde_json::Value = serde_json::from_str(line).unwrap();
        records += 1;
        let desc = rec["d"].as_str().unwrap().to_owned();
        for c in rec["c"].as_array().unwrap() {
            let id = c[0].as_u64().unwrap() as ComponentId;
            if only.as_ref().is_some_and(|o| !o.contains(&id)) {
                continue;
            }
            values += 1;
            let t = per_type.entry(id).or_default();
            let wire = unhex(c[1].as_str().unwrap());
            let mut r = Reader::new(&wire);
            let decoded = Component::read(id, &mut r);
            let typed = match decoded {
                Ok(v) if r.remaining() == 0 => v,
                Ok(_) => {
                    t.check("wire decode", false, || format!("{desc}: {} trailing bytes", r.remaining()));
                    continue;
                }
                Err(e) => {
                    t.check("wire decode", false, || format!("{desc}: {e} in {}", hex(&wire)));
                    continue;
                }
            };
            let mut out = BytesMut::new();
            typed.write(&mut out);
            t.check("wire re-encode", out[..] == wire[..], || format!("{desc}:\n  vanilla {}\n  kiln    {}", hex(&wire), hex(&out)));
            let value = typed.to_value();
            if let Some(nbt_hex) = c[2].as_str() {
                if nbt_hex.starts_with('!') {
                    // Vanilla cannot save this value (e.g. an inline holder whose persistent codec
                    // only takes registry names); there is nothing to compare.
                    unsaveable += 1;
                } else {
                    let tag = read_nbt(nbt_hex);
                    let ours = value.as_ref().map(Value::to_nbt);
                    t.check("to NBT", ours.as_ref() == Some(&tag), || format!("{desc}:\n  vanilla {tag:?}\n  kiln    {ours:?}"));
                    match Component::from_value(id, &Value::from_nbt(&tag)) {
                        Ok(back) => {
                            let mut out2 = BytesMut::new();
                            back.write(&mut out2);
                            // NBT compounds do not keep map entry order, so the network form of
                            // a value read from NBT may list map entries in another order.
                            let reordered = out2.len() == wire.len()
                                && back.to_value().map(|v| hash(&v)) == value.as_ref().map(hash)
                                && back.to_value().map(|v| v.to_nbt()).as_ref() == Some(&tag);
                            if out2[..] != wire[..] && reordered {
                                reordered_values += 1;
                            }
                            t.check("NBT -> wire", out2[..] == wire[..] || reordered, || {
                                format!("{desc}:\n  vanilla {}\n  kiln    {}", hex(&wire), hex(&out2))
                            });
                        }
                        Err(e) => t.check("from NBT", false, || format!("{desc}: {e}\n  {tag:?}")),
                    }
                }
            }
            if let Some(h) = c[3].as_i64() {
                let ours = value.as_ref().map(hash);
                t.check("hash", ours == Some(h as i32), || format!("{desc}: vanilla {h}, kiln {ours:?}\n  {value:?}"));
            }
        }
        if only.is_some() {
            continue;
        }
        // Whole stacks.
        let wire = unhex(rec["w"].as_str().unwrap());
        let mut r = Reader::new(&wire);
        match ItemStack::read_optional(&mut r) {
            Ok(stack) => {
                let mut out = BytesMut::new();
                stack.write_optional(&mut out);
                stacks.check("stack wire", out[..] == wire[..] && r.remaining() == 0, || {
                    format!("{desc}:\n  vanilla {}\n  kiln    {}", hex(&wire), hex(&out))
                });
                if let Some(h) = rec["h"].as_str() {
                    let bytes = unhex(h);
                    let theirs = HashedStack::read(&mut Reader::new(&bytes));
                    let ours = HashedStack::of(&stack);
                    let same = match (&theirs, &ours) {
                        (Ok(t), Some(o)) => sorted(t) == sorted(o) && t.matches(&stack),
                        _ => false,
                    };
                    stacks.check("hashed stack", same, || format!("{desc}:\n  vanilla {theirs:?}\n  kiln    {ours:?}"));
                }
                // Transient components are not saved, so such stacks don't survive NBT.
                let transient = stack.patch().iter().any(|(id, _)| !component::is_persistent(id));
                if let Some(nbt_hex) = rec["n"].as_str().filter(|s| !s.starts_with('!')) {
                    let tag = read_nbt(nbt_hex);
                    let ours = stack.to_nbt();
                    stacks.check("stack to NBT", ours == tag, || format!("{desc}:\n  vanilla {tag:?}\n  kiln    {ours:?}"));
                    match ItemStack::from_nbt(&tag) {
                        _ if transient => {}
                        Ok(back) => {
                            stacks.check("stack NBT round trip", back.to_nbt() == tag, || desc.clone());
                            let mut out = BytesMut::new();
                            back.write_optional(&mut out);
                            let reordered = out.len() == wire.len()
                                && HashedStack::of(&back).map(|h| sorted(&h)) == HashedStack::of(&stack).map(|h| sorted(&h));
                            if out[..] != wire[..] && reordered {
                                reordered_stacks += 1;
                            }
                            stacks.check("stack NBT -> wire", out[..] == wire[..] || reordered, || {
                                format!("{desc}:\n  vanilla {}\n  kiln    {}", hex(&wire), hex(&out))
                            });
                        }
                        Err(e) => stacks.check("stack from NBT", false, || format!("{desc}: {e}")),
                    }
                }
            }
            Err(e) => stacks.check("stack wire decode", false, || format!("{desc}: {e}")),
        }
        let untrusted = unhex(rec["u"].as_str().unwrap());
        let mut r = Reader::new(&untrusted);
        match ItemStack::read_untrusted_optional(&mut r) {
            Ok(stack) => {
                let mut out = BytesMut::new();
                stack.write_untrusted_optional(&mut out);
                stacks.check("stack untrusted wire", out[..] == untrusted[..] && r.remaining() == 0, || {
                    format!("{desc}:\n  vanilla {}\n  kiln    {}", hex(&untrusted), hex(&out))
                });
            }
            Err(e) => stacks.check("stack untrusted decode", false, || format!("{desc}: {e}")),
        }
    }

    println!("{records} stacks, {values} component values");
    println!("NBT -> wire with map entries reordered: {reordered_values} values, {reordered_stacks} stacks");
    println!("values vanilla cannot save (network-only forms): {unsaveable}");
    let mut failed = 0;
    for (id, t) in &per_type {
        let bad: usize = t.fail.values().map(|(n, _)| n).sum();
        println!("{:<40} {:>6} ok {:>6} failed", component::name(*id), t.ok, bad);
        for (what, (n, example)) in &t.fail {
            println!("    {what}: {n}, e.g. {example}");
        }
        failed += bad;
    }
    let bad: usize = stacks.fail.values().map(|(n, _)| n).sum();
    println!("{:<40} {:>6} ok {:>6} failed", "whole stacks", stacks.ok, bad);
    for (what, (n, example)) in &stacks.fail {
        println!("    {what}: {n}, e.g. {example}");
    }
    failed += bad;
    failed
}
