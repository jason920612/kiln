//! Clientbound packet vectors shared by `examples/packet_vectors.rs` (which writes them for
//! `tools/packet_vectors.py` to decode with vanilla's codecs) and `tests/clientbound_golden.rs`
//! (which compares them with the vanilla-checked bytes in `testdata/clientbound.txt`).

#![allow(dead_code)]

mod common;
mod hud;
mod player;
mod world_fx;

use bytes::Bytes;
use kiln_proto::nbt::Tag;

pub fn cases() -> Cases {
    let mut c = Cases::default();
    hud::hud(&mut c);
    hud::scoreboard(&mut c);
    common::common(&mut c);
    world_fx::world_fx(&mut c);
    player::player(&mut c);
    player::entity_status(&mut c);
    c
}

/// A compound with its keys in the order vanilla re-encodes them: `CompoundTag` is a `HashMap`,
/// so small compounds iterate by bucket, `(h ^ h >>> 16) & 15` of `String.hashCode`.
pub fn compound(fields: &[(&str, Tag)]) -> Tag {
    assert!(fields.len() <= 12, "larger maps resize");
    let bucket = |k: &str| {
        let h = k.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
        (h ^ ((h as u32) >> 16) as i32) & 15
    };
    let mut fields: Vec<_> = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    fields.sort_by_key(|(k, _)| bucket(k));
    Tag::Compound(fields)
}

pub fn s(v: &str) -> Tag {
    Tag::String(v.into())
}

/// A registry entry's protocol id.
pub fn builtin(registry: &str, entry: &str) -> i32 {
    kiln_data::builtin_id(registry, entry).unwrap_or_else(|| panic!("{entry} not in {registry}"))
}

/// A synchronized (data-driven) registry entry's network id.
pub fn synced(registry: &str, entry: &str) -> i32 {
    kiln_data::synced_id(registry, entry).unwrap_or_else(|| panic!("{entry} not in {registry}"))
}

/// Expected value of a field in VanillaDump's output: exact (`path=value`) or within 1e-3
/// (`path~value`).
pub enum E {
    Is(String, String),
    Near(String, f64),
}

pub fn is(path: &str, v: impl ToString) -> E {
    E::Is(path.into(), v.to_string())
}

pub fn near(path: &str, v: f64) -> E {
    E::Near(path.into(), v)
}

pub struct Case {
    pub name: String,
    /// Packet class, relative to `net.minecraft.network.protocol.`.
    pub class: String,
    /// `<state>/minecraft:<packet>` for checking the id against packets.json.
    pub key: String,
    /// Packet id + body.
    pub packet: Bytes,
    pub expect: Vec<E>,
}

#[derive(Default)]
pub struct Cases(pub Vec<Case>);

impl Cases {
    /// A play packet in `net.minecraft.network.protocol.game`.
    pub fn play(&mut self, name: &str, class: &str, packet_name: &str, packet: Bytes, expect: Vec<E>) {
        self.add(name, class, "play", packet_name, packet, expect);
    }

    /// `class` is relative to `net.minecraft.network.protocol.`, e.g. `common.ClientboundPingPacket`.
    pub fn add(&mut self, name: &str, class: &str, state: &str, packet_name: &str, packet: Bytes, expect: Vec<E>) {
        assert!(!self.0.iter().any(|c| c.name == name), "duplicate vector {name}");
        let class = if class.contains('.') { class.to_string() } else { format!("game.{class}") };
        let key = format!("{state}/minecraft:{packet_name}");
        self.0.push(Case { name: name.into(), class, key, packet, expect });
    }
}
