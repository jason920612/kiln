//! Shape updates of templates placed without a known shape (`StructureTemplate.placeInWorld`
//! with `knownShape` unset): `updateShapeAtEdge` over the faces of the placed blocks, then
//! `Block.updateFromNeighbourShapes` for each of them. Block behaviour comes from
//! `kiln-blocks`, run against the generation region.

use crate::pos::BlockPos;
use crate::region::Region;
use kiln_blocks::level::{Effect, Level, LevelData, Rules};
use kiln_blocks::ticks::{ChunkTicks, LevelTicks};
use kiln_blocks::{BlockId, Direction, FluidType};
use kiln_javamath::random::LegacyRandom;
use std::cell::RefCell;

type BPos = kiln_blocks::BlockPos;

/// `WorldGenRegion` as a block behaviour level: reads and writes go to the region, ticks
/// are collected and handed to the region's chunks afterwards.
struct ShapeLevel<'r, 'a> {
    r: RefCell<&'r mut Region<'a>>,
    block_ticks: LevelTicks<BlockId>,
    fluid_ticks: LevelTicks<FluidType>,
    sub_tick: i64,
    random: LegacyRandom,
    data: LevelData,
    rules: Rules,
    chunks: Vec<(i32, i32)>,
}

impl<'r, 'a> ShapeLevel<'r, 'a> {
    fn new(r: &'r mut Region<'a>) -> Self {
        let (cx, cz) = (r.cx, r.cz);
        let chunks: Vec<(i32, i32)> = (0..9).map(|i| (cx + i % 3 - 1, cz + i / 3 - 1)).collect();
        let (mut block_ticks, mut fluid_ticks) = (LevelTicks::new(), LevelTicks::new());
        for &c in &chunks {
            block_ticks.add_container(c, ChunkTicks::new());
            fluid_ticks.add_container(c, ChunkTicks::new());
        }
        Self {
            r: RefCell::new(r),
            block_ticks,
            fluid_ticks,
            sub_tick: 0,
            random: LegacyRandom::new(0),
            data: LevelData::new(1_000_000, 0),
            rules: Rules::default(),
            chunks,
        }
    }

    /// Hands the scheduled ticks to the region (`WorldGenRegion.scheduleTick`).
    fn finish(self) {
        let r = self.r.into_inner();
        for c in &self.chunks {
            if let Some(t) = self.block_ticks.container(*c) {
                for t in t.iter() {
                    r.schedule_block_tick(BlockPos::new(t.pos.x, t.pos.y, t.pos.z), t.kind.name(), t.trigger as i32);
                }
            }
            if let Some(t) = self.fluid_ticks.container(*c) {
                for t in t.iter() {
                    r.schedule_fluid_tick(BlockPos::new(t.pos.x, t.pos.y, t.pos.z), t.kind.name(), t.trigger as i32);
                }
            }
        }
    }

    fn get(&self, p: BPos) -> u16 {
        self.r.borrow_mut().get(BlockPos::new(p.x, p.y, p.z))
    }

    fn set(&self, p: BPos, s: u16, flags: i32) {
        self.r.borrow_mut().set(BlockPos::new(p.x, p.y, p.z), s, flags);
    }
}

impl Level for ShapeLevel<'_, '_> {
    type Random = LegacyRandom;

    fn block(&self, pos: BPos) -> u16 {
        self.get(pos)
    }

    fn set_raw(&mut self, pos: BPos, state: u16, flags: u32) -> Option<u16> {
        let old = self.get(pos);
        if old == state {
            return None;
        }
        self.set(pos, state, flags as i32);
        Some(old)
    }

    fn in_bounds(&self, pos: BPos) -> bool {
        !self.r.borrow().is_outside_build_height(pos.y)
    }

    fn game_time(&self) -> i64 {
        0
    }

    fn next_sub_tick(&mut self) -> i64 {
        self.sub_tick += 1;
        self.sub_tick - 1
    }

    fn block_ticks(&mut self) -> &mut LevelTicks<BlockId> {
        &mut self.block_ticks
    }

    fn fluid_ticks(&mut self) -> &mut LevelTicks<FluidType> {
        &mut self.fluid_ticks
    }

    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }

    fn data(&mut self) -> &mut LevelData {
        &mut self.data
    }

    fn rules(&self) -> &Rules {
        &self.rules
    }

    fn effect(&mut self, _effect: Effect) {}

    fn comparator_output(&self, _pos: BPos) -> i32 {
        0
    }

    fn set_comparator_output(&mut self, _pos: BPos, _value: i32) {}
}

/// The placed blocks as `BitSetDiscreteVoxelShape` over their bounds.
struct Filled {
    min: [i32; 3],
    size: [i32; 3],
    bits: Vec<bool>,
}

impl Filled {
    fn full(&self, x: i32, y: i32, z: i32) -> bool {
        self.bits[((x * self.size[1] + y) * self.size[2] + z) as usize]
    }
}

fn dir_of(axis: usize, positive: bool) -> Direction {
    match (axis, positive) {
        (0, false) => Direction::West,
        (0, true) => Direction::East,
        (1, false) => Direction::Down,
        (1, true) => Direction::Up,
        (2, false) => Direction::North,
        _ => Direction::South,
    }
}

/// `StructureTemplate.updateShapeAtEdge` then `updateFromNeighbourShapes` of every placed
/// block, for `flags` given to `placeInWorld`.
pub fn update_shapes(r: &mut Region, placed: &[BlockPos], min: [i32; 3], max: [i32; 3], flags: i32) {
    let size = [max[0] - min[0] + 1, max[1] - min[1] + 1, max[2] - min[2] + 1];
    let mut shape = Filled { min, size, bits: vec![false; (size[0] * size[1] * size[2]) as usize] };
    for p in placed {
        let (x, y, z) = (p.x - min[0], p.y - min[1], p.z - min[2]);
        shape.bits[((x * size[1] + y) * size[2] + z) as usize] = true;
    }
    let mut level = ShapeLevel::new(r);
    // `forAllFaces`: z faces (NONE), then y (FORWARD), then x (BACKWARD); per axis the two
    // outer loops run over the other axes in cycle order.
    for (axis, outer, inner) in [(2usize, 0usize, 1usize), (1, 2, 0), (0, 1, 2)] {
        for i in 0..size[outer] {
            for j in 0..size[inner] {
                let mut prev = false;
                for k in 0..=size[axis] {
                    let at = |k: i32| {
                        let mut c = [0; 3];
                        c[outer] = i;
                        c[inner] = j;
                        c[axis] = k;
                        c
                    };
                    let cur = k != size[axis] && {
                        let c = at(k);
                        shape.full(c[0], c[1], c[2])
                    };
                    if !prev && cur {
                        edge(&mut level, &shape, at(k), dir_of(axis, false), flags);
                    }
                    if prev && !cur {
                        edge(&mut level, &shape, at(k - 1), dir_of(axis, true), flags);
                    }
                    prev = cur;
                }
            }
        }
    }
    for p in placed {
        let pos = BPos { x: p.x, y: p.y, z: p.z };
        let s = level.get(pos);
        let new = kiln_blocks::update::update_from_neighbour_shapes(&mut level, s, pos);
        if new != s {
            level.set(pos, new, (flags & !1) | 16);
        }
    }
    level.finish();
}

/// The `updateShapeAtEdge` face consumer: the placed block and its outside neighbour update
/// each other.
fn edge(level: &mut ShapeLevel, shape: &Filled, c: [i32; 3], dir: Direction, flags: i32) {
    let pos = BPos { x: shape.min[0] + c[0], y: shape.min[1] + c[1], z: shape.min[2] + c[2] };
    let n = pos.relative(dir);
    let s = level.get(pos);
    let ns = level.get(n);
    let s2 = kiln_blocks::behaviour::update_shape(level, s, pos, dir, n, ns);
    if s != s2 {
        level.set(pos, s2, flags & !1);
    }
    let n2 = kiln_blocks::behaviour::update_shape(level, ns, n, dir.opposite(), pos, s2);
    if ns != n2 {
        level.set(n, n2, flags & !1);
    }
}
