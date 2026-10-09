//! Building the wither (`WitherSkullBlock.checkSpawn`): a wither skeleton skull placed to
//! complete the T of soul sand or soul soil with three skulls on top (`BlockPattern.find` in
//! every orientation) clears the pattern and summons an invulnerable wither in its lower
//! middle, facing along the T.

use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Direction, Effect, Level};
use kiln_entity::mob::{self, MobKind};

/// A cell of the pattern (`aisle("^^^", "###", "~#~")`).
#[derive(Clone, Copy)]
enum Cell {
    Skull,
    Base,
    Air,
    Any,
}

const PATTERN: [[Cell; 3]; 3] = [[Cell::Skull, Cell::Skull, Cell::Skull], [Cell::Base, Cell::Base, Cell::Base], [Cell::Air, Cell::Base, Cell::Air]];
/// `getOrCreateWitherBase`: the same without the skulls (`aisle("   ", "###", "~#~")`).
const BASE: [[Cell; 3]; 3] = [[Cell::Any, Cell::Any, Cell::Any], [Cell::Base, Cell::Base, Cell::Base], [Cell::Air, Cell::Base, Cell::Air]];

fn is_skull(state: u16) -> bool {
    matches!(kiln_entity::blocks::block_name(state), "minecraft:wither_skeleton_skull" | "minecraft:wither_skeleton_wall_skull")
}

fn fits(cell: Cell, state: u16) -> bool {
    match cell {
        Cell::Skull => is_skull(state),
        Cell::Base => kiln_entity::ext_entity::wither_skull::block_tag(state, "minecraft:wither_summon_base_blocks"),
        Cell::Air => kiln_data::blocks_types::is_air(state),
        Cell::Any => true,
    }
}

/// `BlockPattern.translateAndRotate`: `right`, `down` and `forwards` steps from the front top
/// left corner, right being `forwards x up`.
fn translate(origin: BlockPos, fwd: Direction, up: Direction, right: i32, down: i32, forwards: i32) -> BlockPos {
    let f = fwd.step();
    let u = up.step();
    let r = [f[1] * u[2] - f[2] * u[1], f[2] * u[0] - f[0] * u[2], f[0] * u[1] - f[1] * u[0]];
    let d = |i: usize| u[i] * -down + r[i] * right + f[i] * forwards;
    BlockPos::new(origin.x + d(0), origin.y + d(1), origin.z + d(2))
}

/// `BlockPattern.find` from the placed skull: the first corner (x fastest, then y, then z) and
/// orientation (`Direction.values()` for forwards, then up) where every cell matches.
fn find(level: &RegionLevel, pos: BlockPos) -> Option<(BlockPos, Direction, Direction)> {
    find_pattern(level, pos, &PATTERN)
}

/// `WitherSkullBlock.canSpawnMob`: a wither skeleton skull at `pos` (not yet placed) would complete the wither
/// pattern: high enough above the bottom of the world, not on peaceful, and the T of soul sand under it.
pub(crate) fn can_spawn_mob(level: &RegionLevel, pos: BlockPos) -> bool {
    let env = level.env;
    pos.y >= env.min_y + 2 && env.mobs.difficulty != 0 && find_pattern(level, pos, &BASE).is_some()
}

fn find_pattern(level: &RegionLevel, pos: BlockPos, pattern: &[[Cell; 3]; 3]) -> Option<(BlockPos, Direction, Direction)> {
    for z in 0..3 {
        for y in 0..3 {
            for x in 0..3 {
                let origin = BlockPos::new(pos.x + x, pos.y + y, pos.z + z);
                for fwd in Direction::ALL {
                    for up in Direction::ALL {
                        if up == fwd || up == fwd.opposite() {
                            continue;
                        }
                        let ok = (0..3).all(|r| (0..3).all(|d| fits(pattern[d as usize][r as usize], level.block(translate(origin, fwd, up, r, d, 0)))));
                        if ok {
                            return Some((origin, fwd, up));
                        }
                    }
                }
            }
        }
    }
    None
}

/// `WitherSkullBlock.checkSpawn` after a wither skeleton skull was placed at `pos`.
pub(crate) fn check_spawn(level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) {
    let env = level.env;
    if !is_skull(level.block(pos)) || pos.y < env.min_y || env.mobs.difficulty == 0 {
        return;
    }
    let Some((origin, fwd, up)) = find(level, pos) else { return };
    // `CarvedPumpkinBlock.clearPatternBlocks`.
    let mut cells = Vec::new();
    for r in 0..3 {
        for d in 0..3 {
            let p = translate(origin, fwd, up, r, d, 0);
            cells.push((p, level.block(p)));
        }
    }
    for &(p, state) in &cells {
        kiln_blocks::set_block(level, p, 0, 2);
        level.effect(Effect::LevelEvent { id: 2001, pos: p, data: state as i32 });
    }
    let at = translate(origin, fwd, up, 1, 2, 0);
    let yaw = if fwd.axis() == kiln_blocks::Axis::X { 0.0 } else { 90.0 };
    let seed = crate::mobs::loot_seed(env.seed, env.game_time, 0, (at.x as u64) << 32 ^ at.z as u64 ^ (at.y as u64) << 16 ^ 0x7769);
    let mut e = mob::new(MobKind::Wither, 0, 0, seed);
    e.set_pos(kiln_entity::math::Vec3::new(at.x as f64 + 0.5, at.y as f64 + 0.55, at.z as f64 + 0.5));
    e.y_rot = yaw;
    e.set_old_pos_and_rot();
    if let Some(m) = mob::data_mut(&mut e) {
        m.y_body_rot = yaw;
        m.y_body_rot_o = yaw;
        mob::kinds::wither::make_invulnerable(m, true);
    }
    let kind = kiln_data::entities::by_name("minecraft:wither").expect("wither type");
    spawns.push(Spawn { kind, pos: [e.x(), e.y(), e.z()], vel: [0.0; 3], body: Body::Ready(Box::new(e)) });
    // `CarvedPumpkinBlock.updatePatternBlocks`.
    for &(p, _) in &cells {
        kiln_blocks::update::update_neighbors_at(level, p, kiln_blocks::BlockId::of(0));
    }
}
