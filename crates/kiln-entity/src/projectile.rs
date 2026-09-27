//! `ThrowableProjectile` (snowballs, eggs, ender pearls, potions, experience bottles): flight
//! with drag and gravity, block and entity hit detection (`ProjectileUtil`). What a hit does
//! (damage, hatching, teleporting, splashing) is reported as [`Event::ProjectileHit`].

use crate::blocks::{Kind, kind};
use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Direction, Vec3};

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
    left_owner_checked: bool,
    has_been_shot: bool,
}

/// What a projectile hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Block { pos: BlockPos, face: Direction, location: Vec3 },
    Entity { id: i32, location: Vec3 },
}

/// A new throwable at `pos` moving with `delta`.
pub fn new(id: i32, uuid: u128, kind: Throwable, pos: Vec3, delta: Vec3, owner: Option<i32>, seed: i64) -> Entity {
    let data = ThrowableData { kind, owner, left_owner: false, left_owner_checked: false, has_been_shot: false };
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
    if let Some(hit) = hit {
        if e.is_alive() {
            on_hit(e, level, hit);
        }
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
fn can_be_hit_by_projectile(e: &Entity) -> bool {
    e.is_alive() && matches!(e.kind, EntityKind::Tnt(_) | EntityKind::FallingBlock(_) | EntityKind::Player(_) | EntityKind::Other { .. })
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

fn lerp_rotation(mut current: f32, target: f32) -> f32 {
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
        r = 1.5707963267948966 - r;
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
    level.emit(Event::ProjectileHit { projectile: e.id, projectile_type: kind_.type_name(), owner, hit });
    if kind_ == Throwable::Snowball || kind_ == Throwable::Egg {
        level.emit(Event::EntityEvent { entity: e.id, event: 3 });
    }
    e.discard();
}
