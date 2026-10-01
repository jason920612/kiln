//! What chest and hopper minecarts carry (`AbstractMinecartContainer` with `ContainerEntity`):
//! 27 or 5 slots that read and save like a chest's, an unopened loot table that rolls the
//! first time anything looks inside, and the item entities and containers a hopper minecart
//! pulls from.

use crate::entity::{Entity, EntityKind};
use crate::ext_entity::minecart::Minecart;
use crate::level::{EntityFilter, EntityLevel};
use crate::math::{Aabb, BlockPos, Vec3, floor};
use crate::persist::{Input, Output};
use kiln_inventory::persist::ItemList;
use kiln_inventory::stack::{StackExt, same_item_same_components};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// The slots of a container minecart, and its loot table while it has not been rolled.
#[derive(Clone, Debug)]
pub struct Contents {
    pub items: Vec<ItemStack>,
    /// Saved `Items` entries that did not decode, written back unchanged.
    undecoded: Vec<(i32, Tag)>,
    /// `LootTable` not yet rolled (`ContainerEntity.getContainerLootTable`).
    pub loot_table: Option<String>,
    pub loot_seed: i64,
}

impl Contents {
    pub fn new(size: usize) -> Contents {
        Contents { items: vec![ItemStack::empty(); size], undecoded: Vec::new(), loot_table: None, loot_seed: 0 }
    }

    /// `readChestVehicleSaveData`: a loot table replaces the saved items.
    pub fn load(r: &mut Input, size: usize) -> Contents {
        let table = r.get("LootTable").and_then(Tag::as_str).and_then(kiln_item::ident::Identifier::parse).map(|i| i.to_string());
        let seed = r.get("LootTableSeed").and_then(Tag::as_i64).unwrap_or(0);
        let saved = r.get("Items");
        let list = if table.is_none() { ItemList::load(saved, size) } else { ItemList::load(None, size) };
        Contents { items: list.stacks, undecoded: list.undecoded, loot_table: table, loot_seed: seed }
    }

    /// `addChestVehicleSaveData`.
    pub fn save(&self, o: &mut Output) {
        match &self.loot_table {
            Some(table) => {
                o.put("LootTable", Tag::String(table.clone()));
                if self.loot_seed != 0 {
                    o.put("LootTableSeed", Tag::Long(self.loot_seed));
                }
            }
            None => o.put("Items", ItemList { stacks: self.items.clone(), undecoded: self.undecoded.clone() }.save()),
        }
    }

    /// `unpackChestVehicleLootTable`: the unrolled loot table fills the empty slots (once).
    pub fn unpack(&mut self, level: &mut dyn EntityLevel, origin: Vec3, player: Option<i32>) {
        if let Some(table) = self.loot_table.take() {
            level.fill_container_loot(&mut self.items, &table, self.loot_seed, origin, player);
        }
    }

    /// `Container.isEmpty` (of the slots as they are).
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(ItemStack::is_empty)
    }

    /// `AbstractContainerMenu.getRedstoneSignalFromContainer`: how full the slots are, 0 to 15.
    pub fn signal(&self) -> i32 {
        if self.items.is_empty() {
            return 0;
        }
        let mut f = 0.0f32;
        for s in self.items.iter().filter(|s| !s.is_empty()) {
            // `min(Container.getMaxStackSize(), stack.getMaxStackSize())`.
            f += s.count() as f32 / 99.min(s.max_stack_size()) as f32;
        }
        f /= self.items.len() as f32;
        // `Mth.lerpDiscrete(f, 0, 15)`.
        (f * 14.0).floor() as i32 + i32::from(f > 0.0)
    }

    /// `HopperBlockEntity.tryMoveInItem` for a plain container (`canPlaceItem` always holds):
    /// moves as much of `stack` as fits into `slot`; returns what is left.
    fn try_move_in(&mut self, mut stack: ItemStack, slot: usize) -> ItemStack {
        let in_slot = &self.items[slot];
        if in_slot.is_empty() {
            let max = 99.min(stack.max_stack_size());
            self.items[slot] = stack;
            self.items[slot].limit_size(max);
            return ItemStack::empty();
        }
        // `canMergeItems`.
        if in_slot.count() <= in_slot.max_stack_size() && same_item_same_components(in_slot, &stack) {
            let room = stack.max_stack_size() - in_slot.count();
            let n = stack.count().min(room);
            stack.shrink_count(n);
            self.items[slot].grow_count(n);
        }
        stack
    }

    /// `HopperBlockEntity.addItem(null, container, stack, null)`: into every slot in order.
    pub fn add_stack(&mut self, mut stack: ItemStack) -> ItemStack {
        for slot in 0..self.items.len() {
            if stack.is_empty() {
                break;
            }
            stack = self.try_move_in(stack, slot);
        }
        stack
    }

    /// `HopperBlockEntity.tryTakeInItemFromSlot` on `src` (no face rules) into `self`.
    fn take_one_from(&mut self, src: &mut [ItemStack], slot: usize) -> bool {
        let item = src[slot].clone();
        if item.is_empty() {
            return false;
        }
        let count = item.count();
        let one = src[slot].split_count(1);
        if self.add_stack(one).is_empty() {
            return true;
        }
        // `setCount(count)`: the stack stays as it was.
        let mut back = item;
        back.set_count(count);
        src[slot] = back;
        false
    }
}

/// `Containers.dropItemStack`: item entities at a random spot in the block of `at`, split
/// into stacks of 10 to 30 and flung up a little (the level's random draws the spot, sizes
/// and throws).
pub fn drop_item_stack(level: &mut dyn EntityLevel, at: Vec3, mut stack: ItemStack) {
    const WIDTH: f64 = 0.25;
    let (span, half) = (1.0 - WIDTH, WIDTH / 2.0);
    let (x, y, z) = {
        let r = level.random();
        let x = floor(at.x) as f64 + r.next_double() * span + half;
        let y = floor(at.y) as f64 + r.next_double() * span;
        let z = floor(at.z) as f64 + r.next_double() * span + half;
        (x, y, z)
    };
    while !stack.is_empty() {
        let (n, vel) = {
            let r = level.random();
            let n = r.next_int_bounded(21) + 10;
            let tri = |r: &mut dyn RandomSource, mode: f64| mode + 0.11485000171139836 * (r.next_double() - r.next_double());
            (n, Vec3::new(tri(r, 0.0), tri(r, 0.2), tri(r, 0.0)))
        };
        let part = stack.split_count(n);
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let mut item = crate::item::new_at(id, 0, part, Vec3::new(x, y, z), seed);
        item.delta = vel;
        level.add_entity(item);
    }
}

impl Minecart {
    /// `Containers.dropContents(level, this, this)`: the loot table rolls (no player), then
    /// every slot drops; the slots are empty afterwards.
    pub fn drop_contents(&mut self, e: &Entity, level: &mut dyn EntityLevel) {
        let at = e.position();
        let Some(c) = &mut self.contents else { return };
        c.unpack(level, at, None);
        for i in 0..c.items.len() {
            let stack = std::mem::take(&mut c.items[i]);
            drop_item_stack(level, at, stack);
        }
    }

    /// `MinecartHopper.tryConsumeItems`.
    pub(super) fn try_consume_items(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if !e.is_removed() && self.enabled && !self.consumed_this_frame && self.suck_in_items(e, level) {
            self.consumed_this_frame = true;
        }
    }

    /// `MinecartHopper.suckInItems`: the container above (`HopperBlockEntity.suckInItems`), or
    /// item entities above and in the minecart.
    fn suck_in_items(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) -> bool {
        let Some(mut contents) = self.contents.take() else { return false };
        let moved = suck_in(&mut contents, e, level);
        self.contents = Some(contents);
        moved
    }
}

/// The hopper part of `MinecartHopper.suckInItems`.
fn suck_in(c: &mut Contents, e: &Entity, level: &mut dyn EntityLevel) -> bool {
    let (x, y, z) = (e.x(), e.y() + 0.5, e.z());
    // `HopperBlockEntity.suckInItems`: the block above the hopper's level position.
    let above = BlockPos::containing(x, y + 1.0, z);
    if let Some(moved) = level.hopper_take_from_block(above, &mut c.items) {
        return moved;
    }
    // `getEntityContainer`: minecarts and boats around the centre of that block, one of them
    // at random.
    let (cx, cy, cz) = (above.x as f64 + 0.5, above.y as f64 + 0.5, above.z as f64 + 0.5);
    let area = Aabb::new(cx - 0.5, cy - 0.5, cz - 0.5, cx + 0.5, cy + 0.5, cz + 0.5);
    let sources: Vec<i32> = level
        .entities_in(&area, EntityFilter::Any, i32::MIN)
        .into_iter()
        .filter(|&id| id != e.id)
        .filter(|&id| level.entity(id).is_some_and(|o| !o.is_removed() && crate::ext_entity::get::<Minecart>(o).is_some_and(|m| m.contents.is_some())))
        .collect();
    if !sources.is_empty() {
        let pick = level.random().next_int_bounded(sources.len() as i32) as usize;
        unpack_other(level, sources[pick]);
        let Some(other) = level.entity_mut(sources[pick]) else { return false };
        let Some(m) = crate::ext_entity::get_mut::<Minecart>(other) else { return false };
        let Some(src) = &mut m.contents else { return false };
        for slot in 0..src.items.len() {
            if c.take_one_from(&mut src.items, slot) {
                return true;
            }
        }
        return false;
    }
    // Item entities in the block above, then in the minecart's own box.
    let suck = Aabb::new(0.0, 11.0 / 16.0, 0.0, 1.0, 2.0, 1.0).offset(x - 0.5, y - 0.5, z - 0.5);
    let items_above = level.entities_in(&suck, EntityFilter::Item, i32::MIN);
    for id in items_above {
        if add_item_entity(c, level, id) {
            return true;
        }
    }
    let own = e.bounding_box().inflate(0.25, 0.0, 0.25);
    for id in level.entities_in(&own, EntityFilter::Item, e.id) {
        if add_item_entity(c, level, id) {
            return true;
        }
    }
    false
}

/// Looking into another container minecart rolls its loot table (`getItem` unpacks).
fn unpack_other(level: &mut dyn EntityLevel, id: i32) {
    let Some(other) = level.entity_mut(id) else { return };
    let at = other.position();
    let Some(c) = crate::ext_entity::get_mut::<Minecart>(other).and_then(|m| m.contents.as_mut()) else { return };
    let Some(table) = c.loot_table.take() else { return };
    let (seed, mut items) = (c.loot_seed, std::mem::take(&mut c.items));
    level.fill_container_loot(&mut items, &table, seed, at, None);
    if let Some(c) = level.entity_mut(id).and_then(|o| crate::ext_entity::get_mut::<Minecart>(o)).and_then(|m| m.contents.as_mut()) {
        c.items = items;
    }
}

/// `HopperBlockEntity.addItem(container, itemEntity)`: true when the whole stack went in.
fn add_item_entity(c: &mut Contents, level: &mut dyn EntityLevel, id: i32) -> bool {
    let Some(item) = level.entity_mut(id) else { return false };
    if item.is_removed() {
        return false;
    }
    let EntityKind::Item(d) = &mut item.kind else { return false };
    let left = c.add_stack(d.stack.copy());
    if left.is_empty() {
        d.stack = ItemStack::empty();
        item.discard();
        true
    } else {
        d.stack = left;
        item.needs_sync = true;
        false
    }
}
