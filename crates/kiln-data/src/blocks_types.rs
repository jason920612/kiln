//! Block state model over the generated `blocks::BLOCKS` table.

use crate::blocks::{BLOCKS, STATE_COUNT};
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct BlockInfo {
    pub name: &'static str,
    pub first: u16,
    pub default: u16,
    pub last: u16,
    /// In state-id order: the last property varies fastest.
    pub properties: &'static [Property],
}

pub struct Property {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

/// Block of a state id.
pub fn block_of(state: u16) -> &'static BlockInfo {
    // One table read instead of a binary search over the blocks (this runs per block read in
    // entity ticks).
    static INDEX: OnceLock<Vec<u16>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let mut out = vec![0u16; STATE_COUNT as usize];
        for (i, b) in BLOCKS.iter().enumerate() {
            out[b.first as usize..=b.last as usize].fill(i as u16);
        }
        out
    });
    match index.get(state as usize) {
        Some(&i) => &BLOCKS[i as usize],
        None => &BLOCKS[BLOCKS.partition_point(|b| b.first <= state) - 1],
    }
}

pub fn block_by_name(name: &str) -> Option<&'static BlockInfo> {
    static INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    let index = INDEX.get_or_init(|| BLOCKS.iter().enumerate().map(|(i, b)| (b.name, i)).collect());
    index.get(name).map(|&i| &BLOCKS[i])
}

impl BlockInfo {
    /// Value index of each property for `state`.
    pub fn property_indices(&self, state: u16) -> Vec<usize> {
        let mut rest = (state - self.first) as usize;
        let mut out = vec![0; self.properties.len()];
        for (i, p) in self.properties.iter().enumerate().rev() {
            out[i] = rest % p.values.len();
            rest /= p.values.len();
        }
        out
    }

    pub fn property(&self, state: u16, name: &str) -> Option<&'static str> {
        let i = self.properties.iter().position(|p| p.name == name)?;
        Some(self.properties[i].values[self.property_indices(state)[i]])
    }

    /// `state` with property `name` set to `value`, if the block has it.
    pub fn with_property(&self, state: u16, name: &str, value: &str) -> Option<u16> {
        let pi = self.properties.iter().position(|p| p.name == name)?;
        let vi = self.properties[pi].values.iter().position(|v| *v == value)?;
        let mut idx = self.property_indices(state);
        idx[pi] = vi;
        let mut offset = 0usize;
        for (p, i) in self.properties.iter().zip(idx) {
            offset = offset * p.values.len() + i;
        }
        Some(self.first + offset as u16)
    }
}

const AIR: u8 = 1;
const FLUID: u8 = 2;

fn flags() -> &'static [u8] {
    static FLAGS: OnceLock<Vec<u8>> = OnceLock::new();
    FLAGS.get_or_init(|| {
        let mut f = vec![0u8; STATE_COUNT as usize];
        for b in BLOCKS {
            let air = matches!(b.name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air");
            // Blocks whose fluid state is always water or lava.
            let fluid = matches!(
                b.name,
                "minecraft:water"
                    | "minecraft:lava"
                    | "minecraft:bubble_column"
                    | "minecraft:kelp"
                    | "minecraft:kelp_plant"
                    | "minecraft:seagrass"
                    | "minecraft:tall_seagrass"
            );
            for s in b.first..=b.last {
                let waterlogged = b.property(s, "waterlogged") == Some("true");
                f[s as usize] = if air { AIR } else { 0 } | if fluid || waterlogged { FLUID } else { 0 };
            }
        }
        f
    })
}

/// Air, cave air or void air (not counted as blocks by the client).
pub fn is_air(state: u16) -> bool {
    flags()[state as usize] & AIR != 0
}

/// Has a non-empty fluid state (water, lava, waterlogged, ...).
pub fn has_fluid(state: u16) -> bool {
    flags()[state as usize] & FLUID != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::default_state as d;

    /// The table behind `block_of` agrees with a binary search over the blocks for every state.
    #[test]
    fn block_of_every_state() {
        for s in 0..STATE_COUNT as u16 {
            let b = block_of(s);
            assert!(b.first <= s && s <= b.last, "state {s} not in {}", b.name);
            assert_eq!(b.name, BLOCKS[BLOCKS.partition_point(|b| b.first <= s) - 1].name);
        }
    }

    #[test]
    fn properties_roundtrip() {
        let stairs = block_by_name("minecraft:oak_stairs").unwrap();
        let s = stairs.with_property(stairs.default, "facing", "east").unwrap();
        let s = stairs.with_property(s, "waterlogged", "true").unwrap();
        assert_eq!(stairs.property(s, "facing"), Some("east"));
        assert_eq!(stairs.property(s, "waterlogged"), Some("true"));
        assert_eq!(block_of(s).name, "minecraft:oak_stairs");
        assert!(has_fluid(s) && !is_air(s));
        assert!(is_air(d::AIR) && is_air(d::CAVE_AIR) && !has_fluid(d::STONE));
        assert!(has_fluid(d::WATER));
    }
}
