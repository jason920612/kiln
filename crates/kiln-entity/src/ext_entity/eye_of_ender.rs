//! The thrown eye of ender (`EyeOfEnder`): it sets out from the player with no speed, rises
//! toward the structure `signalTo` gave it (a point 12 blocks along the way and 8 up when the
//! structure is farther, else the structure itself, which it sinks toward), and after 80 ticks
//! is gone: the item drops in four of five cases, otherwise it shatters (level event 2003).

use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3, lerp};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub const TYPE: &str = "minecraft:eye_of_ender";

#[derive(Clone, Debug)]
pub struct EyeOfEnder {
    pub item: ItemStack,
    /// `target`: where it flies (not saved).
    pub target: Option<Vec3>,
    pub life: i32,
    pub survive_after_death: bool,
}

/// `EyeOfEnder.getDefaultItem`.
fn default_item() -> ItemStack {
    ItemStack::of("minecraft:ender_eye", 1).unwrap_or_else(ItemStack::empty)
}

/// `EyeOfEnder.setItem`: one of the stack, or the default for an empty one.
fn single(item: &ItemStack) -> ItemStack {
    if item.is_empty() { default_item() } else { item.with_count(1) }
}

/// `new EyeOfEnder(level, x, y, z)` with `setItem(item)`; `signalTo` is separate.
pub fn new(pos: Vec3, item: &ItemStack, seed: i64) -> Entity {
    let mut e = Entity::new(TYPE, 0, 0, EntityKind::Other { type_name: TYPE }, seed);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e.needs_sync = true;
    e.kind = EntityKind::Ext(Box::new(EyeOfEnder { item: single(item), target: None, life: 0, survive_after_death: false }));
    e
}

/// `EyeOfEnder.signalTo(target)`: aims at the structure (or a point on the way, when it is more
/// than 12 blocks off horizontally) and draws the one-in-five chance that it shatters.
pub fn signal_to(e: &mut Entity, target: Vec3) {
    let pos = e.position();
    let delta = target - pos;
    let h = delta.horizontal_distance();
    let aim = if h > 12.0 { pos.add(delta.x / h * 12.0, 8.0, delta.z / h * 12.0) } else { target };
    let survive = e.random.next_int_bounded(5) > 0;
    if let Some(x) = crate::ext_entity::get_mut::<EyeOfEnder>(e) {
        x.target = Some(aim);
        x.life = 0;
        x.survive_after_death = survive;
    }
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let item = r.get("Item").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_else(default_item);
    Some(Box::new(EyeOfEnder { item: single(&item), target: None, life: 0, survive_after_death: false }))
}

/// `EyeOfEnder.updateDeltaMovement(delta, pos, target)`.
fn update_delta_movement(delta: Vec3, pos: Vec3, target: Vec3) -> Vec3 {
    let diff = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z);
    let len = diff.length();
    let mut speed = lerp(0.0025, delta.horizontal_distance(), len);
    let mut dy = delta.y;
    if len < 1.0 {
        speed *= 0.8;
        dy *= 0.8;
    }
    let toward = if pos.y - delta.y < target.y { 1.0 } else { -1.0 };
    diff.scale(speed / len).add(0.0, dy + (toward - dy) * 0.015, 0.0)
}

impl EntityExt for EyeOfEnder {
    crate::entity_ext_boilerplate!();

    /// `EyeOfEnder.tick` (the server's half: the particles are the client's).
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        e.base_tick(level);
        let new_pos = e.position() + e.delta;
        if let Some(target) = self.target {
            e.delta = update_delta_movement(e.delta, new_pos, target);
        }
        e.set_pos(new_pos);
        self.life += 1;
        if self.life > 80 {
            e.play_sound(level, "minecraft:entity.ender_eye.death", 1.0, 1.0);
            e.discard();
            if self.survive_after_death {
                let (id, seed) = (level.next_entity_id(), level.fresh_seed());
                let item = crate::item::new_at(id, 0, self.item.clone(), e.position(), seed);
                level.add_entity(item);
            } else {
                let p = e.position();
                level.emit(Event::LevelEvent { event: 2003, pos: BlockPos::containing(p.x, p.y, p.z), data: 0 });
            }
        }
    }

    fn gravity(&self) -> f64 {
        0.0
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("Item", self.item.to_nbt());
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::eye_of_ender as f;
        let mut bytes = bytes::BytesMut::new();
        self.item.write_optional(&mut bytes);
        d.set(f::ITEM_STACK, &DataValue::EncodedItemStack(bytes.freeze()));
    }
}
