//! Thrown tridents (`ThrownTrident`, an `AbstractArrow`), thrown by drowned.
//!
//! Flight as an arrow's (gravity 0.05, drag 0.99 in the air and in water), 8 damage on the first
//! entity hit (`minecraft:trident`), after which the trident bounces back (`deflect(REVERSE)`)
//! and passes through entities; it sticks in blocks and despawns after a minute there. Mob
//! tridents cannot be picked up. Loyalty (return to the thrower) needs an enchanted trident and
//! is not simulated.

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use crate::persist::{Input, Output};
use crate::projectile::{Hit, can_be_hit_by_projectile, lerp_rotation, mth_atan2};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Debug, Default)]
pub struct Trident {
    pub owner: Option<i32>,
    pub left_owner: bool,
    left_owner_checked: bool,
    has_been_shot: bool,
    pub in_ground: bool,
    pub in_ground_time: i32,
    pub shake_time: i32,
    pub life: i32,
    pub last_state: Option<u16>,
    pub dealt_damage: bool,
    /// `pickup == ALLOWED` (a player's trident); mob tridents are `DISALLOWED`.
    pub pickup: bool,
}

/// A trident thrown by `owner` from `pos` (`new ThrownTrident(level, owner, stack)`: at the
/// owner's eyes less 0.1).
pub fn new(id: i32, uuid: u128, pos: Vec3, owner: Option<i32>, seed: i64) -> Entity {
    let data = Trident { owner, ..Default::default() };
    let mut e = Entity::new("minecraft:trident", id, uuid, EntityKind::Ext(Box::new(data)), seed);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
}

/// Reads a saved one.
pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let left_owner = r.bool_or("LeftOwner", false);
    let has_been_shot = r.bool_or("HasBeenShot", false);
    let life = r.short_or("life", 0);
    let last_state = r.get("inBlockState").and_then(crate::persist::state_from_tag);
    let shake_time = (r.byte_or("shake", 0) as i32) & 255;
    let in_ground = r.bool_or("inGround", false);
    let pickup = r.byte_or("pickup", 0) == 1;
    let dealt_damage = r.bool_or("DealtDamage", false);
    Some(Box::new(Trident { owner: None, left_owner, left_owner_checked: false, has_been_shot, in_ground, in_ground_time: 0, shake_time, life, last_state, dealt_damage, pickup }))
}

const GRAVITY: f64 = 0.05;

impl EntityExt for Trident {
    crate::entity_ext_boilerplate!();

    fn gravity(&self) -> f64 {
        GRAVITY
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::abstract_arrow;
        if self.in_ground {
            d.set(abstract_arrow::IN_GROUND, &DataValue::Boolean(true));
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("LeftOwner", Tag::Byte(self.left_owner as i8));
        o.put("HasBeenShot", Tag::Byte(self.has_been_shot as i8));
        o.put("life", Tag::Short(self.life as i16));
        if let Some(s) = self.last_state {
            o.put("inBlockState", crate::persist::state_to_tag(s));
        }
        o.put("shake", Tag::Byte(self.shake_time as i8));
        o.put("inGround", Tag::Byte(self.in_ground as i8));
        o.put("pickup", Tag::Byte(self.pickup as i8));
        o.put("damage", Tag::Double(2.0));
        o.put("crit", Tag::Byte(0));
        o.put("DealtDamage", Tag::Byte(self.dealt_damage as i8));
        o.put("item", Tag::Compound(vec![("id".into(), Tag::String("minecraft:trident".into())), ("count".into(), Tag::Int(1))]));
    }

    /// `ThrownTrident.tick` then `AbstractArrow.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if self.in_ground_time > 4 {
            self.dealt_damage = true;
        }
        let physics = !e.no_physics;
        let v = e.delta;
        let pos = e.block_position();
        let state = level.block(pos);
        if !crate::physics::is_air(state) && physics {
            let (shape, _) = collision::collision_shape(state, pos, &collision::CollisionContext::EMPTY);
            let p = e.position();
            if !shape.is_empty() && shape.boxes().iter().any(|b| b.offset(pos.x as f64, pos.y as f64, pos.z as f64).contains(p)) {
                e.delta = Vec3::ZERO;
                self.in_ground = true;
            }
        }
        if self.shake_time > 0 {
            self.shake_time -= 1;
        }
        if e.is_in_water() || level.is_raining_at(e.block_position()) {
            e.clear_fire();
        }
        if self.in_ground && physics {
            if self.last_state != Some(state) && should_fall(e, level) {
                self.in_ground = false;
                let fx = (e.random.next_float() * 0.2) as f64;
                let fy = (e.random.next_float() * 0.2) as f64;
                let fz = (e.random.next_float() * 0.2) as f64;
                e.delta = e.delta.multiply(fx, fy, fz);
                self.life = 0;
            } else if !self.pickup {
                // `tickDespawn`.
                self.life += 1;
                if self.life >= 1200 {
                    e.discard();
                }
            }
            self.in_ground_time += 1;
            if e.is_alive() {
                e.apply_effects_from_blocks(level);
            }
            return;
        }
        self.in_ground_time = 0;
        let start = e.position();
        if e.is_in_water() {
            // `getWaterInertia`: 0.99 for tridents.
            e.delta = e.delta.scale(0.99f32 as f64);
        }
        let yaw = if physics { mth_atan2(v.x, v.z) } else { mth_atan2(-v.x, -v.z) };
        let pitch = mth_atan2(v.y, v.horizontal_distance());
        e.x_rot = lerp_rotation(e.x_rot, (pitch * 57.2957763671875) as f32);
        e.y_rot = lerp_rotation(e.y_rot, (yaw * 57.2957763671875) as f32);
        self.check_left_owner(e, level);
        if physics {
            let to = start + v;
            let ctx = e.collision_context();
            let block = clip::traverse_blocks(start, to, |pos| {
                let (shape, _) = collision::collision_shape(level.block(pos), pos, &ctx);
                clip::shape_clip(&shape, start, to, pos).map(|(location, face)| (pos, face, location))
            });
            self.step_move_and_hit(e, level, start, to, block);
        } else {
            e.set_pos(start + v);
            e.apply_effects_from_blocks(level);
        }
        if !e.is_in_water() {
            e.delta = e.delta.scale(0.99f32 as f64);
        }
        if physics && !self.in_ground && !e.no_gravity {
            e.delta = e.delta.add(0.0, -GRAVITY, 0.0);
        }
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        self.check_left_owner(e, level);
        e.base_tick(level);
        self.left_owner_checked = false;
    }
}

impl Trident {
    fn check_left_owner(&mut self, e: &Entity, level: &dyn EntityLevel) {
        if self.left_owner || self.left_owner_checked {
            return;
        }
        let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
        self.left_owner = match self.owner.and_then(|id| level.entity(id)) {
            Some(o) => !(can_be_hit_by_projectile(o) && area.intersects(&o.bounding_box())),
            None => true,
        };
        self.left_owner_checked = true;
    }

    /// `stepMoveAndHit`: to the entity (none once it dealt damage) or block hit on the path.
    fn step_move_and_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, from: Vec3, to: Vec3, block: Option<(BlockPos, Direction, Vec3)>) {
        if !e.is_alive() {
            return;
        }
        let end = block.map_or(to, |(_, _, l)| l);
        let mut target: Option<(i32, Vec3)> = None;
        if !self.dealt_damage {
            let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0));
            let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
            let owner = if self.left_owner { None } else { self.owner };
            let mut best = f64::MAX;
            for id in level.entities_in(&area, EntityFilter::Any, e.id) {
                let Some(t) = level.entity(id) else { continue };
                if !can_be_hit_by_projectile(t) || Some(id) == owner {
                    continue;
                }
                if let Some(p) = t.bounding_box().inflate_all(margin as f64).clip(from, end) {
                    let d = from.distance_to_sqr(p);
                    if d < best {
                        best = d;
                        target = Some((id, p));
                    }
                }
            }
        }
        let dest = target.map_or(end, |(_, p)| p);
        e.set_pos(dest);
        e.apply_effects_between(level, from, dest);
        match target {
            None => {
                if let (true, Some((pos, face, location))) = (e.is_alive(), block) {
                    self.hit_block(e, level, pos, face, location);
                    e.needs_sync = true;
                }
            }
            Some((id, location)) => {
                if e.is_alive() && !e.no_physics {
                    self.hit_entity(e, level, id, location);
                    e.needs_sync = true;
                }
            }
        }
    }

    /// `ThrownTrident.onHitEntity`.
    fn hit_entity(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, id: i32, location: Vec3) {
        let is_player = level.player(id).is_some();
        let Some(target) = level.entity(id).map(|t| (t.position(), matches!(t.kind, EntityKind::Mob(_)))) else { return };
        let v = e.delta;
        let source = crate::mob::DamageSource {
            kind: DamageKind::Trident,
            attacker: self.owner.or(Some(e.id)),
            direct: Some(e.id),
            pos: Some(Vec3::new(target.0.x - v.x, target.0.y, target.0.z - v.z)),
            attacker_is_player: self.owner.is_some_and(|o| level.player(o).is_some()),
        };
        self.dealt_damage = true;
        let hurt = if is_player {
            level.hurt_player(id, source, 8.0)
        } else if target.1 {
            let Some(slot) = level.entity_mut(id) else { return };
            let mut t = std::mem::replace(slot, Entity::new("minecraft:marker", i32::MIN, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
            let r = crate::mob::hurt_entity(&mut t, level, source, 8.0);
            if let Some(slot) = level.entity_mut(id) {
                *slot = t;
            }
            r
        } else {
            level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: e.type_name, owner: self.owner, hit: Hit::Entity { id, location } });
            false
        };
        let _ = hurt;
        // `projectileReceivesSideEffectsOnHit`, then `deflect(REVERSE)` from the trident's random.
        e.play_sound(level, "minecraft:item.trident.hit", 1.0, 1.0);
        let yaw = 170.0 + e.random.next_float() * 20.0;
        e.delta = e.delta.multiply(-0.01, -0.1, -0.01);
        e.y_rot += yaw;
        e.y_rot_o += yaw;
    }

    /// `AbstractArrow.onHitBlock`: sticks in the block, backed off 0.05 against the motion.
    fn hit_block(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, pos: BlockPos, face: Direction, location: Vec3) {
        self.last_state = Some(level.block(pos));
        level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: e.type_name, owner: self.owner, hit: Hit::Block { pos, face, location } });
        let d = e.delta;
        let back = Vec3::new(signum(d.x), signum(d.y), signum(d.z)).scale(0.05000000074505806);
        e.set_pos(e.position() - back);
        e.delta = Vec3::ZERO;
        let pitch = 1.2 / (e.random.next_float() * 0.2 + 0.9);
        e.play_sound(level, "minecraft:item.trident.hit_ground", 1.0, pitch);
        self.in_ground = true;
        self.shake_time = 7;
    }
}

fn signum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() { v } else { 1.0f64.copysign(v) }
}

/// `shouldFall`: nothing solid within 0.06 of the tip.
fn should_fall(e: &Entity, level: &dyn EntityLevel) -> bool {
    let p = e.position();
    let area = Aabb::new(p.x, p.y, p.z, p.x, p.y, p.z).inflate_all(0.06);
    collision::no_collision(level, &collision::CollisionContext::EMPTY, i32::MIN, &area)
}
