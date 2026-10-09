//! The dispenser's behaviours for the items vanilla registers one for (`DispenseItemBehavior.bootStrap`):
//! bone meal, flint and steel, honeycomb, glowstone, glass bottles, water bottles, TNT, shears on a hive,
//! shulker boxes, boats, armor stands and the thrown things. A behaviour returns the stack left in the slot
//! and whether it worked (`OptionalDispenseItemBehavior.isSuccess`: a failure sounds the click of 1001);
//! `DefaultDispenseItemBehavior.dispense` then adds the sound (1000, or 1001) and the animation (2000).

use super::dispense::{dispense_position, spawn_item};
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Direction, Effect, Level, flags, state};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_inventory::stack::StackExt;
use kiln_item::{ItemStack, keys};
use kiln_javamath::random::{LegacyRandom, RandomSource};

/// What a behaviour did: the stack for the slot, and `Some(success)` for the optional behaviours.
pub(super) struct Done {
    pub stack: ItemStack,
    pub success: Option<bool>,
}

fn ok(stack: ItemStack) -> Done {
    Done { stack, success: Some(true) }
}

fn failed(stack: ItemStack) -> Done {
    Done { stack, success: Some(false) }
}

/// `ItemStack.hurtAndBreak(1, level, null, ...)` on a stack nobody holds: unbreaking is not asked.
fn hurt(level: &RegionLevel, stack: &mut ItemStack, rng: &mut LegacyRandom) {
    if !stack.is_damageable_item() {
        return;
    }
    let amount = match &level.env.loot {
        Some(loot) => loot.process_durability_change(stack, rng, 1),
        None => 1,
    };
    if amount == 0 {
        return;
    }
    let damage = stack.damage() + amount;
    if damage >= stack.max_damage() {
        stack.shrink_count(1);
    } else {
        stack.insert(kiln_item::keys::DAMAGE, damage.max(0));
    }
}

/// `DefaultDispenseItemBehavior.consumeWithRemainder`: one of the stack is used and `remainder` goes into the
/// dispenser, or out of it.
fn consume_with_remainder(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack, remainder: ItemStack) -> ItemStack {
    stack.shrink_count(1);
    if stack.is_empty() {
        return remainder;
    }
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        let mut left = remainder;
        let mut view = super::hopper::View::one(c);
        left = super::hopper::add_item(&mut view, left, None, None);
        if !left.is_empty() {
            spawn_item(level, rng, left, 6, facing, dispense_position(pos, facing));
            level.effect(Effect::LevelEvent { id: 1000, pos, data: 0 });
            level.effect(Effect::LevelEvent { id: 2000, pos, data: facing as i32 });
        }
    }
    stack
}

/// The behaviour of `stack` if vanilla registers one that this module has; the stack back otherwise.
pub(super) fn behaviour(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, stack: ItemStack) -> Result<Done, ItemStack> {
    let target = pos.relative(facing);
    let name = stack.item_name();
    Ok(match name {
        "minecraft:bone_meal" => bone_meal(level, target, stack),
        "minecraft:flint_and_steel" => flint_and_steel(level, rng, target, facing, stack),
        "minecraft:honeycomb" => honeycomb(level, rng, pos, facing, target, stack),
        "minecraft:glowstone" => glowstone(level, target, rng, pos, facing, stack),
        "minecraft:glass_bottle" => glass_bottle(level, rng, pos, facing, target, stack),
        "minecraft:potion" => potion(level, rng, pos, facing, target, stack),
        "minecraft:tnt" => tnt(level, pos, target, stack),
        "minecraft:shears" => shears(level, rng, pos, facing, target, stack),
        n if n.ends_with("shulker_box") => shulker_box(level, pos, facing, stack),
        n if is_boat(n) => boat(level, pos, facing, stack),
        "minecraft:armor_stand" => armor_stand(level, target, facing, stack),
        "minecraft:water_bucket" | "minecraft:lava_bucket" | "minecraft:powder_snow_bucket" | "minecraft:salmon_bucket" | "minecraft:cod_bucket" | "minecraft:pufferfish_bucket"
        | "minecraft:tropical_fish_bucket" | "minecraft:axolotl_bucket" | "minecraft:sulfur_cube_bucket" | "minecraft:tadpole_bucket" => full_bucket(level, rng, pos, facing, target, stack),
        "minecraft:carved_pumpkin" => carved_pumpkin(level, pos, facing, target, stack),
        "minecraft:wither_skeleton_skull" => wither_skull(level, pos, facing, target, stack),
        "minecraft:brush" => brush(level, rng, target, stack),
        "minecraft:chest" => {
            let mut stack = stack;
            if super::equip::dispense_chest(level, pos, facing, &mut stack) {
                return Ok(ok(stack));
            }
            return Err(stack);
        }
        n if n.ends_with("_spawn_egg") => spawn_egg(level, rng, pos, facing, target, stack),
        // `DispenserBlock.getDefaultDispenseMethod`: anything that can be worn, then what a sulfur cube swallows.
        _ => {
            let mut stack = stack;
            if stack.get(keys::EQUIPPABLE).is_some() {
                if super::equip::dispense_equipment(level, pos, facing, &mut stack) {
                    return Ok(Done { stack, success: None });
                }
            } else if kiln_inventory::tags::contains("minecraft:item", "minecraft:sulfur_cube_swallowable", stack.item()) && super::equip::dispense_swallow(level, pos, facing, &mut stack) {
                return Ok(Done { stack, success: None });
            }
            return Err(stack);
        }
    })
}

/// `DispenseItemBehavior$3`: a full bucket pours out in front, the empty bucket goes back into the dispenser
/// (or out of it); where it cannot, the bucket is dropped like any item.
fn full_bucket(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, target: BlockPos, stack: ItemStack) -> Done {
    if crate::buckets::dispense_empty(level, &stack, target) {
        let remainder = ItemStack::of("minecraft:bucket", 1).unwrap_or_default();
        return Done { stack: consume_with_remainder(level, rng, pos, facing, stack, remainder), success: None };
    }
    // (`defaultDispenseItemBehavior.dispense`: its own sound and animation, and then this behaviour's.)
    Done { stack: super::dispense::default_dispense(level, rng, pos, facing, stack), success: None }
}

/// `SpawnEggItemBehavior`: the mob of the egg appears in front (1 up when the front is blocked), facing a random way.
fn spawn_egg(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, target: BlockPos, mut stack: ItemStack) -> Done {
    use kiln_entity::mob::MobKind;
    let Some(kind) = stack.item_name().strip_suffix("_spawn_egg").and_then(MobKind::by_name) else { return Done { stack, success: None } };
    let width = kiln_data::entities::by_name(kind.type_name()).map_or(0.6, |t| t.width);
    // (`EntityType.create`: aligned to the floor unless the dispenser faces up.)
    let off = if facing != Direction::Up { crate::mobs::align_offset(level, target, width) } else { 0.0 };
    let yaw = kiln_entity::mob::mth::wrap_degrees(rng.next_float() * 360.0);
    let env = level.env;
    let finalize = crate::mobs::Finalize::command(
        crate::mobs::difficulty_instance(env.mobs.difficulty, env.game_time, 0, 1.0),
        crate::mobs::loot_seed(env.seed, env.game_time, 0, (target.x as u64) << 32 ^ target.z as u64 ^ (target.y as u64) << 16),
        false,
        env.mobs.difficulty == 0 || !env.mobs.spawn_monsters,
    );
    level.out.spawns.push(crate::mobs::spawn(kind, [target.x as f64 + 0.5, target.y as f64 + off, target.z as f64 + 0.5], Some(yaw), Some(finalize)));
    stack.shrink_count(1);
    level.effect(Effect::GameEvent { pos, event: "minecraft:entity_place" });
    Done { stack, success: None }
}

/// `DispenseItemBehavior$8`: a carved pumpkin that finishes a golem pattern is put in front (and the golem
/// made); else it is worn by whoever stands there, else nothing happens.
fn carved_pumpkin(level: &mut RegionLevel, pos: BlockPos, facing: Direction, target: BlockPos, mut stack: ItemStack) -> Done {
    if kiln_data::blocks_types::is_air(level.block(target)) && crate::golems::can_spawn_golem(level, target) {
        kiln_blocks::set_block_and_update(level, target, kiln_data::blocks::default_state::CARVED_PUMPKIN);
        level.effect(Effect::GameEvent { pos: target, event: "minecraft:block_place" });
        let mut spawns = Vec::new();
        crate::golems::try_spawn_golem(None, level, target, &mut spawns);
        level.out.spawns.extend(spawns);
        stack.shrink_count(1);
        return ok(stack);
    }
    let worn = super::equip::dispense_equipment(level, pos, facing, &mut stack);
    Done { stack, success: Some(worn) }
}

/// `DispenseItemBehavior$7`: a wither skeleton skull that completes the wither pattern is put in front (and the
/// wither made), else it is worn by whoever stands there.
fn wither_skull(level: &mut RegionLevel, pos: BlockPos, facing: Direction, target: BlockPos, mut stack: ItemStack) -> Done {
    if kiln_data::blocks_types::is_air(level.block(target)) && crate::wither::can_spawn_mob(level, target) {
        // (`RotationSegment.convertToSegment`: 4 steps of 16 to a side, 0 vertically.)
        let segment = if facing.axis() == kiln_blocks::Axis::Y {
            0
        } else {
            match facing.opposite() {
                Direction::South => 0,
                Direction::West => 4,
                Direction::North => 8,
                _ => 12,
            }
        };
        let skull = state::set_int(kiln_data::blocks::default_state::WITHER_SKELETON_SKULL, "rotation", segment);
        kiln_blocks::set_block_and_update(level, target, skull);
        level.effect(Effect::GameEvent { pos: target, event: "minecraft:block_place" });
        let mut spawns = Vec::new();
        crate::wither::check_spawn(level, target, &mut spawns);
        level.out.spawns.extend(spawns);
        stack.shrink_count(1);
        return ok(stack);
    }
    let worn = super::equip::dispense_equipment(level, pos, facing, &mut stack);
    Done { stack, success: Some(worn) }
}

/// `DispenseItemBehavior$11`: a brush scrapes a scute off an armadillo in front, else it fails.
fn brush(level: &mut RegionLevel, _rng: &mut LegacyRandom, target: BlockPos, stack: ItemStack) -> Done {
    let _ = (level, target);
    failed(stack)
}

fn is_boat(name: &str) -> bool {
    crate::boats::is_boat_item(name)
}

/// `DispenseItemBehavior$5`: bone meal grows the plant in front (the stack is used up when the block takes it).
fn bone_meal(level: &mut RegionLevel, target: BlockPos, mut stack: ItemStack) -> Done {
    let mut spawns = Vec::new();
    let grew = crate::tools::bone_meal_block(level, target, &mut spawns);
    level.out.spawns.extend(spawns);
    if !grew {
        return failed(stack);
    }
    stack.shrink_count(1);
    level.effect(Effect::LevelEvent { id: 1505, pos: target, data: 15 });
    ok(stack)
}

/// `FlintAndSteelDispenseItemBehavior`.
fn flint_and_steel(level: &mut RegionLevel, rng: &mut LegacyRandom, target: BlockPos, facing: Direction, mut stack: ItemStack) -> Done {
    let s = level.block(target);
    let mut success = true;
    if kiln_blocks::behaviour::portal::fire_can_be_placed_at(level, target, facing) {
        let fire = kiln_blocks::behaviour::portal::fire_state(level, target);
        kiln_blocks::set_block_and_update(level, target, fire);
        level.effect(Effect::GameEvent { pos: target, event: "minecraft:block_place" });
    } else if (logic::is_instance(s, C::CampfireBlock) || logic::is_instance(s, C::AbstractCandleBlock))
        && state::has(s, "lit")
        && !state::get_bool(s, "lit")
        && !state::get_bool(s, "waterlogged")
    {
        kiln_blocks::set_block_and_update(level, target, state::set_bool(s, "lit", true));
        level.effect(Effect::GameEvent { pos: target, event: "minecraft:block_change" });
    } else if logic::is_instance(s, C::TntBlock) {
        if kiln_blocks::redstone::devices::prime(level, target) {
            kiln_blocks::remove_block(level, target, false);
        } else {
            success = false;
        }
    } else {
        success = false;
    }
    if success {
        hurt(level, &mut stack, rng);
    }
    Done { stack, success: Some(success) }
}

/// `DispenseItemBehavior$12`: honeycomb waxes the copper in front.
fn honeycomb(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, target: BlockPos, mut stack: ItemStack) -> Done {
    let s = level.block(target);
    let Some(waxed) = crate::tools::waxed(s) else {
        // (`OptionalDispenseItemBehavior`'s own execute: dropped as an item, the flag still set.)
        let one = stack.split_count(1);
        spawn_item(level, rng, one, 6, facing, dispense_position(pos, facing));
        return Done { stack, success: None };
    };
    kiln_blocks::set_block_and_update(level, target, waxed);
    level.effect(Effect::LevelEvent { id: 3003, pos: target, data: 0 });
    level.effect(Effect::Sound { pos: target, sound: "minecraft:item.honeycomb.wax_on", volume: 1.0, pitch: 1.0 });
    stack.shrink_count(1);
    ok(stack)
}

/// `DispenseItemBehavior$10`: glowstone charges a respawn anchor, else it is dropped.
fn glowstone(level: &mut RegionLevel, target: BlockPos, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> Done {
    let s = level.block(target);
    if kiln_blocks::BlockId::of(s).name() == "minecraft:respawn_anchor" {
        if state::get_int(s, "charge") != 4 {
            let charged = state::set_int(s, "charge", state::get_int(s, "charge") + 1);
            kiln_blocks::set_block_and_update(level, target, charged);
            level.effect(Effect::BlockGameEvent { pos: target, event: "minecraft:block_change", state: charged });
            level.effect(Effect::Sound { pos: target, sound: "minecraft:block.respawn_anchor.charge", volume: 1.0, pitch: 1.0 });
            stack.shrink_count(1);
            return ok(stack);
        }
        return failed(stack);
    }
    let one = stack.split_count(1);
    spawn_item(level, rng, one, 6, facing, dispense_position(pos, facing));
    ok(stack)
}

/// `DispenseItemBehavior$9`: a glass bottle takes honey from a full hive or water from a fluid in front.
fn glass_bottle(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, target: BlockPos, stack: ItemStack) -> Done {
    let s = level.block(target);
    let take = |level: &mut RegionLevel, rng: &mut LegacyRandom, filled: ItemStack| {
        level.effect(Effect::GameEvent { pos, event: "minecraft:fluid_pickup" });
        consume_with_remainder(level, rng, pos, facing, stack.clone(), filled)
    };
    if logic::is_instance(s, C::BeehiveBlock) && state::get_int(s, "honey_level") >= 5 {
        crate::beehive::release_after_harvest(level, target, s);
        let honey = ItemStack::of("minecraft:honey_bottle", 1).unwrap_or_default();
        return ok(take(level, rng, honey));
    }
    if kiln_data::block_logic::fluid(s).kind == kiln_data::block_logic::FluidKind::Water {
        let mut water = ItemStack::of("minecraft:potion", 1).unwrap_or_default();
        water.insert(kiln_item::keys::POTION_CONTENTS, kiln_item::component::PotionContents { potion: kiln_item::registry::POTION.id("minecraft:water"), ..Default::default() });
        return ok(take(level, rng, water));
    }
    // Nothing to take: `OptionalDispenseItemBehavior`'s own execute (the default drop) with the flag cleared.
    let mut stack = stack;
    let one = stack.split_count(1);
    spawn_item(level, rng, one, 6, facing, dispense_position(pos, facing));
    failed(stack)
}

/// `DispenseItemBehavior$13`: a water bottle turns dirt in front to mud; everything else is dropped.
fn potion(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, target: BlockPos, mut stack: ItemStack) -> Done {
    let water = stack.get(kiln_item::keys::POTION_CONTENTS).is_some_and(|c| {
        c.potion.is_some() && c.potion == kiln_item::registry::POTION.id("minecraft:water") && c.custom_effects.is_empty()
    }) && stack.get(kiln_item::keys::POTION_CONTENTS).is_some();
    let s = level.block(target);
    if water && kiln_blocks::tags::is(s, "minecraft:convertible_to_mud") {
        level.effect(Effect::Sound { pos: target, sound: "minecraft:item.bottle.empty", volume: 1.0, pitch: 1.0 });
        level.effect(Effect::GameEvent { pos: target, event: "minecraft:fluid_place" });
        kiln_blocks::set_block_and_update(level, target, kiln_data::blocks::default_state::MUD);
        let bottle = ItemStack::of("minecraft:glass_bottle", 1).unwrap_or_default();
        return Done { stack: consume_with_remainder(level, rng, pos, facing, stack, bottle), success: None };
    }
    let one = stack.split_count(1);
    spawn_item(level, rng, one, 6, facing, dispense_position(pos, facing));
    Done { stack, success: None }
}

/// `DispenseItemBehavior$6`: TNT is primed in front of the dispenser (no block placed).
fn tnt(level: &mut RegionLevel, pos: BlockPos, target: BlockPos, mut stack: ItemStack) -> Done {
    if !level.env.rules.tnt_explodes {
        return failed(stack);
    }
    level.effect(Effect::PrimedTnt { pos: target });
    let at = [target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5];
    if let Some(id) = kiln_data::builtin_id("minecraft:sound_event", "minecraft:entity.tnt.primed") {
        let seed = level.random().next_long();
        let pkt = kiln_proto::packets::world_fx::sound(&kiln_proto::packets::world_fx::Sound::Registered(id), kiln_proto::packets::world_fx::SoundSource::Blocks, at, 1.0, 1.0, seed);
        level.out.packets.push((at, 16.0, pkt));
    }
    level.effect(Effect::GameEvent { pos, event: "minecraft:entity_place" });
    stack.shrink_count(1);
    ok(stack)
}

/// `ShearsDispenseItemBehavior`: a full hive in front gives its honeycombs (entities are not sheared yet).
fn shears(level: &mut RegionLevel, rng: &mut LegacyRandom, pos: BlockPos, facing: Direction, target: BlockPos, mut stack: ItemStack) -> Done {
    let s = level.block(target);
    if logic::is_instance(s, C::BeehiveBlock) && state::get_int(s, "honey_level") >= 5 {
        level.effect(Effect::Sound { pos: target, sound: "minecraft:block.beehive.shear", volume: 1.0, pitch: 1.0 });
        crate::beehive::drop_honeycombs(level, target);
        crate::beehive::release_after_harvest(level, target, s);
        level.effect(Effect::GameEvent { pos: target, event: "minecraft:shear" });
        hurt(level, &mut stack, rng);
        return ok(stack);
    }
    // `tryShearEntity`.
    if super::equip::dispense_shear(level, pos, facing, &stack) {
        hurt(level, &mut stack, rng);
        return ok(stack);
    }
    failed(stack)
}

/// `ShulkerBoxDispenseBehavior`: the box is put in front (on top of what is under it, else facing the way the
/// dispenser does).
fn shulker_box(level: &mut RegionLevel, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> Done {
    let target = pos.relative(facing);
    let clicked = if kiln_data::blocks_types::is_air(level.block(target.below())) { facing } else { Direction::Up };
    let old = level.block(target);
    if !(kiln_data::block_props::replaceable(old) || kiln_data::blocks_types::is_air(old)) {
        return failed(stack);
    }
    let Some(block) = kiln_blocks::BlockId::by_name(stack.item_name()) else { return failed(stack) };
    let placed = state::set_dir(block.default_state(), "facing", clicked);
    if !kiln_blocks::set_block_and_update(level, target, placed) {
        return failed(stack);
    }
    level.effect(Effect::BlockGameEvent { pos: target, event: "minecraft:block_place", state: placed });
    crate::container::open::apply_item_components(level, target, &stack);
    stack.shrink_count(1);
    ok(stack)
}

/// `BoatDispenseItemBehavior`: the boat is put on the water in front (or on air above water).
fn boat(level: &mut RegionLevel, pos: BlockPos, facing: Direction, mut stack: ItemStack) -> Done {
    let target = pos.relative(facing);
    let wet = |s: u16| kiln_data::block_logic::fluid(s).kind == kiln_data::block_logic::FluidKind::Water;
    let y_offset = if wet(level.block(target)) {
        1.0
    } else if kiln_data::blocks_types::is_air(level.block(target)) && wet(level.block(target.below())) {
        0.0
    } else {
        // `defaultDispenseItemBehavior.dispense`: dropped as an item.
        let mut rng = super::pos_random(level, pos, 4);
        let one = stack.split_count(1);
        spawn_item(level, &mut rng, one, 6, facing, dispense_position(pos, facing));
        return Done { stack, success: None };
    };
    let Some(kind) = kiln_data::entities::by_name(stack.item_name()) else { return Done { stack, success: None } };
    let d = 0.5625 + kind.width as f64 / 2.0;
    let st = facing.step();
    let at = [pos.x as f64 + 0.5 + st[0] as f64 * d, pos.y as f64 + 0.5 + (st[1] as f32 * 1.125f32) as f64 + y_offset, pos.z as f64 + 0.5 + st[2] as f64 * d];
    let seed = level.random().next_long();
    let mut e = kiln_entity::ext_entity::boat::new(kind.name, kiln_entity::math::Vec3::new(at[0], at[1], at[2]), yrot(facing), seed);
    if let Some(name) = stack.get(kiln_item::keys::CUSTOM_NAME) {
        e.extra.push(("CustomName".into(), name.nbt().clone()));
    }
    level.out.spawns.push(Spawn { kind, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(e)) });
    stack.shrink_count(1);
    // (`playSound` is always 1000 here.)
    Done { stack, success: None }
}

/// `Direction.toYRot`: the horizontal directions are 0 (south), 90, 180 and 270 degrees; up and down count as east.
fn yrot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        _ => 270.0,
    }
}

/// `DispenseItemBehavior$1`: an armor stand is put up in front, turned as the dispenser faces.
fn armor_stand(level: &mut RegionLevel, target: BlockPos, facing: Direction, mut stack: ItemStack) -> Done {
    use kiln_entity::ext_entity::armor_stand;
    let at = [target.x as f64 + 0.5, target.y as f64, target.z as f64 + 0.5];
    // `EntityType.create` turns it a random way before the dispenser sets the yaw.
    let _ = level.random().next_float();
    let seed = level.random().next_long();
    let mut stand = armor_stand::new(0, kiln_entity::math::Vec3::new(at[0], at[1], at[2]), yrot(facing), seed);
    if let Some(name) = stack.get(kiln_item::keys::CUSTOM_NAME) {
        stand.extra.push(("CustomName".into(), name.nbt().clone()));
    }
    if let Some(data) = stack.get(kiln_item::keys::ENTITY_DATA) {
        armor_stand::apply_saved(&mut stand, &data.tag);
    }
    if let Some(kind) = kiln_data::entities::by_name(armor_stand::ARMOR_STAND) {
        level.out.spawns.push(Spawn { kind, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(stand)) });
        stack.shrink_count(1);
    }
    Done { stack, success: None }
}
