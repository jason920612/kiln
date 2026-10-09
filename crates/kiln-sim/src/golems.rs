//! Building golems (`CarvedPumpkinBlock.trySpawnGolem`): a carved pumpkin or jack o'lantern
//! placed on two snow blocks becomes a snow golem, on a T of iron blocks (arms either way) a
//! player-created iron golem; the pattern's blocks break (level event 2001) and players
//! within five blocks of the golem get `summoned_entity`.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Effect, Level, flags, state};
use kiln_data::blocks::default_state as d;
use kiln_entity::mob::MobKind;

/// `CarvedPumpkinBlock.canSpawnGolem`: whether a pumpkin at `pos` would finish a golem pattern
/// (the patterns without their head, so the block at `pos` itself is not looked at).
pub(crate) fn can_spawn_golem(level: &RegionLevel, pos: BlockPos) -> bool {
    let snow = |at: BlockPos| state::same_block(level.block(at), d::SNOW_BLOCK);
    let iron = |at: BlockPos| state::same_block(level.block(at), d::IRON_BLOCK);
    let air = |at: BlockPos| kiln_data::blocks_types::is_air(level.block(at));
    let (body, feet) = (pos.below(), pos.below().below());
    if snow(body) && snow(feet) {
        return true;
    }
    if iron(body) && iron(feet) {
        let ok = [[1, 0], [0, 1]].into_iter().any(|a| {
            let side = |s: i32, at: BlockPos| BlockPos::new(at.x + a[0] * s, at.y, at.z + a[1] * s);
            iron(side(1, body)) && iron(side(-1, body)) && air(side(1, feet)) && air(side(-1, feet))
        });
        if ok {
            return true;
        }
    }
    kiln_blocks::tags::is(level.block(body), "minecraft:copper")
}

/// After a block was placed at `pos` by player `p` (none: a dispenser): builds an iron golem if
/// the pumpkin completes the pattern.
pub(crate) fn try_spawn_golem(mut p: Option<&mut Player>, level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) {
    let head = level.block(pos);
    if !(state::same_block(head, d::CARVED_PUMPKIN) || state::same_block(head, d::JACK_O_LANTERN)) {
        return;
    }
    let snow = |l: &RegionLevel, at: BlockPos| state::same_block(l.block(at), d::SNOW_BLOCK);
    let iron = |l: &RegionLevel, at: BlockPos| state::same_block(l.block(at), d::IRON_BLOCK);
    let air = |l: &RegionLevel, at: BlockPos| kiln_data::blocks_types::is_air(l.block(at));
    let body = pos.below();
    let feet = body.below();
    // The snow golem pattern is tried first.
    if snow(level, body) && snow(level, feet) {
        build(p.as_deref_mut(), level, MobKind::SnowGolem, &[pos, body, feet], feet, spawns);
        return;
    }
    if iron(level, body) && iron(level, feet) {
        // Arms along x or z; the corners beside the feet must be empty (`BlockPatternBuilder`
        // `~` cells are air).
        let axes = [[1, 0], [0, 1]];
        let found = axes.into_iter().find(|a| {
            let side = |s: i32, at: BlockPos| BlockPos::new(at.x + a[0] * s, at.y, at.z + a[1] * s);
            iron(level, side(1, body)) && iron(level, side(-1, body)) && air(level, side(1, feet)) && air(level, side(-1, feet))
        });
        if let Some(axis) = found {
            let arms = [
                BlockPos::new(body.x + axis[0], body.y, body.z + axis[1]),
                BlockPos::new(body.x - axis[0], body.y, body.z - axis[1]),
            ];
            build(p.as_deref_mut(), level, MobKind::IronGolem, &[pos, body, feet, arms[0], arms[1]], feet, spawns);
            return;
        }
    }
    // The copper golem pattern is tried last: a copper block under the pumpkin.
    if kiln_blocks::tags::is(level.block(body), "minecraft:copper") {
        build_copper(p, level, pos, body, spawns);
    }
}

/// The copper chest of a copper block (`CopperChestBlock.COPPER_TO_COPPER_CHEST_MAPPING`).
fn copper_chest_of(block: u16) -> u16 {
    let name = kiln_data::blocks_types::block_of(block).name;
    let chest = match name {
        "minecraft:copper_block" => "minecraft:copper_chest".to_owned(),
        "minecraft:waxed_copper_block" => "minecraft:waxed_copper_chest".to_owned(),
        n => n.replace("_copper", "_copper_chest"),
    };
    kiln_data::blocks_types::block_by_name(&chest).or_else(|| kiln_data::blocks_types::block_by_name("minecraft:copper_chest")).map_or(d::AIR, |b| b.default)
}

/// `CopperGolem` weather stage of a copper block (waxed ones count as their unwaxed stage).
fn weather_of(block: u16) -> u8 {
    let name = kiln_data::blocks_types::block_of(block).name;
    if name.contains("oxidized") {
        3
    } else if name.contains("weathered") {
        2
    } else if name.contains("exposed") {
        1
    } else {
        0
    }
}

/// `CarvedPumpkinBlock.trySpawnGolem` for the copper golem: the pumpkin and the copper block go
/// (level event 2001, as for the others), the golem stands where the pumpkin was, the copper
/// block comes back as a copper chest facing the way the pumpkin did, and the golem has the
/// block's weathering stage.
fn build_copper(p: Option<&mut Player>, level: &mut RegionLevel, pos: BlockPos, body: BlockPos, spawns: &mut Vec<Spawn>) {
    let pumpkin = level.block(pos);
    let copper = level.block(body);
    // `clearPatternBlocks`.
    for at in [pos, body] {
        let s = level.block(at);
        kiln_blocks::set_block(level, at, d::AIR, flags::CLIENTS);
        level.effect(Effect::LevelEvent { id: 2001, pos: at, data: s as i32 });
    }
    let at = [pos.x as f64 + 0.5, pos.y as f64 + 0.05, pos.z as f64 + 0.5];
    let mut golem = kiln_entity::mob::new(MobKind::CopperGolem, 0, 0, p.as_ref().map_or(0, |p| p.entity_id) as i64 ^ level.env.game_time);
    golem.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
    golem.y_rot = 0.0;
    golem.set_old_pos_and_rot();
    // `spawn(weatherState)`.
    if let Some(m) = kiln_entity::mob::data_mut(&mut golem) {
        let s = kiln_entity::mob::kinds::copper_golem::st_mut(m);
        s.weather = weather_of(copper);
        s.spawn_sound = true;
    }
    let seen = kiln_entity::level::Seen::of(&golem);
    let entity_type = kiln_data::entities::by_name("minecraft:copper_golem").expect("copper golem type");
    spawns.push(Spawn { kind: entity_type, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(golem)) });
    if let Some(p) = p
        && (0..3).all(|k| (p.pos[k] - at[k]).abs() <= 5.0 + if k == 1 { 0.98 } else { 0.245 })
    {
        let dim = crate::DIMENSIONS[level.env.dim].0;
        let subject = crate::advancements::triggers::seen_subject(&seen, dim);
        p.fire_conds("minecraft:summoned_entity", None, |c, ok, _| c.cap("entity").is_none_or(|cap| ok(cap, &subject)));
    }
    // `updatePatternBlocks`.
    for at in [pos, body] {
        kiln_blocks::update::update_neighbors_at(level, at, kiln_blocks::BlockId::of(d::AIR));
    }
    // `replaceCopperBlockWithChest`: facing as the pumpkin, joined to a chest beside it.
    let facing = state::get_dir(pumpkin, "facing").unwrap_or(kiln_blocks::Direction::North);
    let chest = copper_chest_of(copper);
    let chest = kiln_blocks::behaviour::container::chest_placement(level, chest, body, facing.opposite(), kiln_blocks::Direction::Up, false);
    kiln_blocks::set_block(level, body, chest, flags::CLIENTS);
}

/// `spawnGolemInWorld`: the pattern's blocks go (with their break particles), the golem stands
/// at the feet, players near it get `summoned_entity` and the blocks' neighbours update.
fn build(p: Option<&mut Player>, level: &mut RegionLevel, kind: MobKind, blocks: &[BlockPos], feet: BlockPos, spawns: &mut Vec<Spawn>) {
    let (pos, body) = (blocks[0], blocks[1]);
    // `clearPatternBlocks`.
    for &at in blocks {
        let s = level.block(at);
        kiln_blocks::set_block(level, at, d::AIR, flags::CLIENTS);
        level.effect(Effect::LevelEvent { id: 2001, pos: at, data: s as i32 });
    }
    // At the feet, facing the default way.
    let at = [feet.x as f64 + 0.5, feet.y as f64 + 0.05, feet.z as f64 + 0.5];
    let mut golem = kiln_entity::mob::new(kind, 0, 0, p.as_ref().map_or(0, |p| p.entity_id) as i64 ^ level.env.game_time);
    golem.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
    golem.y_rot = 0.0;
    golem.set_old_pos_and_rot();
    if kind == MobKind::IronGolem
        && let Some(m) = kiln_entity::mob::data_mut(&mut golem)
        && let Some(s) = kiln_entity::mob::ext::state_mut::<kiln_entity::mob::kinds::iron_golem::State>(m)
    {
        s.player_created = true;
    }
    let seen = kiln_entity::level::Seen::of(&golem);
    let entity_type = kiln_data::entities::by_name(kind.type_name()).expect("golem type");
    spawns.push(Spawn { kind: entity_type, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(golem)) });
    // `SummonedEntityTrigger` for players in the golem's box inflated by 5 (Kiln sees the
    // acting player).
    let height = if kind == MobKind::SnowGolem { 1.9 } else { 2.7 };
    if let Some(p) = p
        && (0..3).all(|k| (p.pos[k] - at[k]).abs() <= 5.0 + if k == 1 { height } else { 0.7 })
    {
        let dim = crate::DIMENSIONS[level.env.dim].0;
        let subject = crate::advancements::triggers::seen_subject(&seen, dim);
        p.fire_conds("minecraft:summoned_entity", None, |c, ok, _| c.cap("entity").is_none_or(|cap| ok(cap, &subject)));
    }
    // The golem's blocks updated their neighbours.
    for at in [feet, body, pos] {
        kiln_blocks::update::update_neighbors_at(level, at, kiln_blocks::BlockId::of(d::AIR));
    }
}
