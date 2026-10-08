//! `PrimedTnt`: a lit TNT block counting down its fuse.

use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::EntityLevel;
use crate::math::Vec3;
use kiln_javamath::random::RandomSource;

pub const DEFAULT_FUSE: i32 = 80;
pub const DEFAULT_POWER: f32 = 4.0;

#[derive(Clone, Debug)]
pub struct TntData {
    pub fuse: i32,
    /// The block the entity renders as (`minecraft:tnt` by default).
    pub block_state: u16,
    pub explosion_power: f32,
    pub owner: Option<i32>,
    pub used_portal: bool,
}

impl TntData {
    pub fn new() -> Self {
        TntData {
            fuse: DEFAULT_FUSE,
            block_state: kiln_data::blocks::default_state::TNT,
            explosion_power: DEFAULT_POWER,
            owner: None,
            used_portal: false,
        }
    }
}

impl Default for TntData {
    fn default() -> Self {
        Self::new()
    }
}

/// `new PrimedTnt(level, x, y, z, owner)`: a small random hop.
///
/// Vanilla's `Math.sin`/`Math.cos`.
pub fn ignite(id: i32, uuid: u128, pos: Vec3, owner: Option<i32>, seed: i64) -> Entity {
    let mut e = Entity::new("minecraft:tnt", id, uuid, EntityKind::Tnt(TntData { owner, ..TntData::new() }), seed);
    e.set_pos(pos);
    let angle = e.random.next_double() * (std::f32::consts::PI as f64) * 2.0;
    e.delta = Vec3::new(-kiln_javamath::trig::sin(angle) * 0.02, 0.2f32 as f64, -kiln_javamath::trig::cos(angle) * 0.02);
    e.set_old_pos_and_rot();
    e
}

/// `PrimedTnt.tick` (no `baseTick`).
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    e.apply_gravity();
    e.do_move(level, MoverType::SelfMove, e.delta);
    e.apply_effects_from_blocks(level);
    e.delta = e.delta.scale(e.air_drag() as f64);
    if e.on_ground {
        e.delta = e.delta.multiply(0.7, -0.5, 0.7);
    }
    let fuse = {
        let EntityKind::Tnt(d) = &mut e.kind else { unreachable!() };
        d.fuse -= 1;
        d.fuse
    };
    if fuse <= 0 {
        e.discard();
        explode(e, level);
    } else {
        e.update_fluid_interaction(level);
    }
}

fn explode(e: &mut Entity, level: &mut dyn EntityLevel) {
    let EntityKind::Tnt(d) = &e.kind else { unreachable!() };
    let (power, owner) = (d.explosion_power, d.owner);
    let center = Vec3::new(e.x(), e.y() + e.height as f64 * 0.0625, e.z());
    let rules = crate::explosion::BlockRules { causing: owner, ..Default::default() };
    crate::explosion::explode_ruled(level, Some(e.id), center, power, false, crate::explosion::Interaction::Tnt, rules, true);
}
