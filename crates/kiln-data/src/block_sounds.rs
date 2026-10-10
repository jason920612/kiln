//! Block sounds: `BlockState.getSoundType()` for every state, extracted from the vanilla game
//! (`cargo xtask extract blocks`, `cargo xtask codegen`).
//!
//! Only the server's uses of it matter here: the sound of placing a block (`BlockItem.place`), of
//! stepping on one (`Entity.playStepSound` and the mobs that call it) and of landing on one
//! (`LivingEntity.playBlockFallSound`). Breaking and hitting a block are played by the client from
//! its own copy of this table (the server sends the level event, not the sound).

use crate::blocks::BLOCKS;
use std::sync::OnceLock;

#[path = "gen/sound_types.rs"]
mod table;
pub use table::SoundType;

struct Index {
    /// The type of each block's default state, by index in `BLOCKS`.
    of_block: Vec<u8>,
    /// For a block whose states sound differently: the property that decides and the types by value.
    by_property: Vec<Option<(&'static str, &'static [(&'static str, u8)])>>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut of_block = vec![0u8; BLOCKS.len()];
        let mut by_property = vec![None; BLOCKS.len()];
        let position: std::collections::HashMap<&str, usize> = BLOCKS.iter().enumerate().map(|(i, b)| (b.name, i)).collect();
        for &(name, t) in table::BLOCK_SOUND_TYPES {
            of_block[position[name]] = t;
        }
        for &(name, property, values) in table::SOUND_BY_PROPERTY {
            by_property[position[name]] = Some((property, values));
        }
        Index { of_block, by_property }
    })
}

/// `BlockState.getSoundType()`.
pub fn sound_type(state: u16) -> &'static SoundType {
    let block = crate::block_logic::block_index(state);
    let index = index();
    let mut t = index.of_block[block];
    if let Some((property, values)) = index.by_property[block]
        && let Some(v) = crate::blocks_types::block_of(state).property(state, property)
        && let Some(&(_, found)) = values.iter().find(|(name, _)| *name == v)
    {
        t = found;
    }
    &table::SOUND_TYPES[t as usize]
}

/// Every distinct sound type, in the order of the table.
pub fn sound_types() -> &'static [SoundType] {
    table::SOUND_TYPES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::default_state as d;

    #[test]
    fn well_known_blocks() {
        let stone = sound_type(d::STONE);
        assert_eq!((stone.volume, stone.pitch, stone.step_sound), (1.0, 1.0, "minecraft:block.stone.step"));
        assert_eq!(sound_type(d::OAK_PLANKS).place_sound, "minecraft:block.wood.place");
        assert_eq!(sound_type(d::SLIME_BLOCK).fall_sound, "minecraft:block.slime_block.fall");
        assert_eq!(sound_type(d::ANVIL).volume, 0.3);
    }

    #[test]
    fn a_cracked_pot_sounds_different() {
        let pot = d::DECORATED_POT;
        let cracked = crate::blocks_types::block_of(pot).with_property(pot, "cracked", "true").unwrap();
        assert_eq!(sound_type(pot).break_sound, "minecraft:block.decorated_pot.break");
        assert_eq!(sound_type(cracked).break_sound, "minecraft:block.decorated_pot.shatter");
    }

    #[test]
    fn every_sound_is_a_sound_event() {
        let events = crate::builtin_entries("minecraft:sound_event").expect("sound events");
        let known: std::collections::HashSet<&str> = events.iter().copied().collect();
        for t in sound_types() {
            for s in [t.break_sound, t.step_sound, t.place_sound, t.hit_sound, t.fall_sound] {
                assert!(known.contains(s), "{s} is not a sound event");
            }
        }
    }
}
