//! Block and fluid tags from the generated registry data (`BlockState.is(TagKey)`).

use kiln_data::block_logic as logic;
use kiln_data::blocks::BLOCKS;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Membership of a block tag, indexed by block index.
struct Tag(Vec<bool>);

fn load(name: &str) -> Tag {
    let mut members = vec![false; BLOCKS.len()];
    let ids = kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:block")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == name))
        .map_or(&[][..], |(_, ids)| *ids);
    let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
    for &id in ids {
        if let Some(b) = names.get(id as usize).and_then(|n| crate::state::BlockId::by_name(n)) {
            members[b.0 as usize] = true;
        }
    }
    Tag(members)
}

fn tag(name: &'static str) -> &'static Tag {
    static TAGS: OnceLock<RwLock<HashMap<&'static str, &'static Tag>>> = OnceLock::new();
    let map = TAGS.get_or_init(Default::default);
    if let Some(t) = map.read().unwrap().get(name) {
        return t;
    }
    let t: &'static Tag = Box::leak(Box::new(load(name)));
    map.write().unwrap().entry(name).or_insert(t)
}

/// Whether `state`'s block is in the block tag `name` (e.g. `minecraft:fences`).
pub fn is(state: u16, name: &'static str) -> bool {
    tag(name).0[logic::block_index(state)]
}

pub fn washed_away_by_fluids(state: u16) -> bool {
    is(state, "minecraft:washed_away_by_fluids")
}

pub fn enables_bubble_column(state: u16) -> bool {
    is(state, "minecraft:enables_bubble_column_drag_down") || is(state, "minecraft:enables_bubble_column_push_up")
}

pub fn blocks_lava_fire_spread(state: u16) -> bool {
    is(state, "minecraft:blocks_lava_fire_spread")
}

#[cfg(test)]
mod tests {
    use kiln_data::blocks::default_state as d;

    #[test]
    fn membership() {
        assert!(super::is(d::OAK_FENCE, "minecraft:fences") && super::is(d::OAK_FENCE, "minecraft:wooden_fences"));
        assert!(super::is(d::NETHER_BRICK_FENCE, "minecraft:fences") && !super::is(d::NETHER_BRICK_FENCE, "minecraft:wooden_fences"));
        assert!(super::washed_away_by_fluids(d::AIR) && super::washed_away_by_fluids(d::TORCH));
        assert!(!super::washed_away_by_fluids(d::STONE));
    }
}
