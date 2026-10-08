//! Item frames and glow item frames (`ItemFrame`, `GlowItemFrame`): a 12 by 12 pixel frame on a
//! block's face that holds one item. A click puts an item in (one of it) or, with an item in the
//! frame, turns it an eighth; a hit takes the item out (the frame goes with the second hit);
//! when the wall behind it goes the frame comes off. A comparator behind it reads the item's
//! rotation (1 to 8, 0 for an empty frame).

use super::hanging::{self, center_relative, data_value, of_size};
use crate::entity::{Entity, EntityKind, RemovalReason};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Debug)]
pub struct ItemFrame {
    pub glow: bool,
    pub direction: Direction,
    /// The block the frame is in (`BlockAttachedEntity.pos`).
    pub pos: BlockPos,
    pub item: ItemStack,
    pub rotation: i32,
    pub fixed: bool,
    pub drop_chance: f32,
    pub invisible: bool,
    since_check: i32,
}

pub const ITEM_FRAME: &str = "minecraft:item_frame";
pub const GLOW_ITEM_FRAME: &str = "minecraft:glow_item_frame";

fn type_name(glow: bool) -> &'static str {
    if glow { GLOW_ITEM_FRAME } else { ITEM_FRAME }
}

/// `new ItemFrame(level, pos, direction)`: the frame in block `pos` on the face toward
/// `direction.opposite()`.
pub fn new(id: i32, glow: bool, pos: BlockPos, direction: Direction, seed: i64) -> Entity {
    let frame = ItemFrame { glow, direction, pos, item: ItemStack::empty(), rotation: 0, fixed: false, drop_chance: 1.0, invisible: false, since_check: 0 };
    let mut e = Entity::new(type_name(glow), id, 0, EntityKind::Other { type_name: type_name(glow) }, seed);
    frame.place(&mut e);
    e.kind = EntityKind::Ext(Box::new(frame));
    e.set_old_pos_and_rot();
    e
}

pub fn load(glow: bool, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let direction = r.get("Facing").and_then(Tag::as_i64).and_then(hanging::from_data_value).unwrap_or(Direction::Down);
    let item = r.get("Item").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).map(|mut s| {
        s.set_count(1);
        s
    });
    let rotation = r.byte_or("ItemRotation", 0) as i32;
    Some(Box::new(ItemFrame {
        glow,
        direction,
        // (Fixed up by [`after_load`].)
        pos: BlockPos::new(0, 0, 0),
        item: item.unwrap_or_else(ItemStack::empty),
        rotation: rotation % 8,
        fixed: r.bool_or("Fixed", false),
        drop_chance: r.float_or("ItemDropChance", 1.0),
        invisible: r.bool_or("Invisible", false),
        since_check: 0,
    }))
}

/// After the entity's position is read: `block_pos` (or the block it stands in) and the box.
pub fn after_load(e: &mut Entity, block_pos: Option<&Tag>) {
    let at = e.position();
    let pos = hanging::saved_pos(block_pos, at);
    let Some(f) = crate::ext_entity::get_mut::<ItemFrame>(e) else { return };
    f.pos = pos;
    let f = f.clone();
    f.place(e);
}

impl ItemFrame {
    fn has_map(&self) -> bool {
        self.item.get(kiln_item::keys::MAP_ID).is_some()
    }

    /// `createBoundingBox`.
    fn create_box(&self, has_map: bool) -> Aabb {
        let c = center_relative(self.pos, self.direction, -0.46875);
        let size = if has_map { 1.0 } else { 0.75 };
        let axis = self.direction.axis();
        let sx = if axis == crate::math::Axis::X { 0.0625 } else { size };
        let sy = if axis == crate::math::Axis::Y { 0.0625 } else { size };
        let sz = if axis == crate::math::Axis::Z { 0.0625 } else { size };
        of_size(c, sx, sy, sz)
    }

    fn pop_box(&self) -> Aabb {
        self.create_box(false)
    }

    /// `recalculateBoundingBox`: the entity sits at the middle of its box.
    fn place(&self, e: &mut Entity) {
        let bb = self.create_box(self.has_map());
        let (yaw, pitch) = hanging::rotations(self.direction);
        e.y_rot = yaw;
        e.x_rot = pitch;
        e.y_rot_o = yaw;
        e.x_rot_o = pitch;
        e.set_pos_raw(bb.center());
        e.set_bounding_box(bb);
    }

    /// `survives`.
    pub fn survives(&self, e: &Entity, world: &dyn hanging::HangingWorld) -> bool {
        if self.fixed {
            return true;
        }
        let pop = self.pop_box();
        if world.block_collision(&pop) {
            return false;
        }
        let behind = world.block(hanging::offset(self.pos, self.direction.opposite()));
        if !crate::physics::is_solid(behind) && !(self.direction.axis() != crate::math::Axis::Y && crate::blocks::is_diode(behind)) {
            return false;
        }
        hanging::can_coexist(world, e, self.direction, &pop, true)
    }

    /// `getAnalogOutput`.
    pub fn analog_output(&self) -> i32 {
        if self.item.is_empty() { 0 } else { self.rotation % 8 + 1 }
    }

    fn sound(&self, what: &str) -> &'static str {
        match (self.glow, what) {
            (false, "add") => "minecraft:entity.item_frame.add_item",
            (false, "rotate") => "minecraft:entity.item_frame.rotate_item",
            (false, "remove") => "minecraft:entity.item_frame.remove_item",
            (false, "break") => "minecraft:entity.item_frame.break",
            (false, _) => "minecraft:entity.item_frame.place",
            (true, "add") => "minecraft:entity.glow_item_frame.add_item",
            (true, "rotate") => "minecraft:entity.glow_item_frame.rotate_item",
            (true, "remove") => "minecraft:entity.glow_item_frame.remove_item",
            (true, "break") => "minecraft:entity.glow_item_frame.break",
            (true, _) => "minecraft:entity.glow_item_frame.place",
        }
    }

    /// `playPlacementSound`.
    pub fn placement_sound(&self) -> &'static str {
        self.sound("place")
    }

    /// `setItem(stack, update)`: one of the stack goes in; the sound and the comparators when
    /// `update`.
    fn set_item(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, stack: &ItemStack, update: bool) {
        let mut s = stack.clone();
        if !s.is_empty() {
            s.set_count(1);
        }
        self.item = s;
        // `onItemChanged`: a map makes the box bigger.
        self.place(e);
        if !self.item.is_empty() && update {
            e.play_sound(level, self.sound("add"), 1.0, 1.0);
        }
        if update {
            level.update_neighbours_for_output_signal(self.pos);
        }
    }

    fn set_rotation(&mut self, level: &mut dyn EntityLevel, rotation: i32, update: bool) {
        self.rotation = rotation % 8;
        if update {
            level.update_neighbours_for_output_signal(self.pos);
        }
    }

    /// `spawnAtLocation(level, stack, 0)` of a hanging entity: in front of the wall, a little off
    /// the middle, with the ItemEntity's own random throw.
    fn spawn_at_location(&self, e: &Entity, level: &mut dyn EntityLevel, stack: ItemStack) {
        let (dx, _, dz) = self.direction.step();
        let p = e.position();
        let at = Vec3::new(p.x + (dx as f32 * 0.15f32) as f64, p.y, p.z + (dz as f32 * 0.15f32) as f64);
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let mut item = crate::item::new_at(id, 0, stack, at, seed);
        if let EntityKind::Item(d) = &mut item.kind {
            d.pickup_delay = 10;
        }
        level.add_entity(item);
    }

    /// The frame as an item, with the entity's custom name.
    fn frame_item(&self, e: &Entity) -> ItemStack {
        let mut stack = ItemStack::of(if self.glow { "minecraft:glow_item_frame" } else { "minecraft:item_frame" }, 1).unwrap_or_default();
        if let Some(name) = e.extra.iter().find(|(k, _)| k == "CustomName").and_then(|(_, t)| kiln_item::Text::from_nbt(t.clone())) {
            stack.set(kiln_item::component::Component::CustomName(name));
        }
        stack
    }

    /// `dropItem(level, entity, alsoDropFrame)`.
    fn drop_item(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, by: Option<i32>, also_frame: bool) {
        if self.fixed {
            return;
        }
        let item = std::mem::replace(&mut self.item, ItemStack::empty());
        // `setItem(EMPTY)`: comparators hear of it.
        self.place(e);
        level.update_neighbours_for_output_signal(self.pos);
        if !level.entity_drops() {
            return;
        }
        if by.and_then(|a| level.player(a)).is_some_and(|p| p.creative) {
            return;
        }
        if also_frame {
            let frame = self.frame_item(e);
            self.spawn_at_location(e, level, frame);
        }
        if !item.is_empty() {
            let item = item.clone();
            if e.random.next_float() < self.drop_chance {
                self.spawn_at_location(e, level, item);
            }
        }
    }

    /// `dropItem(level, entity)`: the break sound, everything drops.
    fn drop_all(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, by: Option<i32>) {
        e.play_sound(level, self.sound("break"), 1.0, 1.0);
        self.drop_item(e, level, by, true);
        level.emit(Event::GameEvent { event: "minecraft:block_change", pos: e.position(), entity: by });
    }
}

use kiln_javamath::random::RandomSource;

impl EntityExt for ItemFrame {
    crate::entity_ext_boilerplate!();

    /// `BlockAttachedEntity.tick`: every 100 ticks the wall must still be there.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.since_check += 1;
        if self.since_check >= 100 {
            self.since_check = 0;
            if !e.is_removed() && !self.survives(e, &hanging::LevelWorld { level: &*level, e }) {
                e.discard();
                self.drop_all(e, level, None);
            }
        }
    }

    fn save(&self, e: &Entity, o: &mut Output) {
        let b = self.pos;
        o.put("block_pos", Tag::IntArray(vec![b.x, b.y, b.z]));
        if !self.item.is_empty() {
            o.put("Item", self.item.to_nbt());
        }
        o.put("ItemRotation", Tag::Byte(self.rotation as i8));
        o.put("ItemDropChance", Tag::Float(self.drop_chance));
        o.put("Facing", Tag::Byte(data_value(self.direction) as i8));
        o.put("Invisible", Tag::Byte(self.invisible as i8));
        o.put("Fixed", Tag::Byte(self.fixed as i8));
        let _ = e;
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        if self.invisible {
            d.set(kiln_data::entities::data::entity::SHARED_FLAGS, &DataValue::Byte(kiln_proto::packets::entity::metadata::shared_flags::INVISIBLE as i8));
        }
        d.set(kiln_data::entities::data::hanging_entity::DIRECTION, &DataValue::Direction(proto_direction(self.direction)));
        let mut bytes = bytes::BytesMut::new();
        self.item.write_optional(&mut bytes);
        d.set(kiln_data::entities::data::item_frame::ITEM, &DataValue::EncodedItemStack(bytes.freeze()));
        d.set(kiln_data::entities::data::item_frame::ROTATION, &DataValue::Int(self.rotation));
    }

    /// `getAddEntityPacket`: the data is the facing.
    fn spawn_data(&self) -> i32 {
        data_value(self.direction)
    }

    fn attackable(&self) -> bool {
        true
    }

    /// `hurtServer`.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, _amount: f32, attacker: Option<i32>) -> bool {
        let creative_attacker = attacker.and_then(|a| level.player(a)).is_some_and(|p| p.creative && kind == DamageKind::PlayerAttack);
        if self.fixed {
            // `canHurtWhenFixed`: only what bypasses invulnerability, or a creative player.
            if !(kind.is_tag("minecraft:bypasses_invulnerability") || creative_attacker) {
                return false;
            }
            return self.hurt_hanging(e, level, kind, attacker);
        }
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        // `shouldDamageDropItem`: a hit (not an explosion) takes the item out first.
        if !kind.is_tag("minecraft:is_explosion") && !self.item.is_empty() {
            self.drop_item(e, level, attacker, false);
            level.emit(Event::GameEvent { event: "minecraft:block_change", pos: e.position(), entity: attacker });
            e.play_sound(level, self.sound("remove"), 1.0, 1.0);
            return true;
        }
        self.hurt_hanging(e, level, kind, attacker)
    }

    /// `use` on the frame (`interact`).
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if self.fixed {
            return None;
        }
        let had_item = !self.item.is_empty();
        let has_item = !stack.is_empty();
        if !had_item {
            if has_item && !e.is_removed() {
                self.set_item(e, level, stack, true);
                level.emit(Event::GameEvent { event: "minecraft:block_change", pos: e.position(), entity: Some(who.id) });
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
            return None;
        }
        e.play_sound(level, self.sound("rotate"), 1.0, 1.0);
        let r = self.rotation + 1;
        self.set_rotation(level, r, true);
        level.emit(Event::GameEvent { event: "minecraft:block_change", pos: e.position(), entity: Some(who.id) });
        Some(Outcome::success(HeldChange::None))
    }
}

impl ItemFrame {
    /// `BlockAttachedEntity.hurtServer`: griefing mobs may not break it; else it is killed
    /// and drops itself and its item.
    fn hurt_hanging(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, attacker: Option<i32>) -> bool {
        if e.is_invulnerable_to_base(kind) {
            return false;
        }
        if !level.mob_griefing() && attacker.and_then(|a| level.entity(a)).is_some_and(|a| crate::mob::data(a).is_some()) {
            return false;
        }
        if !e.is_removed() {
            // `kill(level, entity)`: `onKilled`, removal, the game event.
            e.removed = Some(RemovalReason::Killed);
            level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: attacker });
            self.drop_all(e, level, attacker);
        }
        true
    }
}

fn proto_direction(d: Direction) -> kiln_proto::packets::entity::metadata::Direction {
    use kiln_proto::packets::entity::metadata::Direction as P;
    match d {
        Direction::Down => P::Down,
        Direction::Up => P::Up,
        Direction::North => P::North,
        Direction::South => P::South,
        Direction::West => P::West,
        Direction::East => P::East,
    }
}
