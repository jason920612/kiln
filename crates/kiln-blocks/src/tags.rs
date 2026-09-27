//! Block tags from the generated registry data (`BlockState.is(TagKey)`).

use kiln_data::block_logic as logic;
use kiln_data::blocks::BLOCKS;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Membership of every block tag, indexed by block index; built once.
fn tags() -> &'static HashMap<&'static str, Box<[bool]>> {
    static TAGS: OnceLock<HashMap<&'static str, Box<[bool]>>> = OnceLock::new();
    TAGS.get_or_init(|| {
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        let index: Vec<Option<u16>> = names.iter().map(|n| crate::state::BlockId::by_name(n).map(|b| b.0)).collect();
        kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:block")
            .map_or(&[][..], |(_, t)| *t)
            .iter()
            .map(|&(name, ids)| {
                let mut members = vec![false; BLOCKS.len()].into_boxed_slice();
                for &id in ids {
                    if let Some(Some(b)) = index.get(id as usize) {
                        members[*b as usize] = true;
                    }
                }
                (name, members)
            })
            .collect()
    })
}

/// Whether `state`'s block is in the block tag `name` (e.g. `minecraft:fences`); false for
/// unknown tags.
pub fn is(state: u16, name: &str) -> bool {
    tags().get(name).is_some_and(|t| t[logic::block_index(state)])
}

pub fn washed_away_by_fluids(state: u16) -> bool {
    is(state, "minecraft:washed_away_by_fluids")
}

pub fn enables_bubble_column(state: u16) -> bool {
    is(state, "minecraft:enables_bubble_column_drag_down") || is(state, "minecraft:enables_bubble_column_push_up")
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
        assert!(!super::is(d::STONE, "minecraft:no_such_tag"));
    }
}
