//! Vanilla block and fluid tags as compiled into `kiln-data`, for block behaviour that names
//! tags in code (`BlockTags.SUPPORTS_VEGETATION`...).

use crate::block_facts::{Fluid, FluidKind};
use crate::sets::BlockSet;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn tag_ids(registry: &str, tag: &str) -> &'static [i32] {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == registry)
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .map_or(&[][..], |(_, ids)| *ids)
}

/// The states of a vanilla block tag (`minecraft:` prefix optional).
pub fn block_tag(tag: &str) -> &'static BlockSet {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static BlockSet>>> = OnceLock::new();
    let key = if tag.contains(':') { tag.to_string() } else { format!("minecraft:{tag}") };
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(s) = cache.get(&key) {
        return s;
    }
    let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
    let mut set = BlockSet::empty();
    for &id in tag_ids("minecraft:block", &key) {
        if let Some(name) = names.get(id as usize) {
            let _ = set.insert_block(name);
        }
    }
    let set: &'static BlockSet = Box::leak(Box::new(set));
    cache.insert(key, set);
    set
}

/// Whether a state is in a vanilla block tag.
#[inline]
pub fn is(state: u16, tag: &str) -> bool {
    block_tag(tag).contains(state)
}

/// Whether a fluid state's type is in a vanilla fluid tag.
pub fn fluid_is(f: Fluid, tag: &str) -> bool {
    let key = if tag.contains(':') { tag.to_string() } else { format!("minecraft:{tag}") };
    let names = kiln_data::builtin_entries("minecraft:fluid").unwrap_or(&[]);
    let name = match f.kind {
        FluidKind::Empty => "minecraft:empty",
        FluidKind::FlowingWater => "minecraft:flowing_water",
        FluidKind::Water => "minecraft:water",
        FluidKind::FlowingLava => "minecraft:flowing_lava",
        FluidKind::Lava => "minecraft:lava",
    };
    tag_ids("minecraft:fluid", &key).iter().any(|&id| names.get(id as usize) == Some(&name))
}
