//! Llama spit (`LlamaSpit`): a projectile with a little gravity (0.06) and drag (0.99) that does
//! 1 point of `spit` damage to the first entity it meets (and flies on: only a block stops it),
//! and is gone in water or when its box has no air left in it.

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::{self, DamageSource};
use crate::persist::{Input, Output};
use crate::projectile::Hit;
use kiln_proto::nbt::Tag;

pub const TYPE: &str = "minecraft:llama_spit";

#[derive(Clone, Debug)]
pub struct LlamaSpit {
    pub owner: Option<i32>,
    pub left_owner: bool,
    left_owner_checked: bool,
    pub has_been_shot: bool,
}

/// `new LlamaSpit(level, llama)`: owned by the llama, at the front of its body at the height of
/// its eyes (less 0.1). The caller shoots it and adds it.
pub fn new(id: i32, owner: &Entity, y_body_rot: f32, seed: i64) -> Entity {
    let x = LlamaSpit { owner: Some(owner.id), left_owner: false, left_owner_checked: false, has_been_shot: false };
    let mut e = Entity::new(TYPE, id, 0, EntityKind::Ext(Box::new(x)), seed);
    let reach = (owner.width + 1.0) as f64 * 0.5;
    let rot = (y_body_rot * 0.017453292f32) as f64;
    let pos = Vec3::new(owner.x() - reach * mob::mth::sin(rot) as f64, owner.eye_y() - 0.10000000149011612, owner.z() + reach * mob::mth::cos(rot) as f64);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(LlamaSpit { owner: None, left_owner: r.bool_or("LeftOwner", false), left_owner_checked: false, has_been_shot: r.bool_or("HasBeenShot", false) }))
}

/// `Entity.getRootVehicle`.
fn root_vehicle(level: &dyn EntityLevel, mut id: i32) -> i32 {
    while let Some(v) = level.entity(id).and_then(|e| e.vehicle) {
        id = v;
    }
    id
}

impl LlamaSpit {
    /// `Projectile.canHitEntity`.
    fn can_hit(&self, level: &dyn EntityLevel, id: i32, t: &Entity) -> bool {
        if !crate::projectile::can_be_hit_by_projectile(t) {
            return false;
        }
        match self.owner {
            Some(o) if !self.left_owner => id != o && root_vehicle(level, o) != root_vehicle(level, id),
            _ => true,
        }
    }

    /// `ProjectileUtil.getHitResultOnMoveVector`: blocks (`COLLIDER`), then the nearest entity
    /// along the way.
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
            if !self.can_hit(level, id, t) {
                continue;
            }
            if let Some(p) = crate::projectile::clip_entity(t, margin as f64, from, to) {
                let d = from.distance_to_sqr(p);
                if d < best {
                    best = d;
                    hit = Some(Hit::Entity { id, location: p });
                }
            }
        }
        hit.or(block)
    }

    /// `Projectile.checkLeftOwner`.
    fn check_left_owner(&mut self, e: &Entity, level: &dyn EntityLevel) {
        if self.left_owner || self.left_owner_checked {
            return;
        }
        let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
        self.left_owner = match self.owner.and_then(|id| level.entity(id)) {
            Some(o) => !(crate::projectile::can_be_hit_by_projectile(o) && area.intersects(&o.bounding_box())),
            None => true,
        };
        self.left_owner_checked = true;
    }

    /// `hitTargetOrDeflectSelf` (nothing here deflects a spit) and `LlamaSpit.onHitEntity` /
    /// `onHitBlock`.
    fn on_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit) {
        level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: TYPE, owner: self.owner, hit });
        match hit {
            Hit::Entity { id, location } => {
                // (The owner is a living entity: a llama.)
                if let Some(owner) = self.owner
                    && let Some(t) = mob::goals::living(level, id)
                {
                    let source = DamageSource { kind: DamageKind::Spit, attacker: Some(owner), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
                    mob::hurt_living(level, &t, source, 1.0);
                }
                level.emit(Event::GameEvent { event: "minecraft:projectile_land", pos: location, entity: Some(e.id) });
            }
            Hit::Block { pos, .. } => {
                // `LlamaSpit.onHitBlock`: gone (the block's own `onProjectileHit` is the
                // simulation's, from the event above).
                e.discard();
                level.emit(Event::GameEvent { event: "minecraft:projectile_land", pos: Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5), entity: Some(e.id) });
            }
        }
    }

    /// `level.findBlocksIn(box).filterState(isAir).noneMatched()`.
    fn no_air_around(e: &Entity, level: &dyn EntityLevel) -> bool {
        let bb = e.bounding_box();
        let lo = BlockPos::containing(bb.min_x, bb.min_y, bb.min_z);
        let hi = BlockPos::new(crate::math::ceil(bb.max_x), crate::math::ceil(bb.max_y), crate::math::ceil(bb.max_z));
        for x in lo.x..hi.x.max(lo.x + 1) {
            for y in lo.y..hi.y.max(lo.y + 1) {
                for z in lo.z..hi.z.max(lo.z + 1) {
                    if kiln_data::blocks_types::is_air(level.block(BlockPos::new(x, y, z))) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

impl EntityExt for LlamaSpit {
    crate::entity_ext_boilerplate!();

    fn gravity(&self) -> f64 {
        0.06
    }

    /// `LlamaSpit.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `Projectile.tick`.
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        self.check_left_owner(e, level);
        e.base_tick(level);
        self.left_owner_checked = false;
        let movement = e.delta;
        let hit = self.hit_on_move_vector(e, level);
        if let Some(hit) = hit {
            self.on_hit(e, level, hit);
        }
        let to = e.position() + movement;
        update_rotation(e);
        if Self::no_air_around(e, level) {
            e.discard();
            return;
        }
        if e.is_in_water() {
            e.discard();
            return;
        }
        e.delta = movement.scale(0.99f32 as f64);
        e.apply_gravity();
        e.set_pos(to);
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("LeftOwner", Tag::Byte(self.left_owner as i8));
        o.put("HasBeenShot", Tag::Byte(self.has_been_shot as i8));
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }
}

/// `Projectile.updateRotation`.
fn update_rotation(e: &mut Entity) {
    let v = e.delta;
    let h = v.horizontal_distance();
    e.x_rot = crate::projectile::lerp_rotation(e.x_rot_o, (crate::projectile::mth_atan2(v.y, h) * 57.2957763671875) as f32);
    e.y_rot = crate::projectile::lerp_rotation(e.y_rot_o, (crate::projectile::mth_atan2(v.x, v.z) * 57.2957763671875) as f32);
}

