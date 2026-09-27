//! `FallingBlockEntity`: sand, gravel, concrete powder and anvils in flight; lands as a block
//! or breaks into an item.

use crate::blocks::{Kind, Tag, block_name, has_tag, kind};
use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Direction, Vec3, ceil};
use crate::physics;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

#[derive(Clone, Debug)]
pub struct FallingBlockData {
    pub state: u16,
    pub time: i32,
    pub drop_item: bool,
    pub cancel_drop: bool,
    pub hurt_entities: bool,
    pub fall_damage_max: i32,
    pub fall_damage_per_distance: f32,
}

impl FallingBlockData {
    pub fn new(state: u16) -> Self {
        FallingBlockData {
            state,
            time: 0,
            drop_item: true,
            cancel_drop: false,
            hurt_entities: false,
            fall_damage_max: 40,
            fall_damage_per_distance: 0.0,
        }
    }
}

/// `FallingBlockEntity.fall`: replaces the block at `pos` with a falling entity (the caller
/// removes the block, `level.setBlock(pos, fluidState.createLegacyBlock())`).
pub fn fall(id: i32, uuid: u128, pos: BlockPos, state: u16, seed: i64) -> Entity {
    let info = kiln_data::blocks_types::block_of(state);
    let state = info.with_property(state, "waterlogged", "false").unwrap_or(state);
    let mut e = Entity::new("minecraft:falling_block", id, uuid, EntityKind::FallingBlock(FallingBlockData::new(state)), seed);
    e.set_pos(Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5));
    e.set_old_pos_and_rot();
    if let (Kind::Anvil, EntityKind::FallingBlock(d)) = (kind(state), &mut e.kind) {
        d.hurt_entities = true;
        d.fall_damage_per_distance = 2.0;
    }
    e
}

fn data(e: &mut Entity) -> &mut FallingBlockData {
    match &mut e.kind {
        EntityKind::FallingBlock(d) => d,
        _ => unreachable!("not a falling block"),
    }
}

/// `FallingBlockEntity.tick`.
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    let state = data(e).state;
    if physics::is_air(state) {
        e.discard();
        return;
    }
    data(e).time += 1;
    e.apply_gravity();
    e.do_move(level, MoverType::SelfMove, e.delta);
    e.apply_effects_from_blocks(level);
    if e.is_alive() {
        let pos = e.block_position();
        let concrete = kind(state) == Kind::ConcretePowder;
        let in_water = concrete && physics::fluid_state(level.block(pos)).kind.is_water();
        if e.on_ground || in_water {
            let here = level.block(pos);
            e.delta = e.delta.multiply(0.7, -0.5, 0.7);
            if kind(here) != Kind::MovingPiston {
                land(e, level, pos, here, concrete, in_water);
            }
        } else {
            let time = data(e).time;
            let out_of_world = pos.y <= level.min_y() || pos.y > level.max_y();
            if (time > 100 && out_of_world) || time > 600 {
                if data(e).drop_item {
                    spawn_drop(e, level, state);
                }
                e.discard();
            }
        }
    }
    e.delta = e.delta.scale(e.air_drag() as f64);
}

fn land(e: &mut Entity, level: &mut dyn EntityLevel, pos: BlockPos, here: u16, concrete: bool, in_water: bool) {
    let state = data(e).state;
    if data(e).cancel_drop {
        e.discard();
        broken_after_fall(e, level, state, pos);
        return;
    }
    let replaceable = can_be_replaced_by_falling(here);
    let free_below = is_free(level.block(pos.below())) && (!concrete || !in_water);
    let placeable = can_survive(state) && !free_below;
    if replaceable && placeable {
        let mut place = state;
        let info = kiln_data::blocks_types::block_of(state);
        if info.property(state, "waterlogged").is_some() {
            let f = physics::fluid_state(here);
            if f.kind == physics::FluidKind::Water {
                place = info.with_property(state, "waterlogged", "true").unwrap_or(state);
                data(e).state = place;
            }
        }
        if level.set_block(pos, place, 3) {
            e.discard();
            on_land(e, level, pos, place, here);
        } else if data(e).drop_item {
            e.discard();
            broken_after_fall(e, level, state, pos);
            spawn_drop(e, level, state);
        }
    } else {
        e.discard();
        if data(e).drop_item {
            broken_after_fall(e, level, state, pos);
            spawn_drop(e, level, state);
        }
    }
}

/// `BlockState.canBeReplaced(DirectionalPlaceContext(.., ItemStack.EMPTY, ..))`.
fn can_be_replaced_by_falling(state: u16) -> bool {
    let name = block_name(state);
    let info = kiln_data::blocks_types::block_of(state);
    if name == "minecraft:snow" {
        return info.property(state, "layers") == Some("1");
    }
    if name.ends_with("_slab") || name == "minecraft:scaffolding" || name == "minecraft:end_portal" || name == "minecraft:end_gateway" {
        return false;
    }
    if matches!(name, "minecraft:glow_lichen" | "minecraft:sculk_vein" | "minecraft:resin_clump") {
        return true;
    }
    physics::can_be_replaced(state)
}

/// `FallingBlock.isFree`.
pub fn is_free(state: u16) -> bool {
    physics::is_air(state) || has_tag(state, Tag::Fire) || physics::is_liquid(state) || physics::can_be_replaced(state)
}

/// `canSurvive` of the falling block itself (sand, gravel, concrete powder and anvils always do).
fn can_survive(_state: u16) -> bool {
    true
}

/// `Fallable.onLand`.
fn on_land(e: &mut Entity, level: &mut dyn EntityLevel, pos: BlockPos, state: u16, replaced: u16) {
    match kind(state) {
        Kind::Anvil => {
            if !e.silent {
                level.emit(Event::LevelEvent { event: 1031, pos, data: 0 });
            }
        }
        Kind::ConcretePowder if physics::fluid_state(replaced).kind.is_water() || touches_water(level, pos) => {
            let concrete = block_name(state).trim_end_matches("_powder").to_string();
            if let Some(b) = kiln_data::blocks_types::block_by_name(&concrete) {
                level.set_block(pos, b.default, 3);
            }
        }
        _ => {}
    }
}

/// `ConcretePowderBlock.touchesLiquid`.
fn touches_water(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    for d in Direction::ALL {
        let s = level.block(pos.relative(d));
        let water = physics::fluid_state(s).kind.is_water();
        if (d != Direction::Down || water) && water && !physics::is_face_sturdy(s, d.opposite()) {
            return true;
        }
    }
    false
}

/// `Fallable.onBrokenAfterFall`.
fn broken_after_fall(e: &mut Entity, level: &mut dyn EntityLevel, state: u16, pos: BlockPos) {
    if kind(state) == Kind::Anvil && !e.silent {
        level.emit(Event::LevelEvent { event: 1029, pos, data: 0 });
    }
}

/// `spawnAtLocation(level, block)`: the block's item with the default pickup delay.
fn spawn_drop(e: &mut Entity, level: &mut dyn EntityLevel, state: u16) {
    let Some(stack) = ItemStack::of(block_name(state), 1) else { return };
    if stack.is_empty() {
        return;
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new_at(id, 0, stack, e.position(), seed);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
    }
    level.add_entity(item);
}

/// `FallingBlockEntity.causeFallDamage`: hurts living entities it lands in; anvils may chip.
pub fn cause_fall_damage(e: &mut Entity, level: &mut dyn EntityLevel, distance: f64, _multiplier: f32) -> bool {
    if !data(e).hurt_entities {
        return false;
    }
    let i = ceil(distance - 1.0);
    if i < 0 {
        return false;
    }
    let (state, per, max) = { let d = data(e); (d.state, d.fall_damage_per_distance, d.fall_damage_max) };
    let damage = kiln_javamath::math::floor_f32(i as f32 * per).min(max) as f32;
    let kind_ = match kind(state) {
        Kind::Anvil => DamageKind::FallingAnvil,
        Kind::PointedDripstone => DamageKind::FallingStalactite,
        _ => DamageKind::FallingBlock,
    };
    for id in level.entities_in(&e.bounding_box(), EntityFilter::Living, e.id) {
        if level.entity(id).is_some_and(Entity::is_alive) {
            level.emit(Event::Hurt { target: id, amount: damage, kind: kind_, attacker: Some(e.id) });
        }
    }
    if has_tag(state, Tag::Anvil) && damage > 0.0 && e.random.next_float() < 0.05 + i as f32 * 0.05 {
        let next = match block_name(state) {
            "minecraft:anvil" => Some("minecraft:chipped_anvil"),
            "minecraft:chipped_anvil" => Some("minecraft:damaged_anvil"),
            _ => None,
        };
        match next {
            None => data(e).cancel_drop = true,
            Some(n) => {
                let info = kiln_data::blocks_types::block_of(state);
                let facing = info.property(state, "facing").unwrap_or("north");
                let b = kiln_data::blocks_types::block_by_name(n).unwrap();
                data(e).state = b.with_property(b.default, "facing", facing).unwrap_or(b.default);
            }
        }
    }
    false
}
