//! Shulker bullets (`ShulkerBullet`): fly block by block along the axes toward their target,
//! hurt it (4, mob projectile) and make it levitate for 10 seconds.

use crate::entity::{Entity, EntityKind};
use crate::entity_ext_boilerplate;
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Axis, BlockPos, Direction, Vec3};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

#[derive(Clone, Debug)]
pub struct ShulkerBullet {
    pub owner: Option<i32>,
    /// `finalTarget`: the entity id (while known) and its UUID.
    pub target: Option<i32>,
    pub target_uuid: Option<u128>,
    pub dir: Option<Direction>,
    pub flight_steps: i32,
    pub target_delta: Vec3,
    left_owner: bool,
    has_been_shot: bool,
}

/// A bullet from `owner` (at the center of its box) toward `target`, first avoiding `axis`
/// (the shulker's attachment axis).
pub fn new(id: i32, owner: &Entity, target: i32, axis: Axis, level: &mut dyn EntityLevel, seed: i64) -> Entity {
    let target_uuid = level.player(target).map(|p| p.uuid).or_else(|| level.entity(target).map(|t| t.uuid));
    let b = ShulkerBullet {
        owner: Some(owner.id),
        target: Some(target),
        target_uuid,
        dir: Some(Direction::Up),
        flight_steps: 0,
        target_delta: Vec3::ZERO,
        left_owner: false,
        has_been_shot: false,
    };
    let mut e = Entity::new("minecraft:shulker_bullet", id, 0, EntityKind::Other { type_name: "minecraft:shulker_bullet" }, seed);
    e.no_physics = true;
    e.set_pos(owner.bounding_box().center());
    e.set_old_pos_and_rot();
    let mut b = b;
    let t = target_of(level, target);
    b.select_next_move_direction(&mut e, level, Some(axis), t);
    e.kind = EntityKind::Ext(Box::new(b));
    e
}

/// What the bullet needs of its target: position, height, block position, alive.
#[derive(Clone, Copy)]
struct Target {
    pos: Vec3,
    height: f64,
    alive: bool,
    spectator: bool,
}

fn target_of(level: &dyn EntityLevel, id: i32) -> Option<Target> {
    if let Some(p) = level.player(id) {
        return Some(Target { pos: p.pos, height: if p.sneaking { 1.5 } else { 1.8 }, alive: p.alive, spectator: p.spectator });
    }
    let e = level.entity(id)?;
    let alive = e.is_alive() && crate::mob::data(e).is_none_or(|m| m.health > 0.0);
    Some(Target { pos: e.position(), height: e.height as f64, alive, spectator: false })
}

fn empty(level: &dyn EntityLevel, p: BlockPos) -> bool {
    kiln_data::blocks_types::is_air(level.block(p))
}

fn random_direction(e: &mut Entity) -> Direction {
    Direction::ALL[e.random.next_int_bounded(6) as usize]
}

impl ShulkerBullet {
    /// `selectNextMoveDirection`.
    fn select_next_move_direction(&mut self, e: &mut Entity, level: &dyn EntityLevel, avoid: Option<Axis>, target: Option<Target>) {
        let mut y_off = 0.5;
        let tp = match target {
            None => e.block_position().below(),
            Some(t) => {
                y_off = t.height * 0.5;
                BlockPos::containing(t.pos.x, t.pos.y + y_off, t.pos.z)
            }
        };
        let (mut tx, mut ty, mut tz) = (tp.x as f64 + 0.5, tp.y as f64 + y_off, tp.z as f64 + 0.5);
        let mut dir = None;
        let c = tp.center();
        if c.distance_to_sqr(e.position()) >= 4.0 {
            let cur = e.block_position();
            let mut options = Vec::new();
            if avoid != Some(Axis::X) {
                if cur.x < tp.x && empty(level, cur.relative(Direction::East)) {
                    options.push(Direction::East);
                } else if cur.x > tp.x && empty(level, cur.relative(Direction::West)) {
                    options.push(Direction::West);
                }
            }
            if avoid != Some(Axis::Y) {
                if cur.y < tp.y && empty(level, cur.above()) {
                    options.push(Direction::Up);
                } else if cur.y > tp.y && empty(level, cur.below()) {
                    options.push(Direction::Down);
                }
            }
            if avoid != Some(Axis::Z) {
                if cur.z < tp.z && empty(level, cur.relative(Direction::South)) {
                    options.push(Direction::South);
                } else if cur.z > tp.z && empty(level, cur.relative(Direction::North)) {
                    options.push(Direction::North);
                }
            }
            let mut d = random_direction(e);
            if options.is_empty() {
                let mut tries = 5;
                while !empty(level, cur.relative(d)) && tries > 0 {
                    d = random_direction(e);
                    tries -= 1;
                }
            } else {
                d = options[e.random.next_int_bounded(options.len() as i32) as usize];
            }
            let (sx, sy, sz) = d.step();
            tx = e.x() + sx as f64;
            ty = e.y() + sy as f64;
            tz = e.z() + sz as f64;
            dir = Some(d);
        }
        self.dir = dir;
        let (dx, dy, dz) = (tx - e.x(), ty - e.y(), tz - e.z());
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        self.target_delta = if len == 0.0 { Vec3::ZERO } else { Vec3::new(dx / len * 0.15, dy / len * 0.15, dz / len * 0.15) };
        e.needs_sync = true;
        self.flight_steps = 10 + e.random.next_int_bounded(5) * 10;
    }

    /// `Projectile.checkLeftOwner`.
    fn check_left_owner(&mut self, e: &Entity, level: &dyn EntityLevel) {
        if self.left_owner {
            return;
        }
        let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
        self.left_owner = match self.owner.and_then(|id| level.entity(id)) {
            Some(o) => !(o.is_alive() && area.intersects(&o.bounding_box())),
            None => true,
        };
    }

    /// `ProjectileUtil.getHitResultOnMoveVector` (collider shapes, no fluids).
    fn hit(&self, e: &Entity, level: &dyn EntityLevel) -> Option<Hit> {
        let from = e.position();
        let mut to = from + e.delta;
        let ctx = e.collision_context();
        let block = crate::clip::traverse_blocks(from, to, |pos| {
            let (shape, _) = crate::collision::collision_shape(level.block(pos), pos, &ctx);
            crate::clip::shape_clip(&shape, from, to, pos).map(|(location, _)| (pos, location))
        });
        if let Some((_, l)) = block {
            to = l;
        }
        let margin = kiln_javamath::math::max(0.0, kiln_javamath::math::min(0.3, (e.tick_count - 2) as f32 / 20.0)) as f64;
        let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
        let mut best = f64::MAX;
        let mut hit = None;
        for id in level.entities_in(&area, EntityFilter::Any, e.id) {
            let Some(t) = level.entity(id) else { continue };
            let hittable = t.is_alive()
                && !t.no_physics
                && match &t.kind {
                    EntityKind::Mob(m) => m.health > 0.0,
                    EntityKind::Other { type_name } => *type_name == "minecraft:player",
                    EntityKind::Player(_) | EntityKind::Tnt(_) | EntityKind::FallingBlock(_) => true,
                    _ => false,
                };
            if !hittable || (!self.left_owner && Some(id) == self.owner) {
                continue;
            }
            if let Some(p) = t.bounding_box().inflate_all(margin).clip(from, to) {
                let d = from.distance_to_sqr(p);
                if d < best {
                    best = d;
                    hit = Some(Hit::Entity(id));
                }
            }
        }
        hit.or(block.map(|_| Hit::Block))
    }

    fn destroy(&self, e: &mut Entity, level: &mut dyn EntityLevel) {
        e.discard();
        level.emit(Event::GameEvent { event: "minecraft:entity_damage", pos: e.position(), entity: Some(e.id) });
    }

    /// `onHitEntity`: 4 damage, then levitation for 10 seconds. Approximation: mobs have no
    /// effects in Kiln, so only players levitate.
    fn on_hit_entity(&self, e: &mut Entity, level: &mut dyn EntityLevel, id: i32) {
        let source = crate::mob::DamageSource {
            kind: DamageKind::MobProjectile,
            attacker: self.owner,
            direct: Some(e.id),
            pos: Some(e.position()),
            attacker_is_player: false,
        };
        let hurt = if level.player(id).is_some() {
            level.hurt_player(id, source, 4.0)
        } else {
            match level.entity_mut(id) {
                Some(o) => {
                    let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
                    let r = if matches!(o2.kind, EntityKind::Mob(_)) {
                        crate::mob::hurt_entity(&mut o2, level, source, 4.0)
                    } else {
                        o2.hurt(level, DamageKind::MobProjectile, 4.0, self.owner)
                    };
                    if let Some(slot) = level.entity_mut(id) {
                        *slot = o2;
                    }
                    r
                }
                None => false,
            }
        };
        if hurt && level.player(id).is_some() {
            level.add_effect(id, "minecraft:levitation", 200, 0, self.owner.or(Some(e.id)));
        }
    }
}

enum Hit {
    Block,
    Entity(i32),
}

impl EntityExt for ShulkerBullet {
    entity_ext_boilerplate!();

    fn gravity(&self) -> f64 {
        0.04
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `Projectile.tick`.
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        self.check_left_owner(e, level);
        e.base_tick(level);
        // `ShulkerBullet.tick`.
        let mut target = self.target.and_then(|id| target_of(level, id));
        if target.is_none() {
            self.target = None;
        }
        match target {
            Some(t) if t.alive && !t.spectator => {
                let c = |v: f64| crate::mob::mth::clamp_d(v * 1.025, -1.0, 1.0);
                self.target_delta = Vec3::new(c(self.target_delta.x), c(self.target_delta.y), c(self.target_delta.z));
                let v = e.delta;
                let d = self.target_delta;
                e.delta = v.add((d.x - v.x) * 0.2, (d.y - v.y) * 0.2, (d.z - v.z) * 0.2);
            }
            _ => e.apply_gravity(),
        }
        let hit = self.hit(e, level);
        let v = e.delta;
        e.set_pos(e.position() + v);
        e.apply_effects_from_blocks(level);
        if let Some(h) = hit
            && e.is_alive()
        {
            match h {
                Hit::Entity(id) => self.on_hit_entity(e, level, id),
                Hit::Block => {
                    if !e.silent {
                        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.shulker_bullet.hit", source: "hostile", volume: 1.0, pitch: 1.0 });
                    }
                }
            }
            self.destroy(e, level);
        }
        // `ProjectileUtil.rotateTowardsMovement(this, 0.5)`.
        let v = e.delta;
        let h = v.horizontal_distance();
        if v.length_sqr() != 0.0 {
        let yr = (crate::mob::mth::atan2(v.z, v.x) * 57.2957763671875) as f32 + 90.0;
        let xr = (crate::mob::mth::atan2(h, v.y) * 57.2957763671875) as f32 - 90.0;
        e.x_rot = rotate_lerp(e.x_rot_o, xr, 0.5);
        e.y_rot = rotate_lerp(e.y_rot_o, yr, 0.5);
        }
        if e.is_removed() {
            return;
        }
        if target.is_some() {
            target = self.target.and_then(|id| target_of(level, id));
        }
        if let Some(t) = target {
            if self.flight_steps > 0 {
                self.flight_steps -= 1;
                if self.flight_steps == 0 {
                    let axis = self.dir.map(|d| d.axis());
                    self.select_next_move_direction(e, level, axis, Some(t));
                }
            }
            if let Some(d) = self.dir {
                let pos = e.block_position();
                let axis = d.axis();
                let n = pos.relative(d);
                if level.is_loaded(n) && crate::physics::is_face_sturdy(level.block(n), Direction::Up) {
                    self.select_next_move_direction(e, level, Some(axis), Some(t));
                } else {
                    let tb = BlockPos::containing(t.pos.x, t.pos.y, t.pos.z);
                    if (axis == Axis::X && pos.x == tb.x) || (axis == Axis::Z && pos.z == tb.z) || (axis == Axis::Y && pos.y == tb.y) {
                        self.select_next_move_direction(e, level, Some(axis), Some(t));
                    }
                }
            }
        }
    }

    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, _kind: DamageKind, _amount: f32, _attacker: Option<i32>) -> bool {
        if !e.silent {
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.shulker_bullet.hurt", source: "hostile", volume: 1.0, pitch: 1.0 });
        }
        self.destroy(e, level);
        true
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        if let Some(u) = self.target_uuid {
            o.put("Target", crate::persist::uuid_to_tag(u));
        }
        if let Some(d) = self.dir {
            o.put("Dir", Tag::Byte(d.index() as i8));
        }
        o.put("Steps", Tag::Int(self.flight_steps));
        o.put("TXD", Tag::Double(self.target_delta.x));
        o.put("TYD", Tag::Double(self.target_delta.y));
        o.put("TZD", Tag::Double(self.target_delta.z));
    }
}

/// `Mth.rotLerp`-style easing of `ProjectileUtil.rotateTowardsMovement`.
fn rotate_lerp(mut current: f32, target: f32, t: f32) -> f32 {
    while target - current < -180.0 {
        current -= 360.0;
    }
    while target - current >= 180.0 {
        current += 360.0;
    }
    crate::mob::mth::lerp_f(t, current, target)
}

/// Reads a saved one. Its target is known by UUID only (resolved when it is a player the
/// level can name; approximation: otherwise it falls).
pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let target_uuid = r.uuid("Target");
    let dir = r.num("Dir").and_then(|v| Direction::ALL.get(v as i64 as usize).copied());
    let flight_steps = r.int_or("Steps", 0);
    let d = Vec3::new(r.num("TXD").unwrap_or(0.0), r.num("TYD").unwrap_or(0.0), r.num("TZD").unwrap_or(0.0));
    Some(Box::new(ShulkerBullet { owner: None, target: None, target_uuid, dir, flight_steps, target_delta: d, left_owner: true, has_been_shot: true }))
}
