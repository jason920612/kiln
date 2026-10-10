//! The crafter (`CrafterBlock`, `CrafterBlockEntity`): nine slots, any of which can be disabled (an empty slot the
//! player switched off), a recipe it makes when it gets power, and the result it puts into the container in front
//! of it, or throws out when there is none.

use super::hopper::{View, add_item, container_at, with_target};
use super::{BeKind, ContainerBe};
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level, flags, state};
use kiln_inventory::recipe::CraftingInput;
use kiln_inventory::stack::{StackExt, same_item_same_components};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;

/// What a crafter block entity keeps besides its items.
#[derive(Debug, Clone, Default)]
pub(crate) struct Crafter {
    /// `containerData` 0..9: the slot is disabled.
    pub disabled: [bool; 9],
    /// `craftingTicksRemaining`: counts down to when the block's `crafting` shows off.
    pub ticks: i32,
    /// `containerData` 9: the block's `triggered` (the menu shows it).
    pub triggered: bool,
}

impl Crafter {
    /// `CrafterBlockEntity.loadAdditional`.
    pub fn load(nbt: &Tag) -> Crafter {
        let mut c = Crafter { ticks: nbt.get("crafting_ticks_remaining").and_then(Tag::as_i64).map_or(0, |v| v as i32), ..Crafter::default() };
        if let Some(Tag::IntArray(slots)) = nbt.get("disabled_slots") {
            for &i in slots {
                if (0..9).contains(&i) {
                    c.disabled[i as usize] = true;
                }
            }
        }
        c.triggered = nbt.get("triggered").and_then(Tag::as_i64).is_some_and(|v| v == 1);
        c
    }

    /// The fields `saveAdditional` writes besides the items (`crafting_ticks_remaining` first).
    pub fn save_head(&self, out: &mut Vec<(String, Tag)>) {
        out.push(("crafting_ticks_remaining".into(), Tag::Int(self.ticks)));
    }

    /// `addDisabledSlots` and `addTriggered`.
    pub fn save_tail(&self, out: &mut Vec<(String, Tag)>) {
        let disabled: Vec<i32> = (0..9).filter(|&i| self.disabled[i as usize]).collect();
        out.push(("disabled_slots".into(), Tag::IntArray(disabled)));
        out.push(("triggered".into(), Tag::Int(i32::from(self.triggered))));
    }
}

/// `CrafterBlockEntity.slotCanBeDisabled`: an empty slot of the nine.
fn slot_can_be_disabled(c: &ContainerBe, slot: usize) -> bool {
    slot < 9 && c.items[slot].is_empty()
}

/// `CrafterBlockEntity.setSlotState`: switches an empty slot off or on.
pub(crate) fn set_slot_state(c: &mut ContainerBe, slot: usize, enabled: bool) {
    if !slot_can_be_disabled(c, slot) {
        return;
    }
    if let Some(cr) = c.crafter.as_mut() {
        cr.disabled[slot] = !enabled;
    }
    c.mark_changed();
}

/// `CrafterBlockEntity.smallerStackExist`: a later, enabled slot holds a smaller stack of the same item (or nothing).
fn smaller_stack_exists(c: &ContainerBe, count: i32, stack: &ItemStack, slot: usize) -> bool {
    (slot + 1..9).any(|i| {
        let disabled = c.crafter.as_ref().is_some_and(|cr| cr.disabled[i]);
        if disabled {
            return false;
        }
        let item = &c.items[i];
        item.is_empty() || (item.count() < count && same_item_same_components(item, stack))
    })
}

/// `CrafterBlockEntity.canPlaceItem`: hoppers spread items over the slots, filling the earliest of the least full.
pub(crate) fn can_place_item(c: &ContainerBe, slot: usize, _stack: &ItemStack) -> bool {
    if c.crafter.as_ref().is_some_and(|cr| cr.disabled.get(slot) == Some(&true)) {
        return false;
    }
    let item = &c.items[slot];
    let count = item.count();
    if count >= item.max_stack_size() {
        return false;
    }
    if item.is_empty() {
        return true;
    }
    !smaller_stack_exists(c, count, item, slot)
}

/// `CrafterBlockEntity.getRedstoneSignal`: the slots that are full or switched off.
pub(crate) fn analog(c: &ContainerBe) -> i32 {
    (0..9).filter(|&i| !c.items[i].is_empty() || c.crafter.as_ref().is_some_and(|cr| cr.disabled[i])).count() as i32
}

/// `CrafterBlockEntity.serverTick`: the block shows `crafting` for six ticks.
pub(crate) fn tick(level: &mut RegionLevel, pos: BlockPos) {
    let Some(c) = level.blocks.containers.get_mut(pos).and_then(|c| c.crafter.as_mut()) else { return };
    let t = c.ticks - 1;
    if t < 0 {
        return;
    }
    c.ticks = t;
    if t == 0 {
        let s = level.block(pos);
        kiln_blocks::set_block(level, pos, state::set_bool(s, "crafting", false), flags::ALL);
    }
}

/// `CrafterBlock.dispenseFrom`.
pub(crate) fn dispense_from(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    let Some(c) = level.blocks.containers.get(pos) else { return };
    if c.kind != BeKind::Crafter {
        return;
    }
    let rules = level.env.menus.clone();
    let input = CraftingInput::new(3, 3, &c.items);
    let world = kiln_inventory::NoWorld;
    let found = rules.recipes.find_crafting(&input, None, &world);
    let Some(id) = found else {
        level.effect(Effect::LevelEvent { id: 1050, pos, data: 0 });
        return;
    };
    let result = rules.recipes.assemble(id, &input, &world);
    if result.is_empty() {
        level.effect(Effect::LevelEvent { id: 1050, pos, data: 0 });
        return;
    }
    if let Some(cr) = level.blocks.containers.get_mut(pos).and_then(|c| c.crafter.as_mut()) {
        cr.ticks = 6;
    }
    kiln_blocks::set_block(level, pos, state::set_bool(s, "crafting", true), flags::CLIENTS);
    let s = level.block(pos);
    let recipe = rules.recipes.id(id).to_owned();
    let taken: Vec<ItemStack> = level.blocks.containers.get(pos).map(|c| c.items.to_vec()).unwrap_or_default();
    dispense_item(level, pos, result, s, &recipe, &taken);
    // The recipe's remainders (buckets, bottles...) follow.
    for rest in rules.recipes.remaining_items(&input, &world) {
        if !rest.is_empty() {
            dispense_item(level, pos, rest, s, &recipe, &taken);
        }
    }
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        for slot in &mut c.items {
            slot.shrink_count(1);
        }
        c.mark_changed();
    }
    // `setChanged`: comparators read the slots again.
    super::hopper::changed(level, pos);
}

/// `CrafterBlock.dispenseItem`: into the container in front, a crafter taking one item at a time; what does not fit is
/// thrown out of the front face.
fn dispense_item(level: &mut RegionLevel, pos: BlockPos, stack: ItemStack, s: u16, recipe: &str, taken: &[ItemStack]) {
    let front: Direction = kiln_blocks::behaviour::container::crafter_front(s);
    let mut rest = stack.copy();
    if let Some(target) = container_at(level, pos.relative(front)) {
        let into_crafter = target.positions().iter().any(|p| level.blocks.containers.get(*p).is_some_and(|c| c.kind == BeKind::Crafter));
        let face = Some(front.opposite());
        with_target(level, &target, |dest: &mut View| {
            if into_crafter || rest.count() > 99 {
                while !rest.is_empty() {
                    let left = add_item(dest, rest.copy_with_count(1), face, None);
                    if !left.is_empty() {
                        break;
                    }
                    rest.shrink_count(1);
                }
            } else {
                while !rest.is_empty() {
                    let before = rest.count();
                    rest = add_item(dest, std::mem::take(&mut rest), face, None);
                    if before == rest.count() {
                        break;
                    }
                }
            }
        });
        for p in target.positions() {
            crate::jukebox::settle(level, p);
            let st = level.block(p);
            kiln_blocks::update::update_neighbour_for_output_signal(level, p, kiln_blocks::BlockId::of(st));
        }
    }
    if rest.is_empty() {
        return;
    }
    let st = front.step();
    let at = [pos.x as f64 + 0.5 + 0.7 * st[0] as f64, pos.y as f64 + 0.5 + 0.7 * st[1] as f64, pos.z as f64 + 0.5 + 0.7 * st[2] as f64];
    let mut rng = super::pos_random(level, pos, 5);
    super::dispense::spawn_item(level, &mut rng, rest, 6, front, at);
    // The players within 17 blocks (a box that size around the block's centre) are told the crafter crafted.
    let centre = [pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5];
    level.out.player_fx.push(crate::blocks::PlayerFx::CrafterCrafted {
        min: [centre[0] - 8.5, centre[1] - 8.5, centre[2] - 8.5],
        max: [centre[0] + 8.5, centre[1] + 8.5, centre[2] + 8.5],
        recipe: recipe.to_owned(),
        ingredients: taken.to_vec(),
    });
    level.effect(Effect::LevelEvent { id: 1049, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2010, pos, data: front as i32 });
}
