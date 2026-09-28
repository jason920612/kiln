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

/// After a block was placed at `pos` by player `p`: builds an iron golem if the pumpkin
/// completes the pattern.
pub(crate) fn try_spawn_golem(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) {
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
        build(p, level, MobKind::SnowGolem, &[pos, body, feet], feet, spawns);
        return;
    }
    if !iron(level, body) || !iron(level, feet) {
        return;
    }
    // Arms along x or z; the corners beside the feet must be empty (`BlockPatternBuilder`
    // `~` cells are air).
    let axes = [[1, 0], [0, 1]];
    let Some(axis) = axes.into_iter().find(|a| {
        let side = |s: i32, at: BlockPos| BlockPos::new(at.x + a[0] * s, at.y, at.z + a[1] * s);
        iron(level, side(1, body)) && iron(level, side(-1, body)) && air(level, side(1, feet)) && air(level, side(-1, feet))
    }) else {
        return;
    };
    let arms = [
        BlockPos::new(body.x + axis[0], body.y, body.z + axis[1]),
        BlockPos::new(body.x - axis[0], body.y, body.z - axis[1]),
    ];
    build(p, level, MobKind::IronGolem, &[pos, body, feet, arms[0], arms[1]], feet, spawns);
}

/// `spawnGolemInWorld`: the pattern's blocks go (with their break particles), the golem stands
/// at the feet, players near it get `summoned_entity` and the blocks' neighbours update.
fn build(p: &mut Player, level: &mut RegionLevel, kind: MobKind, blocks: &[BlockPos], feet: BlockPos, spawns: &mut Vec<Spawn>) {
    let (pos, body) = (blocks[0], blocks[1]);
    // `clearPatternBlocks`.
    for &at in blocks {
        let s = level.block(at);
        kiln_blocks::set_block(level, at, d::AIR, flags::CLIENTS);
        level.effect(Effect::LevelEvent { id: 2001, pos: at, data: s as i32 });
    }
    // At the feet, facing the default way.
    let at = [feet.x as f64 + 0.5, feet.y as f64 + 0.05, feet.z as f64 + 0.5];
    let mut golem = kiln_entity::mob::new(kind, 0, 0, p.entity_id as i64 ^ level.env.game_time);
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
    let near = (0..3).all(|k| (p.pos[k] - at[k]).abs() <= 5.0 + if k == 1 { height } else { 0.7 });
    if near {
        let dim = crate::DIMENSIONS[level.env.dim].0;
        let subject = crate::advancements::triggers::seen_subject(&seen, dim);
        p.fire_conds("minecraft:summoned_entity", None, |c, ok, _| c.cap("entity").is_none_or(|cap| ok(cap, &subject)));
    }
    // The golem's blocks updated their neighbours.
    for at in [feet, body, pos] {
        kiln_blocks::update::update_neighbors_at(level, at, kiln_blocks::BlockId::of(d::AIR));
    }
}

/// The `minecraft:gameplay/snow_golem_melts` attribute: the Nether, and the biomes that set it.
pub(crate) fn snow_golem_melts(level: &RegionLevel, pos: kiln_entity::math::Vec3) -> bool {
    if crate::DIMENSIONS[level.env.dim].0 == "minecraft:the_nether" {
        return true;
    }
    let at = kiln_blocks::BlockPos::new(pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
    let biome = crate::spawner::biome_at(level, at);
    const MELTING: [&str; 6] = ["minecraft:desert", "minecraft:savanna", "minecraft:savanna_plateau", "minecraft:windswept_savanna", "minecraft:badlands", "minecraft:eroded_badlands"];
    const WOODED: &str = "minecraft:wooded_badlands";
    MELTING.iter().chain([&WOODED]).any(|n| kiln_data::builtin_id("minecraft:worldgen/biome", n) == Some(biome as i32))
}
