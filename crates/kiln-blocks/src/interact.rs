//! Players using and breaking blocks: `useWithoutItem` of the blocks that react to a click
//! (levers, buttons, doors, trapdoors, fence gates, repeaters, comparators, note blocks) and
//! the block half of `ServerPlayerGameMode.destroyBlock` (`playerWillDestroy`, the removal,
//! the drops).
//!
//! Blocks that open menus (chests, furnaces, crafting tables, ...) are the caller's.

use crate::level::{Effect, Level, flags, schedule_fluid_tick};
use crate::pos::{BlockPos, Direction};
use crate::redstone::{components, diode};
use crate::state::{self, BlockId};
use crate::update::{remove_block, set_block, set_block_and_update};
use crate::FluidType;
use kiln_data::block_logic::{self as logic, BlockClass as C, FluidKind};
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

/// The player acting on a block.
#[derive(Clone, Copy, Debug)]
pub struct Actor {
    /// Where the player looks (for fence gates opening away from it).
    pub yaw: f32,
    /// `Abilities.mayBuild` (not adventure or spectator mode).
    pub may_build: bool,
    /// `Player.preventsBlockDrops` (creative mode).
    pub creative: bool,
}

/// Items a note block lets through to be placed on top (`#note_block_top_instruments`).
const NOTE_BLOCK_TOP_INSTRUMENTS: [&str; 7] = [
    "minecraft:zombie_head",
    "minecraft:skeleton_skull",
    "minecraft:creeper_head",
    "minecraft:dragon_head",
    "minecraft:wither_skeleton_skull",
    "minecraft:piglin_head",
    "minecraft:player_head",
];

/// `BlockBehaviour.useItemOn` of the clicked block for `item` (`None`: empty hand): whether it
/// skips its `useWithoutItem` so the item is used directly (`InteractionResult.PASS`).
pub fn passes_to_item(state: u16, item: Option<&str>, face: Direction) -> bool {
    logic::is_instance(state, C::NoteBlock) && face == Direction::Up && item.is_some_and(|i| NOTE_BLOCK_TOP_INSTRUMENTS.contains(&i))
}

/// `BlockBehaviour.useWithoutItem`: returns whether the click was consumed (vanilla's
/// `consumesAction`); if not, the held item gets used.
pub fn use_without_item<L: Level>(level: &mut L, pos: BlockPos, actor: &Actor) -> bool {
    let s = level.block(pos);
    if logic::is_instance(s, C::LeverBlock) {
        components::pull_lever(level, pos, s);
        true
    } else if logic::is_instance(s, C::ButtonBlock) {
        if !state::get_bool(s, "powered") {
            components::press_button(level, pos, s);
            if let Some(sound) = set_sound(s, Sounds::Button) {
                level.effect(Effect::ActorSound { pos, sound, volume: 1.0, pitch: 1.0 });
            }
        }
        true
    } else if logic::is_instance(s, C::DoorBlock) {
        if !logic::params(s).open_by_hand {
            return false;
        }
        let s = cycle(s, "open");
        set_block(level, pos, s, flags::CLIENTS | flags::IMMEDIATE);
        open_close_effects(level, pos, s, Sounds::Door);
        true
    } else if logic::is_instance(s, C::TrapDoorBlock) {
        if !logic::params(s).open_by_hand {
            return false;
        }
        let s = cycle(s, "open");
        set_block(level, pos, s, flags::CLIENTS);
        if state::get_bool(s, "waterlogged") {
            schedule_fluid_tick(level, pos, FluidType::Water, 5);
        }
        open_close_effects(level, pos, s, Sounds::Trapdoor);
        true
    } else if logic::is_instance(s, C::FenceGateBlock) {
        let s = if state::get_bool(s, "open") {
            state::set_bool(s, "open", false)
        } else {
            let facing = Direction::from_yaw(actor.yaw as f64);
            let s = if state::get_dir(s, "facing") == Some(facing.opposite()) { state::set_dir(s, "facing", facing) } else { s };
            state::set_bool(s, "open", true)
        };
        set_block(level, pos, s, flags::CLIENTS | flags::IMMEDIATE);
        open_close_effects(level, pos, s, Sounds::FenceGate);
        true
    } else if logic::is_instance(s, C::RepeaterBlock) {
        if !actor.may_build {
            return false;
        }
        set_block_and_update(level, pos, cycle(s, "delay"));
        true
    } else if logic::is_instance(s, C::ComparatorBlock) {
        if !actor.may_build {
            return false;
        }
        let s = cycle(s, "mode");
        let pitch = if state::get(s, "mode") == Some("subtract") { 0.55 } else { 0.5 };
        level.effect(Effect::ActorSound { pos, sound: "minecraft:block.comparator.click", volume: 0.3, pitch });
        set_block(level, pos, s, flags::CLIENTS);
        let now = level.block(pos);
        if state::same_block(now, s) {
            diode::refresh_comparator(level, pos, now);
        }
        true
    } else if logic::is_instance(s, C::NoteBlock) {
        let s = cycle(s, "note");
        set_block_and_update(level, pos, s);
        crate::redstone::devices::play_note(level, s, pos);
        true
    } else {
        false
    }
}

/// The block half of `ServerPlayerGameMode.destroyBlock`: `playerWillDestroy` (break
/// particles for everyone else, the other half of doors and tall plants removed without
/// drops in creative), `Level.removeBlock`, then the drops when `drops` (survival with the
/// right tool; `playerDestroy`). Returns whether the block was removed.
pub fn player_destroy<L: Level>(level: &mut L, pos: BlockPos, actor: &Actor, drops: bool) -> bool {
    let s = level.block(pos);
    let double_plant = logic::is_instance(s, C::DoublePlantBlock);
    if logic::is_instance(s, C::DoorBlock) {
        if actor.creative || !drops {
            prevent_drop_from_bottom_part(level, pos, s);
        }
    } else if double_plant {
        if actor.creative {
            prevent_drop_from_bottom_part(level, pos, s);
        } else if drops {
            // `DoublePlantBlock` drops here; its `playerDestroy` drops nothing.
            level.effect(Effect::Drop { pos, state: s });
        }
    }
    level.effect(Effect::ActorLevelEvent { id: 2001, pos, data: s as i32 });
    level.effect(Effect::BlockGameEvent { pos, event: "minecraft:block_destroy", state: s });
    let removed = remove_block(level, pos, false);
    if removed && drops && !actor.creative && !double_plant {
        level.effect(Effect::Drop { pos, state: s });
    }
    removed
}

/// `DoublePlantBlock.preventDropFromBottomPart`: breaking the upper half of a door or tall
/// plant removes the lower half first, so it does not pop off with drops.
fn prevent_drop_from_bottom_part<L: Level>(level: &mut L, pos: BlockPos, s: u16) {
    if state::get(s, "half") != Some("upper") {
        return;
    }
    let below = pos.below();
    let b = level.block(below);
    if state::same_block(b, s) && state::get(b, "half") == Some("lower") {
        let replacement = if logic::fluid(b).kind == FluidKind::Water { d::WATER } else { d::AIR };
        set_block(level, below, replacement, flags::NEIGHBORS | flags::CLIENTS | flags::SUPPRESS_DROPS);
        level.effect(Effect::ActorLevelEvent { id: 2001, pos: below, data: b as i32 });
    }
}

/// `BlockState.cycle`: the property's next value, wrapping around.
fn cycle(s: u16, property: &str) -> u16 {
    let info = BlockId::of(s).info();
    let Some(p) = info.properties.iter().find(|p| p.name == property) else { return s };
    let Some(i) = state::value_index(s, property) else { return s };
    state::set(s, property, p.values[(i + 1) % p.values.len()])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sounds {
    Door,
    Trapdoor,
    FenceGate,
    Button,
}

fn open_close_effects<L: Level>(level: &mut L, pos: BlockPos, s: u16, kind: Sounds) {
    let open = state::get_bool(s, "open");
    let pitch = level.random().next_float() * 0.1 + 0.9;
    if let Some(sound) = set_sound(s, kind).map(|open_sound| if open { open_sound } else { close_sound(open_sound) }) {
        level.effect(Effect::ActorSound { pos, sound, volume: 1.0, pitch });
    }
    level.effect(Effect::GameEvent { pos, event: if open { "minecraft:block_open" } else { "minecraft:block_close" } });
}

/// The block set type's sound family, from the block name (`BlockSetType`, `WoodType`).
fn family(s: u16) -> &'static str {
    let name = BlockId::of(s).name();
    if name.contains("copper") {
        "copper"
    } else if name.contains("cherry") {
        "cherry_wood"
    } else if name.contains("bamboo") {
        "bamboo_wood"
    } else if name.contains("crimson") || name.contains("warped") {
        "nether_wood"
    } else if name.contains("stone") {
        "stone"
    } else {
        "wooden"
    }
}

/// The open (or click-on) sound of the block's set type.
fn set_sound(s: u16, kind: Sounds) -> Option<&'static str> {
    Some(match (kind, family(s)) {
        (Sounds::Door, "copper") => "minecraft:block.copper_door.open",
        (Sounds::Door, "cherry_wood") => "minecraft:block.cherry_wood_door.open",
        (Sounds::Door, "bamboo_wood") => "minecraft:block.bamboo_wood_door.open",
        (Sounds::Door, "nether_wood") => "minecraft:block.nether_wood_door.open",
        (Sounds::Door, _) => "minecraft:block.wooden_door.open",
        (Sounds::Trapdoor, "copper") => "minecraft:block.copper_trapdoor.open",
        (Sounds::Trapdoor, "cherry_wood") => "minecraft:block.cherry_wood_trapdoor.open",
        (Sounds::Trapdoor, "bamboo_wood") => "minecraft:block.bamboo_wood_trapdoor.open",
        (Sounds::Trapdoor, "nether_wood") => "minecraft:block.nether_wood_trapdoor.open",
        (Sounds::Trapdoor, _) => "minecraft:block.wooden_trapdoor.open",
        (Sounds::FenceGate, "cherry_wood") => "minecraft:block.cherry_wood_fence_gate.open",
        (Sounds::FenceGate, "bamboo_wood") => "minecraft:block.bamboo_wood_fence_gate.open",
        (Sounds::FenceGate, "nether_wood") => "minecraft:block.nether_wood_fence_gate.open",
        (Sounds::FenceGate, _) => "minecraft:block.fence_gate.open",
        (Sounds::Button, "stone") => "minecraft:block.stone_button.click_on",
        (Sounds::Button, "cherry_wood") => "minecraft:block.cherry_wood_button.click_on",
        (Sounds::Button, "bamboo_wood") => "minecraft:block.bamboo_wood_button.click_on",
        (Sounds::Button, "nether_wood") => "minecraft:block.nether_wood_button.click_on",
        (Sounds::Button, _) => "minecraft:block.wooden_button.click_on",
    })
}

/// The close sound matching an open sound from [`set_sound`].
fn close_sound(open: &'static str) -> &'static str {
    match open {
        "minecraft:block.copper_door.open" => "minecraft:block.copper_door.close",
        "minecraft:block.cherry_wood_door.open" => "minecraft:block.cherry_wood_door.close",
        "minecraft:block.bamboo_wood_door.open" => "minecraft:block.bamboo_wood_door.close",
        "minecraft:block.nether_wood_door.open" => "minecraft:block.nether_wood_door.close",
        "minecraft:block.wooden_door.open" => "minecraft:block.wooden_door.close",
        "minecraft:block.copper_trapdoor.open" => "minecraft:block.copper_trapdoor.close",
        "minecraft:block.cherry_wood_trapdoor.open" => "minecraft:block.cherry_wood_trapdoor.close",
        "minecraft:block.bamboo_wood_trapdoor.open" => "minecraft:block.bamboo_wood_trapdoor.close",
        "minecraft:block.nether_wood_trapdoor.open" => "minecraft:block.nether_wood_trapdoor.close",
        "minecraft:block.wooden_trapdoor.open" => "minecraft:block.wooden_trapdoor.close",
        "minecraft:block.cherry_wood_fence_gate.open" => "minecraft:block.cherry_wood_fence_gate.close",
        "minecraft:block.bamboo_wood_fence_gate.open" => "minecraft:block.bamboo_wood_fence_gate.close",
        "minecraft:block.nether_wood_fence_gate.open" => "minecraft:block.nether_wood_fence_gate.close",
        _ => "minecraft:block.fence_gate.close",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_level::TestLevel;

    fn level() -> TestLevel {
        let mut level = TestLevel::flat(-64, 384, &[d::STONE; 4]);
        level.load_chunks((-1, -1), (1, 1));
        level
    }

    const ACTOR: Actor = Actor { yaw: 0.0, may_build: true, creative: true };

    #[test]
    fn doors_open_both_halves_and_levers_power() {
        let mut level = level();
        let door = crate::placement::BlockItem::of_item("minecraft:oak_door").unwrap();
        let ctx = crate::placement::PlaceContext {
            hit: BlockPos::new(0, -61, 0),
            face: Direction::Up,
            click: [0.5, -60.0, 0.5],
            yaw: 0.0,
            pitch: 30.0,
            sneaking: false,
        };
        let (p, _) = crate::placement::place(&mut level, &door, &ctx).unwrap();
        assert!(use_without_item(&mut level, p, &ACTOR));
        assert!(state::get_bool(level.block(p), "open"));
        assert!(state::get_bool(level.block(p.above()), "open"));
        assert!(level.effects.iter().any(|e| matches!(e, Effect::ActorSound { sound: "minecraft:block.wooden_door.open", .. })));
        // Breaking the upper half in creative takes the lower half first, without drops (the
        // upper half then pops off through its shape update; the door's loot table only drops
        // for lower halves).
        level.effects.clear();
        player_destroy(&mut level, p.above(), &ACTOR, false);
        assert!(state::is(level.block(p), d::AIR) && state::is(level.block(p.above()), d::AIR));
        let lower_drop = |e: &Effect| matches!(e, Effect::Drop { state, .. } if state::get(*state, "half") == Some("lower"));
        assert!(!level.effects.iter().any(lower_drop));
        // Iron doors ignore hands.
        crate::update::set_block(&mut level, p, d::IRON_DOOR, flags::ALL);
        assert!(!use_without_item(&mut level, p, &ACTOR));
    }

    #[test]
    fn repeaters_cycle_and_need_build_rights() {
        let mut level = level();
        let pos = BlockPos::new(2, -60, 2);
        crate::update::set_block(&mut level, pos, d::REPEATER, flags::ALL);
        assert!(use_without_item(&mut level, pos, &ACTOR));
        assert_eq!(state::get_int(level.block(pos), "delay"), 2);
        assert!(!use_without_item(&mut level, pos, &Actor { may_build: false, ..ACTOR }));
        assert_eq!(state::get_int(level.block(pos), "delay"), 2);
    }
}
