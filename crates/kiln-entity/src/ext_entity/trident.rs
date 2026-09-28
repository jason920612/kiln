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
    /// `pickup == CREATIVE_ONLY` (a creative player's trident).
    pub creative_only: bool,
    /// `ID_LOYALTY`: the return acceleration (loyalty's level).
    pub loyalty: u8,
    /// The thrown item (`pickupItemStack`), when a player threw it.
    pub item: Option<kiln_item::ItemStack>,
    /// Channeling: a hit in a thunderstorm under the open sky calls lightning.
    pub channeling: bool,
    /// Damage the weapon's enchantments add to the 8 (without target conditions).
    pub bonus_damage: f32,
    /// `clientSideReturnTridentTickCount`.
    pub returning: i32,
}

/// A trident a player threw: its item, loyalty, channeling and extra damage.
#[allow(clippy::too_many_arguments)]
pub fn thrown_by_player(pos: Vec3, owner: i32, item: kiln_item::ItemStack, loyalty: u8, channeling: bool, bonus_damage: f32, creative: bool, seed: i64) -> Entity {
    let data = Trident {
        owner: Some(owner),
        pickup: !creative,
        creative_only: creative,
        loyalty,
        item: Some(item),
        channeling,
        bonus_damage,
        ..Default::default()
    };
    let mut e = Entity::new("minecraft:trident", 0, 0, EntityKind::Ext(Box::new(data)), seed);
    e.set_pos(pos);
    e.set_old_pos_and_rot();
    e
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
    let pickup_mode = r.byte_or("pickup", 0);
    let dealt_damage = r.bool_or("DealtDamage", false);
    let item = r.get("item").and_then(|t| kiln_item::ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty());
    let enchanted = |name: &str| item.as_ref().is_some_and(|s| enchantment_level(s, name) > 0);
    let loyalty = item.as_ref().map_or(0, |s| enchantment_level(s, "minecraft:loyalty").clamp(0, 127) as u8);
    let channeling = enchanted("minecraft:channeling");
    Some(Box::new(Trident {
        owner: None,
        left_owner,
        left_owner_checked: false,
        has_been_shot,
        in_ground,
        in_ground_time: 0,
        shake_time,
        life,
        last_state,
        dealt_damage,
        pickup: pickup_mode == 1,
        creative_only: pickup_mode == 2,
        loyalty,
        item,
        channeling,
        bonus_damage: 0.0,
        returning: 0,
    }))
}

/// The level of enchantment `name` on a stack (loaded tridents keep loyalty and channeling).
fn enchantment_level(s: &kiln_item::ItemStack, name: &str) -> i32 {
    let Some(id) = kiln_item::registry::ENCHANTMENT.id(name) else { return 0 };
    s.get(kiln_item::keys::ENCHANTMENTS).map_or(0, |e| e.level(id))
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

    fn entity_data(&self, e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::{abstract_arrow, thrown_trident};
        if self.in_ground {
            d.set(abstract_arrow::IN_GROUND, &DataValue::Boolean(true));
        }
        if e.no_physics {
            d.set(abstract_arrow::ID_FLAGS, &DataValue::Byte(2));
        }
        if self.loyalty > 0 {
            d.set(thrown_trident::ID_LOYALTY, &DataValue::Byte(self.loyalty as i8));
        }
        if self.item.as_ref().is_some_and(|s| s.get(kiln_item::keys::ENCHANTMENTS).is_some_and(|e| !e.0.is_empty())) {
            d.set(thrown_trident::ID_FOIL, &DataValue::Boolean(true));
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
        o.put("pickup", Tag::Byte(if self.creative_only { 2 } else { self.pickup as i8 }));
        o.put("damage", Tag::Double(2.0));
        o.put("crit", Tag::Byte(0));
        o.put("DealtDamage", Tag::Byte(self.dealt_damage as i8));
        let item = self
            .item
            .as_ref()
            .map(kiln_item::ItemStack::to_nbt)
            .unwrap_or_else(|| Tag::Compound(vec![("id".into(), Tag::String("minecraft:trident".into())), ("count".into(), Tag::Int(1))]));
        o.put("item", item);
    }

    /// `ThrownTrident.tick` then `AbstractArrow.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        if self.in_ground_time > 4 {
            self.dealt_damage = true;
        }
        // Loyalty: back to a living owner once it hit something.
        if self.loyalty > 0
            && (self.dealt_damage || e.no_physics)
            && let Some(owner) = self.owner
        {
            match level.player(owner).filter(|v| v.alive && !v.spectator) {
                None => {
                    // `spawnAtLocation(pickupItem, 0.1)`.
                    if self.pickup && let Some(item) = self.item.clone() {
                        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
                        let mut it = crate::item::new_at(id, 0, item, e.position().add(0.0, 0.1, 0.0), seed);
                        if let EntityKind::Item(d) = &mut it.kind {
                            d.pickup_delay = 10;
                        }
                        level.add_entity(it);
                    }
                    e.discard();
                    return;
                }
                Some(v) => {
                    e.no_physics = true;
                    let eye = v.pos.add(0.0, v.eye_height as f64, 0.0);
                    let to = eye - e.position();
                    let p = e.position();
                    e.set_pos_raw(Vec3::new(p.x, p.y + to.y * 0.015 * self.loyalty as f64, p.z));
                    let d = 0.05 * self.loyalty as f64;
                    e.delta = e.delta.scale(0.95) + to.normalize().scale(d);
                    if self.returning == 0 {
                        e.play_sound(level, "minecraft:item.trident.return", 10.0, 1.0);
                    }
                    self.returning += 1;
                }
            }
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
    /// Channeling's lightning at `at`: a thundering level and open sky there (approximation:
    /// full sky light for `canSeeSky`); the bolt credits the thrower.
    fn call_lightning(&self, level: &mut dyn EntityLevel, at: Vec3) {
        let Some(owner) = self.owner.filter(|_| self.channeling) else { return };
        if !level.is_thundering() || level.sky_light(BlockPos::containing(at.x, at.y, at.z)) < 15 {
            return;
        }
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let bolt = crate::ext_entity::lightning::channeled(id, 0, at, owner, seed);
        level.add_entity(bolt);
    }

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
        let damage = 8.0 + self.bonus_damage;
        let hurt = if is_player {
            level.hurt_player(id, source, damage)
        } else if target.1 {
            let Some(slot) = level.entity_mut(id) else { return };
            let mut t = std::mem::replace(slot, Entity::new("minecraft:marker", i32::MIN, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
            let r = crate::mob::hurt_entity(&mut t, level, source, damage);
            if let Some(slot) = level.entity_mut(id) {
                *slot = t;
            }
            r
        } else {
            level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: e.type_name, owner: self.owner, hit: Hit::Entity { id, location } });
            false
        };
        // Channeling (`post_attack`: `summon_entity` in a thunderstorm under the open sky).
        if hurt {
            self.call_lightning(level, target.0);
        }
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
        // Channeling's `hit_block`: a lightning rod calls the bolt.
        if crate::blocks::block_name(level.block(pos)) == "minecraft:lightning_rod" {
            self.call_lightning(level, Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 1.0, pos.z as f64 + 0.5));
        }
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
