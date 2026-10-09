//! Ominous item spawners (`OminousItemSpawner`): an ominous trial spawner makes one above the
//! players and mobs of its trial; after 60 to 120 ticks it throws its item (a projectile item is
//! shot downward, anything else is dropped where it hangs) and goes.

use crate::entity::{Entity, EntityKind, RemovalReason};
use crate::entity_ext_boilerplate;
use crate::ext_entity::EntityExt;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub const TYPE: &str = "minecraft:ominous_item_spawner";

#[derive(Clone, Debug)]
pub struct OminousItemSpawner {
    /// `DATA_ITEM`.
    pub item: ItemStack,
    /// `spawnItemAfterTicks`.
    pub spawn_item_after_ticks: i64,
}

/// `OminousItemSpawner.create(level, item)` at `pos`: it throws its item after 60 to 120 ticks,
/// drawn from `level_random`. `snapTo(pos)` leaves the rotation as it was.
pub fn new(id: i32, item: ItemStack, pos: Vec3, level_random: &mut dyn RandomSource, seed: i64) -> Entity {
    // `nextIntBetweenInclusive(60, 120)`.
    let after = (level_random.next_int_bounded(61) + 60) as i64;
    let mut e = Entity::new(TYPE, id, 0, EntityKind::Ext(Box::new(OminousItemSpawner { item, spawn_item_after_ticks: after })), seed);
    e.no_physics = true;
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let item = r.get("item").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_default();
    let after = match r.get("spawn_item_after_ticks") {
        Some(Tag::Long(v)) => *v,
        Some(other) => other.as_i64().unwrap_or(0),
        None => 0,
    };
    Some(Box::new(OminousItemSpawner { item, spawn_item_after_ticks: after }))
}

impl OminousItemSpawner {
    /// `spawnItem`: the item flies down as a projectile, or lies as an item entity where the spawner hangs.
    fn spawn_item(&mut self, e: &Entity, level: &mut dyn EntityLevel) {
        if self.item.is_empty() {
            return;
        }
        let at = BlockPos::containing(e.x(), e.y(), e.z());
        let spawned = match level.spawn_item_projectile(&self.item, e.position(), at, e.id) {
            Some(id) => id,
            None => {
                // `new ItemEntity(level, x, y, z, stack)` (no pick-up delay).
                let id = level.next_entity_id();
                let seed = level.fresh_seed();
                let item = crate::item::new_at(id, 0, self.item.clone(), e.position(), seed);
                level.add_entity(item);
                id
            }
        };
        level.emit(Event::LevelEvent { event: 3021, pos: at, data: 1 });
        level.emit(Event::GameEvent { event: "minecraft:entity_place", pos: e.position(), entity: Some(spawned) });
        self.item = ItemStack::empty();
    }
}

impl EntityExt for OminousItemSpawner {
    entity_ext_boilerplate!();

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `tickServer`: a sound 36 ticks ahead, then the item, then the spawner goes.
        let ticks = e.tick_count as i64;
        if ticks == self.spawn_item_after_ticks - 36 {
            let b = e.block_position();
            level.emit(Event::Sound {
                pos: Vec3::new(b.x as f64 + 0.5, b.y as f64 + 0.5, b.z as f64 + 0.5),
                sound: "minecraft:block.trial_spawner.about_to_spawn_item",
                source: "neutral",
                volume: 1.0,
                pitch: 1.0,
            });
        }
        if ticks >= self.spawn_item_after_ticks {
            self.spawn_item(e, level);
            // `kill`: removed as killed, and the `entity_die` game event.
            e.removed = Some(RemovalReason::Killed);
            level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: Some(e.id) });
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        if !self.item.is_empty() {
            o.put("item", self.item.to_nbt());
        }
        o.put("spawn_item_after_ticks", Tag::Long(self.spawn_item_after_ticks));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        if !self.item.is_empty() {
            let mut bytes = bytes::BytesMut::new();
            self.item.write_optional(&mut bytes);
            d.set(data::ominous_item_spawner::ITEM, &DataValue::EncodedItemStack(bytes.freeze()));
        }
    }
}
