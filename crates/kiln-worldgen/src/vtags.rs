//! Block and fluid tags of the running server (`kiln_data::live`: the enabled data packs' tags, the vanilla ones before
//! they load), for block behaviour that names tags in code (`BlockTags.SUPPORTS_VEGETATION`...).

use crate::block_facts::{Fluid, FluidKind};
use crate::sets::BlockSet;
use std::collections::HashMap;
use std::sync::Mutex;

fn tag_ids(registry: &str, tag: &str) -> &'static [i32] {
    kiln_data::live::tag(registry, tag).unwrap_or(&[])
}

/// The states of a block tag (`minecraft:` prefix optional).
pub fn block_tag(tag: &str) -> &'static BlockSet {
    // The sets built so far, dropped when the tags change.
    static CACHE: Mutex<Option<(u64, HashMap<String, &'static BlockSet>)>> = Mutex::new(None);
    let key = if tag.contains(':') { tag.to_string() } else { format!("minecraft:{tag}") };
    let mut guard = CACHE.lock().unwrap();
    let generation = kiln_data::live::generation();
    let cache = match &mut *guard {
        Some((g, map)) if *g == generation => map,
        slot => &mut slot.insert((generation, HashMap::new())).1,
    };
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

/// Whether a state is in a block tag.
#[inline]
pub fn is(state: u16, tag: &str) -> bool {
    block_tag(tag).contains(state)
}

/// Whether a fluid state's type is in a fluid tag.
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
