//! Shape updates after placing a template without a known shape (`StructurePlaceSettings`
//! `knownShape == false`): `StructureTemplate.updateShapeAtEdge` over the placed blocks, then
//! `Block.updateFromNeighbourShapes` for each of them, with `kiln-blocks`' `updateShape`.
//!
//! Ticks the updates would schedule are dropped (the region's chunks do not take them here).

use crate::block_facts::Dir;
use crate::blocks::state;
use crate::pos::BlockPos;
use crate::region::Region;
use kiln_blocks as kb;
use kiln_javamath::random::WorldgenRandom as LevelRandom;

/// `updateShape` for states read and written through the region.
struct ShapeLevel<'r, 'a> {
    r: &'r mut Region<'a>,
    block_ticks: kb::LevelTicks<kb::BlockId>,
    fluid_ticks: kb::LevelTicks<kb::FluidType>,
    data: kb::LevelData,
    rules: kb::Rules,
    sub_tick: i64,
}

fn to_kb(p: BlockPos) -> kb::BlockPos {
    kb::BlockPos { x: p.x, y: p.y, z: p.z }
}

fn from_kb(p: kb::BlockPos) -> BlockPos {
    BlockPos::new(p.x, p.y, p.z)
}

fn dir_kb(d: Dir) -> kb::Direction {
    kb::Direction::from_index(d as usize)
}

impl kb::Level for ShapeLevel<'_, '_> {
    type Random = LevelRandom;

    fn block(&self, pos: kb::BlockPos) -> u16 {
        match self.r.chunk(pos.x >> 4, pos.z >> 4) {
            Some(c) => c.get((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize),
            None => state::VOID_AIR,
        }
    }

    fn set_raw(&mut self, pos: kb::BlockPos, s: u16, flags: u32) -> Option<u16> {
        let p = from_kb(pos);
        let old = self.r.get(p);
        (old != s && self.r.set(p, s, flags as i32)).then_some(old)
    }

    fn in_bounds(&self, pos: kb::BlockPos) -> bool {
        !self.r.is_outside_build_height(pos.y)
    }

    fn game_time(&self) -> i64 {
        0
    }

    fn next_sub_tick(&mut self) -> i64 {
        self.sub_tick += 1;
        self.sub_tick - 1
    }

    fn block_ticks(&mut self) -> &mut kb::LevelTicks<kb::BlockId> {
        &mut self.block_ticks
    }

    fn fluid_ticks(&mut self) -> &mut kb::LevelTicks<kb::FluidType> {
        &mut self.fluid_ticks
    }

    fn random(&mut self) -> &mut LevelRandom {
        self.r.level_random()
    }

    fn data(&mut self) -> &mut kb::LevelData {
        &mut self.data
    }

    fn rules(&self) -> &kb::Rules {
        &self.rules
    }

    fn effect(&mut self, _effect: kb::Effect) {}

    fn comparator_output(&self, _pos: kb::BlockPos) -> i32 {
        0
    }

    fn set_comparator_output(&mut self, _pos: kb::BlockPos, _value: i32) {}
}

impl ShapeLevel<'_, '_> {
    fn get(&self, p: BlockPos) -> u16 {
        kb::Level::block(self, to_kb(p))
    }

    /// `BlockState.updateShape(level, level, pos, dir, neighborPos, neighborState, random)`.
    fn update_shape(&mut self, s: u16, p: BlockPos, d: Dir) -> u16 {
        let n = p.relative(d);
        let ns = self.get(n);
        self.update_shape_with(s, p, d, ns)
    }

    fn update_shape_with(&mut self, s: u16, p: BlockPos, d: Dir, ns: u16) -> u16 {
        let s = chest_update_shape(s, d, ns);
        kb::behaviour::update_shape(self, s, to_kb(p), dir_kb(d), to_kb(p.relative(d)), ns)
    }
}

/// `ChestBlock.updateShape`: halves of a double chest pair up or fall back to single.
fn chest_update_shape(s: u16, d: Dir, ns: u16) -> u16 {
    use crate::blocks::{prop, same_block, with_prop};
    if !crate::block_facts::is_instance(s, "ChestBlock") {
        return s;
    }
    let facing = |s: u16| prop(s, "facing").and_then(Dir::by_name).unwrap_or(Dir::North);
    let connected = |s: u16| if prop(s, "type") == Some("left") { facing(s).clockwise() } else { facing(s).counter_clockwise() };
    if same_block(s, ns) && d.is_horizontal() {
        let nt = prop(ns, "type").unwrap_or("single");
        if prop(s, "type") == Some("single") && nt != "single" && facing(s) == facing(ns) && connected(ns) == d.opposite() {
            return with_prop(s, "type", if nt == "left" { "right" } else { "left" });
        }
    } else if connected(s) == d {
        return with_prop(s, "type", "single");
    }
    s
}

/// `Block.UPDATE_SHAPE_ORDER`.
const UPDATE_SHAPE_ORDER: [Dir; 6] = [Dir::West, Dir::East, Dir::North, Dir::South, Dir::Down, Dir::Up];

/// The end of `StructureTemplate.placeInWorld` for an unknown shape: `placed` are the blocks
/// set (in placement order) within `min..=max`.
pub fn update_placed_shapes(r: &mut Region, flags: i32, placed: &[BlockPos], min: [i32; 3], max: [i32; 3]) {
    let mut level = ShapeLevel {
        r,
        block_ticks: kb::LevelTicks::new(),
        fluid_ticks: kb::LevelTicks::new(),
        data: kb::LevelData::new(kb::flags::LIMIT, 0),
        rules: kb::Rules::default(),
        sub_tick: 0,
    };
    let size = [max[0] - min[0] + 1, max[1] - min[1] + 1, max[2] - min[2] + 1];
    let mut filled = vec![false; (size[0] * size[1] * size[2]) as usize];
    let index = |x: i32, y: i32, z: i32| ((x * size[1] + y) * size[2] + z) as usize;
    for p in placed {
        filled[index(p.x - min[0], p.y - min[1], p.z - min[2])] = true;
    }
    let full = |x: i32, y: i32, z: i32| filled[index(x, y, z)];
    // `DiscreteVoxelShape.forAllFaces`: along z, then y, then x.
    let edge = flags & !1;
    let face = |level: &mut ShapeLevel, d: Dir, x: i32, y: i32, z: i32| {
        let p = BlockPos::new(min[0] + x, min[1] + y, min[2] + z);
        let n = p.relative(d);
        let s = level.get(p);
        let ns = level.get(n);
        let s2 = level.update_shape(s, p, d);
        if s2 != s {
            level.r.set(p, s2, edge);
        }
        let n2 = level.update_shape_with(ns, n, d.opposite(), s2);
        if n2 != ns {
            level.r.set(n, n2, edge);
        }
    };
    // (main axis, its size, the two outer axes' sizes, coordinates from (a, b, k))
    let axes: [(Dir, Dir, i32, i32, i32, fn(i32, i32, i32) -> (i32, i32, i32)); 3] = [
        (Dir::North, Dir::South, size[0], size[1], size[2], |a, b, k| (a, b, k)),
        (Dir::Down, Dir::Up, size[2], size[0], size[1], |a, b, k| (b, k, a)),
        (Dir::West, Dir::East, size[1], size[2], size[0], |a, b, k| (k, a, b)),
    ];
    for (neg, pos, sa, sb, sk, at) in axes {
        for a in 0..sa {
            for b in 0..sb {
                let mut prev = false;
                for k in 0..=sk {
                    let now = k != sk && {
                        let (x, y, z) = at(a, b, k);
                        full(x, y, z)
                    };
                    if !prev && now {
                        let (x, y, z) = at(a, b, k);
                        face(&mut level, neg, x, y, z);
                    }
                    if prev && !now {
                        let (x, y, z) = at(a, b, k - 1);
                        face(&mut level, pos, x, y, z);
                    }
                    prev = now;
                }
            }
        }
    }
    let known = (flags & !1) | 16;
    for &p in placed {
        let s = level.get(p);
        let mut new = s;
        for d in UPDATE_SHAPE_ORDER {
            new = level.update_shape(new, p, d);
        }
        if new != s {
            level.r.set(p, new, known);
        }
    }
}
