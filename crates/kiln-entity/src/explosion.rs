//! `ServerExplosion`: vanilla's ray-marched block destruction, entity damage and knockback.
//!
//! Block drops come from loot tables, which this crate does not have: every destroyed block is
//! reported as [`Event::BlockExploded`] (in vanilla's shuffled order) for the simulation to drop
//! loot, and is then set to air.

use crate::blocks::{Kind, kind};
use crate::clip;
use crate::collision;
use crate::entity::{Entity, EntityKind};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3, floor, lerp};
use crate::physics;
use kiln_javamath::random::RandomSource;

/// `Explosion.BlockInteraction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    Keep,
    Destroy,
    DestroyWithDecay,
    TriggerBlock,
    /// `Level.ExplosionInteraction.TNT` with default game rules (`Destroy`).
    Tnt,
}

impl Interaction {
    fn resolved(self) -> Interaction {
        if self == Interaction::Tnt { Interaction::Destroy } else { self }
    }
}

/// `ServerLevel.explode` + `ServerExplosion.explode`: returns the destroyed positions in the
/// order they were processed.
pub fn explode(
    level: &mut dyn EntityLevel,
    source: Option<i32>,
    center: Vec3,
    radius: f32,
    fire: bool,
    interaction: Interaction,
) -> Vec<BlockPos> {
    let interaction = interaction.resolved();
    level.emit(Event::GameEvent { event: "minecraft:explode", pos: center, entity: source });
    let mut positions = exploded_positions(level, center, radius);
    hurt_entities(level, source, center, radius, interaction);
    if interaction != Interaction::Keep {
        shuffle(&mut positions, level);
        for &pos in &positions {
            let state = level.block(pos);
            on_explosion_hit(level, source, pos, state, interaction);
        }
    }
    if fire {
        for &pos in &positions {
            if level.random().next_int_bounded(3) == 0
                && physics::is_air(level.block(pos))
                && kiln_data::block_props::solid_render(level.block(pos.below()))
            {
                level.set_block(pos, kiln_data::blocks::default_state::FIRE, 3);
            }
        }
    }
    level.emit(Event::Explosion { pos: center, power: radius, blocks: positions.clone(), source });
    positions
}

/// `calculateExplodedPositions`: 16³ edge rays losing strength through blocks. The result is in
/// `HashSet<BlockPos>` iteration order, which vanilla's shuffle starts from.
fn exploded_positions(level: &mut dyn EntityLevel, center: Vec3, radius: f32) -> Vec<BlockPos> {
    let mut set: Vec<BlockPos> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for i in 0..16 {
        for j in 0..16 {
            for k in 0..16 {
                if i != 0 && i != 15 && j != 0 && j != 15 && k != 0 && k != 15 {
                    continue;
                }
                let mut dx = (i as f32 / 15.0 * 2.0 - 1.0) as f64;
                let mut dy = (j as f32 / 15.0 * 2.0 - 1.0) as f64;
                let mut dz = (k as f32 / 15.0 * 2.0 - 1.0) as f64;
                let len = (dx * dx + dy * dy + dz * dz).sqrt();
                dx /= len;
                dy /= len;
                dz /= len;
                let mut strength = radius * (0.7 + level.random().next_float() * 0.6);
                let (mut x, mut y, mut z) = (center.x, center.y, center.z);
                while strength > 0.0 {
                    let pos = BlockPos::containing(x, y, z);
                    if !in_world_bounds(level, pos) {
                        break;
                    }
                    let state = level.block(pos);
                    let f = physics::fluid_state(state);
                    if !(physics::is_air(state) && f.is_empty()) {
                        let fluid_res = if f.is_empty() { 0.0 } else { 100.0 };
                        let res = physics::block_factors(state).explosion_resistance.max(fluid_res);
                        strength -= (res + 0.3) * 0.3;
                    }
                    if strength > 0.0 && seen.insert(pos) {
                        set.push(pos);
                    }
                    x += dx * 0.30000001192092896;
                    y += dy * 0.30000001192092896;
                    z += dz * 0.30000001192092896;
                    strength -= 0.22500001;
                }
            }
        }
    }
    java_hash_set_order(set)
}

fn in_world_bounds(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    pos.y >= level.min_y() && pos.y <= level.max_y() && pos.x.abs() < 30_000_000 && pos.z.abs() < 30_000_000
}

/// Iteration order of a `java.util.HashSet<BlockPos>` filled in `inserted` order (no bin ever
/// treeifies at these sizes): by bucket of the final table, insertion order within a bucket.
fn java_hash_set_order(inserted: Vec<BlockPos>) -> Vec<BlockPos> {
    let mut cap = 16usize;
    while inserted.len() > cap * 3 / 4 {
        cap *= 2;
    }
    let hash = |p: &BlockPos| {
        let h = (p.y.wrapping_add(p.z.wrapping_mul(31))).wrapping_mul(31).wrapping_add(p.x);
        (h ^ ((h as u32) >> 16) as i32) as u32 as usize
    };
    let mut keyed: Vec<(usize, usize, BlockPos)> =
        inserted.into_iter().enumerate().map(|(i, p)| (hash(&p) & (cap - 1), i, p)).collect();
    keyed.sort_unstable_by_key(|&(b, i, _)| (b, i));
    keyed.into_iter().map(|(_, _, p)| p).collect()
}

/// `Util.shuffle(list, level.random)`.
fn shuffle(list: &mut [BlockPos], level: &mut dyn EntityLevel) {
    let mut i = list.len();
    while i > 1 {
        let j = level.random().next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
        i -= 1;
    }
}

/// `BlockBehaviour.onExplosionHit` (+ `TntBlock.wasExploded`).
fn on_explosion_hit(level: &mut dyn EntityLevel, source: Option<i32>, pos: BlockPos, state: u16, interaction: Interaction) {
    if physics::is_air(state) || interaction == Interaction::TriggerBlock {
        return;
    }
    if kind(state) != Kind::Tnt {
        level.emit(Event::BlockExploded { pos, state, decay: interaction == Interaction::DestroyWithDecay, source });
    }
    level.set_block(pos, 0, 3);
    if kind(state) == Kind::Tnt {
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let owner = source.and_then(|s| match level.entity(s).map(|e| &e.kind) {
            Some(EntityKind::Tnt(d)) => d.owner,
            _ => None,
        });
        let mut tnt = crate::tnt::ignite(id, 0, Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5), owner, seed);
        let fuse = crate::tnt::DEFAULT_FUSE;
        let short = level.random().next_int_bounded((fuse / 4).max(1)) + fuse / 8;
        if let EntityKind::Tnt(d) = &mut tnt.kind {
            d.fuse = short;
        }
        level.add_entity(tnt);
    }
}

/// `hurtEntities`: damage by exposure and distance, and knockback.
fn hurt_entities(level: &mut dyn EntityLevel, source: Option<i32>, center: Vec3, radius: f32, interaction: Interaction) {
    if radius < 1.0e-5 {
        return;
    }
    let r2 = radius * 2.0;
    let area = Aabb::new(
        floor(center.x - r2 as f64 - 1.0) as f64,
        floor(center.y - r2 as f64 - 1.0) as f64,
        floor(center.z - r2 as f64 - 1.0) as f64,
        floor(center.x + r2 as f64 + 1.0) as f64,
        floor(center.y + r2 as f64 + 1.0) as f64,
        floor(center.z + r2 as f64 + 1.0) as f64,
    );
    let affects_blocklike = level.mob_griefing() || matches!(interaction, Interaction::Destroy | Interaction::DestroyWithDecay);
    for id in level.entities_in(&area, EntityFilter::Any, source.unwrap_or(i32::MIN)) {
        let Some(e) = level.entity(id) else { continue };
        if matches!(e.kind, EntityKind::Item(_)) && !affects_blocklike {
            continue;
        }
        let dist = distance_to_sqr(e, center).sqrt() / r2 as f64;
        if dist > 1.0 {
            continue;
        }
        let from = if matches!(e.kind, EntityKind::Tnt(_)) { e.position() } else { Vec3::new(e.x(), e.eye_y(), e.z()) };
        let dir = (from - center).normalize();
        let seen = seen_percent(level, center, e);
        let damage = {
            let d = (1.0 - dist) * seen as f64;
            ((d * d + d) / 2.0 * 7.0 * r2 as f64 + 1.0) as f32
        };
        let knockback = (1.0 - dist) * seen as f64;
        let push = dir.scale(knockback);
        let Some(mut e) = level.entity_mut(id).map(|e| std::mem::replace(e, placeholder())) else { continue };
        e.hurt(level, DamageKind::Explosion, damage, source);
        if push.x.is_finite() && push.y.is_finite() && push.z.is_finite() {
            e.delta = e.delta.add(push.x, push.y, push.z);
            e.needs_sync = true;
        }
        if let Some(slot) = level.entity_mut(id) {
            *slot = e;
        }
    }
}

fn placeholder() -> Entity {
    Entity::new("minecraft:marker", i32::MIN, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0)
}

/// `Entity.distanceToSqr(Vec3)`.
fn distance_to_sqr(e: &Entity, v: Vec3) -> f64 {
    let dx = e.x() - v.x;
    let dy = e.y() - v.y;
    let dz = e.z() - v.z;
    dx * dx + dy * dy + dz * dz
}

/// `ServerExplosion.getSeenPercent`: the share of sample points in the box with a clear line
/// (collision shapes) to the center.
pub fn seen_percent(level: &dyn EntityLevel, center: Vec3, e: &Entity) -> f32 {
    let bb = e.bounding_box();
    let sx = 1.0 / ((bb.max_x - bb.min_x) * 2.0 + 1.0);
    let sy = 1.0 / ((bb.max_y - bb.min_y) * 2.0 + 1.0);
    let sz = 1.0 / ((bb.max_z - bb.min_z) * 2.0 + 1.0);
    let ox = (1.0 - (1.0 / sx).floor() * sx) / 2.0;
    let oz = (1.0 - (1.0 / sz).floor() * sz) / 2.0;
    if sx < 0.0 || sy < 0.0 || sz < 0.0 {
        return 0.0;
    }
    let ctx = e.collision_context();
    let (mut hits, mut total) = (0, 0);
    let mut x = 0.0;
    while x <= 1.0 {
        let mut y = 0.0;
        while y <= 1.0 {
            let mut z = 0.0;
            while z <= 1.0 {
                let p = Vec3::new(lerp(x, bb.min_x, bb.max_x) + ox, lerp(y, bb.min_y, bb.max_y), lerp(z, bb.min_z, bb.max_z) + oz);
                let blocked = clip::traverse_blocks(p, center, |pos| {
                    let (shape, _) = collision::collision_shape(level.block(pos), pos, &ctx);
                    clip::shape_clips(shape, p, center, pos).then_some(())
                });
                if blocked.is_none() {
                    hits += 1;
                }
                total += 1;
                z += sz;
            }
            y += sy;
        }
        x += sx;
    }
    hits as f32 / total as f32
}
