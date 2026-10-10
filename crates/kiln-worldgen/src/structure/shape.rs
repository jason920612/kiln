//! Shape updates after placing a template without a known shape (`StructurePlaceSettings`
//! `knownShape == false`): `StructureTemplate.updateShapeAtEdge` over the placed blocks, then
//! `Block.updateFromNeighbourShapes` for each of them, with `kiln-blocks`' `updateShape`.
//!
//! Ticks the updates schedule go to the region's proto-chunks in scheduling order.

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

impl<'r, 'a> ShapeLevel<'r, 'a> {
    /// Tick containers for the region's nine chunks, so scheduling reaches them.
    fn new(r: &'r mut Region<'a>) -> Self {
        let mut level = ShapeLevel {
            block_ticks: kb::LevelTicks::new(),
            fluid_ticks: kb::LevelTicks::new(),
            data: kb::LevelData::new(kb::flags::LIMIT, 0),
            rules: kb::Rules::default(),
            sub_tick: 0,
            r,
        };
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = (level.r.cx + dx, level.r.cz + dz);
                level.block_ticks.add_container(key, kb::ChunkTicks::new());
                level.fluid_ticks.add_container(key, kb::ChunkTicks::new());
            }
        }
        level
    }

    /// Moves the scheduled ticks to the region (`ProtoChunkTicks`), oldest first.
    fn flush_ticks(&mut self) {
        let mut blocks: Vec<kb::ScheduledTick<kb::BlockId>> = Vec::new();
        let mut fluids: Vec<kb::ScheduledTick<kb::FluidType>> = Vec::new();
        let keys: Vec<_> = self.block_ticks.chunks().collect();
        for key in keys {
            if let Some(c) = self.block_ticks.remove_container(key) {
                blocks.extend(c.iter().copied());
                self.block_ticks.add_container(key, kb::ChunkTicks::new());
            }
            if let Some(c) = self.fluid_ticks.remove_container(key) {
                fluids.extend(c.iter().copied());
                self.fluid_ticks.add_container(key, kb::ChunkTicks::new());
            }
        }
        let mut all: Vec<(i64, bool, BlockPos, &'static str)> = blocks
            .iter()
            .map(|t| (t.sub, true, from_kb(t.pos), t.kind.name()))
            .chain(fluids.iter().map(|t| (t.sub, false, from_kb(t.pos), t.kind.name())))
            .collect();
        all.sort_by_key(|t| t.0);
        for (_, block, p, name) in all {
            if block {
                self.r.schedule_block_tick(p, name, 0);
            } else {
                self.r.schedule_fluid_tick(p, name, 0);
            }
        }
    }
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

    /// Chunks being generated are unlit.
    fn raw_brightness(&self, _pos: kb::BlockPos, _sky_darken: i32) -> i32 {
        0
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
        if let Some(new) = attached_stem_update_shape(s, d, ns) {
            return new;
        }
        if let Some(new) = multiface_update_shape(s, d, ns) {
            return new;
        }
        if crate::blocks::is_block(s, "minecraft:pale_moss_carpet") {
            return self.mossy_carpet_update_shape(s, p);
        }
        if let Some(new) = self.banner_update_shape(s, p, d) {
            return new;
        }
        if crate::blocks::is_block(s, "minecraft:chorus_plant") {
            return self.chorus_plant_update_shape(s, p, d, ns);
        }
        let s = chest_update_shape(s, d, ns);
        kb::behaviour::update_shape(self, s, to_kb(p), dir_kb(d), to_kb(p.relative(d)), ns)
    }
}

impl ShapeLevel<'_, '_> {
    /// `ChorusPlantBlock.updateShape`: unsupported plants schedule their removal (and keep
    /// their state); others connect toward `d` if the neighbour is chorus (or, downward, a
    /// block supporting chorus).
    fn chorus_plant_update_shape(&mut self, s: u16, p: BlockPos, d: Dir, ns: u16) -> u16 {
        use crate::blocks::{is_air, is_block, with_prop};
        let chorus = |s: u16| is_block(s, "minecraft:chorus_plant");
        let supports = |s: u16| crate::vtags::is(s, "supports_chorus_plant");
        // `canSurvive`.
        let below = self.get(p.below());
        let vertical = !is_air(self.get(p.above())) && !is_air(below);
        let mut survives = None;
        for h in Dir::HORIZONTAL {
            let q = p.relative(h);
            if chorus(self.get(q)) {
                if vertical {
                    survives = Some(false);
                    break;
                }
                let qb = self.get(q.below());
                if chorus(qb) || supports(qb) {
                    survives = Some(true);
                    break;
                }
            }
        }
        let survives = survives.unwrap_or_else(|| chorus(below) || supports(below));
        if !survives {
            self.r.schedule_block_tick(p, "minecraft:chorus_plant", 1);
            return s;
        }
        let connects = chorus(ns) || is_block(ns, "minecraft:chorus_flower") || (d == Dir::Down && supports(ns));
        with_prop(s, d.name(), if connects { "true" } else { "false" })
    }

    /// `WallBannerBlock` / `BannerBlock.updateShape` (and the signs': `StandingSignBlock`, `WallSignBlock`): gone without a
    /// (legacy) solid block behind or below.
    fn banner_update_shape(&self, s: u16, p: BlockPos, d: Dir) -> Option<u16> {
        use crate::block_facts::{block_class, is_solid};
        let support = match block_class(s) {
            "WallBannerBlock" | "WallSignBlock" => crate::blocks::prop(s, "facing").and_then(Dir::by_name)?.opposite(),
            "BannerBlock" | "StandingSignBlock" => Dir::Down,
            _ => return None,
        };
        (d == support && !is_solid(self.get(p.relative(support)))).then_some(state::AIR)
    }

    /// `MossyCarpetBlock.updateShape`: gone when unsupported or without faces, else
    /// `getUpdatedState(state, level, pos, false)`.
    fn mossy_carpet_update_shape(&self, s: u16, p: BlockPos) -> u16 {
        use crate::blocks::{is_block, prop, with_prop};
        let carpet = |s: u16| is_block(s, "minecraft:pale_moss_carpet");
        let base = |s: u16| prop(s, "bottom") == Some("true");
        let below = self.get(p.below());
        let survives = if base(s) { !crate::blocks::is_air(below) } else { carpet(below) && base(below) };
        if !survives {
            return state::AIR;
        }
        let create = base(s);
        let mut out = s;
        for d in Dir::HORIZONTAL {
            let key = d.name();
            let supported = crate::feature::vegetation::shape::can_attach_to(self.get(p.relative(d)), d);
            let mut side = if !supported {
                "none"
            } else if create {
                "low"
            } else {
                prop(s, key).unwrap_or("none")
            };
            if side == "low" {
                let above = self.get(p.above());
                if carpet(above) && prop(above, key) != Some("none") && !base(above) {
                    side = "tall";
                }
                if !base(s) && carpet(below) && prop(below, key) == Some("none") {
                    side = "none";
                }
            }
            out = with_prop(out, key, side);
        }
        let faces = base(out) || Dir::HORIZONTAL.iter().any(|d| prop(out, d.name()) != Some("none"));
        if faces { out } else { state::AIR }
    }
}

/// `AttachedStemBlock.updateShape`: without its fruit in front it is a grown stem again.
fn attached_stem_update_shape(s: u16, d: Dir, ns: u16) -> Option<u16> {
    use crate::blocks::{is_block, prop, with_prop};
    let (fruit, stem) = match kiln_data::blocks_types::block_of(s).name {
        "minecraft:attached_pumpkin_stem" => ("minecraft:pumpkin", "minecraft:pumpkin_stem"),
        "minecraft:attached_melon_stem" => ("minecraft:melon", "minecraft:melon_stem"),
        _ => return None,
    };
    let facing = prop(s, "facing").and_then(Dir::by_name)?;
    (d == facing && !is_block(ns, fruit)).then(|| with_prop(super::legacy::st(stem), "age", "7"))
}

/// `MultifaceBlock.updateShape` (glow lichen, sculk veins, resin clumps): a face loses its
/// support and goes; no faces left leaves air (water when waterlogged).
fn multiface_update_shape(s: u16, d: Dir, ns: u16) -> Option<u16> {
    use crate::blocks::{prop, with_prop};
    if !crate::block_facts::is_instance(s, "MultifaceBlock") {
        return None;
    }
    let empty = |s: u16| if prop(s, "waterlogged") == Some("true") { state::WATER } else { state::AIR };
    let has = |s: u16, d: Dir| prop(s, d.name()) == Some("true");
    if !Dir::ALL.iter().any(|&f| has(s, f)) {
        return Some(state::AIR);
    }
    if has(s, d) && !crate::feature::vegetation::shape::can_attach_to(ns, d) {
        let out = with_prop(s, d.name(), "false");
        return Some(if Dir::ALL.iter().any(|&f| has(out, f)) { out } else { empty(s) });
    }
    Some(s)
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

/// One `forAllAxisFaces` pass: the faces' directions, the outer and main axis sizes, and the
/// cell at `(a, b, k)`.
type FaceAxis = (Dir, Dir, i32, i32, i32, fn(i32, i32, i32) -> (i32, i32, i32));

/// `Block.UPDATE_SHAPE_ORDER`.
const UPDATE_SHAPE_ORDER: [Dir; 6] = [Dir::West, Dir::East, Dir::North, Dir::South, Dir::Down, Dir::Up];

/// `BlockState.updateShape` of `s` at `p` toward `d` (neighbour `ns`) with `kiln-blocks`'
/// behaviour over the region.
pub fn update_shape(r: &mut Region, s: u16, p: BlockPos, d: Dir, ns: u16) -> u16 {
    let mut level = ShapeLevel::new(r);
    let out = level.update_shape_with(s, p, d, ns);
    level.flush_ticks();
    out
}

/// The end of `StructureTemplate.placeInWorld` for an unknown shape: `placed` are the blocks
/// set (in placement order) within `min..=max`.
pub fn update_placed_shapes(r: &mut Region, flags: i32, placed: &[BlockPos], min: [i32; 3], max: [i32; 3]) {
    let mut level = ShapeLevel::new(r);
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
    let axes: [FaceAxis; 3] = [
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
    level.flush_ticks();
}
