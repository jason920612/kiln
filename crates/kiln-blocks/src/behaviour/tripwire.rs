//! Tripwire (`TripWireBlock`) and its hook (`TripWireHookBlock`): the string tells the hooks at
//! its ends when something steps on it, the hooks attach to each other, power and click.

use crate::level::{Effect, EntityKind, Level, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::ticks::TickPriority;
use crate::update::{set_block_and_update, update_neighbors_at};
use kiln_data::block_logic::{self as logic, Support};
use kiln_data::blocks::default_state as d;
use kiln_javamath::random::RandomSource;

/// `TripWireBlock.RECHECK_PERIOD`.
const RECHECK_PERIOD: i32 = 10;

fn is_hook(s: u16) -> bool {
    state::is(s, d::TRIPWIRE_HOOK)
}

fn is_wire(s: u16) -> bool {
    state::is(s, d::TRIPWIRE)
}

/// `TripWireBlock.shouldConnectTo`: a hook that faces back along the string, or more string.
pub fn should_connect_to(s: u16, dir: Direction) -> bool {
    if is_hook(s) { state::get_dir(s, "facing") == Some(dir.opposite()) } else { is_wire(s) }
}

/// `TripWireBlock.updateShape`: the sides follow what is beside the string.
pub fn wire_update_shape(s: u16, dir: Direction, neighbor: u16) -> u16 {
    if dir.is_horizontal() { state::set_bool(s, dir.name(), should_connect_to(neighbor, dir)) } else { s }
}

/// `TripWireBlock.onPlace`.
pub fn wire_on_place<L: Level>(level: &mut L, s: u16, pos: BlockPos, old: u16) {
    if !state::same_block(old, s) {
        update_source(level, pos, s);
    }
}

/// `TripWireBlock.affectNeighborsAfterRemoval`: a removed string tells the hooks it is powered
/// (so they notice it is gone).
pub fn wire_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if !moved_by_piston {
        update_source(level, pos, state::set_bool(s, "powered", true));
    }
}

/// `TripWireBlock.updateSource`: the hook at either end of the string (looking south and west,
/// at most 41 blocks) recomputes with this string's state.
fn update_source<L: Level>(level: &mut L, pos: BlockPos, wire: u16) {
    for dir in [Direction::South, Direction::West] {
        for i in 1..42 {
            let p = pos.relative_by(dir, i);
            let s = level.block(p);
            if is_hook(s) {
                if state::get_dir(s, "facing") == Some(dir.opposite()) {
                    calculate_state(level, p, s, false, true, i, Some(wire));
                }
                break;
            }
            if !is_wire(s) {
                break;
            }
        }
    }
}

/// `TripWireBlock.tick` (scheduled while something stands on the string): look again.
pub fn wire_tick<L: Level>(level: &mut L, pos: BlockPos) {
    let s = level.block(pos);
    if !state::get_bool(s, "powered") {
        return;
    }
    // `getShape(...).bounds().move(pos)`: the attached string is a flat strip, the loose one a
    // low slab; `getEntities(null, box)` sees every entity.
    let (y0, y1) = if state::get_bool(s, "attached") { (1.0 / 16.0, 2.5 / 16.0) } else { (0.0, 0.5) };
    let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
    let any = level.count_entities([x, y + y0, z], [x + 1.0, y + y1, z + 1.0], EntityKind::Any) > 0;
    check_pressed(level, pos, any);
}

/// `TripWireBlock.entityInside`: an idle string that something touches is pressed (when that
/// something does not ignore block triggers).
pub fn wire_entity_inside<L: Level>(level: &mut L, pos: BlockPos, ignores_triggers: bool) {
    let s = level.block(pos);
    if state::get_bool(s, "powered") || level.block_ticks().has_scheduled_tick(pos, BlockId::of(s)) {
        return;
    }
    check_pressed(level, pos, !ignores_triggers);
}

/// `TripWireBlock.checkPressed(level, pos, entities)`: powered while pressed, rechecked every 10
/// ticks while pressed and the tick after it was let go.
fn check_pressed<L: Level>(level: &mut L, pos: BlockPos, pressed: bool) {
    let mut s = level.block(pos);
    let was = state::get_bool(s, "powered");
    if pressed != was {
        s = state::set_bool(s, "powered", pressed);
        set_block_and_update(level, pos, s);
        update_source(level, pos, s);
    }
    let id = BlockId::of(s);
    if pressed {
        schedule_block_tick(level, pos, id, RECHECK_PERIOD, TickPriority::Normal);
    } else if was {
        schedule_block_tick(level, pos, id, 1, TickPriority::Normal);
    }
}

// ---------------------------------------------------------------------- the hook

/// `TripWireHookBlock.canSurvive`: a solid face behind it.
pub fn hook_can_survive<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos) -> bool {
    let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
    let behind = level.block(pos.relative(facing.opposite()));
    facing.is_horizontal() && logic::face_sturdy(behind, facing as u8, Support::Full)
}

/// `TripWireHookBlock.updateShape`: it falls off when its wall goes.
pub fn hook_update_shape<L: Level + ?Sized>(level: &L, s: u16, pos: BlockPos, dir: Direction) -> u16 {
    if Some(dir.opposite()) == state::get_dir(s, "facing") && !hook_can_survive(level, s, pos) { d::AIR } else { s }
}

/// `TripWireHookBlock.tick`: re-read the string.
pub fn hook_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    calculate_state(level, pos, s, false, true, -1, None);
}

/// `TripWireHookBlock.affectNeighborsAfterRemoval`.
pub fn hook_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos, moved_by_piston: bool) {
    if !moved_by_piston {
        on_removed(level, s, pos);
    }
}

/// `onRemoved`: the partner hook lets go of the string; a powered hook stops powering.
fn on_removed<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let attached = state::get_bool(s, "attached");
    let powered = state::get_bool(s, "powered");
    if attached || powered {
        calculate_state(level, pos, s, true, false, -1, None);
    }
    if powered {
        notify_neighbors(level, s, pos, state::get_dir(s, "facing").unwrap_or(Direction::North));
    }
}

/// `TripWireHookBlock.notifyNeighbors`: the hook and the block it hangs on.
fn notify_neighbors<L: Level>(level: &mut L, hook: u16, pos: BlockPos, dir: Direction) {
    let id = BlockId::of(hook);
    update_neighbors_at(level, pos, id);
    update_neighbors_at(level, pos.relative(dir.opposite()), id);
}

/// `TripWireHookBlock.calculateState`: walk along the string (up to 41 blocks) to the other hook,
/// work out whether the pair is attached and powered, and set both hooks and the string.
/// `attaching` is true when a hook is being removed, `search_range` the position of the string
/// whose state `wire` replaces what is in the world.
pub fn calculate_state<L: Level>(level: &mut L, pos: BlockPos, hook: u16, attaching: bool, notify: bool, search_range: i32, wire: Option<u16>) {
    let Some(direction) = state::get_dir(hook, "facing") else { return };
    let was_attached = state::get_bool(hook, "attached");
    let was_powered = state::get_bool(hook, "powered");
    let block = BlockId::of(hook);
    let mut can_attach = !attaching;
    let mut powered = false;
    let mut distance = 0;
    let mut wires: [Option<u16>; 42] = [None; 42];
    for i in 1..42i32 {
        let p = pos.relative_by(direction, i);
        let mut s = level.block(p);
        if is_hook(s) {
            if state::get_dir(s, "facing") == Some(direction.opposite()) {
                distance = i;
            }
            break;
        }
        if !is_wire(s) && i != search_range {
            wires[i as usize] = None;
            can_attach = false;
        } else {
            if i == search_range {
                s = wire.unwrap_or(s);
            }
            let armed = !state::get_bool(s, "disarmed");
            let wire_powered = state::get_bool(s, "powered");
            powered |= armed && wire_powered;
            wires[i as usize] = Some(s);
            if i == search_range {
                schedule_block_tick(level, pos, block, 10, TickPriority::Normal);
                can_attach &= armed;
            }
        }
    }
    can_attach &= distance > 1;
    powered &= can_attach;
    let attached_state = state::set_bool(state::set_bool(block.default_state(), "attached", can_attach), "powered", powered);
    if distance > 0 {
        let other = pos.relative_by(direction, distance);
        let opposite = direction.opposite();
        set_block_and_update(level, other, state::set_dir(attached_state, "facing", opposite));
        notify_neighbors(level, hook, other, opposite);
        if !is_hook(level.block(pos)) {
            on_removed(level, attached_state, pos);
            return;
        }
        emit_state(level, other, can_attach, powered, was_attached, was_powered);
    }
    emit_state(level, pos, can_attach, powered, was_attached, was_powered);
    if !attaching {
        set_block_and_update(level, pos, state::set_dir(attached_state, "facing", direction));
        if notify {
            notify_neighbors(level, hook, pos, direction);
        }
    }
    if was_attached != can_attach {
        for i in 1..distance {
            let p = pos.relative_by(direction, i);
            let Some(w) = wires[i as usize] else { continue };
            let cur = level.block(p);
            if is_wire(cur) || is_hook(cur) {
                set_block_and_update(level, p, state::set_bool(w, "attached", can_attach));
            }
        }
    }
}

/// `TripWireHookBlock.emitState`: the click and attach sounds (the detach sound draws a pitch
/// from the level random).
fn emit_state<L: Level>(level: &mut L, pos: BlockPos, attached: bool, powered: bool, was_attached: bool, was_powered: bool) {
    if powered && !was_powered {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.tripwire.click_on", volume: 0.4, pitch: 0.6 });
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_activate" });
    } else if !powered && was_powered {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.tripwire.click_off", volume: 0.4, pitch: 0.5 });
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_deactivate" });
    } else if attached && !was_attached {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.tripwire.attach", volume: 0.4, pitch: 0.7 });
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_attach" });
    } else if !attached && was_attached {
        let pitch = 1.2 / (level.random().next_float() * 0.2 + 0.9);
        level.effect(Effect::Sound { pos, sound: "minecraft:block.tripwire.detach", volume: 0.4, pitch });
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_detach" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_level::TestLevel;
    use crate::update::set_block;

    /// Something stepping on the string powers both hooks; when nothing is left on it the hooks
    /// let go again a few ticks later.
    #[test]
    fn an_entity_on_the_string_powers_the_hooks() {
        let mut level = TestLevel::flat(-64, 384, &[d::BEDROCK, d::STONE]);
        level.load_chunks((-2, -2), (2, 2));
        let y = -62;
        let (west, east) = (BlockPos::new(0, y, 0), BlockPos::new(5, y, 0));
        for p in [west.relative(Direction::West), east.relative(Direction::East)] {
            set_block(&mut level, p, d::STONE, crate::flags::ALL);
        }
        let hook = |facing: &str| state::set(d::TRIPWIRE_HOOK, "facing", facing);
        set_block(&mut level, west, hook("east"), crate::flags::ALL);
        set_block(&mut level, east, hook("west"), crate::flags::ALL);
        // The strings attach the hooks as they are placed (a hook placed by a player attaches
        // itself; `setBlock` of one does not).
        for x in 1..5 {
            set_block(&mut level, BlockPos::new(x, y, 0), d::TRIPWIRE, crate::flags::ALL);
        }
        for p in [west, east] {
            assert!(state::get_bool(level.block(p), "attached") && !state::get_bool(level.block(p), "powered"));
        }
        wire_entity_inside(&mut level, BlockPos::new(2, y, 0), false);
        for p in [west, east] {
            assert!(state::get_bool(level.block(p), "powered"), "hook at {p:?} powered");
        }
        // Nobody on it any more (TestLevel has no entities): the recheck lets go.
        for _ in 0..12 {
            level.tick(0, &[]);
        }
        for p in [west, east] {
            assert!(!state::get_bool(level.block(p), "powered"), "hook at {p:?} released");
        }
    }
}
