//! The ender dragon's fireball (`DragonFireball`, an `AbstractHurtingProjectile`): straight
//! accelerating flight that does not burn; where it hits (anything but its dragon) it leaves a
//! spreading cloud of dragon's breath (radius 3 growing to 7 over 30 seconds, harming).

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::Vec3;
use crate::persist::{Input, Output};
use crate::projectile::Hit;
use kiln_proto::nbt::Tag;

#[derive(Clone, Debug)]
pub struct DragonFireball {
    pub owner: Option<i32>,
    pub acceleration_power: f64,
    pub left_owner: bool,
    pub has_been_shot: bool,
}

/// `new DragonFireball(level, dragon, direction)`: at the dragon's feet with its rotation,
/// heading along `dir`; the caller places it.
pub fn new(id: i32, owner: &Entity, dir: Vec3, seed: i64) -> Entity {
    let x = DragonFireball { owner: Some(owner.id), acceleration_power: 0.1, left_owner: false, has_been_shot: false };
    let mut e = Entity::new("minecraft:dragon_fireball", id, 0, EntityKind::Ext(Box::new(x)), seed);
    e.set_pos(owner.position());
    e.delta = dir.normalize().scale(0.1);
    e.needs_sync = true;
    e.y_rot = owner.y_rot;
    e.x_rot = owner.x_rot;
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(DragonFireball {
        owner: None,
        acceleration_power: r.num("acceleration_power").unwrap_or(0.1),
        left_owner: r.bool_or("leftOwner", false),
        has_been_shot: r.bool_or("HasBeenShot", false),
    }))
}

impl DragonFireball {
    /// `ProjectileUtil.getHitResultOnMoveVector` with `canHitEntity` (no physics-less
    /// entities; the owner is no passenger of anything, so it counts from the start).
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
        let mut best = f64::MAX;
        let mut hit = None;
        for id in level.entities_in(&area, EntityFilter::Any, e.id) {
            let Some(t) = level.entity(id) else { continue };
            // An ender dragon is hit on its parts (pickable, with physics; even its own: they
            // are no passengers of it). Other entities without physics are not hit.
            let clip = if t.type_name == "minecraft:ender_dragon" {
                crate::mob::kinds::ender_dragon::clip_parts(t, margin as f64, from, to)
            } else if !crate::projectile::can_be_hit_by_projectile(t) || t.no_physics {
                continue;
            } else {
                t.bounding_box().inflate_all(margin as f64).clip(from, to)
            };
            if let Some(p) = clip {
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

    /// `DragonFireball.onHit`: the breath cloud, on the first living entity within 4 blocks
    /// when there is one.
    fn on_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit) {
        level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: "minecraft:dragon_fireball", owner: self.owner, hit });
        // `ownedBy`: the owner itself (a hit on a dragon is on one of its parts, never owned).
        if let Hit::Entity { id, .. } = hit
            && Some(id) == self.owner
            && level.entity(id).is_none_or(|t| t.type_name != "minecraft:ender_dragon")
        {
            return;
        }
        let mut at = e.position();
        for id in level.entities_in(&e.bounding_box().inflate(4.0, 2.0, 4.0), EntityFilter::Living, i32::MIN) {
            let p = match level.player(id) {
                Some(p) => p.pos,
                None => match level.entity(id) {
                    Some(t) if crate::mob::data(t).is_some() => t.position(),
                    _ => continue,
                },
            };
            if p.distance_to_sqr(e.position()) < 16.0 {
                at = p;
                break;
            }
        }
        level.emit(Event::LevelEvent { event: 2006, pos: e.block_position(), data: if e.silent { -1 } else { 1 } });
        // `setOwner` when the owner is a living entity.
        let owner = self.owner.and_then(|o| level.entity(o).filter(|o| crate::mob::data(o).is_some()).map(|o| (o.id, o.uuid)));
        crate::mob::kinds::ender_dragon::spawn_breath_cloud(level, owner, at, 3.0, 600, (7.0 - 3.0) / 600.0, 1, 1);
        e.discard();
    }
}

impl EntityExt for DragonFireball {
    crate::entity_ext_boilerplate!();

    /// `AbstractHurtingProjectile.tick` (`shouldBurn` false).
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        let v = e.delta;
        let inertia = if e.is_in_water() { 0.8f32 } else { 0.95 };
        e.delta = (v + v.normalize().scale(self.acceleration_power)).scale(inertia as f64);
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
        super::fireball::rotate_towards_movement(e, 0.2);
        e.set_pos(to);
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
        o.put("acceleration_power", Tag::Double(self.acceleration_power));
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }
}
