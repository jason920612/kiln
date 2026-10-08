//! `HopperBlockEntity`: the transfer cooldown, pushing into the container it faces, pulling
//! from the container above or picking up item entities, and the redstone lock (the
//! `enabled` state, kept by kiln-blocks). Shared with droppers (`HopperBlockEntity.addItem`).

use super::{BeKind, ContainerBe};
use crate::blocks::RegionLevel;
use kiln_blocks::behaviour::container::{chest_partner, is_chest};
use kiln_blocks::{BlockPos, Direction, Level, state};
use kiln_inventory::Container;
use kiln_inventory::stack::{StackExt, same_item_same_components};
use kiln_item::ItemStack;

/// `HopperBlockEntity.MOVE_ITEM_SPEED`.
pub(crate) const MOVE_ITEM_SPEED: i32 = 8;

/// Item entities a hopper can pick up.
pub(crate) trait ItemEntities {
    /// Alive item entities whose box intersects `lo..hi`, in the level's order.
    fn items_in(&self, lo: [f64; 3], hi: [f64; 3]) -> Vec<usize>;
    fn stack_mut(&mut self, i: usize) -> &mut ItemStack;
    /// `ItemEntity.setItem` happened (an empty stack discards the entity).
    fn changed(&mut self, i: usize);
    /// Alive chest and hopper minecarts whose box meets `lo..hi`, in the level's order
    /// (`EntitySelector.CONTAINER_ENTITY_SELECTOR`).
    fn carts_in(&self, lo: [f64; 3], hi: [f64; 3]) -> Vec<usize>;
    /// The slots of minecart `i` (from [`ItemEntities::carts_in`]) and where it is.
    fn cart(&mut self, i: usize) -> Option<(&mut kiln_entity::ext_entity::minecart::Contents, [f64; 3])>;
}

/// A container a hopper moves items into or out of: one block entity, or a double chest.
pub(crate) enum Target {
    One(BlockPos),
    Two(BlockPos, BlockPos),
}

impl Target {
    pub fn positions(&self) -> Vec<BlockPos> {
        match *self {
            Target::One(p) => vec![p],
            Target::Two(a, b) => vec![a, b],
        }
    }
}

/// `HopperBlockEntity.getBlockContainer` (via `getContainerAt`): the container block entity
/// at `pos`, a chest joined with its other half (blocked chests count here). Entity containers
/// (minecarts) are not simulated.
pub(crate) fn container_at(level: &RegionLevel, pos: BlockPos) -> Option<Target> {
    let c = level.blocks.containers.get(pos)?;
    if !c.kind.is_container() {
        return None;
    }
    let s = level.block(pos);
    if is_chest(s)
        && let Some(other) = chest_partner(s, pos)
        && level.blocks.containers.get(other).is_some_and(|o| o.kind == c.kind)
        && kiln_blocks::behaviour::container::chest_can_connect_to(s, level.block(other))
    {
        // `DoubleBlockCombiner`: the right half comes first.
        return Some(if state::get(s, "type") == Some("right") { Target::Two(pos, other) } else { Target::Two(other, pos) });
    }
    Some(Target::One(pos))
}

/// Container operations with the `WorldlyContainer` and `canPlaceItem` rules hoppers follow.
pub(crate) struct View<'a> {
    parts: Vec<&'a mut ContainerBe>,
    /// Recipes and item rules (what a brewing stand accepts); without them it accepts nothing.
    rules: Option<std::sync::Arc<kiln_inventory::Rules>>,
}

impl<'a> View<'a> {
    pub fn one(c: &'a mut ContainerBe) -> Self {
        View { parts: vec![c], rules: None }
    }

    fn locate(&self, slot: usize) -> (usize, usize) {
        let mut s = slot;
        for (i, p) in self.parts.iter().enumerate() {
            if s < p.items.len() {
                return (i, s);
            }
            s -= p.items.len();
        }
        (self.parts.len() - 1, s)
    }

    pub fn size(&self) -> usize {
        self.parts.iter().map(|p| p.items.len()).sum()
    }

    pub fn item(&self, slot: usize) -> &ItemStack {
        let (i, s) = self.locate(slot);
        &self.parts[i].items[s]
    }

    pub fn item_mut(&mut self, slot: usize) -> &mut ItemStack {
        let (i, s) = self.locate(slot);
        &mut self.parts[i].items[s]
    }

    pub fn set_item(&mut self, slot: usize, stack: ItemStack) {
        let (i, s) = self.locate(slot);
        self.parts[i].set_item(s, stack);
    }

    pub fn remove_item(&mut self, slot: usize, count: i32) -> ItemStack {
        let (i, s) = self.locate(slot);
        self.parts[i].remove_item(s, count)
    }

    pub fn is_empty(&self) -> bool {
        self.parts.iter().all(|p| p.is_empty())
    }

    pub fn set_changed(&mut self) {
        for p in &mut self.parts {
            p.mark_changed();
        }
    }

    fn kind(&self) -> BeKind {
        self.parts[0].kind
    }

    /// The hopper this view is, if it is one.
    fn hopper(&mut self) -> Option<&mut ContainerBe> {
        (self.parts.len() == 1 && self.parts[0].kind == BeKind::Hopper).then(|| &mut *self.parts[0])
    }

    /// `getSlots`: a `WorldlyContainer`'s slots for the face, else every slot.
    fn slots(&self, face: Direction) -> Vec<usize> {
        match self.kind() {
            BeKind::Furnace(_) if self.parts.len() == 1 => match face {
                Direction::Down => vec![2, 1],
                Direction::Up => vec![0],
                _ => vec![1],
            },
            // `BrewingStandBlockEntity.getSlotsForFace`.
            BeKind::BrewingStand => match face {
                Direction::Up => vec![3],
                Direction::Down => vec![0, 1, 2, 3],
                _ => vec![0, 1, 2, 4],
            },
            _ => (0..self.size()).collect(),
        }
    }

    /// `Container.canPlaceItem`.
    fn can_place_item(&self, slot: usize, stack: &ItemStack) -> bool {
        match self.kind() {
            BeKind::Furnace(_) => match slot {
                2 => false,
                1 => {
                    let bucket = |s: &ItemStack| !s.is_empty() && s.item_name() == "minecraft:bucket";
                    stack.has(kiln_item::component::ids::COOKING_FUEL) || bucket(stack) && !bucket(&self.parts[0].items[1])
                }
                _ => true,
            },
            BeKind::BrewingStand => self.rules.as_deref().is_some_and(|r| super::brewing::can_place_item(&self.parts[0], slot, stack, r)),
            // `JukeboxBlockEntity.canPlaceItem`: a disc, into the empty slot.
            BeKind::Jukebox => stack.get(kiln_item::keys::JUKEBOX_PLAYABLE).is_some() && self.parts[0].items[0].is_empty(),
            _ => true,
        }
    }

    /// `canPlaceItemInContainer`: `canPlaceItem`, and for a worldly container
    /// `canPlaceItemThroughFace` (`None`: no face, as for item entities).
    fn can_place_in(&self, stack: &ItemStack, slot: usize, face: Option<Direction>) -> bool {
        if !self.can_place_item(slot, stack) {
            return false;
        }
        match (self.kind(), face) {
            (BeKind::ShulkerBox, Some(_)) => kiln_inventory::slot::can_fit_inside_container_items(stack),
            (BeKind::Furnace(_), Some(_)) => true,
            _ => true,
        }
    }

    /// `canTakeItemFromContainer`: `canTakeItem` (always) and `canTakeItemThroughFace`.
    fn can_take_from(&self, stack: &ItemStack, slot: usize, face: Direction) -> bool {
        match self.kind() {
            BeKind::Furnace(_) if face == Direction::Down && slot == 1 => {
                kiln_inventory::tags::contains("minecraft:item", "minecraft:furnace_fuel_bottom_takeable", stack.effective_item())
            }
            // `BrewingStandBlockEntity.canTakeItemThroughFace`: the ingredient slot gives up
            // only glass bottles.
            BeKind::BrewingStand if slot == 3 => !stack.is_empty() && stack.item_name() == "minecraft:glass_bottle",
            _ => true,
        }
    }

    /// `isWorldly`: a `WorldlyContainer` (face-specific slots).
    fn worldly(&self) -> bool {
        matches!(self.kind(), BeKind::Furnace(_) | BeKind::ShulkerBox | BeKind::BrewingStand) && self.parts.len() == 1
    }
}

/// Takes the target's block entities out of the store, runs `f` on them as one view, and puts
/// them back.
pub(crate) fn with_target<R>(level: &mut RegionLevel, target: &Target, f: impl FnOnce(&mut View) -> R) -> Option<R> {
    let positions = target.positions();
    let mut taken: Vec<(BlockPos, ContainerBe)> = Vec::new();
    for p in &positions {
        match level.blocks.containers.map.remove(p) {
            Some(c) => taken.push((*p, c)),
            None => {
                for (p, c) in taken {
                    level.blocks.containers.map.insert(p, c);
                }
                return None;
            }
        }
    }
    let r = {
        let mut view = View { parts: taken.iter_mut().map(|(_, c)| c).collect(), rules: Some(level.env.menus.clone()) };
        f(&mut view)
    };
    // A furnace's new input item restarts its cooking at once (`setItem`).
    let rules = level.env.menus.clone();
    for (_, c) in &mut taken {
        super::furnace::apply_input_change(c, &rules);
    }
    for (p, c) in taken {
        level.blocks.containers.map.insert(p, c);
    }
    Some(r)
}

/// `HopperBlockEntity.canMergeItems`.
fn can_merge(a: &ItemStack, b: &ItemStack) -> bool {
    a.count() <= a.max_stack_size() && same_item_same_components(a, b)
}

/// `HopperBlockEntity.tryMoveInItem`: moves as much of `stack` as fits into `slot`.
/// `source_ticked` is the game time the source hopper last ticked (`None` if the source is
/// not a hopper). Returns what is left.
fn try_move_in(dest: &mut View, mut stack: ItemStack, slot: usize, face: Option<Direction>, source_ticked: Option<i64>) -> ItemStack {
    let in_slot = dest.item(slot).clone();
    if !dest.can_place_in(&stack, slot, face) {
        return stack;
    }
    let mut moved = false;
    let was_empty = dest.is_empty();
    if in_slot.is_empty() {
        dest.set_item(slot, stack);
        stack = ItemStack::empty();
        moved = true;
    } else if can_merge(&in_slot, &stack) {
        let room = stack.max_stack_size() - in_slot.count();
        let n = stack.count().min(room);
        stack.shrink_count(n);
        dest.item_mut(slot).grow_count(n);
        moved = n > 0;
    }
    if moved {
        if was_empty
            && let Some(h) = dest.hopper()
            && h.cooldown <= MOVE_ITEM_SPEED
        {
            let k = i32::from(source_ticked.is_some_and(|t| h.ticked_game_time >= t));
            h.cooldown = MOVE_ITEM_SPEED - k;
        }
        dest.set_changed();
    }
    stack
}

/// `HopperBlockEntity.addItem(source, destination, stack, face)`: into the face's slots of a
/// worldly container, else every slot in order.
pub(crate) fn add_item(dest: &mut View, mut stack: ItemStack, face: Option<Direction>, source_ticked: Option<i64>) -> ItemStack {
    let slots: Vec<usize> = match face {
        Some(f) if dest.worldly() => dest.slots(f),
        _ => (0..dest.size()).collect(),
    };
    for slot in slots {
        if stack.is_empty() {
            break;
        }
        stack = try_move_in(dest, stack, slot, face, source_ticked);
    }
    stack
}

/// `HopperBlockEntity.isFullContainer`.
fn is_full(view: &View, face: Direction) -> bool {
    view.slots(face).iter().all(|&s| {
        let item = view.item(s);
        item.count() >= item.max_stack_size()
    })
}

/// `HopperBlockEntity.inventoryFull`.
fn inventory_full(h: &ContainerBe) -> bool {
    h.items.iter().all(|s| !s.is_empty() && s.count() == s.max_stack_size())
}

/// `BlockEntity.setChanged(level, pos, state)`: comparators re-read the container.
pub(crate) fn changed(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    if !kiln_data::blocks_types::is_air(s) {
        kiln_blocks::update::update_neighbour_for_output_signal(level, pos, kiln_blocks::BlockId::of(s));
    }
}

/// `HopperBlockEntity.pushItemsTick`.
pub(crate) fn push_items_tick(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos) {
    let now = level.env.game_time;
    let Some(h) = level.blocks.containers.get_mut(pos) else { return };
    h.cooldown -= 1;
    h.ticked_game_time = now;
    if h.cooldown <= 0 {
        h.cooldown = 0;
        try_move_items(level, items, pos);
    }
}

/// `HopperBlockEntity.tryMoveItems` with the default sucking (the container above, or item
/// entities).
fn try_move_items(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos) -> bool {
    let s = level.block(pos);
    let Some(h) = level.blocks.containers.get(pos) else { return false };
    if h.cooldown > 0 || !state::get_bool(s, "enabled") {
        return false;
    }
    let mut moved = false;
    if !h.is_empty() {
        moved = eject_items(level, items, pos, s);
    }
    if level.blocks.containers.get(pos).is_some_and(|h| !inventory_full(h)) {
        moved |= suck_in_items(level, items, pos);
    }
    if moved {
        if let Some(h) = level.blocks.containers.get_mut(pos) {
            h.cooldown = MOVE_ITEM_SPEED;
            h.mark_changed();
        }
        changed(level, pos);
        return true;
    }
    false
}

/// `HopperBlockEntity.ejectItems`: one item into the container it faces.
fn eject_items(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos, s: u16) -> bool {
    let facing = state::get_dir(s, "facing").unwrap_or(Direction::Down);
    let Some(target) = container_at(level, pos.relative(facing)) else { return eject_into_cart(level, items, pos, facing) };
    let face = facing.opposite();
    let Some(mut hopper) = level.blocks.containers.map.remove(&pos) else { return false };
    let ticked = hopper.ticked_game_time;
    let result = with_target(level, &target, |dest| {
        if is_full(dest, face) {
            return false;
        }
        for slot in 0..hopper.items.len() {
            let item = hopper.items[slot].clone();
            if item.is_empty() {
                continue;
            }
            let count = item.count();
            let one = hopper.remove_item(slot, 1);
            let left = add_item(dest, one, Some(face), Some(ticked));
            if left.is_empty() {
                dest.set_changed();
                return true;
            }
            // `original.setCount(count)`: the stack in the slot is restored in place.
            let mut back = item;
            back.set_count(count);
            if count == 1 {
                hopper.set_item(slot, back);
            } else {
                hopper.items[slot] = back;
            }
        }
        false
    });
    level.blocks.containers.map.insert(pos, hopper);
    for p in target.positions() {
        crate::jukebox::settle(level, p);
    }
    let moved = result.unwrap_or(false);
    if moved {
        for p in target.positions() {
            changed(level, p);
        }
    }
    moved
}

/// `getEntityContainer`: the minecart among `carts` a hopper deals with (`level.random` picks
/// one of several), with its loot table rolled.
fn pick_cart(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos, carts: &[usize]) -> usize {
    use kiln_javamath::random::RandomSource;
    let pick = super::pos_random(level, pos, 9).next_int_bounded(carts.len() as i32) as usize;
    let idx = carts[pick];
    let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    if let Some((c, at)) = items.cart(idx)
        && let Some(table) = c.loot_table.take()
        && let Some(loot) = loot
    {
        let cell = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
        super::fill_from_table(&mut c.items, &loot, &table, c.loot_seed, at, cell, false, game_time, seed);
    }
    idx
}

/// `HopperBlockEntity.ejectItems` into a container minecart in the block the hopper faces.
fn eject_into_cart(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos, facing: Direction) -> bool {
    let target = pos.relative(facing);
    let (lo, hi) = ([target.x as f64, target.y as f64, target.z as f64], [target.x as f64 + 1.0, target.y as f64 + 1.0, target.z as f64 + 1.0]);
    let carts = items.carts_in(lo, hi);
    if carts.is_empty() {
        return false;
    }
    let idx = pick_cart(level, items, pos, &carts);
    let Some((cart, _)) = items.cart(idx) else { return false };
    // `isFullContainer`.
    if cart.items.iter().all(|s| !s.is_empty() && s.count() >= s.max_stack_size()) {
        return false;
    }
    let Some(hopper) = level.blocks.containers.get_mut(pos) else { return false };
    for slot in 0..hopper.items.len() {
        let item = hopper.items[slot].clone();
        if item.is_empty() {
            continue;
        }
        let count = item.count();
        let one = hopper.remove_item(slot, 1);
        if cart.add_stack(one).is_empty() {
            return true;
        }
        // `original.setCount(count)`: the stack in the slot is restored in place.
        let mut back = item;
        back.set_count(count);
        if count == 1 {
            hopper.set_item(slot, back);
        } else {
            hopper.items[slot] = back;
        }
    }
    false
}

/// `HopperBlockEntity.suckInItems` from a container minecart in the block above.
fn suck_from_cart(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos) -> Option<bool> {
    let above = pos.above();
    let (lo, hi) = ([above.x as f64, above.y as f64, above.z as f64], [above.x as f64 + 1.0, above.y as f64 + 1.0, above.z as f64 + 1.0]);
    let carts = items.carts_in(lo, hi);
    if carts.is_empty() {
        return None;
    }
    let idx = pick_cart(level, items, pos, &carts);
    let (cart, _) = items.cart(idx)?;
    let hopper = level.blocks.containers.get_mut(pos)?;
    for slot in 0..cart.items.len() {
        let item = cart.items[slot].clone();
        if item.is_empty() {
            continue;
        }
        let count = item.count();
        let one = cart.items[slot].split_count(1);
        let left = add_item(&mut View::one(hopper), one, None, None);
        if left.is_empty() {
            return Some(true);
        }
        let mut back = item;
        back.set_count(count);
        cart.items[slot] = back;
    }
    Some(false)
}

/// `HopperBlockEntity.suckInItems`.
fn suck_in_items(level: &mut RegionLevel, items: &mut dyn ItemEntities, pos: BlockPos) -> bool {
    let above = pos.above();
    let above_state = level.block(above);
    if container_at(level, above).is_none()
        && let Some(moved) = suck_from_cart(level, items, pos)
    {
        return moved;
    }
    if let Some(source) = container_at(level, above) {
        let Some(mut hopper) = level.blocks.containers.map.remove(&pos) else { return false };
        let source_is_hopper = matches!(source, Target::One(p) if level.blocks.containers.get(p).is_some_and(|c| c.kind == BeKind::Hopper));
        let source_ticked = if source_is_hopper { level.blocks.containers.get(above).map(|c| c.ticked_game_time) } else { None };
        let result = with_target(level, &source, |src| {
            for slot in src.slots(Direction::Down) {
                if try_take_in_item_from_slot(&mut hopper, src, slot, Direction::Down, source_ticked) {
                    return true;
                }
            }
            false
        });
        level.blocks.containers.map.insert(pos, hopper);
        for p in source.positions() {
            crate::jukebox::settle(level, p);
        }
        let moved = result.unwrap_or(false);
        if moved {
            for p in source.positions() {
                changed(level, p);
            }
        }
        return moved;
    }
    // A full, solid block above (not a beehive) stops item pickup.
    let blocked = kiln_data::block_props::full_collision(above_state) && !kiln_blocks::tags::is(above_state, "minecraft:does_not_block_hoppers");
    if blocked {
        return false;
    }
    // `Hopper.SUCK_AABB` (0,11,0)..(16,32,16) pixels, moved to the hopper.
    let lo = [pos.x as f64, pos.y as f64 + 11.0 / 16.0, pos.z as f64];
    let hi = [pos.x as f64 + 1.0, pos.y as f64 + 2.0, pos.z as f64 + 1.0];
    for i in items.items_in(lo, hi) {
        let Some(hopper) = level.blocks.containers.get_mut(pos) else { return false };
        if add_item_entity(hopper, items, i) {
            return true;
        }
    }
    false
}

/// A hopper minecart's `HopperBlockEntity.suckInItems` from the container block at `pos`:
/// one item from the first slot the container gives out downwards goes into `dest` (the
/// minecart's five slots). `None` when there is no container block there.
pub(crate) fn take_into_cart(level: &mut RegionLevel, pos: BlockPos, dest: &mut Vec<ItemStack>) -> Option<bool> {
    let source = container_at(level, pos)?;
    // The minecart's slots stand in as a hopper for the transfer.
    let type_id = kiln_world::block_entity::type_id("minecraft:hopper")?;
    let mut hopper = ContainerBe::load(BeKind::Hopper, type_id, &kiln_proto::nbt::Tag::Compound(Vec::new()));
    hopper.items = std::mem::take(dest);
    let moved = with_target(level, &source, |src| {
        for slot in src.slots(Direction::Down) {
            if try_take_in_item_from_slot(&mut hopper, src, slot, Direction::Down, None) {
                return true;
            }
        }
        false
    });
    *dest = hopper.items;
    for p in source.positions() {
        crate::jukebox::settle(level, p);
    }
    let moved = moved.unwrap_or(false);
    if moved {
        for p in source.positions() {
            changed(level, p);
        }
    }
    Some(moved)
}

/// `HopperBlockEntity.tryTakeInItemFromSlot`.
fn try_take_in_item_from_slot(hopper: &mut ContainerBe, src: &mut View, slot: usize, face: Direction, source_ticked: Option<i64>) -> bool {
    let item = src.item(slot).clone();
    if item.is_empty() || !src.can_take_from(&item, slot, face) {
        return false;
    }
    // `JukeboxBlockEntity.canTakeItem`: only where the hopper has room (`hasAnyMatching(isEmpty)`).
    if src.kind() == BeKind::Jukebox && !hopper.items.iter().any(ItemStack::is_empty) {
        return false;
    }
    let count = item.count();
    let one = src.remove_item(slot, 1);
    let left = {
        let mut dest = View::one(hopper);
        add_item(&mut dest, one, None, source_ticked)
    };
    if left.is_empty() {
        src.set_changed();
        return true;
    }
    let mut back = item;
    back.set_count(count);
    if count == 1 {
        src.set_item(slot, back);
    } else {
        *src.item_mut(slot) = back;
    }
    false
}

/// `HopperBlockEntity.addItem(container, itemEntity)`: true when the whole stack went in.
pub(crate) fn add_item_entity(hopper: &mut ContainerBe, items: &mut dyn ItemEntities, i: usize) -> bool {
    let stack = items.stack_mut(i).copy();
    let left = {
        let mut dest = View::one(hopper);
        add_item(&mut dest, stack, None, None)
    };
    let all = left.is_empty();
    *items.stack_mut(i) = left;
    items.changed(i);
    all
}

/// A region's item entities for hoppers, remembering which ones they changed.
pub(crate) struct EntityItems<'a> {
    entities: &'a mut crate::entities::Entities,
    /// The item entities at the start of the phase with their boxes (hoppers only look at
    /// these, not at every entity).
    items: Vec<(usize, [f64; 3], [f64; 3])>,
    /// The container minecarts, likewise.
    carts: Vec<(usize, [f64; 3], [f64; 3])>,
    touched: Vec<usize>,
}

impl<'a> EntityItems<'a> {
    /// The region's entities.
    pub fn entities(&self) -> &crate::entities::Entities {
        self.entities
    }

    pub fn entities_mut(&mut self) -> &mut crate::entities::Entities {
        self.entities
    }

    pub fn new(entities: &'a mut crate::entities::Entities) -> Self {
        let items = entities
            .list
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed && matches!(e.phys.as_deref().map(|p| &p.kind), Some(kiln_entity::EntityKind::Item(_))))
            .map(|(i, e)| {
                let (min, max, _) = e.body();
                (i, min, max)
            })
            .collect();
        let carts = entities
            .list
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed && e.phys.as_deref().is_some_and(is_container_cart))
            .map(|(i, e)| {
                let (min, max, _) = e.body();
                (i, min, max)
            })
            .collect();
        EntityItems { entities, items, carts, touched: Vec::new() }
    }

    /// Indices of the item entities whose stack changed.
    pub fn touched(mut self) -> Vec<usize> {
        self.touched.sort_unstable();
        self.touched.dedup();
        self.touched
    }
}

impl ItemEntities for EntityItems<'_> {
    fn items_in(&self, lo: [f64; 3], hi: [f64; 3]) -> Vec<usize> {
        self.items
            .iter()
            .filter(|(i, min, max)| !self.entities.list[*i].removed && (0..3).all(|k| min[k] < hi[k] && max[k] > lo[k]))
            .map(|(i, _, _)| *i)
            .collect()
    }
    fn stack_mut(&mut self, i: usize) -> &mut ItemStack {
        self.entities.stack_mut(i)
    }
    fn changed(&mut self, i: usize) {
        self.touched.push(i);
        self.entities.changed(i);
    }
    fn carts_in(&self, lo: [f64; 3], hi: [f64; 3]) -> Vec<usize> {
        self.carts
            .iter()
            .filter(|(i, min, max)| !self.entities.list[*i].removed && (0..3).all(|k| min[k] < hi[k] && max[k] > lo[k]))
            .map(|(i, _, _)| *i)
            .collect()
    }
    fn cart(&mut self, i: usize) -> Option<(&mut kiln_entity::ext_entity::minecart::Contents, [f64; 3])> {
        self.entities.cart(i)
    }
}

/// A chest or hopper minecart or a chest boat (`EntitySelector.CONTAINER_ENTITY_SELECTOR`).
fn is_container_cart(e: &kiln_entity::Entity) -> bool {
    kiln_entity::ext_entity::container(e).is_some()
}

/// Viewers of item entities a hopper took from see the remaining count (removed ones leave
/// with entity tracking).
pub(crate) fn send_item_counts(entities: &crate::entities::Entities, players: &mut [&mut crate::Player], touched: &[usize]) {
    for &i in touched {
        let Some(e) = entities.list.get(i) else { continue };
        if e.removed {
            continue;
        }
        let pkt = kiln_proto::packets::entity::set_entity_data(e.id, &e.metadata());
        for v in &e.seen_by {
            if let Ok(j) = players.binary_search_by_key(v, |q| q.conn) {
                players[j].send(pkt.clone());
            }
        }
    }
}

impl ItemEntities for crate::entities::Entities {
    fn items_in(&self, lo: [f64; 3], hi: [f64; 3]) -> Vec<usize> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed)
            .filter(|(_, e)| matches!(e.phys.as_deref().map(|p| &p.kind), Some(kiln_entity::EntityKind::Item(_))))
            .filter(|(_, e)| {
                let (min, max, _) = e.body();
                (0..3).all(|k| min[k] < hi[k] && max[k] > lo[k])
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn stack_mut(&mut self, i: usize) -> &mut ItemStack {
        match self.list[i].phys.as_deref_mut().map(|p| &mut p.kind) {
            Some(kiln_entity::EntityKind::Item(d)) => &mut d.stack,
            _ => unreachable!("not an item entity"),
        }
    }

    fn carts_in(&self, lo: [f64; 3], hi: [f64; 3]) -> Vec<usize> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.removed && e.phys.as_deref().is_some_and(is_container_cart))
            .filter(|(_, e)| {
                let (min, max, _) = e.body();
                (0..3).all(|k| min[k] < hi[k] && max[k] > lo[k])
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn cart(&mut self, i: usize) -> Option<(&mut kiln_entity::ext_entity::minecart::Contents, [f64; 3])> {
        let phys = self.list.get_mut(i)?.phys.as_deref_mut()?;
        let p = phys.position();
        let contents = kiln_entity::ext_entity::container_mut(phys)?;
        Some((contents, [p.x, p.y, p.z]))
    }

    fn changed(&mut self, i: usize) {
        let e = &mut self.list[i];
        let empty = matches!(e.phys.as_deref().map(|p| &p.kind), Some(kiln_entity::EntityKind::Item(d)) if d.stack.is_empty());
        if empty {
            e.removed = true;
            if let Some(p) = e.phys.as_deref_mut() {
                p.discard();
            }
        }
    }
}
