//! Lightning bolts (`LightningBolt`): on their first tick they set fire around the strike (on
//! normal and hard), power a lightning rod they hit, then flash one to three times, each flash
//! striking every entity within 3 blocks (`Entity.thunderHit`) and lighting the block again.
//! The thunder and impact sounds are the client's; the struck block (rods, copper) is the
//! level's (`lightning_strike_block`). When it ends, players within 256 blocks get
//! `lightning_strike`.

use crate::entity::{Entity, EntityKind};
use crate::entity_ext_boilerplate;
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use kiln_javamath::random::RandomSource;

#[derive(Clone, Debug)]
pub struct LightningBolt {
    pub life: i32,
    pub seed: i64,
    pub flashes: i32,
    pub visual_only: bool,
    pub blocks_set_on_fire: i32,
    /// Entities struck so far (`hitEntities`).
    pub hit: Vec<i32>,
    /// The player whose channeling trident called the bolt (`cause`).
    pub cause: Option<i32>,
}

/// A bolt called by player `cause`'s channeling trident.
pub fn channeled(id: i32, uuid: u128, pos: Vec3, cause: i32, seed: i64) -> Entity {
    let mut e = new(id, uuid, pos, false, seed);
    if let EntityKind::Ext(x) = &mut e.kind
        && let Some(b) = x.as_any_mut().downcast_mut::<LightningBolt>()
    {
        b.cause = Some(cause);
    }
    e
}

/// A bolt at `pos` (`EntityType.create` + `snapTo`): the constructor draws its seed and flash
/// count from the entity's random.
pub fn new(id: i32, uuid: u128, pos: Vec3, visual_only: bool, seed: i64) -> Entity {
    let mut e = Entity::new("minecraft:lightning_bolt", id, uuid, EntityKind::Other { type_name: "minecraft:lightning_bolt" }, seed);
    let bolt_seed = e.random.next_long();
    let flashes = e.random.next_int_bounded(3) + 1;
    e.no_physics = true;
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e.kind = EntityKind::Ext(Box::new(LightningBolt { life: 2, seed: bolt_seed, flashes, visual_only, blocks_set_on_fire: 0, hit: Vec::new(), cause: None }));
    e
}

impl LightningBolt {
    /// `spawnFire`: the block the bolt stands in and `extra` random blocks around it.
    fn spawn_fire(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, extra: i32) {
        if self.visual_only {
            return;
        }
        let at = e.block_position();
        if !level.can_spread_fire_around(at) {
            return;
        }
        if level.place_lightning_fire(at) {
            self.blocks_set_on_fire += 1;
        }
        for _ in 0..extra {
            let dx = e.random.next_int_bounded(3) - 1;
            let dy = e.random.next_int_bounded(3) - 1;
            let dz = e.random.next_int_bounded(3) - 1;
            if level.place_lightning_fire(BlockPos::new(at.x + dx, at.y + dy, at.z + dz)) {
                self.blocks_set_on_fire += 1;
            }
        }
    }
}

impl LightningBolt {
    /// `LightningStrikeTrigger` for every player within 256 blocks: the bolt and the living
    /// entities around it that it did not strike.
    fn strike_criteria(&self, e: &Entity, level: &mut dyn EntityLevel) {
        let p = e.position();
        let area = Aabb::new(p.x - 15.0, p.y - 15.0, p.z - 15.0, p.x + 15.0, p.y + 6.0 + 15.0, p.z + 15.0);
        let near: Vec<crate::level::Seen> = level
            .entities_in(&area, EntityFilter::Any, e.id)
            .into_iter()
            .filter(|id| !self.hit.contains(id))
            .filter_map(|id| level.entity(id).filter(|o| o.is_alive()).map(crate::level::Seen::of))
            .collect();
        let bolt = crate::level::Seen { lightning_fires: Some(self.blocks_set_on_fire), ..crate::level::Seen::of(e) };
        // (The players whose box touches the cube of 257 blocks around the bolt: the ones
        // within 256 blocks in `f32` stand in it.)
        let reach = Aabb::new(p.x - 257.0, p.y - 257.0, p.z - 257.0, p.x + 257.0, p.y + 257.0, p.z + 257.0);
        let players: Vec<i32> = level
            .players_in(&reach)
            .iter()
            .filter(|v| {
                let (dx, dy, dz) = ((v.pos.x - p.x) as f32, (v.pos.y - p.y) as f32, (v.pos.z - p.z) as f32);
                (dx * dx + dy * dy + dz * dz).sqrt() < 256.0
            })
            .map(|v| v.id)
            .collect();
        for player in players {
            let criterion = crate::level::Criterion::LightningStrike { lightning: bolt.clone(), victims: near.clone(), blocks_set_on_fire: self.blocks_set_on_fire };
            level.emit(Event::Criterion { player, criterion });
        }
        // `ChanneledLightningTrigger`: what the called bolt struck.
        if let Some(cause) = self.cause {
            let victims: Vec<crate::level::Seen> = self.hit.iter().filter_map(|&id| level.entity(id).map(crate::level::Seen::of)).collect();
            level.emit(Event::Criterion { player: cause, criterion: crate::level::Criterion::ChanneledLightning { victims } });
        }
    }
}

impl EntityExt for LightningBolt {
    entity_ext_boilerplate!();

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if self.life == 2 {
            if level.difficulty() >= 2 {
                self.spawn_fire(e, level, 4);
            }
            // `getStrikePosition`: just below the bolt.
            let p = e.position();
            let strike = BlockPos::containing(p.x, p.y - 1.0e-6, p.z);
            level.lightning_strike_block(strike);
            level.emit(Event::GameEvent { event: "minecraft:lightning_strike", pos: p, entity: Some(e.id) });
        }
        self.life -= 1;
        if self.life < 0 {
            if self.flashes == 0 {
                self.strike_criteria(e, level);
                e.discard();
            } else if self.life < -e.random.next_int_bounded(10) {
                self.flashes -= 1;
                self.life = 1;
                self.seed = e.random.next_long();
                self.spawn_fire(e, level, 0);
            }
        }
        if self.life >= 0 && !self.visual_only {
            let p = e.position();
            let area = Aabb::new(p.x - 3.0, p.y - 3.0, p.z - 3.0, p.x + 3.0, p.y + 6.0 + 3.0, p.z + 3.0);
            for id in level.entities_in(&area, EntityFilter::Any, e.id) {
                thunder_hit(level, id, e.id);
                if !self.hit.contains(&id) {
                    self.hit.push(id);
                }
            }
        }
    }
}

/// `Entity.thunderHit` of entity or player `id`: fire (one more tick, 8 seconds if that made
/// it 0) and 5 lightning damage; creepers become charged, pigs zombified piglins and
/// villagers witches (not on peaceful).
pub fn thunder_hit(level: &mut dyn EntityLevel, id: i32, bolt: i32) {
    if level.player(id).is_some() {
        level.thunder_hit_player(id);
        return;
    }
    let Some(t) = level.entity_mut(id) else { return };
    if !t.is_alive() {
        return;
    }
    let marker = Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0);
    let mut t2 = std::mem::replace(t, marker);
    if !crate::mob::thunder_hit(&mut t2, level, bolt) && !ext_thunder_hit(&mut t2, level, bolt) {
        base_thunder_hit(&mut t2, level);
    }
    if let Some(slot) = level.entity_mut(id) {
        *slot = t2;
    }
}

/// The `thunderHit` an extension entity brings (`BlockAttachedEntity`: nothing).
fn ext_thunder_hit(e: &mut Entity, level: &mut dyn EntityLevel, bolt: i32) -> bool {
    if !matches!(e.kind, EntityKind::Ext(_)) {
        return false;
    }
    let placeholder = EntityKind::Other { type_name: e.type_name };
    let EntityKind::Ext(mut x) = std::mem::replace(&mut e.kind, placeholder) else { return false };
    let handled = x.thunder_hit(e, level, bolt);
    e.kind = EntityKind::Ext(x);
    handled
}

/// The plain `Entity.thunderHit`.
pub fn base_thunder_hit(e: &mut Entity, level: &mut dyn EntityLevel) {
    e.remaining_fire_ticks += 1;
    if e.remaining_fire_ticks == 0 {
        e.ignite_for_seconds(8.0);
    }
    e.hurt(level, DamageKind::LightningBolt, 5.0, None);
}
