//! Firework rockets (`FireworkRocketEntity`): launched from a block face they rise (x1.15
//! horizontal drag-up, +0.04 upward a tick) for `10 * (1 + flight) + nextInt(6) + nextInt(7)`
//! ticks and burst; used while gliding they ride along with the player (the client applies
//! the boost itself) and burst the same way. A rocket with explosions hurts every living
//! entity within five blocks that it has an unobstructed line to (`5 + 2 * explosions`,
//! less with distance) and the glider it is attached to takes the full amount.

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind, MoverType};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, Vec3};
use crate::mob::{self, DamageSource};
use crate::persist::{Input, Output};
use crate::projectile::Hit;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Debug)]
pub struct Firework {
    pub owner: Option<i32>,
    pub life: i32,
    pub lifetime: i32,
    /// The glider it rides along with (`DATA_ATTACHED_TO_TARGET`).
    pub attached: Option<i32>,
    pub shot_at_angle: bool,
    pub item: ItemStack,
    left_owner: bool,
    has_been_shot: bool,
}

/// `FireworkRocketEntity.getDefaultItem`.
fn default_item() -> ItemStack {
    ItemStack::of("minecraft:firework_rocket", 1).unwrap_or_else(ItemStack::empty)
}

fn fireworks(item: &ItemStack) -> Option<&kiln_item::component::Fireworks> {
    item.get(kiln_item::keys::FIREWORKS)
}

fn explosions(item: &ItemStack) -> i32 {
    fireworks(item).map_or(0, |f| f.explosions.len() as i32)
}

/// `FireworkRocketEntity(level, x, y, z, item)`: at `pos`, drifting up, with its lifetime from
/// the item's flight duration. `owner` and `attached` are the launching player.
pub fn new(pos: Vec3, item: ItemStack, owner: Option<i32>, attached: Option<i32>, shot_at_angle: bool, seed: i64) -> Entity {
    let flight = 1 + fireworks(&item).map_or(0, |f| f.flight_duration);
    let mut e = Entity::new("minecraft:firework_rocket", 0, 0, EntityKind::Other { type_name: "minecraft:firework_rocket" }, seed);
    e.set_pos(pos);
    e.delta = Vec3::new(mob::mth::triangle(&mut e.random, 0.0, 0.002297), 0.05, mob::mth::triangle(&mut e.random, 0.0, 0.002297));
    let lifetime = 10 * flight + e.random.next_int_bounded(6) + e.random.next_int_bounded(7);
    e.set_old_pos_and_rot();
    e.needs_sync = true;
    let x = Firework { owner, life: 0, lifetime, attached, shot_at_angle, item, left_owner: false, has_been_shot: false };
    e.kind = EntityKind::Ext(Box::new(x));
    e
}

/// Reads a saved one.
pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let item = r.get("FireworksItem").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_else(default_item);
    Some(Box::new(Firework {
        owner: None,
        life: r.int_or("Life", 0),
        lifetime: r.int_or("LifeTime", 0),
        attached: None,
        shot_at_angle: r.bool_or("ShotAtAngle", false),
        item,
        left_owner: r.bool_or("LeftOwner", false),
        has_been_shot: r.bool_or("HasBeenShot", false),
    }))
}

/// `Level.clip(ClipContext(from, to, COLLIDER, NONE))` finds nothing.
fn line_clear(level: &dyn EntityLevel, from: Vec3, to: Vec3) -> bool {
    let ctx = collision::CollisionContext::EMPTY;
    clip::traverse_blocks(from, to, |pos| {
        let (shape, _) = collision::collision_shape(level.block(pos), pos, &ctx);
        clip::shape_clip(&shape, from, to, pos).map(|_| ())
    })
    .is_none()
}

impl Firework {
    /// `canHitEntity`: not the owner until the rocket has left it.
    fn can_hit(&self, id: i32) -> bool {
        Some(id) != self.owner || self.left_owner
    }

    /// `ProjectileUtil.getHitResultOnMoveVector`.
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
        let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0)) as f64;
        let area = e.bounding_box().expand_towards_vec(delta).inflate_all(1.0);
        let mut best = f64::MAX;
        let mut hit = None;
        for id in level.entities_in(&area, EntityFilter::Any, e.id) {
            let Some(t) = level.entity(id) else { continue };
            if !crate::projectile::can_be_hit_by_projectile(t) || !self.can_hit(id) || t.no_physics {
                continue;
            }
            if let Some(p) = t.bounding_box().inflate_all(margin).clip(from, to) {
                let d = from.distance_to_sqr(p);
                if d < best {
                    best = d;
                    hit = Some(Hit::Entity { id, location: p });
                }
            }
        }
        // (The players whose box touches the swept area, which holds the whole segment with a
        // block to spare: a player the margin would let the segment touch is among them.)
        for v in &level.players_in(&area) {
            if !v.alive || v.spectator || !self.can_hit(v.id) {
                continue;
            }
            let h = v.height as f64;
            let bb = Aabb::new(v.pos.x - 0.3, v.pos.y, v.pos.z - 0.3, v.pos.x + 0.3, v.pos.y + h, v.pos.z + 0.3);
            if let Some(p) = bb.inflate_all(margin).clip(from, to) {
                let d = from.distance_to_sqr(p);
                if d < best {
                    best = d;
                    hit = Some(Hit::Entity { id: v.id, location: p });
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
        self.left_owner = match self.owner {
            None => true,
            Some(o) => {
                let bb = match level.player(o) {
                    Some(v) => Some(Aabb::new(v.pos.x - 0.3, v.pos.y, v.pos.z - 0.3, v.pos.x + 0.3, v.pos.y + 1.8, v.pos.z + 0.3)),
                    None => level.entity(o).map(|t| t.bounding_box()),
                };
                bb.is_none_or(|b| !area.intersects(&b))
            }
        };
    }

    /// `explode`: the burst event, then `dealExplosionDamage`; the rocket is gone.
    fn explode(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        level.emit(Event::EntityEvent { entity: e.id, event: 17 });
        level.emit(Event::GameEvent { event: "minecraft:explode", pos: e.position(), entity: self.owner });
        self.deal_explosion_damage(e, level);
        e.discard();
    }

    fn deal_explosion_damage(&self, e: &Entity, level: &mut dyn EntityLevel) {
        let n = explosions(&self.item);
        if n == 0 {
            return;
        }
        let f = 5.0 + (n * 2) as f32;
        let source = DamageSource { kind: DamageKind::Fireworks, attacker: self.owner, direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
        if let Some(a) = self.attached {
            hurt(level, a, source, f);
        }
        let at = e.position();
        let area = e.bounding_box().inflate_all(5.0);
        let mut targets: Vec<(i32, Vec3, f64)> = Vec::new();
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            if let Some(t) = level.entity(id) {
                targets.push((id, t.position(), t.height as f64));
            }
        }
        // (Players within 5 blocks of the burst stand in the box: `area` holds every point
        // within 5 blocks of it.)
        for v in &level.players_in(&area) {
            if v.alive && !v.spectator {
                targets.push((v.id, v.pos, v.height as f64));
            }
        }
        for (id, pos, height) in targets {
            if Some(id) == self.attached {
                continue;
            }
            let dist_sqr = pos.distance_to_sqr(at);
            if dist_sqr > 25.0 {
                continue;
            }
            // Two probes, at the target's feet and mid height.
            let visible = (0..2).any(|i| line_clear(level, Vec3::new(pos.x, pos.y + height * 0.5 * i as f64, pos.z), at));
            if visible {
                let d = (dist_sqr as f32).sqrt();
                let amount = f * (((5.0 - d as f64) / 5.0).sqrt() as f32);
                hurt(level, id, source, amount);
            }
        }
    }
}

/// `target.hurtServer(source, amount)` for a mob, a player or another entity.
fn hurt(level: &mut dyn EntityLevel, id: i32, source: DamageSource, amount: f32) -> bool {
    if level.player(id).is_some() {
        return level.hurt_player(id, source, amount);
    }
    let Some(t) = level.entity_mut(id) else { return false };
    let marker = Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0);
    let mut t2 = std::mem::replace(t, marker);
    let r = if matches!(t2.kind, EntityKind::Mob(_)) { mob::hurt_entity(&mut t2, level, source, amount) } else { t2.hurt(level, source.kind, amount, source.attacker) };
    if let Some(slot) = level.entity_mut(id) {
        *slot = t2;
    }
    r
}

impl EntityExt for Firework {
    crate::entity_ext_boilerplate!();

    /// `FireworkRocketEntity.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `Projectile.tick`.
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        self.check_left_owner(e, level);
        e.base_tick(level);
        let mut hit = None;
        if let Some(a) = self.attached {
            match level.player(a).filter(|v| v.alive) {
                // The glider left or died: the rocket has nothing to ride.
                None => self.attached = None,
                Some(v) => {
                    e.set_pos(v.pos);
                    hit = None;
                }
            }
        }
        if self.attached.is_none() {
            if !self.shot_at_angle {
                let d = if e.horizontal_collision { 1.0 } else { 1.15 };
                e.delta = e.delta.multiply(d, 1.0, d).add(0.0, 0.04, 0.0);
            }
            let movement = e.delta;
            hit = self.hit_on_move_vector(e, level);
            e.do_move(level, MoverType::SelfMove, movement);
            e.apply_effects_from_blocks(level);
            e.delta = movement;
        }
        if let Some(hit) = hit
            && e.is_alive()
        {
            // `hitTargetOrDeflectSelf`: a breeze turns the rocket back.
            if let Hit::Entity { id, .. } = hit
                && crate::projectile::deflected_by_target(e, level, id)
            {
                e.needs_sync = true;
            } else {
            level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: "minecraft:firework_rocket", owner: self.owner, hit });
            match hit {
                Hit::Entity { .. } => self.explode(e, level),
                Hit::Block { .. } if explosions(&self.item) > 0 => self.explode(e, level),
                Hit::Block { .. } => {}
            }
            e.needs_sync = true;
            }
        }
        if !e.is_alive() {
            return;
        }
        // `updateRotation`.
        rotate(e);
        if self.life == 0 {
            e.play_sound(level, "minecraft:entity.firework_rocket.launch", 3.0, 1.0);
        }
        self.life += 1;
        if self.life > self.lifetime {
            self.explode(e, level);
        }
    }

    fn gravity(&self) -> f64 {
        0.0
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("Life", Tag::Int(self.life));
        o.put("LifeTime", Tag::Int(self.lifetime));
        o.put("FireworksItem", self.item.to_nbt());
        o.put("ShotAtAngle", Tag::Byte(self.shot_at_angle as i8));
        o.put("LeftOwner", Tag::Byte(self.left_owner as i8));
        o.put("HasBeenShot", Tag::Byte(self.has_been_shot as i8));
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::firework_rocket_entity as f;
        let mut bytes = bytes::BytesMut::new();
        self.item.write_optional(&mut bytes);
        d.set(f::ID_FIREWORKS_ITEM, &DataValue::EncodedItemStack(bytes.freeze()));
        if let Some(a) = self.attached {
            d.set(f::ATTACHED_TO_TARGET, &DataValue::OptionalUnsignedInt(Some(a as u32)));
        }
        if self.shot_at_angle {
            d.set(f::SHOT_AT_ANGLE, &DataValue::Boolean(true));
        }
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }
}

/// `Projectile.updateRotation` (`lerpRotation` with 0.2).
fn rotate(e: &mut Entity) {
    let v = e.delta;
    if v.length_sqr() == 0.0 {
        return;
    }
    let h = v.horizontal_distance();
    let yaw = (crate::projectile::mth_atan2(v.x, v.z) * 57.2957763671875) as f32;
    let pitch = (crate::projectile::mth_atan2(v.y, h) * 57.2957763671875) as f32;
    e.x_rot = crate::projectile::lerp_rotation(e.x_rot_o, pitch);
    e.y_rot = crate::projectile::lerp_rotation(e.y_rot_o, yaw);
}
