//! Strongholds (`StrongholdStructure`, `StrongholdPieces`).

use super::{bool_tag, move_below_sea_level, set_spawner_entity, st, with};
use crate::block_facts::Dir;
use crate::blocks::state;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::sync::atomic::{AtomicBool, Ordering};

const CAVE_AIR: u16 = state::CAVE_AIR;

/// `StrongholdStructure`.
pub struct Stronghold;

impl Kind for Stronghold {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let (cx, cz) = ctx.chunk;
        Some(Stub { pos: BlockPos::new(cx << 4, 0, cz << 4), build: Box::new(move |ctx, out| *out = generate(ctx)) })
    }
}

/// `StrongholdStructure.generatePieces`: retries with the next seed until a portal room exists.
fn generate(ctx: &mut GenCtx) -> Vec<Box<dyn Piece>> {
    let (cx, cz) = ctx.chunk;
    let mut attempt: i64 = 0;
    loop {
        ctx.random.set_large_feature_seed(ctx.seed.wrapping_add(attempt), cx, cz);
        attempt += 1;
        let mut g = Gen { list: Vec::new(), weights: WEIGHTS.iter().map(|w| (w.0, w.1, 0, w.2)).collect(), imposed: None, previous: None, pending: Vec::new(), portal: false };
        let dir = PieceBase::random_horizontal(&mut ctx.random);
        let mut base = PieceBase::new("minecraft:shstart", 0, PieceBase::make_bbox((cx << 4) + 2, 64, (cz << 4) + 2, dir, 5, 11, 5));
        base.set_orientation(Some(dir));
        g.list.push(ShPiece { base, door: Door::Opening, part: Part::StairsDown { source: true } });
        g.add_children(0, &mut ctx.random);
        while !g.pending.is_empty() {
            let i = ctx.random.next_int_bounded(g.pending.len() as i32) as usize;
            let p = g.pending.remove(i);
            g.add_children(p, &mut ctx.random);
        }
        let mut pieces: Vec<Box<dyn Piece>> = g.list.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect();
        let (sea, min_y) = (ctx.generator.sea_level, ctx.min_y());
        move_below_sea_level(&mut pieces, sea, min_y, &mut ctx.random, 10);
        if !pieces.is_empty() && g.portal {
            return pieces;
        }
    }
}

/// Piece types in `STRONGHOLD_PIECE_WEIGHTS` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PieceKind {
    Straight,
    PrisonHall,
    LeftTurn,
    RightTurn,
    RoomCrossing,
    StraightStairsDown,
    StairsDown,
    FiveCrossing,
    ChestCorridor,
    Library,
    PortalRoom,
}

/// (type, weight, max count).
const WEIGHTS: [(PieceKind, i32, i32); 11] = [
    (PieceKind::Straight, 40, 0),
    (PieceKind::PrisonHall, 5, 5),
    (PieceKind::LeftTurn, 20, 0),
    (PieceKind::RightTurn, 20, 0),
    (PieceKind::RoomCrossing, 10, 6),
    (PieceKind::StraightStairsDown, 5, 5),
    (PieceKind::StairsDown, 5, 5),
    (PieceKind::FiveCrossing, 5, 4),
    (PieceKind::ChestCorridor, 5, 4),
    (PieceKind::Library, 10, 2),
    (PieceKind::PortalRoom, 20, 1),
];

/// `StrongholdPiece.SmallDoorType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Door {
    Opening,
    Wood,
    Grates,
    Iron,
}

impl Door {
    fn random(random: &mut WorldgenRandom) -> Door {
        match random.next_int_bounded(5) {
            2 => Door::Wood,
            3 => Door::Grates,
            4 => Door::Iron,
            _ => Door::Opening,
        }
    }

    fn name(self) -> &'static str {
        ["OPENING", "WOOD_DOOR", "GRATES", "IRON_DOOR"][self as usize]
    }
}

#[derive(Debug)]
enum Part {
    StairsDown { source: bool },
    Straight { left: bool, right: bool },
    ChestCorridor { chest: AtomicBool },
    Filler { steps: i32 },
    FiveCrossing { left_low: bool, left_high: bool, right_low: bool, right_high: bool },
    LeftTurn,
    RightTurn,
    Library { tall: bool },
    PortalRoom { spawner: AtomicBool },
    PrisonHall,
    RoomCrossing { ty: i32 },
    StraightStairsDown,
}

/// `StrongholdPieces.StrongholdPiece` and its subclasses.
#[derive(Debug)]
struct ShPiece {
    base: PieceBase,
    door: Door,
    part: Part,
}

/// Piece generation state (`StrongholdPieces`' statics and the start piece's fields).
struct Gen {
    list: Vec<ShPiece>,
    /// (type, weight, placed, max).
    weights: Vec<(PieceKind, i32, i32, i32)>,
    imposed: Option<PieceKind>,
    previous: Option<PieceKind>,
    pending: Vec<usize>,
    portal: bool,
}

fn is_ok_box(b: &BoundingBox) -> bool {
    b.min_y > 10
}

impl Gen {
    fn collides(&self, b: &BoundingBox) -> Option<usize> {
        self.list.iter().position(|p| p.base.bbox.intersects(b))
    }

    /// `PieceWeight.doPlace` with the library and portal room depth limits.
    fn do_place(w: &(PieceKind, i32, i32, i32), depth: i32) -> bool {
        let base = w.3 == 0 || w.2 < w.3;
        match w.0 {
            PieceKind::Library => base && depth > 4,
            PieceKind::PortalRoom => base && depth > 5,
            _ => base,
        }
    }

    /// `findAndCreatePieceFactory`.
    #[allow(clippy::too_many_arguments)]
    fn create(&self, kind: PieceKind, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32) -> Option<ShPiece> {
        let boxed = |ox: i32, oy: i32, w: i32, h: i32, d: i32| {
            let b = BoundingBox::orient(x, y, z, ox, oy, 0, w, h, d, dir);
            (is_ok_box(&b) && self.collides(&b).is_none()).then_some(b)
        };
        let piece = |kind: &'static str, b: BoundingBox, door: Door, part: Part| {
            let mut base = PieceBase::new(kind, depth, b);
            base.set_orientation(Some(dir));
            ShPiece { base, door, part }
        };
        Some(match kind {
            PieceKind::Straight => {
                let b = boxed(-1, -1, 5, 5, 7)?;
                let door = Door::random(random);
                let left = random.next_int_bounded(2) == 0;
                let right = random.next_int_bounded(2) == 0;
                piece("minecraft:shs", b, door, Part::Straight { left, right })
            }
            PieceKind::PrisonHall => {
                let b = boxed(-1, -1, 9, 5, 11)?;
                piece("minecraft:shph", b, Door::random(random), Part::PrisonHall)
            }
            PieceKind::LeftTurn => {
                let b = boxed(-1, -1, 5, 5, 5)?;
                piece("minecraft:shlt", b, Door::random(random), Part::LeftTurn)
            }
            PieceKind::RightTurn => {
                let b = boxed(-1, -1, 5, 5, 5)?;
                piece("minecraft:shrt", b, Door::random(random), Part::RightTurn)
            }
            PieceKind::RoomCrossing => {
                let b = boxed(-4, -1, 11, 7, 11)?;
                let door = Door::random(random);
                let ty = random.next_int_bounded(5);
                piece("minecraft:shrc", b, door, Part::RoomCrossing { ty })
            }
            PieceKind::StraightStairsDown => {
                let b = boxed(-1, -7, 5, 11, 8)?;
                piece("minecraft:shssd", b, Door::random(random), Part::StraightStairsDown)
            }
            PieceKind::StairsDown => {
                let b = boxed(-1, -7, 5, 11, 5)?;
                piece("minecraft:shsd", b, Door::random(random), Part::StairsDown { source: false })
            }
            PieceKind::FiveCrossing => {
                let b = boxed(-4, -3, 10, 9, 11)?;
                let door = Door::random(random);
                let left_low = random.next_bool();
                let left_high = random.next_bool();
                let right_low = random.next_bool();
                let right_high = random.next_int_bounded(3) > 0;
                piece("minecraft:sh5c", b, door, Part::FiveCrossing { left_low, left_high, right_low, right_high })
            }
            PieceKind::ChestCorridor => {
                let b = boxed(-1, -1, 5, 5, 7)?;
                piece("minecraft:shcc", b, Door::random(random), Part::ChestCorridor { chest: AtomicBool::new(false) })
            }
            PieceKind::Library => {
                let b = boxed(-4, -1, 14, 11, 15).or_else(|| boxed(-4, -1, 14, 6, 15))?;
                let door = Door::random(random);
                piece("minecraft:shli", b, door, Part::Library { tall: b.y_span() > 6 })
            }
            PieceKind::PortalRoom => {
                let b = boxed(-4, -1, 11, 8, 16)?;
                piece("minecraft:shpr", b, Door::Opening, Part::PortalRoom { spawner: AtomicBool::new(false) })
            }
        })
    }

    /// `generatePieceFromSmallDoor`.
    #[allow(clippy::too_many_arguments)]
    fn piece_at_small_door(&mut self, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32) -> Option<ShPiece> {
        let mut any = false;
        for w in &self.weights {
            if w.3 > 0 && w.2 < w.3 {
                any = true;
            }
        }
        let total: i32 = self.weights.iter().map(|w| w.1).sum();
        if !any {
            return None;
        }
        if let Some(k) = self.imposed.take()
            && let Some(p) = self.create(k, random, x, y, z, dir, depth)
        {
            return Some(p);
        }
        for _ in 0..5 {
            let mut n = random.next_int_bounded(total);
            for i in 0..self.weights.len() {
                let w = self.weights[i];
                n -= w.1;
                if n >= 0 {
                    continue;
                }
                if !Self::do_place(&w, depth) || Some(w.0) == self.previous {
                    break;
                }
                if let Some(p) = self.create(w.0, random, x, y, z, dir, depth) {
                    self.weights[i].2 += 1;
                    self.previous = Some(w.0);
                    let w = self.weights[i];
                    if !(w.3 == 0 || w.2 < w.3) {
                        self.weights.remove(i);
                    }
                    return Some(p);
                }
            }
        }
        // FillerCorridor.findPieceBox
        let b = BoundingBox::orient(x, y, z, -1, -1, 0, 5, 5, 4, dir);
        let c = self.collides(&b)?;
        let cb = self.list[c].base.bbox;
        if cb.min_y != b.min_y {
            return None;
        }
        for d in (1..=2).rev() {
            let t = BoundingBox::orient(x, y, z, -1, -1, 0, 5, 5, d, dir);
            if !cb.intersects(&t) {
                let b = BoundingBox::orient(x, y, z, -1, -1, 0, 5, 5, d + 1, dir);
                if b.min_y <= 1 {
                    return None;
                }
                let steps = if matches!(dir, Dir::North | Dir::South) { b.z_span() } else { b.x_span() };
                let mut base = PieceBase::new("minecraft:shfc", depth, b);
                base.set_orientation(Some(dir));
                return Some(ShPiece { base, door: Door::Opening, part: Part::Filler { steps } });
            }
        }
        None
    }

    /// `StrongholdPieces.generateAndAddPiece`.
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32) {
        let start = self.list[0].base.bbox;
        if depth > 50 || (x - start.min_x).abs() > 112 || (z - start.min_z).abs() > 112 {
            return;
        }
        if let Some(p) = self.piece_at_small_door(random, x, y, z, dir, depth + 1) {
            self.list.push(p);
            self.pending.push(self.list.len() - 1);
        }
    }

    /// `generateSmallDoorChildForward`.
    fn forward(&mut self, i: usize, random: &mut WorldgenRandom, dx: i32, dy: i32) {
        let (b, d) = (self.list[i].base.bbox, self.list[i].base.gen_depth);
        let Some(o) = self.list[i].base.orientation() else { return };
        let (x, z) = match o {
            Dir::North => (b.min_x + dx, b.min_z - 1),
            Dir::South => (b.min_x + dx, b.max_z + 1),
            Dir::West => (b.min_x - 1, b.min_z + dx),
            _ => (b.max_x + 1, b.min_z + dx),
        };
        self.add(random, x, b.min_y + dy, z, o, d);
    }

    /// `generateSmallDoorChildLeft`.
    fn left(&mut self, i: usize, random: &mut WorldgenRandom, dy: i32, dxz: i32) {
        let (b, d) = (self.list[i].base.bbox, self.list[i].base.gen_depth);
        match self.list[i].base.orientation() {
            Some(Dir::North | Dir::South) => self.add(random, b.min_x - 1, b.min_y + dy, b.min_z + dxz, Dir::West, d),
            Some(Dir::West | Dir::East) => self.add(random, b.min_x + dxz, b.min_y + dy, b.min_z - 1, Dir::North, d),
            _ => {}
        }
    }

    /// `generateSmallDoorChildRight`.
    fn right(&mut self, i: usize, random: &mut WorldgenRandom, dy: i32, dxz: i32) {
        let (b, d) = (self.list[i].base.bbox, self.list[i].base.gen_depth);
        match self.list[i].base.orientation() {
            Some(Dir::North | Dir::South) => self.add(random, b.max_x + 1, b.min_y + dy, b.min_z + dxz, Dir::East, d),
            Some(Dir::West | Dir::East) => self.add(random, b.min_x + dxz, b.min_y + dy, b.max_z + 1, Dir::South, d),
            _ => {}
        }
    }

    /// `addChildren` of the piece at `i`.
    fn add_children(&mut self, i: usize, random: &mut WorldgenRandom) {
        let o = self.list[i].base.orientation();
        let ne = matches!(o, Some(Dir::North | Dir::East));
        match self.list[i].part {
            Part::StairsDown { source } => {
                if source {
                    self.imposed = Some(PieceKind::FiveCrossing);
                }
                self.forward(i, random, 1, 1);
            }
            Part::Straight { left, right } => {
                self.forward(i, random, 1, 1);
                if left {
                    self.left(i, random, 1, 2);
                }
                if right {
                    self.right(i, random, 1, 2);
                }
            }
            Part::ChestCorridor { .. } | Part::PrisonHall | Part::StraightStairsDown => self.forward(i, random, 1, 1),
            Part::LeftTurn => {
                if ne {
                    self.left(i, random, 1, 1)
                } else {
                    self.right(i, random, 1, 1)
                }
            }
            Part::RightTurn => {
                if ne {
                    self.right(i, random, 1, 1)
                } else {
                    self.left(i, random, 1, 1)
                }
            }
            Part::RoomCrossing { .. } => {
                self.forward(i, random, 4, 1);
                self.left(i, random, 1, 4);
                self.right(i, random, 1, 4);
            }
            Part::FiveCrossing { left_low, left_high, right_low, right_high } => {
                let (mut lo, mut hi) = (3, 5);
                if matches!(o, Some(Dir::West | Dir::North)) {
                    lo = 8 - lo;
                    hi = 8 - hi;
                }
                self.forward(i, random, 5, 1);
                if left_low {
                    self.left(i, random, lo, 1);
                }
                if left_high {
                    self.left(i, random, hi, 7);
                }
                if right_low {
                    self.right(i, random, lo, 1);
                }
                if right_high {
                    self.right(i, random, hi, 7);
                }
            }
            Part::PortalRoom { .. } => self.portal = true,
            Part::Filler { .. } | Part::Library { .. } => {}
        }
    }
}

/// `StrongholdPieces.SmoothStoneSelector`.
fn smooth(random: &mut WorldgenRandom, _x: i32, _y: i32, _z: i32, edge: bool) -> u16 {
    if !edge {
        return CAVE_AIR;
    }
    let f = random.next_float();
    if f < 0.2 {
        st("minecraft:cracked_stone_bricks")
    } else if f < 0.5 {
        st("minecraft:mossy_stone_bricks")
    } else if f < 0.55 {
        st("minecraft:infested_stone_bricks")
    } else {
        st("minecraft:stone_bricks")
    }
}

/// `StrongholdPiece.generateSmallDoor`.
fn small_door(b: &PieceBase, r: &mut Region, cb: &BoundingBox, door: Door, x: i32, y: i32, z: i32) {
    let bricks = st("minecraft:stone_bricks");
    let frame = |r: &mut Region| {
        for (dx, dy) in [(0, 0), (0, 1), (0, 2), (1, 2), (2, 2), (2, 1), (2, 0)] {
            b.place_block(r, bricks, x + dx, y + dy, z, cb);
        }
    };
    match door {
        Door::Opening => b.generate_box(r, cb, x, y, z, x + 3 - 1, y + 3 - 1, z, CAVE_AIR, CAVE_AIR, false),
        Door::Wood => {
            frame(r);
            b.place_block(r, st("minecraft:oak_door"), x + 1, y, z, cb);
            b.place_block(r, with(st("minecraft:oak_door"), &[("half", "upper")]), x + 1, y + 1, z, cb);
        }
        Door::Grates => {
            let bars = st("minecraft:iron_bars");
            b.place_block(r, CAVE_AIR, x + 1, y, z, cb);
            b.place_block(r, CAVE_AIR, x + 1, y + 1, z, cb);
            b.place_block(r, with(bars, &[("west", "true")]), x, y, z, cb);
            b.place_block(r, with(bars, &[("west", "true")]), x, y + 1, z, cb);
            b.place_block(r, with(bars, &[("east", "true"), ("west", "true")]), x, y + 2, z, cb);
            b.place_block(r, with(bars, &[("east", "true"), ("west", "true")]), x + 1, y + 2, z, cb);
            b.place_block(r, with(bars, &[("east", "true"), ("west", "true")]), x + 2, y + 2, z, cb);
            b.place_block(r, with(bars, &[("east", "true")]), x + 2, y + 1, z, cb);
            b.place_block(r, with(bars, &[("east", "true")]), x + 2, y, z, cb);
        }
        Door::Iron => {
            frame(r);
            b.place_block(r, st("minecraft:iron_door"), x + 1, y, z, cb);
            b.place_block(r, with(st("minecraft:iron_door"), &[("half", "upper")]), x + 1, y + 1, z, cb);
            b.place_block(r, with(st("minecraft:stone_button"), &[("facing", "north")]), x + 2, y + 1, z + 1, cb);
            b.place_block(r, with(st("minecraft:stone_button"), &[("facing", "south")]), x + 2, y + 1, z - 1, cb);
        }
    }
}

impl Piece for ShPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        tag.push(("EntryDoor".into(), Tag::String(self.door.name().into())));
        let mut flag = |k: &str, v: bool| tag.push((k.into(), bool_tag(v)));
        match &self.part {
            Part::StairsDown { source } => flag("Source", *source),
            Part::Straight { left, right } => {
                flag("Left", *left);
                flag("Right", *right);
            }
            Part::ChestCorridor { chest } => flag("Chest", chest.load(Ordering::Relaxed)),
            Part::FiveCrossing { left_low, left_high, right_low, right_high } => {
                flag("leftLow", *left_low);
                flag("leftHigh", *left_high);
                flag("rightLow", *right_low);
                flag("rightHigh", *right_high);
            }
            Part::Library { tall } => flag("Tall", *tall),
            Part::PortalRoom { spawner } => flag("Mob", spawner.load(Ordering::Relaxed)),
            Part::Filler { steps } => tag.push(("Steps".into(), Tag::Int(*steps))),
            Part::RoomCrossing { ty } => tag.push(("Type".into(), Tag::Int(*ty))),
            Part::LeftTurn | Part::RightTurn | Part::PrisonHall | Part::StraightStairsDown => {}
        }
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        let b = &self.base;
        let mut smooth = smooth;
        let ne = matches!(b.orientation(), Some(Dir::North | Dir::East));
        match &self.part {
            Part::StairsDown { .. } => {
                b.generate_box_with(r, cb, 0, 0, 0, 4, 10, 4, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 1, 7, 0);
                small_door(b, r, cb, Door::Opening, 1, 1, 4);
                b.place_block(r, st("minecraft:stone_bricks"), 2, 6, 1, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 1, 5, 1, cb);
                b.place_block(r, st("minecraft:smooth_stone_slab"), 1, 6, 1, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 1, 5, 2, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 1, 4, 3, cb);
                b.place_block(r, st("minecraft:smooth_stone_slab"), 1, 5, 3, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 2, 4, 3, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 3, 3, 3, cb);
                b.place_block(r, st("minecraft:smooth_stone_slab"), 3, 4, 3, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 3, 3, 2, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 3, 2, 1, cb);
                b.place_block(r, st("minecraft:smooth_stone_slab"), 3, 3, 1, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 2, 2, 1, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 1, 1, 1, cb);
                b.place_block(r, st("minecraft:smooth_stone_slab"), 1, 2, 1, cb);
                b.place_block(r, st("minecraft:stone_bricks"), 1, 1, 2, cb);
                b.place_block(r, st("minecraft:smooth_stone_slab"), 1, 1, 3, cb);
            }
            Part::Straight { left, right } => {
                b.generate_box_with(r, cb, 0, 0, 0, 4, 4, 6, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 1, 1, 0);
                small_door(b, r, cb, Door::Opening, 1, 1, 6);
                let v8 = with(st("minecraft:wall_torch"), &[("facing", "east")]);
                let v9 = with(st("minecraft:wall_torch"), &[("facing", "west")]);
                b.maybe_generate_block(r, cb, random, 0.1, 1, 2, 1, v8);
                b.maybe_generate_block(r, cb, random, 0.1, 3, 2, 1, v9);
                b.maybe_generate_block(r, cb, random, 0.1, 1, 2, 5, v8);
                b.maybe_generate_block(r, cb, random, 0.1, 3, 2, 5, v9);
                if *left {
                    b.generate_box(r, cb, 0, 1, 2, 0, 3, 4, CAVE_AIR, CAVE_AIR, false);
                }
                if *right {
                    b.generate_box(r, cb, 4, 1, 2, 4, 3, 4, CAVE_AIR, CAVE_AIR, false);
                }
            }
            Part::ChestCorridor { chest } => {
                b.generate_box_with(r, cb, 0, 0, 0, 4, 4, 6, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 1, 1, 0);
                small_door(b, r, cb, Door::Opening, 1, 1, 6);
                b.generate_box(r, cb, 3, 1, 2, 3, 1, 4, st("minecraft:stone_bricks"), st("minecraft:stone_bricks"), false);
                b.place_block(r, st("minecraft:stone_brick_slab"), 3, 1, 1, cb);
                b.place_block(r, st("minecraft:stone_brick_slab"), 3, 1, 5, cb);
                b.place_block(r, st("minecraft:stone_brick_slab"), 3, 2, 2, cb);
                b.place_block(r, st("minecraft:stone_brick_slab"), 3, 2, 4, cb);
                for v8 in 2..=4 {
                    b.place_block(r, st("minecraft:stone_brick_slab"), 2, 1, v8, cb);
                }
                if !chest.load(Ordering::Relaxed) && cb.is_inside(b.world_pos(3, 2, 3)) {
                    chest.store(true, Ordering::Relaxed);
                    b.create_chest(r, cb, random, 3, 2, 3, "minecraft:chests/stronghold_corridor");
                }
            }
            Part::Filler { steps } => {
                let (bricks, air) = (st("minecraft:stone_bricks"), CAVE_AIR);
                for z in 0..*steps {
                    for x in 0..5 {
                        b.place_block(r, bricks, x, 0, z, cb);
                    }
                    for y in 1..=3 {
                        b.place_block(r, bricks, 0, y, z, cb);
                        b.place_block(r, air, 1, y, z, cb);
                        b.place_block(r, air, 2, y, z, cb);
                        b.place_block(r, air, 3, y, z, cb);
                        b.place_block(r, bricks, 4, y, z, cb);
                    }
                    for x in 0..5 {
                        b.place_block(r, bricks, x, 4, z, cb);
                    }
                }
            }
            Part::FiveCrossing { left_low, left_high, right_low, right_high } => {
                b.generate_box_with(r, cb, 0, 0, 0, 9, 8, 10, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 4, 3, 0);
                if *left_low {
                    b.generate_box(r, cb, 0, 3, 1, 0, 5, 3, CAVE_AIR, CAVE_AIR, false);
                }
                if *right_low {
                    b.generate_box(r, cb, 9, 3, 1, 9, 5, 3, CAVE_AIR, CAVE_AIR, false);
                }
                if *left_high {
                    b.generate_box(r, cb, 0, 5, 7, 0, 7, 9, CAVE_AIR, CAVE_AIR, false);
                }
                if *right_high {
                    b.generate_box(r, cb, 9, 5, 7, 9, 7, 9, CAVE_AIR, CAVE_AIR, false);
                }
                b.generate_box(r, cb, 5, 1, 10, 7, 3, 10, CAVE_AIR, CAVE_AIR, false);
                b.generate_box_with(r, cb, 1, 2, 1, 8, 2, 6, false, random, &mut smooth);
                b.generate_box_with(r, cb, 4, 1, 5, 4, 4, 9, false, random, &mut smooth);
                b.generate_box_with(r, cb, 8, 1, 5, 8, 4, 9, false, random, &mut smooth);
                b.generate_box_with(r, cb, 1, 4, 7, 3, 4, 9, false, random, &mut smooth);
                b.generate_box_with(r, cb, 1, 3, 5, 3, 3, 6, false, random, &mut smooth);
                b.generate_box(r, cb, 1, 3, 4, 3, 3, 4, st("minecraft:smooth_stone_slab"), st("minecraft:smooth_stone_slab"), false);
                b.generate_box(r, cb, 1, 4, 6, 3, 4, 6, st("minecraft:smooth_stone_slab"), st("minecraft:smooth_stone_slab"), false);
                b.generate_box_with(r, cb, 5, 1, 7, 7, 1, 8, false, random, &mut smooth);
                b.generate_box(r, cb, 5, 1, 9, 7, 1, 9, st("minecraft:smooth_stone_slab"), st("minecraft:smooth_stone_slab"), false);
                b.generate_box(r, cb, 5, 2, 7, 7, 2, 7, st("minecraft:smooth_stone_slab"), st("minecraft:smooth_stone_slab"), false);
                b.generate_box(r, cb, 4, 5, 7, 4, 5, 9, st("minecraft:smooth_stone_slab"), st("minecraft:smooth_stone_slab"), false);
                b.generate_box(r, cb, 8, 5, 7, 8, 5, 9, st("minecraft:smooth_stone_slab"), st("minecraft:smooth_stone_slab"), false);
                b.generate_box(r, cb, 5, 5, 7, 7, 5, 9, with(st("minecraft:smooth_stone_slab"), &[("type", "double")]), with(st("minecraft:smooth_stone_slab"), &[("type", "double")]), false);
                b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "south")]), 6, 5, 6, cb);
            }
            Part::LeftTurn | Part::RightTurn => {
                b.generate_box_with(r, cb, 0, 0, 0, 4, 4, 4, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 1, 1, 0);
                let left = matches!(self.part, Part::LeftTurn) == ne;
                if left {
                    b.generate_box(r, cb, 0, 1, 1, 0, 3, 3, CAVE_AIR, CAVE_AIR, false);
                } else {
                    b.generate_box(r, cb, 4, 1, 1, 4, 3, 3, CAVE_AIR, CAVE_AIR, false);
                }
            }
            Part::Library { tall } => {
                let v8 = if *tall { 11 } else { 6 };
                b.generate_box_with(r, cb, 0, 0, 0, 13, v8 - 1, 14, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 4, 1, 0);
                b.generate_maybe_box(r, cb, random, 0.07, 2, 1, 1, 11, 4, 13, st("minecraft:cobweb"), st("minecraft:cobweb"), false, false);
                for v11 in 1..=13 {
                    if (v11 - 1) % 4 == 0 {
                        b.generate_box(r, cb, 1, 1, v11, 1, 4, v11, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                        b.generate_box(r, cb, 12, 1, v11, 12, 4, v11, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                        b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "east")]), 2, 3, v11, cb);
                        b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "west")]), 11, 3, v11, cb);
                        if *tall {
                            b.generate_box(r, cb, 1, 6, v11, 1, 9, v11, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                            b.generate_box(r, cb, 12, 6, v11, 12, 9, v11, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                        }
                    } else {
                        b.generate_box(r, cb, 1, 1, v11, 1, 4, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                        b.generate_box(r, cb, 12, 1, v11, 12, 4, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                        if *tall {
                            b.generate_box(r, cb, 1, 6, v11, 1, 9, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                            b.generate_box(r, cb, 12, 6, v11, 12, 9, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                        }
                    }
                }
                for v11 in (3..12).step_by(2) {
                    b.generate_box(r, cb, 3, 1, v11, 4, 3, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                    b.generate_box(r, cb, 6, 1, v11, 7, 3, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                    b.generate_box(r, cb, 9, 1, v11, 10, 3, v11, st("minecraft:bookshelf"), st("minecraft:bookshelf"), false);
                }
                if *tall {
                    b.generate_box(r, cb, 1, 5, 1, 3, 5, 13, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                    b.generate_box(r, cb, 10, 5, 1, 12, 5, 13, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                    b.generate_box(r, cb, 4, 5, 1, 9, 5, 2, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                    b.generate_box(r, cb, 4, 5, 12, 9, 5, 13, st("minecraft:oak_planks"), st("minecraft:oak_planks"), false);
                    b.place_block(r, st("minecraft:oak_planks"), 9, 5, 11, cb);
                    b.place_block(r, st("minecraft:oak_planks"), 8, 5, 11, cb);
                    b.place_block(r, st("minecraft:oak_planks"), 9, 5, 10, cb);
                    let v11 = with(st("minecraft:oak_fence"), &[("west", "true"), ("east", "true")]);
                    let v12 = with(st("minecraft:oak_fence"), &[("north", "true"), ("south", "true")]);
                    b.generate_box(r, cb, 3, 6, 3, 3, 6, 11, v12, v12, false);
                    b.generate_box(r, cb, 10, 6, 3, 10, 6, 9, v12, v12, false);
                    b.generate_box(r, cb, 4, 6, 2, 9, 6, 2, v11, v11, false);
                    b.generate_box(r, cb, 4, 6, 12, 7, 6, 12, v11, v11, false);
                    b.place_block(r, with(st("minecraft:oak_fence"), &[("north", "true"), ("east", "true")]), 3, 6, 2, cb);
                    b.place_block(r, with(st("minecraft:oak_fence"), &[("south", "true"), ("east", "true")]), 3, 6, 12, cb);
                    b.place_block(r, with(st("minecraft:oak_fence"), &[("north", "true"), ("west", "true")]), 10, 6, 2, cb);
                    for v13 in 0..=2 {
                        b.place_block(r, with(st("minecraft:oak_fence"), &[("south", "true"), ("west", "true")]), 8 + v13, 6, 12 - v13, cb);
                        if v13 != 2 {
                            b.place_block(r, with(st("minecraft:oak_fence"), &[("north", "true"), ("east", "true")]), 8 + v13, 6, 11 - v13, cb);
                        }
                    }
                    let v13 = with(st("minecraft:ladder"), &[("facing", "south")]);
                    b.place_block(r, v13, 10, 1, 13, cb);
                    b.place_block(r, v13, 10, 2, 13, cb);
                    b.place_block(r, v13, 10, 3, 13, cb);
                    b.place_block(r, v13, 10, 4, 13, cb);
                    b.place_block(r, v13, 10, 5, 13, cb);
                    b.place_block(r, v13, 10, 6, 13, cb);
                    b.place_block(r, v13, 10, 7, 13, cb);
                    let v16 = with(st("minecraft:oak_fence"), &[("east", "true")]);
                    b.place_block(r, v16, 6, 9, 7, cb);
                    let v17 = with(st("minecraft:oak_fence"), &[("west", "true")]);
                    b.place_block(r, v17, 7, 9, 7, cb);
                    b.place_block(r, v16, 6, 8, 7, cb);
                    b.place_block(r, v17, 7, 8, 7, cb);
                    let v18 = with(v12, &[("west", "true"), ("east", "true")]);
                    b.place_block(r, v18, 6, 7, 7, cb);
                    b.place_block(r, v18, 7, 7, 7, cb);
                    b.place_block(r, v16, 5, 7, 7, cb);
                    b.place_block(r, v17, 8, 7, 7, cb);
                    b.place_block(r, with(v16, &[("north", "true")]), 6, 7, 6, cb);
                    b.place_block(r, with(v16, &[("south", "true")]), 6, 7, 8, cb);
                    b.place_block(r, with(v17, &[("north", "true")]), 7, 7, 6, cb);
                    b.place_block(r, with(v17, &[("south", "true")]), 7, 7, 8, cb);
                    let v19 = st("minecraft:torch");
                    b.place_block(r, v19, 5, 8, 7, cb);
                    b.place_block(r, v19, 8, 8, 7, cb);
                    b.place_block(r, v19, 6, 8, 6, cb);
                    b.place_block(r, v19, 6, 8, 8, cb);
                    b.place_block(r, v19, 7, 8, 6, cb);
                    b.place_block(r, v19, 7, 8, 8, cb);
                }
                b.create_chest(r, cb, random, 3, 3, 5, "minecraft:chests/stronghold_library");
                if *tall {
                    b.place_block(r, CAVE_AIR, 12, 9, 1, cb);
                    b.create_chest(r, cb, random, 12, 8, 1, "minecraft:chests/stronghold_library");
                }
            }
            Part::PortalRoom { spawner } => {
                b.generate_box_with(r, cb, 0, 0, 0, 10, 7, 15, false, random, &mut smooth);
                small_door(b, r, cb, Door::Grates, 4, 1, 0);
                b.generate_box_with(r, cb, 1, 6, 1, 1, 6, 14, false, random, &mut smooth);
                b.generate_box_with(r, cb, 9, 6, 1, 9, 6, 14, false, random, &mut smooth);
                b.generate_box_with(r, cb, 2, 6, 1, 8, 6, 2, false, random, &mut smooth);
                b.generate_box_with(r, cb, 2, 6, 14, 8, 6, 14, false, random, &mut smooth);
                b.generate_box_with(r, cb, 1, 1, 1, 2, 1, 4, false, random, &mut smooth);
                b.generate_box_with(r, cb, 8, 1, 1, 9, 1, 4, false, random, &mut smooth);
                b.generate_box(r, cb, 1, 1, 1, 1, 1, 3, st("minecraft:lava"), st("minecraft:lava"), false);
                b.generate_box(r, cb, 9, 1, 1, 9, 1, 3, st("minecraft:lava"), st("minecraft:lava"), false);
                b.generate_box_with(r, cb, 3, 1, 8, 7, 1, 12, false, random, &mut smooth);
                b.generate_box(r, cb, 4, 1, 9, 6, 1, 11, st("minecraft:lava"), st("minecraft:lava"), false);
                let v9 = with(st("minecraft:iron_bars"), &[("north", "true"), ("south", "true")]);
                let v10 = with(st("minecraft:iron_bars"), &[("west", "true"), ("east", "true")]);
                for v11 in (3..14).step_by(2) {
                    b.generate_box(r, cb, 0, 3, v11, 0, 4, v11, v9, v9, false);
                    b.generate_box(r, cb, 10, 3, v11, 10, 4, v11, v9, v9, false);
                }
                for v11 in (2..9).step_by(2) {
                    b.generate_box(r, cb, v11, 3, 15, v11, 4, 15, v10, v10, false);
                }
                let v11 = with(st("minecraft:stone_brick_stairs"), &[("facing", "north")]);
                b.generate_box_with(r, cb, 4, 1, 5, 6, 1, 7, false, random, &mut smooth);
                b.generate_box_with(r, cb, 4, 2, 6, 6, 2, 7, false, random, &mut smooth);
                b.generate_box_with(r, cb, 4, 3, 7, 6, 3, 7, false, random, &mut smooth);
                for v12 in 4..=6 {
                    b.place_block(r, v11, v12, 1, 4, cb);
                    b.place_block(r, v11, v12, 2, 5, cb);
                    b.place_block(r, v11, v12, 3, 6, cb);
                }
                let v12 = with(st("minecraft:end_portal_frame"), &[("facing", "north")]);
                let v13 = with(st("minecraft:end_portal_frame"), &[("facing", "south")]);
                let v14 = with(st("minecraft:end_portal_frame"), &[("facing", "east")]);
                let v15 = with(st("minecraft:end_portal_frame"), &[("facing", "west")]);
                let mut all = true;
                let mut eyes = [false; 12];
                for e in &mut eyes {
                    *e = random.next_float() > 0.9;
                    all &= *e;
                }
                let eye = |s: u16, i: usize| with(s, &[("eye", if eyes[i] { "true" } else { "false" })]);
                for (i, (s, x, z)) in [(v12, 4, 8), (v12, 5, 8), (v12, 6, 8), (v13, 4, 12), (v13, 5, 12), (v13, 6, 12), (v14, 3, 9), (v14, 3, 10), (v14, 3, 11), (v15, 7, 9), (v15, 7, 10), (v15, 7, 11)].into_iter().enumerate() {
                    b.place_block(r, eye(s, i), x, 3, z, cb);
                }
                if all {
                    let v18 = st("minecraft:end_portal");
                    b.place_block(r, v18, 4, 3, 9, cb);
                    b.place_block(r, v18, 5, 3, 9, cb);
                    b.place_block(r, v18, 6, 3, 9, cb);
                    b.place_block(r, v18, 4, 3, 10, cb);
                    b.place_block(r, v18, 5, 3, 10, cb);
                    b.place_block(r, v18, 6, 3, 10, cb);
                    b.place_block(r, v18, 4, 3, 11, cb);
                    b.place_block(r, v18, 5, 3, 11, cb);
                    b.place_block(r, v18, 6, 3, 11, cb);
                }
                let p = b.world_pos(5, 3, 6);
                if !spawner.load(Ordering::Relaxed) && cb.is_inside(p) {
                    spawner.store(true, Ordering::Relaxed);
                    r.set(p, st("minecraft:spawner"), 2);
                    set_spawner_entity(r, p, "minecraft:silverfish");
                }
            }
            Part::PrisonHall => {
                b.generate_box_with(r, cb, 0, 0, 0, 8, 4, 10, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 1, 1, 0);
                b.generate_box(r, cb, 1, 1, 10, 3, 3, 10, CAVE_AIR, CAVE_AIR, false);
                b.generate_box_with(r, cb, 4, 1, 1, 4, 3, 1, false, random, &mut smooth);
                b.generate_box_with(r, cb, 4, 1, 3, 4, 3, 3, false, random, &mut smooth);
                b.generate_box_with(r, cb, 4, 1, 7, 4, 3, 7, false, random, &mut smooth);
                b.generate_box_with(r, cb, 4, 1, 9, 4, 3, 9, false, random, &mut smooth);
                for v8 in 1..=3 {
                    b.place_block(r, with(st("minecraft:iron_bars"), &[("north", "true"), ("south", "true")]), 4, v8, 4, cb);
                    b.place_block(r, with(st("minecraft:iron_bars"), &[("north", "true"), ("south", "true"), ("east", "true")]), 4, v8, 5, cb);
                    b.place_block(r, with(st("minecraft:iron_bars"), &[("north", "true"), ("south", "true")]), 4, v8, 6, cb);
                    b.place_block(r, with(st("minecraft:iron_bars"), &[("west", "true"), ("east", "true")]), 5, v8, 5, cb);
                    b.place_block(r, with(st("minecraft:iron_bars"), &[("west", "true"), ("east", "true")]), 6, v8, 5, cb);
                    b.place_block(r, with(st("minecraft:iron_bars"), &[("west", "true"), ("east", "true")]), 7, v8, 5, cb);
                }
                b.place_block(r, with(st("minecraft:iron_bars"), &[("north", "true"), ("south", "true")]), 4, 3, 2, cb);
                b.place_block(r, with(st("minecraft:iron_bars"), &[("north", "true"), ("south", "true")]), 4, 3, 8, cb);
                let v8 = with(st("minecraft:iron_door"), &[("facing", "west")]);
                let v9 = with(st("minecraft:iron_door"), &[("facing", "west"), ("half", "upper")]);
                b.place_block(r, v8, 4, 1, 2, cb);
                b.place_block(r, v9, 4, 2, 2, cb);
                b.place_block(r, v8, 4, 1, 8, cb);
                b.place_block(r, v9, 4, 2, 8, cb);
            }
            Part::RoomCrossing { ty } => {
                b.generate_box_with(r, cb, 0, 0, 0, 10, 6, 10, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 4, 1, 0);
                b.generate_box(r, cb, 4, 1, 10, 6, 3, 10, CAVE_AIR, CAVE_AIR, false);
                b.generate_box(r, cb, 0, 1, 4, 0, 3, 6, CAVE_AIR, CAVE_AIR, false);
                b.generate_box(r, cb, 10, 1, 4, 10, 3, 6, CAVE_AIR, CAVE_AIR, false);
                match ty {
                    0 => {
                        b.place_block(r, st("minecraft:stone_bricks"), 5, 1, 5, cb);
                        b.place_block(r, st("minecraft:stone_bricks"), 5, 2, 5, cb);
                        b.place_block(r, st("minecraft:stone_bricks"), 5, 3, 5, cb);
                        b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "west")]), 4, 3, 5, cb);
                        b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "east")]), 6, 3, 5, cb);
                        b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "south")]), 5, 3, 4, cb);
                        b.place_block(r, with(st("minecraft:wall_torch"), &[("facing", "north")]), 5, 3, 6, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 4, 1, 4, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 4, 1, 5, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 4, 1, 6, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 6, 1, 4, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 6, 1, 5, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 6, 1, 6, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 5, 1, 4, cb);
                        b.place_block(r, st("minecraft:smooth_stone_slab"), 5, 1, 6, cb);
                    }
                    1 => {
                        for v8 in 0..5 {
                            b.place_block(r, st("minecraft:stone_bricks"), 3, 1, 3 + v8, cb);
                            b.place_block(r, st("minecraft:stone_bricks"), 7, 1, 3 + v8, cb);
                            b.place_block(r, st("minecraft:stone_bricks"), 3 + v8, 1, 3, cb);
                            b.place_block(r, st("minecraft:stone_bricks"), 3 + v8, 1, 7, cb);
                        }
                        b.place_block(r, st("minecraft:stone_bricks"), 5, 1, 5, cb);
                        b.place_block(r, st("minecraft:stone_bricks"), 5, 2, 5, cb);
                        b.place_block(r, st("minecraft:stone_bricks"), 5, 3, 5, cb);
                        b.place_block(r, st("minecraft:water"), 5, 4, 5, cb);
                    }
                    2 => {
                        for v8 in 1..=9 {
                            b.place_block(r, st("minecraft:cobblestone"), 1, 3, v8, cb);
                            b.place_block(r, st("minecraft:cobblestone"), 9, 3, v8, cb);
                        }
                        for v8 in 1..=9 {
                            b.place_block(r, st("minecraft:cobblestone"), v8, 3, 1, cb);
                            b.place_block(r, st("minecraft:cobblestone"), v8, 3, 9, cb);
                        }
                        b.place_block(r, st("minecraft:cobblestone"), 5, 1, 4, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 5, 1, 6, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 5, 3, 4, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 5, 3, 6, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 4, 1, 5, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 6, 1, 5, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 4, 3, 5, cb);
                        b.place_block(r, st("minecraft:cobblestone"), 6, 3, 5, cb);
                        for v8 in 1..=3 {
                            b.place_block(r, st("minecraft:cobblestone"), 4, v8, 4, cb);
                            b.place_block(r, st("minecraft:cobblestone"), 6, v8, 4, cb);
                            b.place_block(r, st("minecraft:cobblestone"), 4, v8, 6, cb);
                            b.place_block(r, st("minecraft:cobblestone"), 6, v8, 6, cb);
                        }
                        b.place_block(r, st("minecraft:wall_torch"), 5, 3, 5, cb);
                        for v8 in 2..=8 {
                            b.place_block(r, st("minecraft:oak_planks"), 2, 3, v8, cb);
                            b.place_block(r, st("minecraft:oak_planks"), 3, 3, v8, cb);
                            if v8 <= 3 || v8 >= 7 {
                                b.place_block(r, st("minecraft:oak_planks"), 4, 3, v8, cb);
                                b.place_block(r, st("minecraft:oak_planks"), 5, 3, v8, cb);
                                b.place_block(r, st("minecraft:oak_planks"), 6, 3, v8, cb);
                            }
                            b.place_block(r, st("minecraft:oak_planks"), 7, 3, v8, cb);
                            b.place_block(r, st("minecraft:oak_planks"), 8, 3, v8, cb);
                        }
                        let v8 = with(st("minecraft:ladder"), &[("facing", "west")]);
                        b.place_block(r, v8, 9, 1, 3, cb);
                        b.place_block(r, v8, 9, 2, 3, cb);
                        b.place_block(r, v8, 9, 3, 3, cb);
                        b.create_chest(r, cb, random, 3, 4, 8, "minecraft:chests/stronghold_crossing");
                    }
                    _ => {}
                }
            }
            Part::StraightStairsDown => {
                b.generate_box_with(r, cb, 0, 0, 0, 4, 10, 7, true, random, &mut smooth);
                small_door(b, r, cb, self.door, 1, 7, 0);
                small_door(b, r, cb, Door::Opening, 1, 1, 7);
                let v8 = with(st("minecraft:cobblestone_stairs"), &[("facing", "south")]);
                for v9 in 0..6 {
                    b.place_block(r, v8, 1, 6 - v9, 1 + v9, cb);
                    b.place_block(r, v8, 2, 6 - v9, 1 + v9, cb);
                    b.place_block(r, v8, 3, 6 - v9, 1 + v9, cb);
                    if v9 < 5 {
                        b.place_block(r, st("minecraft:stone_bricks"), 1, 5 - v9, 1 + v9, cb);
                        b.place_block(r, st("minecraft:stone_bricks"), 2, 5 - v9, 1 + v9, cb);
                        b.place_block(r, st("minecraft:stone_bricks"), 3, 5 - v9, 1 + v9, cb);
                    }
                }
            }
        }
    }
}
