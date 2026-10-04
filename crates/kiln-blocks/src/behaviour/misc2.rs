//! Self-driven behaviour of a few single blocks: turtle eggs (`TurtleEggBlock`), redstone ore
//! (`RedStoneOreBlock`), budding amethyst (`BuddingAmethystBlock`) and dried ghasts
//! (`DriedGhastBlock`).

use crate::level::{Effect, Level, flags, schedule_block_tick};
use crate::pos::{BlockPos, Direction};
use crate::state::{self, BlockId};
use crate::tags;
use crate::ticks::TickPriority;
use crate::update::{remove_block, set_block, set_block_and_update};
use kiln_data::block_logic::{self as logic, FluidKind};
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;

// ---------------------------------------------------------------------- turtle egg

/// `TurtleEggBlock.isSand` of the block below.
pub fn turtle_egg_on_sand<L: Level + ?Sized>(level: &L, pos: BlockPos) -> bool {
    tags::is(level.block(pos.below()), "minecraft:sand")
}

/// `TurtleEggBlock.onPlace`: eggs on sand puff the sand particles.
pub fn turtle_egg_on_place<L: Level>(level: &mut L, pos: BlockPos) {
    if turtle_egg_on_sand(level, pos) {
        level.effect(Effect::LevelEvent { id: 2012, pos, data: 15 });
    }
}

/// `0.9f + random.nextFloat() * 0.2f`, the pitch of the egg sounds.
fn egg_pitch<L: Level>(level: &mut L) -> f32 {
    0.9 + level.random().next_float() * 0.2
}

/// `TurtleEggBlock.randomTick`: with the hatch chance (and sand below) the egg cracks one stage
/// (`hatch` 0 to 2) and then hatches into its baby turtles.
pub fn turtle_egg_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    // `shouldUpdateHatchLevel`: a draw whenever the chance is positive.
    let chance = level.turtle_egg_hatch_chance(pos);
    if !(chance > 0.0 && level.random().next_float() < chance) || !turtle_egg_on_sand(level, pos) {
        return;
    }
    let hatch = state::get_int(s, "hatch");
    let pitch = egg_pitch(level);
    if hatch < 2 {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.turtle_egg.crack", volume: 0.7, pitch });
        set_block(level, pos, state::set_int(s, "hatch", hatch + 1), flags::CLIENTS);
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    } else {
        level.effect(Effect::Sound { pos, sound: "minecraft:block.turtle_egg.hatch", volume: 0.7, pitch });
        remove_block(level, pos, false);
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_destroy" });
        let eggs = state::get_int(s, "eggs");
        for _ in 0..eggs {
            level.effect(Effect::LevelEvent { id: 2001, pos, data: s as i32 });
        }
        level.effect(Effect::HatchTurtles { pos, eggs });
    }
}

// ---------------------------------------------------------------------- redstone ore

/// `RedStoneOreBlock.randomTick`: lit ore goes out.
pub fn redstone_ore_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    if state::get_bool(s, "lit") {
        set_block_and_update(level, pos, state::set_bool(s, "lit", false));
    }
}

// ---------------------------------------------------------------------- budding amethyst

/// `BuddingAmethystBlock.canClusterGrowAtState`: air or full water (the block, any level 0 or 8+).
fn can_cluster_grow_at(s: u16) -> bool {
    is_air(s) || (state::is(s, kiln_data::blocks::default_state::WATER) && logic::fluid(s).amount == 8)
}

/// `BuddingAmethystBlock.randomTick`: one in five, a random side grows its bud one stage.
pub fn budding_amethyst_random_tick<L: Level>(level: &mut L, pos: BlockPos) {
    if level.random().next_int_bounded(5) != 0 {
        return;
    }
    let dir = Direction::ALL[level.random().next_int_bounded(6) as usize];
    let at = pos.relative(dir);
    let neighbor = level.block(at);
    let name = |n: &str| BlockId::by_name(n).expect("amethyst block");
    let facing_dir = |s: u16| state::get_dir(s, "facing") == Some(dir);
    let is = |s: u16, n: &str| BlockId::of(s) == name(n);
    let next = if can_cluster_grow_at(neighbor) {
        Some("minecraft:small_amethyst_bud")
    } else if is(neighbor, "minecraft:small_amethyst_bud") && facing_dir(neighbor) {
        Some("minecraft:medium_amethyst_bud")
    } else if is(neighbor, "minecraft:medium_amethyst_bud") && facing_dir(neighbor) {
        Some("minecraft:large_amethyst_bud")
    } else if is(neighbor, "minecraft:large_amethyst_bud") && facing_dir(neighbor) {
        Some("minecraft:amethyst_cluster")
    } else {
        None
    };
    if let Some(next) = next {
        let f = logic::fluid(neighbor);
        let grown = state::set_bool(state::set_dir(name(next).default_state(), "facing", dir), "waterlogged", f.kind == FluidKind::Water && f.source);
        set_block_and_update(level, at, grown);
    }
}

// ---------------------------------------------------------------------- dried ghast

/// `DriedGhastBlock.randomTick`: a wet (or waterlogged) ghast schedules its next hydration step
/// in 5000 ticks unless one is pending.
pub fn dried_ghast_random_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let id = BlockId::of(s);
    if (state::get_bool(s, "waterlogged") || state::get_int(s, "hydration") > 0) && !level.block_ticks().has_scheduled_tick(pos, id) {
        schedule_block_tick(level, pos, id, 5000, TickPriority::Normal);
    }
}

/// `Direction.getYRot`.
fn y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        _ => 0.0,
    }
}

/// `DriedGhastBlock.tick`: waterlogged it soaks up a level (and at 3 hatches a ghastling); dry
/// it loses one.
pub fn dried_ghast_tick<L: Level>(level: &mut L, s: u16, pos: BlockPos) {
    let hydration = state::get_int(s, "hydration");
    if state::get_bool(s, "waterlogged") {
        if hydration != 3 {
            level.effect(Effect::Sound { pos, sound: "minecraft:block.dried_ghast.transition", volume: 1.0, pitch: 1.0 });
            set_block(level, pos, state::set_int(s, "hydration", hydration + 1), flags::CLIENTS);
            level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
        } else {
            remove_block(level, pos, false);
            let yaw = y_rot(state::get_dir(s, "facing").unwrap_or(Direction::North));
            // Creating the baby happy ghast takes one step of the level random (found with the
            // vanilla vectors; turtles and sniffers take none).
            level.random().next_int();
            level.effect(Effect::HatchGhastling { pos, yaw });
        }
    } else if hydration > 0 {
        set_block(level, pos, state::set_int(s, "hydration", hydration - 1), flags::CLIENTS);
        level.effect(Effect::GameEvent { pos, event: "minecraft:block_change" });
    }
}
