//! Single-piece surface structures on `ScatteredFeaturePiece`: desert pyramids
//! (`DesertPyramidStructure`, `DesertPyramidPiece`), jungle temples (`JungleTempleStructure`,
//! `JungleTemplePiece`) and swamp huts (`SwampHutStructure`, `SwampHutPiece`).

use super::{bool_tag, st, with};
use crate::block_facts::Dir;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext, set_loot_table};
use crate::structure::{GenCtx, Start, Stub};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// `ScatteredFeaturePiece`: a piece that settles onto the ground the first time it is placed.
#[derive(Debug)]
struct Scattered {
    base: PieceBase,
    width: i32,
    height: i32,
    depth: i32,
    /// `heightPosition`: -1 until measured.
    hpos: AtomicI32,
    /// How far the box has moved up since it was built.
    dy: AtomicI32,
}

impl Scattered {
    fn new(kind: &'static str, random: &mut WorldgenRandom, x: i32, z: i32, width: i32, height: i32, depth: i32) -> Self {
        let dir = PieceBase::random_horizontal(random);
        let mut base = PieceBase::new(kind, 0, PieceBase::make_bbox(x, 64, z, dir, width, height, depth));
        base.set_orientation(Some(dir));
        Self { base, width, height, depth, hpos: AtomicI32::new(-1), dy: AtomicI32::new(0) }
    }

    /// The piece as placed now (its box moved to the measured height).
    fn current(&self) -> PieceBase {
        let mut b = self.base.clone();
        b.bbox.shift(0, self.dy.load(Ordering::Relaxed), 0);
        b
    }

    fn settle(&self, h: i32, offset: i32) {
        self.hpos.store(h, Ordering::Relaxed);
        let min_y = self.base.bbox.min_y + self.dy.load(Ordering::Relaxed);
        self.dy.fetch_add(h - min_y + offset, Ordering::Relaxed);
    }

    /// `updateAverageGroundHeight`: the mean surface height over the part inside `cb`.
    fn update_average_ground_height(&self, r: &mut Region, cb: &BoundingBox, offset: i32) -> bool {
        if self.hpos.load(Ordering::Relaxed) >= 0 {
            return true;
        }
        let b = self.base.bbox;
        let (mut sum, mut count) = (0, 0);
        for z in b.min_z..=b.max_z {
            for x in b.min_x..=b.max_x {
                if cb.is_inside(BlockPos::new(x, 64, z)) {
                    sum += r.height_at(Heightmap::MotionBlockingNoLeaves, x, z);
                    count += 1;
                }
            }
        }
        if count == 0 {
            return false;
        }
        self.settle(sum / count, offset);
        true
    }

    /// `updateHeightPositionToLowestGroundHeight`: the lowest surface height over the whole box.
    fn update_to_lowest_ground_height(&self, r: &mut Region, offset: i32) -> bool {
        if self.hpos.load(Ordering::Relaxed) >= 0 {
            return true;
        }
        let b = self.base.bbox;
        let mut h = r.max_y() + 1;
        for z in b.min_z..=b.max_z {
            for x in b.min_x..=b.max_x {
                h = h.min(r.height_at(Heightmap::MotionBlockingNoLeaves, x, z));
            }
        }
        self.settle(h, offset);
        true
    }

    /// `StructurePiece.createTag` with the current box and `ScatteredFeaturePiece`'s fields.
    fn save(&self, extra: Vec<(String, Tag)>) -> Tag {
        let b = self.current();
        let mut tag = vec![
            ("id".to_string(), Tag::String(b.kind.to_string())),
            ("BB".to_string(), b.bbox.to_tag()),
            ("O".to_string(), Tag::Int(b.orientation().map_or(-1, Dir::index_2d))),
            ("GD".to_string(), Tag::Int(b.gen_depth)),
            ("Width".to_string(), Tag::Int(self.width)),
            ("Height".to_string(), Tag::Int(self.height)),
            ("Depth".to_string(), Tag::Int(self.depth)),
            ("HPos".to_string(), Tag::Int(self.hpos.load(Ordering::Relaxed))),
        ];
        tag.extend(extra);
        Tag::Compound(tag)
    }
}

/// `SinglePieceStructure.findGenerationPoint`: at the chunk center, if the chunk's corners are
/// not below sea level.
fn single_piece<'k>(ctx: &mut GenCtx, width: i32, depth: i32, make: fn(&mut WorldgenRandom, i32, i32) -> Box<dyn Piece>) -> Option<Stub<'k>> {
    if !ctx.could_exist_at_chunk_center() || ctx.lowest_y(width, depth) < ctx.generator.sea_level {
        return None;
    }
    let pos = ctx.chunk_center_on(Heightmap::WorldSurfaceWg);
    let (x, z) = (ctx.chunk.0 << 4, ctx.chunk.1 << 4);
    Some(Stub { pos, build: Box::new(move |ctx, pieces| pieces.push(make(&mut ctx.random, x, z))) })
}

fn flags_tag(names: &[&str], flags: &[AtomicBool]) -> Vec<(String, Tag)> {
    names.iter().zip(flags).map(|(n, f)| (n.to_string(), bool_tag(f.load(Ordering::Relaxed)))).collect()
}

macro_rules! piece_impl {
    () => {
        fn base(&self) -> &PieceBase {
            &self.s.base
        }

        fn base_mut(&mut self) -> &mut PieceBase {
            &mut self.s.base
        }

        fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
            self.s.base.bbox.shift(dx, dy, dz);
        }
    };
}

// ---- Desert pyramid ----

pub struct DesertPyramid;

impl Kind for DesertPyramid {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        single_piece(ctx, 21, 21, |random, x, z| {
            Box::new(PyramidPiece { s: Scattered::new("minecraft:tedp", random, x, z, 21, 15, 21), chests: Default::default() })
        })
    }

    /// Turns some of the cellar's sand into suspicious sand.
    fn after_place(&self, _cx: &PlaceContext, r: &mut Region, _random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), start: &Start) {
        let mut sand: Vec<BlockPos> = Vec::new();
        for p in &start.pieces {
            if let Some(p) = p.as_any().and_then(|a| a.downcast_ref::<PyramidPiece>()) {
                let (list, roof) = p.sand_positions(r.seed());
                sand.extend(list);
                place_suspicious_sand(r, cb, roof);
            }
        }
        sand.sort_by_key(|p| (p.y, p.z, p.x));
        sand.dedup();
        let Some(b) = start.pieces.iter().map(|p| p.bbox_now()).reduce(|mut a, b| {
            a.encapsulate(&b);
            a
        }) else {
            return;
        };
        let c = b.center();
        let mut random = LegacyRandom::new(r.seed()).fork_positional().at(c.x, c.y, c.z);
        for i in (2..=sand.len()).rev() {
            let j = random.next_int_bounded(i as i32) as usize;
            sand.swap(i - 1, j);
        }
        let mut count = sand.len().min((5 + random.next_int_bounded(3)) as usize);
        for p in sand {
            if count > 0 {
                count -= 1;
                place_suspicious_sand(r, cb, p);
            } else if cb.is_inside(p) {
                r.set(p, st("minecraft:sand"), 2);
            }
        }
    }
}

fn place_suspicious_sand(r: &mut Region, cb: &BoundingBox, p: BlockPos) {
    if cb.is_inside(p) {
        r.set(p, st("minecraft:suspicious_sand"), 2);
        set_loot_table(r, p, "minecraft:archaeology/desert_pyramid", p.as_long());
    }
}

#[derive(Debug)]
struct PyramidPiece {
    s: Scattered,
    chests: [AtomicBool; 4],
}

impl PyramidPiece {
    /// `potentialSuspiciousSandWorldPositions` and `randomCollapsedRoofPos` of the placed piece.
    fn sand_positions(&self, seed: i64) -> (Vec<BlockPos>, BlockPos) {
        let b = self.s.current();
        let (x, y, z) = (16, -4, 13);
        let mut list = Vec::new();
        for yy in y + 1..=y + 3 {
            for xx in x - 2..=x + 2 {
                for zz in z - 2..=z + 2 {
                    list.push(b.world_pos(xx, yy, zz));
                }
            }
        }
        for (dx, dz) in [(3, 0), (-3, 0), (0, 3), (0, -3)] {
            list.push(b.world_pos(x + dx, y + 1, z + dz));
            list.push(b.world_pos(x + dx, y + 2, z + dz));
        }
        let (x0, x1, z0, z1, ry) = (x - 2, x + 2, z - 2, z + 2, y + 4);
        let o = b.world_pos(x0, ry, z0);
        let mut random = LegacyRandom::new(seed).fork_positional().at(o.x, o.y, o.z);
        let rx = random.next_int_bounded(x1 - x0 + 1) + x0;
        let rz = random.next_int_bounded(z1 - z0 + 1) + z0;
        (list, BlockPos::new(b.world_x(rx, rz), b.world_y(ry), b.world_z(rx, rz)))
    }

    fn cellar(&self, b: &PieceBase, r: &mut Region, cb: &BoundingBox) {
        let (v4, v5, v6) = (16, -4, 13);
        let v7 = st("minecraft:sandstone_stairs");
        let turned = crate::structure::transform::rotate(v7, crate::structure::transform::Rotation::CounterClockwise90);
        b.place_block(r, turned, 13, -1, 17, cb);
        b.place_block(r, turned, 14, -2, 17, cb);
        b.place_block(r, turned, 15, -3, 17, cb);
        let v8 = st("minecraft:sand");
        let v9 = st("minecraft:sandstone");
        let v10 = r.level_random().next_bool();
        b.place_block(r, v8, v4 - 4, v5 + 4, v6 + 4, cb);
        b.place_block(r, v8, v4 - 3, v5 + 4, v6 + 4, cb);
        b.place_block(r, v8, v4 - 2, v5 + 4, v6 + 4, cb);
        b.place_block(r, v8, v4 - 1, v5 + 4, v6 + 4, cb);
        b.place_block(r, v8, v4, v5 + 4, v6 + 4, cb);
        b.place_block(r, v8, v4 - 2, v5 + 3, v6 + 4, cb);
        b.place_block(r, if v10 { v8 } else { v9 }, v4 - 1, v5 + 3, v6 + 4, cb);
        b.place_block(r, if v10 { v9 } else { v8 }, v4, v5 + 3, v6 + 4, cb);
        b.place_block(r, v8, v4 - 1, v5 + 2, v6 + 4, cb);
        b.place_block(r, v9, v4, v5 + 2, v6 + 4, cb);
        b.place_block(r, v8, v4, v5 + 1, v6 + 4, cb);
        // addCellarRoom
        let v7 = st("minecraft:cut_sandstone");
        let v8 = st("minecraft:chiseled_sandstone");
        b.generate_box(r, cb, v4 - 3, v5 + 1, v6 - 3, v4 - 3, v5 + 1, v6 + 2, v7, v7, true);
        b.generate_box(r, cb, v4 + 3, v5 + 1, v6 - 3, v4 + 3, v5 + 1, v6 + 2, v7, v7, true);
        b.generate_box(r, cb, v4 - 3, v5 + 1, v6 - 3, v4 + 3, v5 + 1, v6 - 2, v7, v7, true);
        b.generate_box(r, cb, v4 - 3, v5 + 1, v6 + 3, v4 + 3, v5 + 1, v6 + 3, v7, v7, true);
        b.generate_box(r, cb, v4 - 3, v5 + 2, v6 - 3, v4 - 3, v5 + 2, v6 + 2, v8, v8, true);
        b.generate_box(r, cb, v4 + 3, v5 + 2, v6 - 3, v4 + 3, v5 + 2, v6 + 2, v8, v8, true);
        b.generate_box(r, cb, v4 - 3, v5 + 2, v6 - 3, v4 + 3, v5 + 2, v6 - 2, v8, v8, true);
        b.generate_box(r, cb, v4 - 3, v5 + 2, v6 + 3, v4 + 3, v5 + 2, v6 + 3, v8, v8, true);
        b.generate_box(r, cb, v4 - 3, -1, v6 - 3, v4 - 3, -1, v6 + 2, v7, v7, true);
        b.generate_box(r, cb, v4 + 3, -1, v6 - 3, v4 + 3, -1, v6 + 2, v7, v7, true);
        b.generate_box(r, cb, v4 - 3, -1, v6 - 3, v4 + 3, -1, v6 - 2, v7, v7, true);
        b.generate_box(r, cb, v4 - 3, -1, v6 + 3, v4 + 3, -1, v6 + 3, v7, v7, true);
        for x in v4 - 2..=v4 + 2 {
            for z in v6 - 2..=v6 + 2 {
                let s = if r.level_random().next_float() < 0.33 { "minecraft:sandstone" } else { "minecraft:sand" };
                b.place_block(r, st(s), x, v5 + 4, z, cb);
            }
        }
        let v9 = st("minecraft:orange_terracotta");
        let v10 = st("minecraft:blue_terracotta");
        b.place_block(r, v10, v4, v5, v6, cb);
        b.place_block(r, v9, v4 + 1, v5, v6 - 1, cb);
        b.place_block(r, v9, v4 + 1, v5, v6 + 1, cb);
        b.place_block(r, v9, v4 - 1, v5, v6 - 1, cb);
        b.place_block(r, v9, v4 - 1, v5, v6 + 1, cb);
        b.place_block(r, v9, v4 + 2, v5, v6, cb);
        b.place_block(r, v9, v4 - 2, v5, v6, cb);
        b.place_block(r, v9, v4, v5, v6 + 2, cb);
        b.place_block(r, v9, v4, v5, v6 - 2, cb);
        b.place_block(r, v9, v4 + 3, v5, v6, cb);
        b.place_block(r, v7, v4 + 4, v5 + 1, v6, cb);
        b.place_block(r, v8, v4 + 4, v5 + 2, v6, cb);
        b.place_block(r, v9, v4 - 3, v5, v6, cb);
        b.place_block(r, v7, v4 - 4, v5 + 1, v6, cb);
        b.place_block(r, v8, v4 - 4, v5 + 2, v6, cb);
        b.place_block(r, v9, v4, v5, v6 + 3, cb);
        b.place_block(r, v9, v4, v5, v6 - 3, cb);
        b.place_block(r, v7, v4, v5 + 1, v6 - 4, cb);
        b.place_block(r, v8, v4, -2, v6 - 4, cb);
    }
}

impl Piece for PyramidPiece {
    piece_impl!();

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn bbox_now(&self) -> BoundingBox {
        self.s.current().bbox
    }

    fn save_extra(&self, _tag: &mut Vec<(String, Tag)>) {}

    fn save(&self) -> Tag {
        self.s.save(flags_tag(&["hasPlacedChest0", "hasPlacedChest1", "hasPlacedChest2", "hasPlacedChest3"], &self.chests))
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        let offset = -random.next_int_bounded(3);
        if !self.s.update_to_lowest_ground_height(r, offset) {
            return;
        }
        let b = &self.s.current();
        let (w, d) = (self.s.width, self.s.depth);
        b.generate_box(r, cb, 0, -4, 0, w - 1, 0, d - 1, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        for v8 in 1..=9 {
            b.generate_box(r, cb, v8, v8, v8, (w - 1) - v8, v8, (d - 1) - v8, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
            b.generate_box(r, cb, v8 + 1, v8, v8 + 1, (w - 2) - v8, v8, (d - 2) - v8, st("minecraft:air"), st("minecraft:air"), false);
        }
        for v8 in 0..w {
            for v9 in 0..d {
                b.fill_column_down(r, st("minecraft:sandstone"), v8, -5, v9, cb);
            }
        }
        let v8 = with(st("minecraft:sandstone_stairs"), &[("facing", "north")]);
        let v9 = with(st("minecraft:sandstone_stairs"), &[("facing", "south")]);
        let v10 = with(st("minecraft:sandstone_stairs"), &[("facing", "east")]);
        let v11 = with(st("minecraft:sandstone_stairs"), &[("facing", "west")]);
        b.generate_box(r, cb, 0, 0, 0, 4, 9, 4, st("minecraft:sandstone"), st("minecraft:air"), false);
        b.generate_box(r, cb, 1, 10, 1, 3, 10, 3, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.place_block(r, v8, 2, 10, 0, cb);
        b.place_block(r, v9, 2, 10, 4, cb);
        b.place_block(r, v10, 0, 10, 2, cb);
        b.place_block(r, v11, 4, 10, 2, cb);
        b.generate_box(r, cb, w - 5, 0, 0, w - 1, 9, 4, st("minecraft:sandstone"), st("minecraft:air"), false);
        b.generate_box(r, cb, w - 4, 10, 1, w - 2, 10, 3, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.place_block(r, v8, w - 3, 10, 0, cb);
        b.place_block(r, v9, w - 3, 10, 4, cb);
        b.place_block(r, v10, w - 5, 10, 2, cb);
        b.place_block(r, v11, w - 1, 10, 2, cb);
        b.generate_box(r, cb, 8, 0, 0, 12, 4, 4, st("minecraft:sandstone"), st("minecraft:air"), false);
        b.generate_box(r, cb, 9, 1, 0, 11, 3, 4, st("minecraft:air"), st("minecraft:air"), false);
        b.place_block(r, st("minecraft:cut_sandstone"), 9, 1, 1, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 9, 2, 1, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 9, 3, 1, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 10, 3, 1, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 11, 3, 1, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 11, 2, 1, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 11, 1, 1, cb);
        b.generate_box(r, cb, 4, 1, 1, 8, 3, 3, st("minecraft:sandstone"), st("minecraft:air"), false);
        b.generate_box(r, cb, 4, 1, 2, 8, 2, 2, st("minecraft:air"), st("minecraft:air"), false);
        b.generate_box(r, cb, 12, 1, 1, 16, 3, 3, st("minecraft:sandstone"), st("minecraft:air"), false);
        b.generate_box(r, cb, 12, 1, 2, 16, 2, 2, st("minecraft:air"), st("minecraft:air"), false);
        b.generate_box(r, cb, 5, 4, 5, w - 6, 4, d - 6, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, 9, 4, 9, 11, 4, 11, st("minecraft:air"), st("minecraft:air"), false);
        b.generate_box(r, cb, 8, 1, 8, 8, 3, 8, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, 12, 1, 8, 12, 3, 8, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, 8, 1, 12, 8, 3, 12, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, 12, 1, 12, 12, 3, 12, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, 1, 1, 5, 4, 4, 11, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, w - 5, 1, 5, w - 2, 4, 11, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, 6, 7, 9, 6, 7, 11, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, w - 7, 7, 9, w - 7, 7, 11, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, 5, 5, 9, 5, 7, 11, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, w - 6, 5, 9, w - 6, 7, 11, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.place_block(r, st("minecraft:air"), 5, 5, 10, cb);
        b.place_block(r, st("minecraft:air"), 5, 6, 10, cb);
        b.place_block(r, st("minecraft:air"), 6, 6, 10, cb);
        b.place_block(r, st("minecraft:air"), w - 6, 5, 10, cb);
        b.place_block(r, st("minecraft:air"), w - 6, 6, 10, cb);
        b.place_block(r, st("minecraft:air"), w - 7, 6, 10, cb);
        b.generate_box(r, cb, 2, 4, 4, 2, 6, 4, st("minecraft:air"), st("minecraft:air"), false);
        b.generate_box(r, cb, w - 3, 4, 4, w - 3, 6, 4, st("minecraft:air"), st("minecraft:air"), false);
        b.place_block(r, v8, 2, 4, 5, cb);
        b.place_block(r, v8, 2, 3, 4, cb);
        b.place_block(r, v8, w - 3, 4, 5, cb);
        b.place_block(r, v8, w - 3, 3, 4, cb);
        b.generate_box(r, cb, 1, 1, 3, 2, 2, 3, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, w - 3, 1, 3, w - 2, 2, 3, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.place_block(r, st("minecraft:sandstone"), 1, 1, 2, cb);
        b.place_block(r, st("minecraft:sandstone"), w - 2, 1, 2, cb);
        b.place_block(r, st("minecraft:sandstone_slab"), 1, 2, 2, cb);
        b.place_block(r, st("minecraft:sandstone_slab"), w - 2, 2, 2, cb);
        b.place_block(r, v11, 2, 1, 2, cb);
        b.place_block(r, v10, w - 3, 1, 2, cb);
        b.generate_box(r, cb, 4, 3, 5, 4, 3, 17, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, w - 5, 3, 5, w - 5, 3, 17, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, 3, 1, 5, 4, 2, 16, st("minecraft:air"), st("minecraft:air"), false);
        b.generate_box(r, cb, w - 6, 1, 5, w - 5, 2, 16, st("minecraft:air"), st("minecraft:air"), false);
        for v12 in (5..=17).step_by(2) {
            b.place_block(r, st("minecraft:cut_sandstone"), 4, 1, v12, cb);
            b.place_block(r, st("minecraft:chiseled_sandstone"), 4, 2, v12, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), w - 5, 1, v12, cb);
            b.place_block(r, st("minecraft:chiseled_sandstone"), w - 5, 2, v12, cb);
        }
        b.place_block(r, st("minecraft:orange_terracotta"), 10, 0, 7, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 10, 0, 8, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 9, 0, 9, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 11, 0, 9, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 8, 0, 10, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 12, 0, 10, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 7, 0, 10, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 13, 0, 10, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 9, 0, 11, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 11, 0, 11, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 10, 0, 12, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 10, 0, 13, cb);
        b.place_block(r, st("minecraft:blue_terracotta"), 10, 0, 10, cb);
        for v12 in (0..=w - 1).step_by((w - 1) as usize) {
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 2, 1, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 2, 2, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 2, 3, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 3, 1, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 3, 2, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 3, 3, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 4, 1, cb);
            b.place_block(r, st("minecraft:chiseled_sandstone"), v12, 4, 2, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 4, 3, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 5, 1, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 5, 2, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 5, 3, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 6, 1, cb);
            b.place_block(r, st("minecraft:chiseled_sandstone"), v12, 6, 2, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 6, 3, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 7, 1, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 7, 2, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 7, 3, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 8, 1, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 8, 2, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 8, 3, cb);
        }
        for v12 in (2..=w - 3).step_by((w - 5) as usize) {
            b.place_block(r, st("minecraft:cut_sandstone"), v12 - 1, 2, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 2, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 + 1, 2, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 - 1, 3, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 3, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 + 1, 3, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12 - 1, 4, 0, cb);
            b.place_block(r, st("minecraft:chiseled_sandstone"), v12, 4, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12 + 1, 4, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 - 1, 5, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 5, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 + 1, 5, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12 - 1, 6, 0, cb);
            b.place_block(r, st("minecraft:chiseled_sandstone"), v12, 6, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12 + 1, 6, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12 - 1, 7, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12, 7, 0, cb);
            b.place_block(r, st("minecraft:orange_terracotta"), v12 + 1, 7, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 - 1, 8, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12, 8, 0, cb);
            b.place_block(r, st("minecraft:cut_sandstone"), v12 + 1, 8, 0, cb);
        }
        b.generate_box(r, cb, 8, 4, 0, 12, 6, 0, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.place_block(r, st("minecraft:air"), 8, 6, 0, cb);
        b.place_block(r, st("minecraft:air"), 12, 6, 0, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 9, 5, 0, cb);
        b.place_block(r, st("minecraft:chiseled_sandstone"), 10, 5, 0, cb);
        b.place_block(r, st("minecraft:orange_terracotta"), 11, 5, 0, cb);
        b.generate_box(r, cb, 8, -14, 8, 12, -11, 12, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, 8, -10, 8, 12, -10, 12, st("minecraft:chiseled_sandstone"), st("minecraft:chiseled_sandstone"), false);
        b.generate_box(r, cb, 8, -9, 8, 12, -9, 12, st("minecraft:cut_sandstone"), st("minecraft:cut_sandstone"), false);
        b.generate_box(r, cb, 8, -8, 8, 12, -1, 12, st("minecraft:sandstone"), st("minecraft:sandstone"), false);
        b.generate_box(r, cb, 9, -11, 9, 11, -1, 11, st("minecraft:air"), st("minecraft:air"), false);
        b.place_block(r, st("minecraft:stone_pressure_plate"), 10, -11, 10, cb);
        b.generate_box(r, cb, 9, -13, 9, 11, -13, 11, st("minecraft:tnt"), st("minecraft:air"), false);
        b.place_block(r, st("minecraft:air"), 8, -11, 10, cb);
        b.place_block(r, st("minecraft:air"), 8, -10, 10, cb);
        b.place_block(r, st("minecraft:chiseled_sandstone"), 7, -10, 10, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 7, -11, 10, cb);
        b.place_block(r, st("minecraft:air"), 12, -11, 10, cb);
        b.place_block(r, st("minecraft:air"), 12, -10, 10, cb);
        b.place_block(r, st("minecraft:chiseled_sandstone"), 13, -10, 10, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 13, -11, 10, cb);
        b.place_block(r, st("minecraft:air"), 10, -11, 8, cb);
        b.place_block(r, st("minecraft:air"), 10, -10, 8, cb);
        b.place_block(r, st("minecraft:chiseled_sandstone"), 10, -10, 7, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 10, -11, 7, cb);
        b.place_block(r, st("minecraft:air"), 10, -11, 12, cb);
        b.place_block(r, st("minecraft:air"), 10, -10, 12, cb);
        b.place_block(r, st("minecraft:chiseled_sandstone"), 10, -10, 13, cb);
        b.place_block(r, st("minecraft:cut_sandstone"), 10, -11, 13, cb);
        for dir in Dir::HORIZONTAL {
            let i = dir.index_2d() as usize;
            if !self.chests[i].load(Ordering::Relaxed) {
                let (dx, _, dz) = dir.offset();
                let placed = b.create_chest(r, cb, random, 10 + dx * 2, -11, 10 + dz * 2, "minecraft:chests/desert_pyramid");
                self.chests[i].store(placed, Ordering::Relaxed);
            }
        }
        self.cellar(b, r, cb);
    }
}

// ---- Jungle temple ----

pub struct JungleTemple;

impl Kind for JungleTemple {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        single_piece(ctx, 12, 15, |random, x, z| {
            Box::new(TemplePiece { s: Scattered::new("minecraft:tejp", random, x, z, 12, 10, 15), flags: Default::default() })
        })
    }
}

/// Flags: main chest, hidden chest, trap 1, trap 2.
#[derive(Debug)]
struct TemplePiece {
    s: Scattered,
    flags: [AtomicBool; 4],
}

/// `JungleTemplePiece.MossStoneSelector`.
fn moss_stone(random: &mut WorldgenRandom, _x: i32, _y: i32, _z: i32, _edge: bool) -> u16 {
    if random.next_float() < 0.4 { st("minecraft:cobblestone") } else { st("minecraft:mossy_cobblestone") }
}

impl Piece for TemplePiece {
    piece_impl!();

    fn bbox_now(&self) -> BoundingBox {
        self.s.current().bbox
    }

    fn save_extra(&self, _tag: &mut Vec<(String, Tag)>) {}

    fn save(&self) -> Tag {
        self.s.save(flags_tag(&["placedMainChest", "placedHiddenChest", "placedTrap1", "placedTrap2"], &self.flags))
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        if !self.s.update_average_ground_height(r, cb, 0) {
            return;
        }
        let b = &self.s.current();
        let (w, d) = (self.s.width, self.s.depth);
        let mut stone_selector = moss_stone;
        b.generate_box_with(r, cb, 0, -4, 0, w - 1, 0, d - 1, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 2, 1, 2, 9, 2, 2, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 2, 1, 12, 9, 2, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 2, 1, 3, 2, 2, 11, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 9, 1, 3, 9, 2, 11, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 1, 3, 1, 10, 6, 1, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 1, 3, 13, 10, 6, 13, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 1, 3, 2, 1, 6, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 10, 3, 2, 10, 6, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 2, 3, 2, 9, 3, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 2, 6, 2, 9, 6, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 3, 7, 3, 8, 7, 11, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 4, 8, 4, 7, 8, 10, false, random, &mut stone_selector);
        b.generate_air_box(r, cb, 3, 1, 3, 8, 2, 11);
        b.generate_air_box(r, cb, 4, 3, 6, 7, 3, 9);
        b.generate_air_box(r, cb, 2, 4, 2, 9, 5, 12);
        b.generate_air_box(r, cb, 4, 6, 5, 7, 6, 9);
        b.generate_air_box(r, cb, 5, 7, 6, 6, 7, 8);
        b.generate_air_box(r, cb, 5, 1, 2, 6, 2, 2);
        b.generate_air_box(r, cb, 5, 2, 12, 6, 2, 12);
        b.generate_air_box(r, cb, 5, 5, 1, 6, 5, 1);
        b.generate_air_box(r, cb, 5, 5, 13, 6, 5, 13);
        b.place_block(r, st("minecraft:air"), 1, 5, 5, cb);
        b.place_block(r, st("minecraft:air"), 10, 5, 5, cb);
        b.place_block(r, st("minecraft:air"), 1, 5, 9, cb);
        b.place_block(r, st("minecraft:air"), 10, 5, 9, cb);
        for v8 in (0..=14).step_by(14) {
            b.generate_box_with(r, cb, 2, 4, v8, 2, 5, v8, false, random, &mut stone_selector);
            b.generate_box_with(r, cb, 4, 4, v8, 4, 5, v8, false, random, &mut stone_selector);
            b.generate_box_with(r, cb, 7, 4, v8, 7, 5, v8, false, random, &mut stone_selector);
            b.generate_box_with(r, cb, 9, 4, v8, 9, 5, v8, false, random, &mut stone_selector);
        }
        b.generate_box_with(r, cb, 5, 6, 0, 6, 6, 0, false, random, &mut stone_selector);
        for v8 in (0..=11).step_by(11) {
            for v9 in (2..=12).step_by(2) {
                b.generate_box_with(r, cb, v8, 4, v9, v8, 5, v9, false, random, &mut stone_selector);
            }
            b.generate_box_with(r, cb, v8, 6, 5, v8, 6, 5, false, random, &mut stone_selector);
            b.generate_box_with(r, cb, v8, 6, 9, v8, 6, 9, false, random, &mut stone_selector);
        }
        b.generate_box_with(r, cb, 2, 7, 2, 2, 9, 2, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 9, 7, 2, 9, 9, 2, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 2, 7, 12, 2, 9, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 9, 7, 12, 9, 9, 12, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 4, 9, 4, 4, 9, 4, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 7, 9, 4, 7, 9, 4, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 4, 9, 10, 4, 9, 10, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 7, 9, 10, 7, 9, 10, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 5, 9, 7, 6, 9, 7, false, random, &mut stone_selector);
        let v8 = with(st("minecraft:cobblestone_stairs"), &[("facing", "east")]);
        let v9 = with(st("minecraft:cobblestone_stairs"), &[("facing", "west")]);
        let v10 = with(st("minecraft:cobblestone_stairs"), &[("facing", "south")]);
        let v11 = with(st("minecraft:cobblestone_stairs"), &[("facing", "north")]);
        b.place_block(r, v11, 5, 9, 6, cb);
        b.place_block(r, v11, 6, 9, 6, cb);
        b.place_block(r, v10, 5, 9, 8, cb);
        b.place_block(r, v10, 6, 9, 8, cb);
        b.place_block(r, v11, 4, 0, 0, cb);
        b.place_block(r, v11, 5, 0, 0, cb);
        b.place_block(r, v11, 6, 0, 0, cb);
        b.place_block(r, v11, 7, 0, 0, cb);
        b.place_block(r, v11, 4, 1, 8, cb);
        b.place_block(r, v11, 4, 2, 9, cb);
        b.place_block(r, v11, 4, 3, 10, cb);
        b.place_block(r, v11, 7, 1, 8, cb);
        b.place_block(r, v11, 7, 2, 9, cb);
        b.place_block(r, v11, 7, 3, 10, cb);
        b.generate_box_with(r, cb, 4, 1, 9, 4, 1, 9, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 7, 1, 9, 7, 1, 9, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 4, 1, 10, 7, 2, 10, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 5, 4, 5, 6, 4, 5, false, random, &mut stone_selector);
        b.place_block(r, v8, 4, 4, 5, cb);
        b.place_block(r, v9, 7, 4, 5, cb);
        for v12 in 0..4 {
            b.place_block(r, v10, 5, 0 - v12, 6 + v12, cb);
            b.place_block(r, v10, 6, 0 - v12, 6 + v12, cb);
            b.generate_air_box(r, cb, 5, 0 - v12, 7 + v12, 6, 0 - v12, 9 + v12);
        }
        b.generate_air_box(r, cb, 1, -3, 12, 10, -1, 13);
        b.generate_air_box(r, cb, 1, -3, 1, 3, -1, 13);
        b.generate_air_box(r, cb, 1, -3, 1, 9, -1, 5);
        for v12 in (1..=13).step_by(2) {
            b.generate_box_with(r, cb, 1, -3, v12, 1, -2, v12, false, random, &mut stone_selector);
        }
        for v12 in (2..=12).step_by(2) {
            b.generate_box_with(r, cb, 1, -1, v12, 3, -1, v12, false, random, &mut stone_selector);
        }
        b.generate_box_with(r, cb, 2, -2, 1, 5, -2, 1, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 7, -2, 1, 9, -2, 1, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 6, -3, 1, 6, -3, 1, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 6, -1, 1, 6, -1, 1, false, random, &mut stone_selector);
        b.place_block(r, with(st("minecraft:tripwire_hook"), &[("facing", "east"), ("attached", "true")]), 1, -3, 8, cb);
        b.place_block(r, with(st("minecraft:tripwire_hook"), &[("facing", "west"), ("attached", "true")]), 4, -3, 8, cb);
        b.place_block(r, with(st("minecraft:tripwire"), &[("east", "true"), ("west", "true"), ("attached", "true")]), 2, -3, 8, cb);
        b.place_block(r, with(st("minecraft:tripwire"), &[("east", "true"), ("west", "true"), ("attached", "true")]), 3, -3, 8, cb);
        let v12 = with(st("minecraft:redstone_wire"), &[("north", "side"), ("south", "side")]);
        b.place_block(r, v12, 5, -3, 7, cb);
        b.place_block(r, v12, 5, -3, 6, cb);
        b.place_block(r, v12, 5, -3, 5, cb);
        b.place_block(r, v12, 5, -3, 4, cb);
        b.place_block(r, v12, 5, -3, 3, cb);
        b.place_block(r, v12, 5, -3, 2, cb);
        b.place_block(r, with(st("minecraft:redstone_wire"), &[("north", "side"), ("west", "side")]), 5, -3, 1, cb);
        b.place_block(r, with(st("minecraft:redstone_wire"), &[("east", "side"), ("west", "side")]), 4, -3, 1, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 3, -3, 1, cb);
        if !self.flags[2].load(Ordering::Relaxed) {
            let placed = b.create_dispenser(r, cb, random, 3, -2, 1, Dir::North, "minecraft:dispensers/jungle_temple");
            self.flags[2].store(placed, Ordering::Relaxed);
        }
        b.place_block(r, with(st("minecraft:vine"), &[("south", "true")]), 3, -2, 2, cb);
        b.place_block(r, with(st("minecraft:tripwire_hook"), &[("facing", "north"), ("attached", "true")]), 7, -3, 1, cb);
        b.place_block(r, with(st("minecraft:tripwire_hook"), &[("facing", "south"), ("attached", "true")]), 7, -3, 5, cb);
        b.place_block(r, with(st("minecraft:tripwire"), &[("north", "true"), ("south", "true"), ("attached", "true")]), 7, -3, 2, cb);
        b.place_block(r, with(st("minecraft:tripwire"), &[("north", "true"), ("south", "true"), ("attached", "true")]), 7, -3, 3, cb);
        b.place_block(r, with(st("minecraft:tripwire"), &[("north", "true"), ("south", "true"), ("attached", "true")]), 7, -3, 4, cb);
        b.place_block(r, with(st("minecraft:redstone_wire"), &[("east", "side"), ("west", "side")]), 8, -3, 6, cb);
        b.place_block(r, with(st("minecraft:redstone_wire"), &[("west", "side"), ("south", "side")]), 9, -3, 6, cb);
        b.place_block(r, with(st("minecraft:redstone_wire"), &[("north", "side"), ("south", "up")]), 9, -3, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 9, -3, 4, cb);
        b.place_block(r, v12, 9, -2, 4, cb);
        if !self.flags[3].load(Ordering::Relaxed) {
            let placed = b.create_dispenser(r, cb, random, 9, -2, 3, Dir::West, "minecraft:dispensers/jungle_temple");
            self.flags[3].store(placed, Ordering::Relaxed);
        }
        b.place_block(r, with(st("minecraft:vine"), &[("east", "true")]), 8, -1, 3, cb);
        b.place_block(r, with(st("minecraft:vine"), &[("east", "true")]), 8, -2, 3, cb);
        if !self.flags[0].load(Ordering::Relaxed) {
            let placed = b.create_chest(r, cb, random, 8, -3, 3, "minecraft:chests/jungle_temple");
            self.flags[0].store(placed, Ordering::Relaxed);
        }
        b.place_block(r, st("minecraft:mossy_cobblestone"), 9, -3, 2, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 8, -3, 1, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 4, -3, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 5, -2, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 5, -1, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 6, -3, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 7, -2, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 7, -1, 5, cb);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 8, -3, 5, cb);
        b.generate_box_with(r, cb, 9, -1, 1, 9, -1, 5, false, random, &mut stone_selector);
        b.generate_air_box(r, cb, 8, -3, 8, 10, -1, 10);
        b.place_block(r, st("minecraft:chiseled_stone_bricks"), 8, -2, 11, cb);
        b.place_block(r, st("minecraft:chiseled_stone_bricks"), 9, -2, 11, cb);
        b.place_block(r, st("minecraft:chiseled_stone_bricks"), 10, -2, 11, cb);
        let v13 = with(st("minecraft:lever"), &[("facing", "north"), ("face", "wall")]);
        b.place_block(r, v13, 8, -2, 12, cb);
        b.place_block(r, v13, 9, -2, 12, cb);
        b.place_block(r, v13, 10, -2, 12, cb);
        b.generate_box_with(r, cb, 8, -3, 8, 8, -3, 10, false, random, &mut stone_selector);
        b.generate_box_with(r, cb, 10, -3, 8, 10, -3, 10, false, random, &mut stone_selector);
        b.place_block(r, st("minecraft:mossy_cobblestone"), 10, -2, 9, cb);
        b.place_block(r, v12, 8, -2, 9, cb);
        b.place_block(r, v12, 8, -2, 10, cb);
        b.place_block(r, with(st("minecraft:redstone_wire"), &[("north", "side"), ("south", "side"), ("east", "side"), ("west", "side")]), 10, -1, 9, cb);
        b.place_block(r, with(st("minecraft:sticky_piston"), &[("facing", "up")]), 9, -2, 8, cb);
        b.place_block(r, with(st("minecraft:sticky_piston"), &[("facing", "west")]), 10, -2, 8, cb);
        b.place_block(r, with(st("minecraft:sticky_piston"), &[("facing", "west")]), 10, -1, 8, cb);
        b.place_block(r, with(st("minecraft:repeater"), &[("facing", "north")]), 10, -2, 10, cb);
        if !self.flags[1].load(Ordering::Relaxed) {
            let placed = b.create_chest(r, cb, random, 9, -3, 10, "minecraft:chests/jungle_temple");
            self.flags[1].store(placed, Ordering::Relaxed);
        }
    }
}

// ---- Swamp hut ----

pub struct SwampHut;

impl Kind for SwampHut {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let pos = ctx.on_top_of_chunk_center(Heightmap::WorldSurfaceWg)?;
        let (x, z) = (ctx.chunk.0 << 4, ctx.chunk.1 << 4);
        Some(Stub {
            pos,
            build: Box::new(move |ctx, pieces| {
                pieces.push(Box::new(HutPiece { s: Scattered::new("minecraft:tesh", &mut ctx.random, x, z, 7, 7, 9), mobs: Default::default() }))
            }),
        })
    }
}

/// Flags: witch spawned, cat spawned.
#[derive(Debug)]
struct HutPiece {
    s: Scattered,
    mobs: [AtomicBool; 2],
}

impl Piece for HutPiece {
    piece_impl!();

    fn bbox_now(&self) -> BoundingBox {
        self.s.current().bbox
    }

    fn save_extra(&self, _tag: &mut Vec<(String, Tag)>) {}

    fn save(&self) -> Tag {
        self.s.save(flags_tag(&["Witch", "Cat"], &self.mobs))
    }

    /// The hut; Kiln does not spawn its witch and cat, only records that they would have been.
    fn place(&self, _cx: &PlaceContext, r: &mut Region, _random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        if !self.s.update_average_ground_height(r, cb, 0) {
            return;
        }
        let b = &self.s.current();
        b.generate_box(r, cb, 1, 1, 1, 5, 1, 7, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 1, 4, 2, 5, 4, 7, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 2, 1, 0, 4, 1, 0, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 2, 2, 2, 3, 3, 2, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 1, 2, 3, 1, 3, 6, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 5, 2, 3, 5, 3, 6, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 2, 2, 7, 4, 3, 7, st("minecraft:spruce_planks"), st("minecraft:spruce_planks"), false);
        b.generate_box(r, cb, 1, 0, 2, 1, 3, 2, st("minecraft:oak_log"), st("minecraft:oak_log"), false);
        b.generate_box(r, cb, 5, 0, 2, 5, 3, 2, st("minecraft:oak_log"), st("minecraft:oak_log"), false);
        b.generate_box(r, cb, 1, 0, 7, 1, 3, 7, st("minecraft:oak_log"), st("minecraft:oak_log"), false);
        b.generate_box(r, cb, 5, 0, 7, 5, 3, 7, st("minecraft:oak_log"), st("minecraft:oak_log"), false);
        b.place_block(r, st("minecraft:oak_fence"), 2, 3, 2, cb);
        b.place_block(r, st("minecraft:oak_fence"), 3, 3, 7, cb);
        b.place_block(r, st("minecraft:air"), 1, 3, 4, cb);
        b.place_block(r, st("minecraft:air"), 5, 3, 4, cb);
        b.place_block(r, st("minecraft:air"), 5, 3, 5, cb);
        b.place_block(r, st("minecraft:potted_red_mushroom"), 1, 3, 5, cb);
        b.place_block(r, st("minecraft:crafting_table"), 3, 2, 6, cb);
        b.place_block(r, st("minecraft:cauldron"), 4, 2, 6, cb);
        b.place_block(r, st("minecraft:oak_fence"), 1, 2, 1, cb);
        b.place_block(r, st("minecraft:oak_fence"), 5, 2, 1, cb);
        let v8 = with(st("minecraft:spruce_stairs"), &[("facing", "north")]);
        let v9 = with(st("minecraft:spruce_stairs"), &[("facing", "east")]);
        let v10 = with(st("minecraft:spruce_stairs"), &[("facing", "west")]);
        let v11 = with(st("minecraft:spruce_stairs"), &[("facing", "south")]);
        b.generate_box(r, cb, 0, 4, 1, 6, 4, 1, v8, v8, false);
        b.generate_box(r, cb, 0, 4, 2, 0, 4, 7, v9, v9, false);
        b.generate_box(r, cb, 6, 4, 2, 6, 4, 7, v10, v10, false);
        b.generate_box(r, cb, 0, 4, 8, 6, 4, 8, v11, v11, false);
        b.place_block(r, with(v8, &[("shape", "outer_right")]), 0, 4, 1, cb);
        b.place_block(r, with(v8, &[("shape", "outer_left")]), 6, 4, 1, cb);
        b.place_block(r, with(v11, &[("shape", "outer_left")]), 0, 4, 8, cb);
        b.place_block(r, with(v11, &[("shape", "outer_right")]), 6, 4, 8, cb);
        for v12 in (2..=7).step_by(5) {
            for v13 in (1..=5).step_by(4) {
                b.fill_column_down(r, st("minecraft:oak_log"), v13, -1, v12, cb);
            }
        }
        if cb.is_inside(b.world_pos(2, 2, 5)) {
            for f in &self.mobs {
                f.store(true, Ordering::Relaxed);
            }
        }
    }
}
