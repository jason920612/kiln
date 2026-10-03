//! `DispenserBlock.dispenseFrom` and `DropperBlock.dispenseFrom`: a random non-empty slot's
//! item is dropped in front (`DefaultDispenseItemBehavior`) or, for a dropper facing a
//! container, moved into it. Dispensers use the default behaviour for every item, except
//! arrows, spectral arrows, snowballs and eggs (shot), water and lava buckets (placed) and
//! empty buckets (filled from a source) — the other vanilla dispense behaviours (tipped arrows,
//! potions, fire charges, armor, bone meal, shulker boxes, boats, ...) are not simulated yet.

use super::hopper::{View, add_item, container_at, with_target};
use super::triangle;
use crate::blocks::RegionLevel;
use kiln_blocks::{BlockPos, Direction, Effect, Level, state};
use kiln_inventory::stack::StackExt;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};

/// `DispenserBlockEntity.getRandomSlot`: each non-empty slot replaces the pick with chance
/// 1/(its rank).
fn random_slot(items: &[ItemStack], rng: &mut dyn RandomSource) -> Option<usize> {
    let mut slot = None;
    let mut j = 1;
    for (i, s) in items.iter().enumerate() {
        if !s.is_empty() {
            let r = rng.next_int_bounded(j);
            j += 1;
            if r == 0 {
                slot = Some(i);
            }
        }
    }
    slot
}

/// `DispenserBlock.getDispensePosition`: 0.7 blocks out of the front face's centre.
fn dispense_position(pos: BlockPos, facing: Direction) -> [f64; 3] {
    let st = facing.step();
    [pos.x as f64 + 0.5 + 0.7 * st[0] as f64, pos.y as f64 + 0.5 + 0.7 * st[1] as f64, pos.z as f64 + 0.5 + 0.7 * st[2] as f64]
}

/// `DefaultDispenseItemBehavior.spawnItem`.
fn spawn_item(level: &mut RegionLevel, rng: &mut LegacyRandom, stack: ItemStack, accuracy: i32, facing: Direction, at: [f64; 3]) {
    let y = at[1] - if facing.axis() == kiln_blocks::Axis::Y { 0.125 } else { 0.15625 };
    // The `ItemEntity` constructor's random throw, replaced below.
    rng.next_double();
    rng.next_double();
    let speed = rng.next_double() * 0.1 + 0.2;
    let st = facing.step();
    let spread = 0.0172275 * accuracy as f64;
    let vel = [triangle(rng, st[0] as f64 * speed, spread), triangle(rng, 0.2, spread), triangle(rng, st[2] as f64 * speed, spread)];
    level.out.spawns.push(crate::entities::Spawn {
        kind: &kiln_data::entities::types::ITEM,
        pos: [at[0], y, at[2]],
        vel,
        body: crate::entities::Body::Item { stack, pickup_delay: 0, thrower: None },
    });
}

/// `DefaultDispenseItemBehavior.dispense`: one item flies out, with the click sound (level
/// event 1000) and smoke (2000).
fn default_dispense(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> ItemStack {
    let one = stack.split_count(1);
    spawn_item(level, rng, one, 6, facing, dispense_position(pos, facing));
    level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
    stack
}

/// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
pub(crate) fn dispense_from(level: &mut RegionLevel, pos: BlockPos, s: u16) {
    use kiln_data::block_logic::{self as logic, BlockClass as C};
    let dropper = logic::block_class(s) == C::DropperBlock;
    let (loot, game_time, seed) = (level.env.loot.clone(), level.env.game_time, level.env.seed);
    let Some(c) = level.blocks.containers.get_mut(pos) else { return };
    super::unpack_loot(c, pos, loot.as_deref(), false, game_time, seed);
    let items = c.items.clone();
    let mut rng = super::pos_random(level, pos, 4);
    let Some(slot) = random_slot(&items, &mut rng) else {
        level.effect(Effect::LevelEvent { id: 1001, pos, data: 0 });
        if !dropper {
            level.effect(Effect::GameEvent { pos, event: "minecraft:block_activate" });
        }
        return;
    };
    let stack = items[slot].clone();
    let facing = state::get_dir(level.block(pos), "facing").unwrap_or(Direction::North);
    let result = if dropper {
        if stack.is_empty() {
            return;
        }
        match container_at(level, pos.relative(facing)) {
            Some(target) => {
                let left = with_target(level, &target, |dest: &mut View| add_item(dest, stack.copy_with_count(1), Some(facing.opposite()), None));
                for p in target.positions() {
                    crate::jukebox::settle(level, p);
                }
                match left {
                    Some(left) if left.is_empty() => {
                        for p in target.positions() {
                            let st = level.block(p);
                            kiln_blocks::update::update_neighbour_for_output_signal(level, p, kiln_blocks::BlockId::of(st));
                        }
                        let mut r = stack.copy();
                        r.shrink_count(1);
                        r
                    }
                    _ => stack.copy(),
                }
            }
            None => default_dispense(level, &mut rng, pos, facing, stack),
        }
    } else {
        dispense_behaviour(level, &mut rng, pos, facing, stack)
    };
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        kiln_inventory::Container::set_item(c, slot, result);
    }
    let st = level.block(pos);
    kiln_blocks::update::update_neighbour_for_output_signal(level, pos, kiln_blocks::BlockId::of(st));
}

/// The dispenser's behaviour for an item (`DispenserBlock.getDispenseMethod`).
/// `ProjectileDispenseBehavior` with the default `DispenseConfig` (power 1.1, uncertainty 6):
/// the projectile flies out of the front face (`Projectile.shoot`), click sound 1002.
fn dispense_projectile(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> ItemStack {
    use kiln_entity::math::Vec3;
    let at = dispense_position(pos, facing);
    let (kind, entity) = {
        let seed = rng.next_long();
        let origin = Vec3::new(at[0], at[1], at[2]);
        match stack.item_name() {
            "minecraft:snowball" | "minecraft:egg" => {
                let (t, k) = if stack.item_name() == "minecraft:snowball" {
                    (kiln_entity::projectile::Throwable::Snowball, &kiln_data::entities::types::SNOWBALL)
                } else {
                    (kiln_entity::projectile::Throwable::Egg, &kiln_data::entities::types::EGG)
                };
                (k, kiln_entity::projectile::new(0, 0, t, origin, Vec3::new(0.0, 0.0, 0.0), None, seed))
            }
            name => {
                let (type_name, k) = if name == "minecraft:spectral_arrow" {
                    ("minecraft:spectral_arrow", &kiln_data::entities::types::SPECTRAL_ARROW)
                } else {
                    ("minecraft:arrow", &kiln_data::entities::types::ARROW)
                };
                (k, kiln_entity::arrow::new(0, 0, type_name, origin, Vec3::new(0.0, 0.0, 0.0), None, seed))
            }
        }
    };
    let mut entity = entity;
    // `Projectile.getMovementToShoot` and `shoot`: the facing, spread by the entity's random.
    let st = facing.step();
    let len = ((st[0] * st[0] + st[1] * st[1] + st[2] * st[2]) as f64).sqrt();
    let spread = 0.0172275 * 6.0;
    let mut v = [st[0] as f64 / len, st[1] as f64 / len, st[2] as f64 / len];
    for c in &mut v {
        *c += triangle(&mut entity.random, 0.0, spread);
    }
    let v = v.map(|c| c * 1.1);
    entity.delta = Vec3::new(v[0], v[1], v[2]);
    let horizontal = (v[0] * v[0] + v[2] * v[2]).sqrt();
    entity.y_rot = (v[0].atan2(v[2]) * 57.2957763671875) as f32;
    entity.x_rot = (v[1].atan2(horizontal) * 57.2957763671875) as f32;
    entity.y_rot_o = entity.y_rot;
    entity.x_rot_o = entity.x_rot;
    level.out.spawns.push(crate::entities::Spawn { kind, pos: at, vel: v, body: crate::entities::Body::Ready(Box::new(entity)) });
    stack.shrink_count(1);
    level.effect(Effect::LevelEvent { id: 1002, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
    stack
}

/// `MinecartDispenseItemBehavior.execute`: the minecart lands on the rail in front (or on
/// the one below the empty block in front); anything else is dispensed as an item.
fn dispense_minecart(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> ItemStack {
    use kiln_entity::ext_entity::minecart::rail_shape;
    let st = facing.step();
    let (x, y, z) = (pos.x as f64 + 0.5 + st[0] as f64 * 1.125, (pos.y as f64 + 0.5).floor() + st[1] as f64, pos.z as f64 + 0.5 + st[2] as f64 * 1.125);
    let front = pos.relative(facing);
    let state = level.block(front);
    let y_offset = if let Some(shape) = rail_shape(state) {
        if shape.is_slope() { 0.6 } else { 0.1 }
    } else if kiln_data::blocks_types::is_air(state)
        && let Some(below) = rail_shape(level.block(front.below()))
    {
        if facing == Direction::Down || !below.is_slope() { -0.9 } else { -0.4 }
    } else {
        // `DefaultDispenseItemBehavior.dispense`, then this behaviour's own sound and animation.
        let out = default_dispense(level, rng, pos, facing, stack);
        level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
        level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
        return out;
    };
    if let Some(kind) = kiln_data::entities::by_name(stack.item_name()) {
        let cart = crate::boats::new_cart(kind.name, kiln_entity::math::Vec3::new(x, y + y_offset, z), rng.next_long(), &stack);
        level.out.spawns.push(crate::entities::Spawn {
            kind,
            pos: [x, y + y_offset, z],
            vel: [0.0; 3],
            body: crate::entities::Body::Ready(Box::new(cart)),
        });
        stack.shrink_count(1);
    }
    level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
    level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
    stack
}

fn dispense_behaviour(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, stack: ItemStack) -> ItemStack {
    let target = pos.relative(facing);
    match stack.item_name() {
        name if kiln_entity::ext_entity::minecart::is_minecart(name) => dispense_minecart(level, rng, pos, facing, stack),
        "minecraft:arrow" | "minecraft:spectral_arrow" | "minecraft:snowball" | "minecraft:egg" => dispense_projectile(level, rng, pos, facing, stack),
        "minecraft:water_bucket" | "minecraft:lava_bucket" => {
            // `DispenseItemBehavior` for filled buckets: `BucketItem.emptyContents`, then an
            // empty bucket.
            let t = level.block(target);
            if kiln_data::block_props::replaceable(t) || kiln_data::blocks_types::is_air(t) {
                let fluid = if stack.item_name() == "minecraft:water_bucket" {
                    kiln_data::blocks::default_state::WATER
                } else {
                    kiln_data::blocks::default_state::LAVA
                };
                let sound = if fluid == kiln_data::blocks::default_state::WATER { "minecraft:item.bucket.empty" } else { "minecraft:item.bucket.empty_lava" };
                level.effect(Effect::Sound { pos: target, sound, volume: 1.0, pitch: 1.0 });
                kiln_blocks::set_block(level, target, fluid, kiln_blocks::flags::ALL_IMMEDIATE);
                return ItemStack::of("minecraft:bucket", 1).unwrap_or_default();
            }
            default_dispense(level, rng, pos, facing, stack)
        }
        "minecraft:bucket" => {
            // Picks up a fluid source in front (`BucketPickup`).
            let t = level.block(target);
            let f = kiln_data::block_logic::fluid(t);
            if f.source && kiln_blocks::state::is(t, kiln_data::blocks::default_state::WATER) || f.source && kiln_blocks::state::is(t, kiln_data::blocks::default_state::LAVA) {
                let filled = if kiln_blocks::state::is(t, kiln_data::blocks::default_state::WATER) { "minecraft:water_bucket" } else { "minecraft:lava_bucket" };
                kiln_blocks::set_block(level, target, kiln_data::blocks::default_state::AIR, kiln_blocks::flags::ALL_IMMEDIATE);
                let mut rest = stack.clone();
                rest.shrink_count(1);
                let bucket = ItemStack::of(filled, 1).unwrap_or_default();
                if rest.is_empty() {
                    return bucket;
                }
                // `consumeWithRemainder`: into the dispenser, else dropped.
                if let Some(c) = level.blocks.containers.get_mut(pos) {
                    let mut left = bucket;
                    let mut view = View::one(c);
                    left = add_item(&mut view, left, None, None);
                    if !left.is_empty() {
                        spawn_item(level, rng, left, 6, facing, dispense_position(pos, facing));
                        level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
                        level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
                    }
                }
                return rest;
            }
            default_dispense(level, rng, pos, facing, stack)
        }
        _ => default_dispense(level, rng, pos, facing, stack),
    }
}
