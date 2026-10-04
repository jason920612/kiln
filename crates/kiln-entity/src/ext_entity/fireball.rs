//! Ghast fireballs and blaze small fireballs (`AbstractHurtingProjectile` → `Fireball` →
//! `LargeFireball` / `SmallFireball`): straight flight that accelerates along its direction
//! (no gravity), then on a hit an explosion with fire (large) or burning damage and a fire
//! block (small). A player's hit sends a fireball back where the player looks.

use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::Vec3;
use crate::mob::{self, DamageSource};
use crate::persist::{Input, Output};
use crate::projectile::Hit;
use kiln_proto::nbt::Tag;

#[derive(Clone, Debug)]
pub struct Fireball {
    /// `SmallFireball` (else `LargeFireball`).
    pub small: bool,
    pub owner: Option<i32>,
    pub acceleration_power: f64,
    pub explosion_power: i32,
    pub left_owner: bool,
    pub has_been_shot: bool,
}

/// A new fireball from `owner` (its feet, facing its rotation) heading along `dir`
/// (`AbstractHurtingProjectile(type, owner, direction, level)`); the caller places it.
pub fn new(small: bool, id: i32, owner: &Entity, dir: Vec3, explosion_power: i32, seed: i64) -> Entity {
    let x = Fireball { small, owner: Some(owner.id), acceleration_power: 0.1, explosion_power, left_owner: false, has_been_shot: false };
    let name = if small { "minecraft:small_fireball" } else { "minecraft:fireball" };
    let mut e = Entity::new(name, id, 0, EntityKind::Ext(Box::new(x)), seed);
    e.set_pos(owner.position());
    // `assignDirectionalMovement(direction, accelerationPower)`.
    e.delta = dir.normalize().scale(0.1);
    e.needs_sync = true;
    e.y_rot = owner.y_rot;
    e.x_rot = owner.x_rot;
    e.set_old_pos_and_rot();
    e
}

/// Reads a saved one.
pub fn load(type_name: &'static str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    Some(Box::new(Fireball {
        small: type_name == "minecraft:small_fireball",
        owner: None,
        acceleration_power: r.num("acceleration_power").unwrap_or(0.1),
        explosion_power: r.byte_or("ExplosionPower", 1) as i32,
        left_owner: r.bool_or("leftOwner", false),
        has_been_shot: r.bool_or("HasBeenShot", false),
    }))
}

/// `ProjectileUtil.getHitResultOnMoveVector` (`COLLIDER` blocks, no fluids) with
/// `canHitEntity` of an `AbstractHurtingProjectile`: not the owner until it left it.
pub(crate) fn hit_on_move_vector(e: &Entity, level: &dyn EntityLevel, owner: Option<i32>, left_owner: bool) -> Option<Hit> {
    Fireball { small: false, owner, acceleration_power: 0.0, explosion_power: 0, left_owner, has_been_shot: false }.hit_on_move_vector(e, level)
}

/// `Projectile.checkLeftOwner`: whether the projectile is clear of its owner's box.
pub(crate) fn left_owner(e: &Entity, level: &dyn EntityLevel, owner: Option<i32>) -> bool {
    let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
    match owner.and_then(|id| level.entity(id)) {
        Some(o) => !(crate::projectile::can_be_hit_by_projectile(o) && area.intersects(&o.bounding_box())),
        None => true,
    }
}

impl Fireball {
    /// `ProjectileUtil.getHitResultOnMoveVector` (`COLLIDER` blocks, no fluids) with
    /// `canHitEntity`: not the owner until the fireball left it.
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
            if !crate::projectile::can_be_hit_by_projectile(t) || Some(id) == owner || t.no_physics {
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

    /// `checkLeftOwner`.
    fn check_left_owner(&mut self, e: &Entity, level: &dyn EntityLevel) {
        if self.left_owner {
            return;
        }
        self.left_owner = left_owner(e, level, self.owner);
    }

    fn on_hit(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit) {
        // `hitTargetOrDeflectSelf`: a breeze turns it back (`onDeflection(false)`: half the
        // acceleration) and nothing is hit.
        if let Hit::Entity { id, .. } = hit
            && let Some(turned) = crate::projectile::try_deflect(e, level, id)
        {
            if turned {
                self.acceleration_power *= 0.5;
            }
            return;
        }
        let name = if self.small { "minecraft:small_fireball" } else { "minecraft:fireball" };
        level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: name, owner: self.owner, hit });
        let source = DamageSource { kind: DamageKind::Fireball, attacker: self.owner, direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
        match hit {
            Hit::Entity { id, .. } => {
                if self.small {
                    // Five seconds of fire, taken back if the hit did not land.
                    let old = level.entity(id).map(|t| t.remaining_fire_ticks);
                    level.ignite(id, 5.0);
                    if !hurt(level, id, source, 5.0)
                        && let (Some(old), Some(t)) = (old, level.entity_mut(id))
                    {
                        t.remaining_fire_ticks = old;
                    }
                } else {
                    hurt(level, id, source, 6.0);
                }
            }
            Hit::Block { pos, face, .. } => {
                if self.small {
                    let owner_is_mob = self.owner.and_then(|o| level.entity(o)).is_some_and(|o| matches!(o.kind, EntityKind::Mob(_)));
                    if !owner_is_mob || level.mob_griefing() {
                        let p = pos.relative(face);
                        if crate::physics::is_air(level.block(p)) {
                            level.set_block(p, fire_state(level, p), 3);
                        }
                    }
                }
            }
        }
        if !self.small {
            let griefing = level.mob_griefing();
            let interaction = if griefing { crate::explosion::Interaction::Mob } else { crate::explosion::Interaction::Keep };
            crate::explosion::explode(level, Some(e.id), e.position(), self.explosion_power as f32, griefing, interaction);
        }
        e.discard();
    }
}

/// `BaseFireBlock.getState`: soul fire on soul blocks, else fire (`FireBlock
/// .getStateForPlacement`: off a burnable or sturdy floor, a face toward each burnable side).
pub(crate) fn fire_state(level: &dyn EntityLevel, p: crate::math::BlockPos) -> u16 {
    use kiln_data::blocks::default_state as d;
    let below = level.block(p.below());
    if crate::blocks::block_name(below) == "minecraft:soul_sand" || crate::blocks::block_name(below) == "minecraft:soul_soil" {
        return d::SOUL_FIRE;
    }
    let can_burn = |s: u16| {
        let wet = kiln_data::blocks_types::block_of(s).property(s, "waterlogged") == Some("true");
        !wet && kiln_data::block_logic::flammability(s).0 > 0
    };
    // Face 1 is up in `Direction` order (down, up, north, south, west, east).
    if can_burn(below) || kiln_data::block_logic::face_sturdy(below, 1, kiln_data::block_logic::Support::Full) {
        return d::FIRE;
    }
    let fire = kiln_data::blocks_types::block_of(d::FIRE);
    let sides = [
        ("up", p.above()),
        ("north", p.offset(0, 0, -1)),
        ("south", p.offset(0, 0, 1)),
        ("west", p.offset(-1, 0, 0)),
        ("east", p.offset(1, 0, 0)),
    ];
    sides.iter().fold(d::FIRE, |s, &(name, at)| {
        fire.with_property(s, name, if can_burn(level.block(at)) { "true" } else { "false" }).unwrap_or(s)
    })
}

/// `target.hurtServer(fireball source, amount)` for a mob, a player or another entity.
pub(crate) fn hurt(level: &mut dyn EntityLevel, id: i32, source: DamageSource, amount: f32) -> bool {
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

impl EntityExt for Fireball {
    crate::entity_ext_boilerplate!();

    /// `AbstractHurtingProjectile.tick`.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `applyInertia` (bubbles in water draw nothing).
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
        rotate_towards_movement(e, 0.2);
        e.set_pos(to);
        e.apply_effects_from_blocks(level);
        // `Projectile.tick`.
        if !self.has_been_shot {
            level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: self.owner });
            self.has_been_shot = true;
        }
        self.check_left_owner(e, level);
        e.base_tick(level);
        // `shouldBurn`.
        e.ignite_for_seconds(1.0);
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
        if !self.small {
            o.put("ExplosionPower", Tag::Byte(self.explosion_power as i8));
        }
    }

    fn spawn_data(&self) -> i32 {
        self.owner.unwrap_or(0)
    }

    /// A hit (a player's attack) sends the fireball back along the attacker's look
    /// (`ProjectileDeflection.AIM_DEFLECT`), with the attacker as its new owner.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, amount: f32, attacker: Option<i32>) -> bool {
        let _ = (kind, amount);
        let Some(a) = attacker else { return false };
        // Players' rotations are not known here: straight back where it came from.
        let look = match level.player(a) {
            Some(_) => e.delta.normalize().scale(-1.0),
            None => match level.entity(a) {
                Some(o) => view_vector(o.x_rot, o.y_rot),
                None => return false,
            },
        };
        e.delta = look;
        e.needs_sync = true;
        self.owner = Some(a);
        self.left_owner = true;
        self.acceleration_power = 0.1;
        true
    }
}

/// `Entity.calculateViewVector(xRot, yRot)`.
pub fn view_vector(x_rot: f32, y_rot: f32) -> Vec3 {
    let f = x_rot * 0.017453292;
    let g = -y_rot * 0.017453292;
    let h = mob::mth::cos(g as f64);
    let i = mob::mth::sin(g as f64);
    let j = mob::mth::cos(f as f64);
    let k = mob::mth::sin(f as f64);
    Vec3::new((i * j) as f64, (-k) as f64, (h * j) as f64)
}

/// `ProjectileUtil.rotateTowardsMovement`.
pub(crate) fn rotate_towards_movement(e: &mut Entity, amount: f32) {
    let v = e.delta;
    if v.length_sqr() == 0.0 {
        return;
    }
    let h = v.horizontal_distance();
    e.y_rot = (mob::mth::atan2(v.z, v.x) * 57.2957763671875) as f32 + 90.0;
    e.x_rot = (mob::mth::atan2(h, v.y) * 57.2957763671875) as f32 - 90.0;
    while e.x_rot - e.x_rot_o < -180.0 {
        e.x_rot_o -= 360.0;
    }
    while e.x_rot - e.x_rot_o >= 180.0 {
        e.x_rot_o += 360.0;
    }
    while e.y_rot - e.y_rot_o < -180.0 {
        e.y_rot_o -= 360.0;
    }
    while e.y_rot - e.y_rot_o >= 180.0 {
        e.y_rot_o += 360.0;
    }
    e.x_rot = e.x_rot_o + amount * (e.x_rot - e.x_rot_o);
    e.y_rot = e.y_rot_o + amount * (e.y_rot - e.y_rot_o);
}
