//! Wind charges (`AbstractWindCharge`, the breeze's `BreezeWindCharge`): a projectile that flies
//! straight without drag or acceleration and bursts where it hits — 1 damage to the entity it
//! strikes, then the wind burst: an explosion of radius 3 that damages nothing, knocks entities
//! back and triggers blocks (`ExplosionInteraction.TRIGGER`; buttons and doors are not toggled
//! here).

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, Vec3};
use crate::mob::{self, DamageSource};
use crate::persist::{Input, Output};
use crate::projectile::Hit;
use kiln_proto::nbt::Tag;

pub const TYPE: &str = "minecraft:breeze_wind_charge";

#[derive(Clone, Debug)]
pub struct WindCharge {
    pub owner: Option<i32>,
    /// The owner's UUID, as it is saved (`Owner`).
    pub owner_uuid: Option<u128>,
    pub left_owner: bool,
    pub has_been_shot: bool,
    /// `AbstractHurtingProjectile.accelerationPower`: 0 until a player deflects the charge.
    pub acceleration_power: f64,
}

/// `new BreezeWindCharge(breeze, level)` at `pos` (the breeze's firing height), owned by
/// `owner`; the caller shoots it (`Projectile.shoot`) and adds it.
pub fn new(id: i32, owner: i32, owner_uuid: u128, pos: Vec3, seed: i64) -> Entity {
    let x = WindCharge { owner: Some(owner), owner_uuid: Some(owner_uuid), left_owner: false, has_been_shot: false, acceleration_power: 0.0 };
    let mut e = Entity::new(TYPE, id, 0, EntityKind::Ext(Box::new(x)), seed);
    set_pos(&mut e, pos);
    e.set_old_pos_and_rot();
    e
}

/// `AbstractWindCharge.makeBoundingBox`: the box sits 0.15 below the position.
fn set_pos(e: &mut Entity, p: Vec3) {
    e.set_pos(p);
    let (w, h) = (e.width as f64 / 2.0, e.height as f64);
    e.set_bounding_box(Aabb::new(p.x - w, p.y - 0.15000000596046448, p.z - w, p.x + w, p.y - 0.15000000596046448 + h, p.z + w));
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(WindCharge {
        owner: None,
        owner_uuid: r.uuid("Owner"),
        left_owner: r.bool_or("leftOwner", false),
        has_been_shot: r.bool_or("HasBeenShot", false),
        acceleration_power: r.num("acceleration_power").unwrap_or(0.0),
    }))
}

/// The wind burst at `center` from charge `source`.
pub fn burst(level: &mut dyn EntityLevel, source: Option<i32>, center: Vec3) {
    // The wind calculator (`SimpleExplosionDamageCalculator` with `#blocks_wind_charge_explosions`
    // immune): blocks have no resistance at all (the rays go through them, so they draw the
    // level random as long as vanilla's), except those of the tag (3600000).
    let resist = |state: u16, _res: f32| if crate::mob::kinds::wolf::block_in_tag(state, "minecraft:blocks_wind_charge_explosions") { 3600000.0 } else { -0.3 };
    crate::explosion::explode_with(level, source, center, 3.0, false, crate::explosion::Interaction::TriggerBlock, Some(&resist), false);
    level.emit(Event::Sound { pos: center, sound: "minecraft:entity.breeze.wind_burst", source: "hostile", volume: 1.0, pitch: 1.0 });
}

impl WindCharge {
    /// `getHitResultOnMoveVector`: blocks (`COLLIDER`), then entities other than wind charges
    /// and (until it left) the owner.
    fn hit_on_move_vector(&self, e: &Entity, level: &dyn EntityLevel) -> Option<Hit> {
        let from = e.position();
        let delta = e.delta;
        let mut to = from + delta;
        let ctx = e.collision_context();
        let block = clip::traverse_blocks(from, to, |pos| {
            let (shape, _) = collision::collision_shape(level.block(pos), pos, &ctx);
            clip::shape_clip(&shape, from, to, pos).map(|(location, face)| Hit::Block { pos, face, location })
        });
        if let Some(Hit::Block { location, .. }) = block {
            to = location;
        }
        let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0));
        let area = e.bounding_box().expand_towards_vec(delta).inflate_all(1.0);
        let owner = if self.left_owner { None } else { self.owner };
        let mut best = f64::MAX;
        let mut hit = None;
        for id in level.entities_in(&area, EntityFilter::Any, e.id) {
            let Some(t) = level.entity(id) else { continue };
            if !crate::projectile::can_be_hit_by_projectile(t) || Some(id) == owner || t.no_physics || t.type_name == TYPE {
                continue;
            }
            if let Some(p) = t.bounding_box().inflate_all(margin as f64).clip(from, to) {
                let d = from.distance_to_sqr(p);
                if d < best {
                    best = d;
                    hit = Some(Hit::Entity { id, location: p });
                }
            }
        }
        hit.or(block)
    }

    fn check_left_owner(&mut self, e: &Entity, level: &dyn EntityLevel) {
        if self.left_owner {
            return;
        }
        let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
        self.left_owner = match self.owner.and_then(|id| level.entity(id)) {
            Some(o) => !(crate::projectile::can_be_hit_by_projectile(o) && area.intersects(&o.bounding_box())),
            None => true,
        };
    }

    fn on_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit) {
        level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: TYPE, owner: self.owner, hit });
        match hit {
            Hit::Entity { id, .. } => {
                let source = DamageSource { kind: DamageKind::WindCharge, attacker: self.owner, direct: Some(e.id), pos: Some(e.position()), attacker_is_player: self.owner.is_some_and(|o| level.player(o).is_some()) };
                if let Some(t) = mob::goals::living(level, id) {
                    mob::hurt_living(level, &t, source, 1.0);
                } else if let Some(t) = level.entity_mut(id) {
                    let mut t2 = std::mem::replace(t, Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
                    t2.hurt(level, DamageKind::WindCharge, 1.0, self.owner);
                    if let Some(slot) = level.entity_mut(id) {
                        *slot = t2;
                    }
                }
                burst(level, Some(e.id), e.position());
            }
            Hit::Block { face, location, .. } => {
                let (dx, dy, dz) = face.step();
                let at = location.add(dx as f64 * 0.25, dy as f64 * 0.25, dz as f64 * 0.25);
                burst(level, Some(e.id), at);
            }
        }
        e.discard();
    }
}

impl EntityExt for WindCharge {
    crate::entity_ext_boilerplate!();

    /// `AbstractWindCharge.tick` → `AbstractHurtingProjectile.tick` (no acceleration, inertia 1).
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `applyInertia`: inertia 1 in the air (`getInertia`) and in water (`getLiquidInertia` of
        // `AbstractWindCharge` is the same), plus the acceleration a player's deflection gave it.
        let v = e.delta;
        e.delta = (v + v.normalize().scale(self.acceleration_power)).scale(1.0);
        if crate::math::floor(e.y()) > level.max_y() + 30 {
            burst(level, Some(e.id), e.position());
            e.discard();
            return;
        }
        let owner_gone = self.owner.and_then(|o| level.entity(o)).is_some_and(|o| o.is_removed());
        if owner_gone || !level.is_loaded(e.block_position()) {
            e.discard();
            return;
        }
        let hit = self.hit_on_move_vector(e, level);
        let to = match hit {
            Some(Hit::Block { location, .. } | Hit::Entity { location, .. }) => location,
            None => e.position() + e.delta,
        };
        set_pos(e, to);
        e.apply_effects_from_blocks(level);
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        self.check_left_owner(e, level);
        e.base_tick(level);
        if let Some(hit) = hit
            && e.is_alive()
        {
            self.on_hit(e, level, hit);
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("leftOwner", Tag::Byte(self.left_owner as i8));
        o.put("HasBeenShot", Tag::Byte(self.has_been_shot as i8));
        if let Some(u) = self.owner_uuid {
            o.put("Owner", crate::persist::uuid_to_tag(u));
        }
        o.put("acceleration_power", Tag::Double(self.acceleration_power));
    }

    /// `AIM_DEFLECT` by a player (`onDeflection(true)`: the acceleration starts at 0.1).
    fn aim_deflect(&mut self, e: &mut Entity, by: (i32, u128), look: Vec3) -> bool {
        e.delta = look;
        e.needs_sync = true;
        self.owner = Some(by.0);
        self.owner_uuid = Some(by.1);
        self.acceleration_power = 0.1;
        true
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }
}
