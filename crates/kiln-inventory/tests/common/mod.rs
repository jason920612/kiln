//! Shared helpers for the vanilla parity tests.

#![allow(dead_code)]

use kiln_inventory::Rules;
use kiln_item::{Component, ItemStack};
use kiln_proto::Reader;
use std::path::PathBuf;
use std::sync::OnceLock;

/// `KILN_WORK`, or `<workspace>/work`.
pub fn work() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

pub fn full_parity() -> bool {
    std::env::var("KILN_PARITY").is_ok_and(|v| v == "1")
}

/// Rules loaded from the vanilla datapack in `work/generated`, if present.
pub fn rules() -> Option<&'static Rules> {
    static RULES: OnceLock<Option<Rules>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            let dir = work().join("generated");
            dir.join("data").is_dir().then(|| Rules::load(&dir).expect("load datapack"))
        })
        .as_ref()
}

pub fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

pub fn stack(hex: &str) -> ItemStack {
    let bytes = unhex(hex);
    let mut r = Reader::new(&bytes);
    let s = ItemStack::read_optional(&mut r).unwrap_or_else(|e| panic!("stack {hex}: {e:?}"));
    assert_eq!(r.remaining(), 0, "trailing bytes in stack {hex}");
    s
}

pub fn stacks(v: &serde_json::Value) -> Vec<ItemStack> {
    v.as_array().unwrap().iter().map(|h| stack(h.as_str().unwrap())).collect()
}

/// Order-insensitive form of a component (enchantment maps have no order in vanilla).
fn normalized(c: &Component) -> Component {
    match c {
        Component::Enchantments(e) => {
            let mut e = e.clone();
            e.0.sort();
            Component::Enchantments(e)
        }
        Component::StoredEnchantments(e) => {
            let mut e = e.clone();
            e.0.sort();
            Component::StoredEnchantments(e)
        }
        other => other.clone(),
    }
}

/// Vanilla's `ItemStack.matches`, with component maps compared as maps.
pub fn same(a: &ItemStack, b: &ItemStack) -> bool {
    if a.is_empty() || b.is_empty() {
        return a.is_empty() && b.is_empty();
    }
    if a.item() != b.item() || a.count() != b.count() || a.patch().len() != b.patch().len() {
        return false;
    }
    a.patch().iter().all(|(id, v)| match (v, b.patch().get(id)) {
        (None, Some(None)) => true,
        (Some(x), Some(Some(y))) => normalized(x) == normalized(y),
        _ => false,
    })
}

pub fn show(s: &ItemStack) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let mut out = format!("{}x{}", s.count(), s.item_name().trim_start_matches("minecraft:"));
    if !s.patch().is_empty() {
        out += &format!("{:?}", s.patch().iter().map(|(id, v)| (kiln_item::component::name(id), v.is_some())).collect::<Vec<_>>());
        let mut b = bytes::BytesMut::new();
        s.write_optional(&mut b);
        out += &format!(" [{}]", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
    }
    out
}
