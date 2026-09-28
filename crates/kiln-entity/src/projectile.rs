//! `ThrowableProjectile` (snowballs, eggs, ender pearls, potions, experience bottles): flight
//! with drag and gravity, block and entity hit detection (`ProjectileUtil`). What a hit does
//! (damage, hatching, teleporting, splashing) is reported as [`Event::ProjectileHit`].

use crate::blocks::{Kind, kind};
use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};
use kiln_javamath::random::RandomSource;

/// Which throwable this is (their flight differs only in gravity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Throwable {
    Snowball,
    Egg,
    EnderPearl,
    SplashPotion,
    LingeringPotion,
    ExperienceBottle,
}

impl Throwable {
    pub fn type_name(self) -> &'static str {
        match self {
            Throwable::Snowball => "minecraft:snowball",
            Throwable::Egg => "minecraft:egg",
            Throwable::EnderPearl => "minecraft:ender_pearl",
            Throwable::SplashPotion => "minecraft:splash_potion",
            Throwable::LingeringPotion => "minecraft:lingering_potion",
            Throwable::ExperienceBottle => "minecraft:experience_bottle",
        }
    }

    fn gravity(self) -> f64 {
        match self {
            Throwable::SplashPotion | Throwable::LingeringPotion => 0.05,
            Throwable::ExperienceBottle => 0.07,
            _ => 0.03,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ThrowableData {
    pub kind: Throwable,
    pub owner: Option<i32>,
    pub left_owner: bool,
    pub(crate) left_owner_checked: bool,
    pub(crate) has_been_shot: bool,
    /// The thrown item (`DATA_ITEM_STACK`) when known: a splash potion with it splashes here.
    pub item: Option<kiln_item::ItemStack>,
}

/// What a projectile hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Block { pos: BlockPos, face: Direction, location: Vec3 },
    Entity { id: i32, location: Vec3 },
}

/// A new throwable at `pos` moving with `delta`.
pub fn new(id: i32, uuid: u128, kind: Throwable, pos: Vec3, delta: Vec3, owner: Option<i32>, seed: i64) -> Entity {
    let data = ThrowableData { kind, owner, left_owner: false, left_owner_checked: false, has_been_shot: false, item: None };
    let mut e = Entity::new(kind.type_name(), id, uuid, EntityKind::Throwable(data), seed);
    e.set_pos(pos);
    e.delta = delta;
    e.set_old_pos_and_rot();
    e
}

pub(crate) fn gravity(d: &ThrowableData) -> f64 {
    d.kind.gravity()
}

fn data(e: &mut Entity) -> &mut ThrowableData {
    match &mut e.kind {
        EntityKind::Throwable(d) => d,
        _ => unreachable!("not a throwable"),
    }
}

/// `ThrowableProjectile.tick`.
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    if e.first_tick {
        first_tick_bubble_columns(e, level);
    }
    e.apply_gravity();
    let drag = if e.is_in_water() { 0.8f32 } else { 0.99 };
    e.delta = e.delta.scale(drag as f64);
    let hit = hit_on_move_vector(e, level);
    let to = match hit {
        Some(Hit::Block { location, .. } | Hit::Entity { location, .. }) => location,
        None => e.position() + e.delta,
    };
    e.set_pos(to);
    update_rotation(e);
    e.apply_effects_from_blocks(level);
    // Projectile.tick
    if !data(e).has_been_shot {
        level.emit(Event::GameEvent { event: "minecraft:projectile_shoot", pos: e.position(), entity: data(e).owner });
        data(e).has_been_shot = true;
    }
    check_left_owner(e, level);
    e.base_tick(level);
    data(e).left_owner_checked = false;
    if let (Some(hit), true) = (hit, e.is_alive()) {
        on_hit(e, level, hit);
    }
}

/// `handleFirstTickBubbleColumn`: bubble columns overlapping the spawn box act at once.
fn first_tick_bubble_columns(e: &mut Entity, level: &mut dyn EntityLevel) {
    let bb = e.bounding_box();
    let lo = BlockPos::containing(bb.min_x, bb.min_y, bb.min_z);
    let hi = BlockPos::containing(bb.max_x, bb.max_y, bb.max_z);
    for z in lo.z..=hi.z {
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                let pos = BlockPos::new(x, y, z);
                let state = level.block(pos);
                if kind(state) == Kind::BubbleColumn {
                    e.bubble_column_inside(level, pos, state);
                }
            }
        }
    }
}

/// `ProjectileUtil.getHitResultOnMoveVector` with `COLLIDER` block shapes and no fluids.
fn hit_on_move_vector(e: &Entity, level: &dyn EntityLevel) -> Option<Hit> {
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
    entity_hit(e, level, from, to, &area, margin).or(block)
}

/// `ProjectileUtil.getEntityHitResult`: the nearest hittable entity on the segment.
fn entity_hit(e: &Entity, level: &dyn EntityLevel, from: Vec3, to: Vec3, area: &Aabb, margin: f32) -> Option<Hit> {
    let owner = match &e.kind {
        EntityKind::Throwable(d) if !d.left_owner => d.owner,
        _ => None,
    };
    let mut best = f64::MAX;
    let mut hit = None;
    for id in level.entities_in(area, EntityFilter::Any, e.id) {
        let Some(target) = level.entity(id) else { continue };
        if !can_be_hit_by_projectile(target) || Some(id) == owner {
            continue;
        }
        if let Some(p) = target.bounding_box().inflate_all(margin as f64).clip(from, to) {
            let d = from.distance_to_sqr(p);
            if d < best {
                best = d;
                hit = Some(Hit::Entity { id, location: p });
            }
        }
    }
    hit
}

/// `canBeHitByProjectile`: alive and pickable.
pub(crate) fn can_be_hit_by_projectile(e: &Entity) -> bool {
    e.is_alive()
        && match &e.kind {
            EntityKind::Tnt(_) | EntityKind::FallingBlock(_) | EntityKind::Player(_) | EntityKind::Other { .. } => true,
            EntityKind::Mob(m) => m.health > 0.0,
            _ => false,
        }
}

/// `checkLeftOwner`.
fn check_left_owner(e: &mut Entity, level: &dyn EntityLevel) {
    let (owner, left, checked) = { let d = data(e); (d.owner, d.left_owner, d.left_owner_checked) };
    if left || checked {
        return;
    }
    let area = e.bounding_box().expand_towards_vec(e.delta).inflate_all(1.0);
    let outside = match owner.and_then(|id| level.entity(id)) {
        Some(o) => !(can_be_hit_by_projectile(o) && area.intersects(&o.bounding_box())),
        None => true,
    };
    let d = data(e);
    d.left_owner = outside;
    d.left_owner_checked = true;
}

/// `Projectile.updateRotation`: eases toward the direction of travel.
fn update_rotation(e: &mut Entity) {
    let v = e.delta;
    let h = v.horizontal_distance();
    e.x_rot = lerp_rotation(e.x_rot_o, (mth_atan2(v.y, h) * 57.2957763671875) as f32);
    e.y_rot = lerp_rotation(e.y_rot_o, (mth_atan2(v.x, v.z) * 57.2957763671875) as f32);
}

pub(crate) fn lerp_rotation(mut current: f32, target: f32) -> f32 {
    while target - current < -180.0 {
        current -= 360.0;
    }
    while target - current >= 180.0 {
        current += 360.0;
    }
    current + 0.2 * (target - current)
}

/// `Mth.atan2`: vanilla's table-driven arc tangent.
pub fn mth_atan2(mut y: f64, mut x: f64) -> f64 {
    use std::sync::OnceLock;
    static TABLES: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();
    let (asin_tab, cos_tab) = TABLES.get_or_init(|| {
        (0..257)
            .map(|i| {
                let a = (i as f64 / 256.0).asin();
                (a, a.cos())
            })
            .unzip()
    });
    let frac_bias = f64::from_bits(4805340802404319232);
    let d = x * x + y * y;
    if d.is_nan() {
        return f64::NAN;
    }
    let neg_y = y < 0.0;
    if neg_y {
        y = -y;
    }
    let neg_x = x < 0.0;
    if neg_x {
        x = -x;
    }
    let swap = y > x;
    if swap {
        std::mem::swap(&mut x, &mut y);
    }
    let half = 0.5 * d;
    let inv = {
        let i = 6910469410427058090i64.wrapping_sub((d.to_bits() as i64) >> 1);
        let g = f64::from_bits(i as u64);
        g * (1.5 - half * g * g)
    };
    x *= inv;
    y *= inv;
    let yb = frac_bias + y;
    let idx = yb.to_bits() as i32 as usize;
    let asin = asin_tab[idx];
    let cos = cos_tab[idx];
    let yd = yb - frac_bias;
    let s = y * cos - x * yd;
    let f = (6.0 + s * s) * s * 0.16666666666666666;
    let mut r = asin + f;
    if swap {
        r = std::f64::consts::FRAC_PI_2 - r;
    }
    if neg_x {
        r = std::f64::consts::PI - r;
    }
    if neg_y {
        r = -r;
    }
    r
}

/// `onHit`: every throwable breaks on impact; the effect is the simulation's.
fn on_hit(e: &mut Entity, level: &mut dyn EntityLevel, hit: Hit) {
    let (kind_, owner) = { let d = data(e); (d.kind, d.owner) };
    if kind_ == Throwable::SplashPotion
        && let Some(item) = data(e).item.clone()
    {
        crate::mob::kinds::witch::splash(e, level, hit, &item, owner);
        e.discard();
        return;
    }
    // `onHitEntity` of snowballs, eggs and pearls: `thrown` damage (3 to blazes from a snowball,
    // else none; the hit still knocks back).
    if let Hit::Entity { id, .. } = hit
        && matches!(kind_, Throwable::Snowball | Throwable::Egg | Throwable::EnderPearl)
    {
        let blaze = level.entity(id).is_some_and(|t| t.type_name == "minecraft:blaze");
        let amount = if kind_ == Throwable::Snowball && blaze { 3.0 } else { 0.0 };
        thrown_damage(e, level, id, owner, amount);
    }
    level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: kind_.type_name(), owner, hit });
    match kind_ {
        Throwable::Egg => hatch(e, level),
        Throwable::ExperienceBottle => {
            // `ThrownExperienceBottle.onHit`: the splash and 3 to 11 experience.
            level.emit(Event::LevelEvent { event: 2002, pos: e.block_position(), data: -13083194 });
            let amount = 3 + e.random.next_int_bounded(5) + e.random.next_int_bounded(5);
            crate::mob::award_experience(level, e.position(), amount);
        }
        Throwable::EnderPearl => {
            // The portal particles are the client's; their random draws are the pearl's.
            for _ in 0..32 {
                e.random.next_double();
                e.random.next_gaussian();
                e.random.next_gaussian();
            }
            // A player's pearl: one in twenty leaves an endermite where the player was (the
            // teleport itself is the simulation's).
            if let Some(v) = owner.and_then(|o| level.player(o)).filter(|v| v.alive)
                && e.random.next_float() < 0.05
                && level.difficulty() > 0
            {
                let (id, seed) = (level.next_entity_id(), level.fresh_seed());
                let mut mite = crate::mob::new(crate::mob::MobKind::Endermite, id, 0, seed);
                mite.set_pos(v.pos);
                mite.set_old_pos_and_rot();
                level.add_entity(mite);
            }
        }
        _ => {}
    }
    if kind_ == Throwable::Snowball || kind_ == Throwable::Egg {
        level.emit(Event::EntityEvent { entity: e.id, event: 3 });
    }
    e.discard();
}

/// `entity.hurt(damageSources().thrown(this, owner), amount)` on a mob or player.
fn thrown_damage(e: &Entity, level: &mut dyn EntityLevel, id: i32, owner: Option<i32>, amount: f32) {
    let source = crate::mob::DamageSource {
        kind: crate::level::DamageKind::Thrown,
        attacker: owner.or(Some(e.id)),
        direct: Some(e.id),
        pos: Some(e.position()),
        attacker_is_player: owner.is_some_and(|o| level.player(o).is_some()),
    };
    if level.player(id).is_some() {
        level.hurt_player(id, source, amount);
        return;
    }
    if !level.entity(id).is_some_and(|t| matches!(t.kind, EntityKind::Mob(_))) {
        return;
    }
    let Some(slot) = level.entity_mut(id) else { return };
    let mut t = std::mem::replace(slot, Entity::new("minecraft:marker", i32::MIN, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
    crate::mob::hurt_entity(&mut t, level, source, amount);
    if let Some(slot) = level.entity_mut(id) {
        *slot = t;
    }
}

/// `ThrownEgg.onHit`: one in eight eggs hatches a chick (one in 32 of those, four).
fn hatch(e: &mut Entity, level: &mut dyn EntityLevel) {
    if e.random.next_int_bounded(8) != 0 {
        return;
    }
    let n = if e.random.next_int_bounded(32) == 0 { 4 } else { 1 };
    for _ in 0..n {
        let (id, seed) = (level.next_entity_id(), level.fresh_seed());
        let mut chick = crate::mob::new(crate::mob::MobKind::Chicken, id, 0, seed);
        if let Some(mut m) = crate::mob::data(&chick).cloned() {
            crate::mob::set_age(&mut chick, &mut m, -24000);
            if let Some(slot) = crate::mob::data_mut(&mut chick) {
                *slot = m;
            }
        }
        chick.set_pos(e.position());
        chick.y_rot = e.y_rot;
        chick.x_rot = 0.0;
        chick.set_old_pos_and_rot();
        level.add_entity(chick);
    }
}
