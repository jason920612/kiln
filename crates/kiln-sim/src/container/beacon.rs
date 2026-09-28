//! Beacons (`BeaconBlockEntity.tick`): the beam checked ten blocks a tick up to the world
//! surface (beam blocks, see-through blocks and bedrock let it through), the pyramid of
//! `#beacon_base_blocks` below counted every 80 ticks, the powers given to players in range
//! (the primary, doubled at four levels when chosen twice, and the secondary), the activate,
//! ambient and deactivate sounds, and `construct_beacon` for players nearby when it lights.
//! The powers are chosen in `BeaconMenu` ([`set_powers`]).
//!
//! Beam colors are the client's: only whether there is a beam matters here.

use crate::blocks::{PlayerFx, RegionLevel};
use kiln_blocks::{BlockPos, Level};
use kiln_data::block_logic as logic;

/// `BeaconBlockEntity.BEACON_EFFECTS`: the powers of each pyramid level.
const BEACON_EFFECTS: [&[&str]; 4] = [
    &["minecraft:speed", "minecraft:haste"],
    &["minecraft:resistance", "minecraft:jump_boost"],
    &["minecraft:strength"],
    &["minecraft:regeneration"],
];

/// A beacon's state beyond what [`ContainerBe`] holds for every block entity.
#[derive(Debug, Clone, Default)]
pub(crate) struct Beacon {
    pub levels: i32,
    /// `primaryPower` and `secondaryPower` (`minecraft:mob_effect` network ids).
    pub primary: Option<i32>,
    pub secondary: Option<i32>,
    pub last_check_y: i32,
    /// `checkingBeamSections` and `beamSections`, as the beam blocks that start each section.
    pub checking: Vec<u16>,
    pub beam: Vec<u16>,
}

impl Beacon {
    /// Reads `primary_effect` and `secondary_effect` (only beacon powers count).
    pub fn load(nbt: &kiln_proto::nbt::Tag) -> Beacon {
        let power = |key: &str| nbt.get(key).and_then(kiln_proto::nbt::Tag::as_str).and_then(filter_name);
        Beacon { primary: power("primary_effect"), secondary: power("secondary_effect"), ..Beacon::default() }
    }

    pub fn save(&self, out: &mut Vec<(String, kiln_proto::nbt::Tag)>) {
        use kiln_proto::nbt::Tag;
        let name = |id: Option<i32>| id.and_then(|i| kiln_item::registry::MOB_EFFECT.name(i));
        if let Some(n) = name(self.primary) {
            out.push(("primary_effect".into(), Tag::String(n.into())));
        }
        if let Some(n) = name(self.secondary) {
            out.push(("secondary_effect".into(), Tag::String(n.into())));
        }
        out.push(("Levels".into(), Tag::Int(self.levels)));
    }

    /// The menu's data values: levels and `BeaconMenu.encodeEffect` of each power.
    pub fn data(&self, index: usize) -> i32 {
        match index {
            0 => self.levels,
            1 => self.primary.map_or(0, |e| e + 1),
            2 => self.secondary.map_or(0, |e| e + 1),
            _ => 0,
        }
    }
}

/// `filterEffect` by name.
fn filter_name(name: &str) -> Option<i32> {
    let name = kiln_item::Identifier::parse(name)?.to_string();
    BEACON_EFFECTS.iter().any(|l| l.contains(&name.as_str())).then(|| crate::effects::effect_id(&name)).flatten()
}

/// `filterEffect` by id.
fn filter(id: Option<i32>) -> Option<i32> {
    id.filter(|&i| kiln_item::registry::MOB_EFFECT.name(i).is_some_and(|n| BEACON_EFFECTS.iter().any(|l| l.contains(&n))))
}

/// `getRequiredLevelsFor`.
fn required_levels(id: Option<i32>) -> i32 {
    let Some(i) = id else { return 0 };
    let Some(name) = kiln_item::registry::MOB_EFFECT.name(i) else { return i32::MAX };
    BEACON_EFFECTS.iter().position(|l| l.contains(&name)).map_or(i32::MAX, |p| p as i32 + 1)
}

/// `BeaconBlockEntity.validateEffects`.
pub(crate) fn valid_powers(primary: Option<i32>, secondary: Option<i32>, levels: i32) -> bool {
    if secondary.is_some() && levels < 4 {
        return false;
    }
    let (p, s) = (required_levels(primary), required_levels(secondary));
    if p > levels || s > levels || p >= 4 {
        return false;
    }
    s == 0 || s >= 4 || primary == secondary
}

/// `BeaconMenu.updateEffects` on the beacon's side: the powers chosen (after the menu checked
/// them and took the payment); the power select sound when the beam is up.
pub(crate) fn set_powers(level: &mut RegionLevel, pos: BlockPos, primary: Option<i32>, secondary: Option<i32>) {
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    let Some(b) = c.beacon.as_mut() else { return };
    b.primary = filter(primary);
    b.secondary = filter(secondary);
    let beam = !b.beam.is_empty();
    c.mark_changed();
    if beam {
        level.effect(kiln_blocks::Effect::Sound { pos, sound: "minecraft:block.beacon.power_select", volume: 1.0, pitch: 1.0 });
    }
}

/// `updateBase`: the pyramid levels (up to 4) of `#beacon_base_blocks` below.
fn update_base(level: &RegionLevel, x: i32, y: i32, z: i32) -> i32 {
    let mut levels = 0;
    for i in 1..=4 {
        let yy = y - i;
        if yy < level.min_y() {
            break;
        }
        let full = (x - i..=x + i).all(|xx| (z - i..=z + i).all(|zz| kiln_blocks::tags::is(level.block(BlockPos::new(xx, yy, zz)), "minecraft:beacon_base_blocks")));
        if !full {
            break;
        }
        levels = i;
    }
    levels
}

/// `Heightmap.Types.WORLD_SURFACE` at `(x, z)`: above the topmost block that is not air.
fn world_surface(level: &RegionLevel, x: i32, z: i32) -> i32 {
    use kiln_world::Blocks;
    let Some(chunk) = level.cells.chunk(kiln_world::ChunkPos::of_block(x, z)) else { return level.min_y() };
    chunk.column_height((x & 15) as usize, (z & 15) as usize, |s| !kiln_data::blocks_types::is_air(s))
}

/// `BeaconBlockEntity.tick` at `pos`.
pub(crate) fn tick(level: &mut RegionLevel, pos: BlockPos) {
    let Some(mut b) = level.blocks.containers.get_mut(pos).and_then(|c| c.beacon.take()) else { return };
    let (x, y, z) = (pos.x, pos.y, pos.z);
    let mut at = if b.last_check_y < y {
        b.checking.clear();
        b.last_check_y = y - 1;
        pos
    } else {
        BlockPos::new(x, b.last_check_y + 1, z)
    };
    let surface = world_surface(level, x, z);
    for _ in 0..10 {
        if at.y > surface {
            break;
        }
        let s = level.block(at);
        if logic::implements(s, logic::interface::BEACON_BEAM_BLOCK) {
            let color = logic::block_index(s) as u16;
            if b.checking.len() <= 1 || b.checking.last() != Some(&color) {
                b.checking.push(color);
            }
        } else if !b.checking.is_empty() && (kiln_data::block_props::light_dampening(s) < 15 || kiln_blocks::state::is(s, kiln_data::blocks::default_state::BEDROCK)) {
            // The section grows.
        } else {
            b.checking.clear();
            b.last_check_y = surface;
            break;
        }
        at = at.above();
        b.last_check_y += 1;
    }
    let was = b.levels;
    if level.env.game_time % 80 == 0 {
        if !b.beam.is_empty() {
            b.levels = update_base(level, x, y, z);
        }
        if b.levels > 0 && !b.beam.is_empty() {
            apply_effects(level, pos, &b);
            level.effect(kiln_blocks::Effect::Sound { pos, sound: "minecraft:block.beacon.ambient", volume: 1.0, pitch: 1.0 });
        }
    }
    if b.last_check_y >= surface {
        b.last_check_y = level.min_y() - 1;
        let was_active = was > 0;
        b.beam = b.checking.clone();
        let active = b.levels > 0;
        if !was_active && active {
            level.effect(kiln_blocks::Effect::Sound { pos, sound: "minecraft:block.beacon.activate", volume: 1.0, pitch: 1.0 });
            // `CONSTRUCT_BEACON` for the players around (the box from 4 below, 10 across, 5 up
            // and down).
            let min = [x as f64 - 10.0, (y - 4) as f64 - 5.0, z as f64 - 10.0];
            let max = [x as f64 + 10.0, y as f64 + 5.0, z as f64 + 10.0];
            level.out.player_fx.push(PlayerFx::BeaconActivated { min, max, levels: b.levels });
        } else if was_active && !active {
            level.effect(kiln_blocks::Effect::Sound { pos, sound: "minecraft:block.beacon.deactivate", volume: 1.0, pitch: 1.0 });
        }
    }
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.beacon = Some(b);
    }
}

/// `applyEffects`: the powers for players in range (levels * 10 + 10 around, up to the top of
/// the world), lasting `(9 + levels * 2) * 20` ticks, ambient.
fn apply_effects(level: &mut RegionLevel, pos: BlockPos, b: &Beacon) {
    let Some(primary) = b.primary else { return };
    let range = (b.levels * 10 + 10) as f64;
    let amplifier = i32::from(b.levels >= 4 && Some(primary) == b.secondary);
    let duration = (9 + b.levels * 2) * 20;
    let min = [pos.x as f64 - range, pos.y as f64 - range, pos.z as f64 - range];
    let max = [pos.x as f64 + 1.0 + range, pos.y as f64 + 1.0 + range + level.env.height as f64, pos.z as f64 + 1.0 + range];
    level.out.player_fx.push(PlayerFx::Effect { min, max, effect: crate::effects::Effect::new(primary, duration, amplifier, true, true, true) });
    if b.levels >= 4
        && let Some(secondary) = b.secondary
        && secondary != primary
    {
        level.out.player_fx.push(PlayerFx::Effect { min, max, effect: crate::effects::Effect::new(secondary, duration, 0, true, true, true) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powers_follow_the_levels() {
        let id = |n: &str| crate::effects::effect_id(n);
        assert!(valid_powers(id("minecraft:speed"), None, 1));
        assert!(!valid_powers(id("minecraft:strength"), None, 2));
        assert!(valid_powers(id("minecraft:strength"), None, 3));
        assert!(!valid_powers(id("minecraft:regeneration"), None, 4), "regeneration is secondary only");
        assert!(valid_powers(id("minecraft:speed"), id("minecraft:regeneration"), 4));
        assert!(valid_powers(id("minecraft:speed"), id("minecraft:speed"), 4));
        assert!(!valid_powers(id("minecraft:speed"), id("minecraft:haste"), 4));
        assert!(!valid_powers(id("minecraft:speed"), id("minecraft:regeneration"), 3));
        assert_eq!(filter(id("minecraft:poison")), None);
    }
}
