//! Fishing bobbers (`FishingHook`): cast from a player's fishing rod, they fly until they hit
//! an entity (hooking it), land in water (bobbing, then waiting for a fish: lured, approaching,
//! biting) or on the ground. The bobber goes when its owner stops holding a rod, moves more
//! than 32 blocks away or leaves; reeling in ([`Retrieve`]) is the simulation's (the loot, the
//! pull, the rod's wear).
//!
//! The fish timings draw from the bobber's own random as vanilla's do; the particles of an
//! approaching fish are the client's to miss (their draws still happen). Bobbers are not saved
//! (`fishing_bobber` is `noSave`).

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind, MoverType};
use crate::ext_entity::EntityExt;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::mth;
use crate::projectile::{Hit, can_be_hit_by_projectile, lerp_rotation, mth_atan2};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::entity::{DataValue, EntityData};

/// `FishingHook.FishHookState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Flying,
    HookedInEntity,
    Bobbing,
}

#[derive(Clone, Debug)]
pub struct FishingHook {
    /// The player casting (entity id).
    pub owner: i32,
    /// Luck of the sea and lure from the rod.
    pub luck: i32,
    pub lure_speed: i32,
    pub state: State,
    pub hooked: Option<i32>,
    pub life: i32,
    pub out_of_water_time: i32,
    pub open_water: bool,
    pub nibble: i32,
    pub time_until_lured: i32,
    pub time_until_hooked: i32,
    pub fish_angle: f32,
    pub biting: bool,
    has_been_shot: bool,
    left_owner: bool,
}

/// `new FishingHook(player, level, luck, lureSpeed)`: in front of the owner's eyes, thrown
/// along its look with a little spread (from the bobber's random).
pub fn cast(id: i32, uuid: u128, owner: i32, owner_pos: Vec3, eye_height: f64, yaw: f32, pitch: f32, luck: i32, lure_speed: i32, seed: i64) -> Entity {
    let data = FishingHook {
        owner,
        luck: luck.max(0),
        lure_speed: lure_speed.max(0),
        state: State::Flying,
        hooked: None,
        life: 0,
        out_of_water_time: 0,
        open_water: true,
        nibble: 0,
        time_until_lured: 0,
        time_until_hooked: 0,
        fish_angle: 0.0,
        biting: false,
        has_been_shot: false,
        left_owner: false,
    };
    let mut e = Entity::new("minecraft:fishing_bobber", id, uuid, EntityKind::Ext(Box::new(data)), seed);
    let rad = 0.017453292f32;
    let cos_y = mth::cos((-yaw * rad - std::f32::consts::PI) as f64);
    let sin_y = mth::sin((-yaw * rad - std::f32::consts::PI) as f64);
    let cos_p = -mth::cos((-pitch * rad) as f64);
    let sin_p = mth::sin((-pitch * rad) as f64);
    let x = owner_pos.x - sin_y as f64 * 0.3;
    let y = owner_pos.y + eye_height;
    let z = owner_pos.z - cos_y as f64 * 0.3;
    e.set_pos(Vec3::new(x, y, z));
    e.y_rot = yaw;
    e.x_rot = pitch;
    let dir = Vec3::new(-sin_y as f64, (-(sin_p / cos_p)).clamp(-5.0, 5.0) as f64, -cos_y as f64);
    let len = dir.length();
    let fx = 0.6 / len + mth::triangle(&mut e.random, 0.5, 0.0103365);
    let fy = 0.6 / len + mth::triangle(&mut e.random, 0.5, 0.0103365);
    let fz = 0.6 / len + mth::triangle(&mut e.random, 0.5, 0.0103365);
    e.delta = dir.multiply(fx, fy, fz);
    e.y_rot = (mth_atan2(e.delta.x, e.delta.z) * 57.2957763671875) as f32;
    e.x_rot = (mth_atan2(e.delta.y, e.delta.horizontal_distance()) * 57.2957763671875) as f32;
    e.y_rot_o = e.y_rot;
    e.x_rot_o = e.x_rot;
    e.set_old_pos_and_rot();
    e
}

/// The bobber's state for reeling in.
pub fn get(e: &Entity) -> Option<&FishingHook> {
    match &e.kind {
        EntityKind::Ext(x) => x.as_any().downcast_ref::<FishingHook>(),
        _ => None,
    }
}

/// `Mth.nextInt(random, min, max)`.
fn next_int(r: &mut dyn RandomSource, min: i32, max: i32) -> i32 {
    if min >= max { min } else { r.next_int_bounded(max - min + 1) + min }
}

/// `Mth.nextFloat(random, min, max)`.
fn next_float(r: &mut dyn RandomSource, min: f32, max: f32) -> f32 {
    if min >= max { min } else { r.next_float() * (max - min) + min }
}

/// `FishingHook.shouldStopFishing` without the discard: the owner is alive, holds a rod and is
/// within 32 blocks.
pub fn owner_still_fishing(e: &Entity, level: &dyn EntityLevel, owner: i32) -> bool {
    let Some(p) = level.player(owner) else { return false };
    let rod = kiln_data::builtin_id("minecraft:item", "minecraft:fishing_rod").unwrap_or(-1);
    p.alive && !p.spectator && (p.main_hand == rod || p.off_hand == rod) && e.position().distance_to_sqr(p.pos) <= 1024.0
}

impl FishingHook {
    /// `catchingFish`: the fish's approach, the bite, and the wait for the next one.
    fn catching_fish(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, pos: BlockPos) {
        let mut speed = 1;
        let above = pos.above();
        if e.random.next_float() < 0.25 && level.is_raining_at(above) {
            speed += 1;
        }
        if e.random.next_float() < 0.5 && !level.can_see_sky(above) {
            speed -= 1;
        }
        if self.nibble > 0 {
            self.nibble -= 1;
            if self.nibble <= 0 {
                self.time_until_lured = 0;
                self.time_until_hooked = 0;
                self.biting = false;
            }
        } else if self.time_until_hooked > 0 {
            self.time_until_hooked -= speed;
            if self.time_until_hooked > 0 {
                self.fish_angle += mth::triangle(&mut e.random, 0.0, 9.188) as f32;
                let a = self.fish_angle * 0.017453292;
                let (s, c) = (mth::sin(a as f64), mth::cos(a as f64));
                let x = e.position().x + (s * self.time_until_hooked as f32 * 0.1) as f64;
                let y = (crate::math::floor(e.position().y) as f32 + 1.0) as f64;
                let z = e.position().z + (c * self.time_until_hooked as f32 * 0.1) as f64;
                if crate::blocks::block_name(level.block(BlockPos::containing(x, y - 1.0, z))) == "minecraft:water" {
                    // The bubbles and the fish's wake (particles).
                    let _ = e.random.next_float() < 0.15;
                }
            } else {
                let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.4;
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.fishing_bobber.splash", source: "neutral", volume: 0.25, pitch });
                self.nibble = next_int(&mut e.random, 20, 40);
                self.biting = true;
            }
        } else if self.time_until_lured > 0 {
            self.time_until_lured -= speed;
            let mut chance = 0.15f32;
            if self.time_until_lured < 20 {
                chance += (20 - self.time_until_lured) as f32 * 0.05;
            } else if self.time_until_lured < 40 {
                chance += (40 - self.time_until_lured) as f32 * 0.02;
            } else if self.time_until_lured < 60 {
                chance += (60 - self.time_until_lured) as f32 * 0.01;
            }
            if e.random.next_float() < chance {
                let a = next_float(&mut e.random, 0.0, 360.0) * 0.017453292;
                let d = next_float(&mut e.random, 25.0, 60.0);
                let x = e.position().x + (mth::sin(a as f64) * d) as f64 * 0.1;
                let y = (crate::math::floor(e.position().y) as f32 + 1.0) as f64;
                let z = e.position().z + (mth::cos(a as f64) * d) as f64 * 0.1;
                if crate::blocks::block_name(level.block(BlockPos::containing(x, y - 1.0, z))) == "minecraft:water" {
                    // Splash particles: their count is drawn.
                    let _ = e.random.next_int_bounded(2);
                }
            }
            if self.time_until_lured <= 0 {
                self.fish_angle = next_float(&mut e.random, 0.0, 360.0);
                self.time_until_hooked = next_int(&mut e.random, 20, 80);
            }
        } else {
            self.time_until_lured = next_int(&mut e.random, 100, 600);
            self.time_until_lured -= self.lure_speed;
        }
    }

    /// `calculateOpenWater`: a 5x4x5 area around the bobber of water sources (or air and lily
    /// pads above them) in layers that do not switch back.
    fn calculate_open_water(&self, level: &dyn EntityLevel, pos: BlockPos) -> bool {
        #[derive(Clone, Copy, PartialEq)]
        enum T {
            AboveWater,
            InsideWater,
            Invalid,
        }
        let of_block = |p: BlockPos| {
            let s = level.block(p);
            if crate::physics::is_air(s) || crate::blocks::block_name(s) == "minecraft:lily_pad" {
                return T::AboveWater;
            }
            let f = crate::physics::fluid_state(s);
            let (shape, _) = collision::collision_shape(s, p, &collision::CollisionContext::EMPTY);
            if f.kind == crate::physics::FluidKind::Water && f.source && shape.is_empty() { T::InsideWater } else { T::Invalid }
        };
        let mut last = T::Invalid;
        for dy in -1..=2 {
            let mut layer: Option<T> = None;
            for x in pos.x - 2..=pos.x + 2 {
                for y in pos.y + dy..=pos.y + dy {
                    for z in pos.z - 2..=pos.z + 2 {
                        let t = of_block(BlockPos::new(x, y, z));
                        layer = Some(match layer {
                            None => t,
                            Some(a) if a == t => a,
                            Some(_) => T::Invalid,
                        });
                    }
                }
            }
            let t = layer.unwrap_or(T::Invalid);
            match t {
                T::Invalid => return false,
                T::AboveWater if last == T::Invalid => return false,
                T::InsideWater if last == T::AboveWater => return false,
                _ => {}
            }
            last = t;
        }
        true
    }

    /// The flying bobber's hit (`checkCollision`): the nearest entity it may hook on the move,
    /// else the block.
    fn check_collision(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
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
        let area = e.bounding_box().expand_towards_vec(delta).inflate_all(1.0);
        let mut best = f64::MAX;
        let mut hit = None;
        for id in level.entities_in(&area, crate::level::EntityFilter::Any, e.id) {
            if id == self.owner && !self.left_owner {
                continue;
            }
            let Some(t) = level.entity(id) else { continue };
            let item = matches!(t.kind, EntityKind::Item(_)) && t.is_alive();
            if !can_be_hit_by_projectile(t) && !item {
                continue;
            }
            if let Some(p) = t.bounding_box().inflate_all(0.3).clip(from, to) {
                let d = from.distance_to_sqr(p);
                if d < best {
                    best = d;
                    hit = Some(id);
                }
            }
        }
        if let Some(id) = hit {
            self.hooked = Some(id);
        } else if let Some(Hit::Block { location, .. }) = block {
            // `onHitBlock`: up to the block, no further.
            e.delta = e.delta.normalize().scale(location.distance_to_sqr(from).sqrt());
        }
    }
}

impl EntityExt for FishingHook {
    crate::entity_ext_boilerplate!();

    fn gravity(&self) -> f64 {
        0.029999999329447746
    }

    fn spawn_data(&self) -> i32 {
        self.owner
    }

    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::fishing_hook;
        d.set(fishing_hook::HOOKED_ENTITY, &DataValue::Int(self.hooked.map_or(0, |h| h + 1)));
        d.set(fishing_hook::BITING, &DataValue::Boolean(self.biting));
    }

    /// `FishingHook.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        let mut synced = LegacyRandom::new((e.uuid as u64 as i64) ^ level.game_time());
        // `Projectile.tick`: shot, left the owner, the entity base tick.
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: Some(self.owner) });
            self.has_been_shot = true;
        }
        if !self.left_owner {
            let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
            self.left_owner = level.player(self.owner).is_none_or(|p| {
                let b = crate::math::Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + 1.8, p.pos.z + 0.3);
                !area.intersects(&b)
            });
        }
        e.base_tick(level);
        if !owner_still_fishing(e, level, self.owner) {
            e.discard();
            return;
        }
        if e.on_ground {
            self.life += 1;
            if self.life >= 1200 {
                e.discard();
                return;
            }
        } else {
            self.life = 0;
        }
        let pos = e.block_position();
        let f = crate::fluid::fluid_at(level, pos);
        let height = if f.kind == crate::physics::FluidKind::Water { crate::fluid::height(level, pos, &f) } else { 0.0 };
        let in_water = height > 0.0;
        match self.state {
            State::Flying => {
                if self.hooked.is_some() {
                    e.delta = Vec3::ZERO;
                    self.state = State::HookedInEntity;
                    return;
                }
                if in_water {
                    e.delta = e.delta.multiply(0.3, 0.2, 0.3);
                    self.state = State::Bobbing;
                    return;
                }
                self.check_collision(e, level);
            }
            State::HookedInEntity => {
                match self.hooked.and_then(|h| level.entity(h)).filter(|t| t.is_alive()) {
                    Some(t) => {
                        let p = t.position();
                        let y = p.y + t.height as f64 * 0.8;
                        e.set_pos(Vec3::new(p.x, y, p.z));
                    }
                    None => {
                        self.hooked = None;
                        self.state = State::Flying;
                    }
                }
                return;
            }
            State::Bobbing => {
                let v = e.delta;
                let mut d = e.position().y + v.y - pos.y as f64 - height as f64;
                if d.abs() < 0.01 {
                    d += d.signum() * 0.1;
                }
                e.delta = Vec3::new(v.x * 0.9, v.y - d * e.random.next_float() as f64 * 0.2, v.z * 0.9);
                if self.nibble <= 0 && self.time_until_hooked <= 0 {
                    self.open_water = true;
                } else {
                    self.open_water = self.open_water && self.out_of_water_time < 10 && self.calculate_open_water(level, pos);
                }
                if in_water {
                    self.out_of_water_time = (self.out_of_water_time - 1).max(0);
                    if self.biting {
                        let a = synced.next_float() as f64;
                        let b = synced.next_float() as f64;
                        e.delta = e.delta + Vec3::new(0.0, -0.1 * a * b, 0.0);
                    }
                    self.catching_fish(e, level, pos);
                } else {
                    self.out_of_water_time = (self.out_of_water_time + 1).min(10);
                }
            }
        }
        if f.kind != crate::physics::FluidKind::Water && !e.on_ground && self.hooked.is_none() {
            e.delta = e.delta + Vec3::new(0.0, -0.029999999329447746, 0.0);
        }
        let delta = e.delta;
        e.do_move(level, MoverType::SelfMove, delta);
        e.apply_effects_from_blocks(level);
        // `updateRotation`.
        let v = e.delta;
        e.x_rot = lerp_rotation(e.x_rot_o, (mth_atan2(v.y, v.horizontal_distance()) * 57.2957763671875) as f32);
        e.y_rot = lerp_rotation(e.y_rot_o, (mth_atan2(v.x, v.z) * 57.2957763671875) as f32);
        if self.state == State::Flying && (e.on_ground || e.horizontal_collision) {
            e.delta = Vec3::ZERO;
        }
        e.delta = e.delta.scale(0.92);
    }
}
