//! Golems built from blocks (`CarvedPumpkinBlock.trySpawnGolem`): a carved pumpkin or jack
//! o'lantern placed to complete two snow blocks (a snow golem) or a T of four iron blocks (an
//! iron golem, player-created), in any orientation (`BlockPattern.find`). The blocks go, the
//! golem stands where the bottom of the pattern was.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Level};
use kiln_entity::mob::MobKind;

/// What a pattern cell wants.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cell {
    Pumpkin,
    Snow,
    Iron,
    Air,
}

/// `BlockPatternBuilder.aisle(...)`: rows top to bottom, one aisle.
const SNOW_GOLEM: [&[Cell]; 3] = [&[Cell::Pumpkin], &[Cell::Snow], &[Cell::Snow]];
const IRON_GOLEM: [&[Cell]; 3] = [&[Cell::Air, Cell::Pumpkin, Cell::Air], &[Cell::Iron, Cell::Iron, Cell::Iron], &[Cell::Air, Cell::Iron, Cell::Air]];

/// `Direction.values()` steps: down, up, north, south, west, east.
const DIRS: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];

fn matches(state: u16, cell: Cell) -> bool {
    let name = kiln_data::blocks_types::block_of(state).name;
    match cell {
        Cell::Pumpkin => name == "minecraft:carved_pumpkin" || name == "minecraft:jack_o_lantern",
        Cell::Snow => name == "minecraft:snow_block",
        Cell::Iron => name == "minecraft:iron_block",
        Cell::Air => kiln_data::blocks_types::is_air(state),
    }
}

/// `BlockPattern.translateAndRotate`.
fn translate(origin: BlockPos, forward: [i32; 3], up: [i32; 3], right: i32, down: i32, depth: i32) -> BlockPos {
    let r = [forward[1] * up[2] - forward[2] * up[1], forward[2] * up[0] - forward[0] * up[2], forward[0] * up[1] - forward[1] * up[0]];
    BlockPos::new(
        origin.x + up[0] * -down + r[0] * right + forward[0] * depth,
        origin.y + up[1] * -down + r[1] * right + forward[1] * depth,
        origin.z + up[2] * -down + r[2] * right + forward[2] * depth,
    )
}

/// A match: the front top left corner and the orientation.
struct Match {
    origin: BlockPos,
    forward: [i32; 3],
    up: [i32; 3],
}

impl Match {
    fn block(&self, x: i32, y: i32) -> BlockPos {
        translate(self.origin, self.forward, self.up, x, y, 0)
    }
}

/// `BlockPattern.find`: from `pos` over the box of the pattern's size above it, every
/// orientation in `Direction` order.
fn find(level: &RegionLevel, pos: BlockPos, rows: &[&[Cell]]) -> Option<Match> {
    let (w, h) = (rows[0].len() as i32, rows.len() as i32);
    let max = w.max(h);
    for dz in 0..max {
        for dy in 0..max {
            for dx in 0..max {
                let origin = BlockPos::new(pos.x + dx, pos.y + dy, pos.z + dz);
                for f in 0..6 {
                    for u in 0..6 {
                        if u == f || u == (f ^ 1) {
                            continue;
                        }
                        let (forward, up) = (DIRS[f], DIRS[u]);
                        let ok = (0..w).all(|x| (0..h).all(|y| matches(level.block(translate(origin, forward, up, x, y, 0)), rows[y as usize][x as usize])));
                        if ok {
                            return Some(Match { origin, forward, up });
                        }
                    }
                }
            }
        }
    }
    None
}

/// `trySpawnGolem` after a pumpkin was placed at `pos` by player `p` (the one who sees the
/// `summoned_entity` criterion).
pub(crate) fn try_spawn(level: &mut RegionLevel, pos: BlockPos, p: &mut Player, spawns: &mut Vec<Spawn>) {
    let (m, kind, rows, spawn_at) = if let Some(m) = find(level, pos, &SNOW_GOLEM) {
        let at = m.block(0, 2);
        (m, MobKind::SnowGolem, &SNOW_GOLEM, at)
    } else if let Some(m) = find(level, pos, &IRON_GOLEM) {
        let at = m.block(1, 2);
        (m, MobKind::IronGolem, &IRON_GOLEM, at)
    } else {
        return;
    };
    // `clearPatternBlocks`: each block to air (clients only), with its break particles.
    let (w, h) = (rows[0].len() as i32, rows.len() as i32);
    let mut cleared = Vec::new();
    for x in 0..w {
        for y in 0..h {
            let at = m.block(x, y);
            let old = level.block(at);
            kiln_blocks::set_block(level, at, 0, kiln_blocks::flags::CLIENTS);
            level.effect(kiln_blocks::Effect::LevelEvent { id: 2001, pos: at, data: old as i32 });
            cleared.push(at);
        }
    }
    let mut golem = kiln_entity::mob::new(kind, 0, 0, crate::mobs::loot_seed(level.env.seed, level.env.game_time, p.entity_id, 0x676f_6c65));
    golem.set_pos(kiln_entity::math::Vec3::new(spawn_at.x as f64 + 0.5, spawn_at.y as f64 + 0.05, spawn_at.z as f64 + 0.5));
    golem.y_rot = 0.0;
    golem.x_rot = 0.0;
    golem.set_old_pos_and_rot();
    if let Some(md) = kiln_entity::mob::data_mut(&mut golem) {
        md.y_head_rot = 0.0;
        md.y_body_rot = 0.0;
        if kind == MobKind::IronGolem
            && let Some(s) = kiln_entity::mob::ext::state_mut::<kiln_entity::mob::kinds::iron_golem::State>(md)
        {
            s.player_created = true;
        }
    }
    // `SummonedEntityTrigger` for players within 5 blocks of the golem.
    let bb = golem.bounding_box().inflate_all(5.0);
    let pb = kiln_entity::math::Aabb::new(p.pos[0] - 0.3, p.pos[1], p.pos[2] - 0.3, p.pos[0] + 0.3, p.pos[1] + 1.8, p.pos[2] + 0.3);
    if bb.intersects(&pb) {
        let criterion = kiln_entity::level::Criterion::SummonedEntity { entity: kiln_entity::level::Seen::of(&golem) };
        p.entity_criterion(crate::DIMENSIONS[level.env.dim].0, &criterion);
    }
    let kind_t = kiln_data::entities::by_name(kind.type_name()).expect("golem type");
    let pos3 = golem.position();
    spawns.push(Spawn { kind: kind_t, pos: [pos3.x, pos3.y, pos3.z], vel: [0.0; 3], body: Body::Ready(Box::new(golem)) });
    // `updatePatternBlocks`.
    for at in cleared {
        kiln_blocks::update::update_neighbors_at(level, at, kiln_blocks::BlockId::of(0));
    }
}
