//! Nether fortresses (`NetherFortressStructure`, `NetherFortressPieces`).

use super::{bool_tag, move_inside_heights, set_spawner_entity, st, with};
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

/// `NetherFortressStructure`.
pub struct Fortress;

impl Kind for Fortress {
    /// `findGenerationPoint`: the chunk's corner at y 64 (where the biome is checked).
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let (cx, cz) = ctx.chunk;
        Some(Stub { pos: BlockPos::new(cx << 4, 64, cz << 4), build: Box::new(move |ctx, out| *out = generate(ctx)) })
    }
}

/// `NetherFortressStructure.generatePieces`.
fn generate(ctx: &mut GenCtx) -> Vec<Box<dyn Piece>> {
    let (cx, cz) = ctx.chunk;
    let random = &mut ctx.random;
    let mut g = Gen {
        list: Vec::new(),
        bridge: BRIDGE_WEIGHTS.iter().map(|&(kind, weight, max, row)| Weight { kind, weight, count: 0, max, row }).collect(),
        castle: CASTLE_WEIGHTS.iter().map(|&(kind, weight, max, row)| Weight { kind, weight, count: 0, max, row }).collect(),
        previous: None,
        pending: Vec::new(),
    };
    // `StartPiece`: a bridge crossing (saved as one) at block 2 of the chunk.
    let dir = PieceBase::random_horizontal(random);
    let mut base = PieceBase::new("minecraft:nebcr", 0, PieceBase::make_bbox((cx << 4) + 2, 64, (cz << 4) + 2, dir, 19, 10, 19));
    base.set_orientation(Some(dir));
    g.list.push(FPiece { base, part: Part::BridgeCrossing });
    g.add_children(0, random);
    while !g.pending.is_empty() {
        let i = random.next_int_bounded(g.pending.len() as i32) as usize;
        let p = g.pending.remove(i);
        g.add_children(p, random);
    }
    let mut pieces: Vec<Box<dyn Piece>> = g.list.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect();
    move_inside_heights(&mut pieces, random, 48, 70);
    pieces
}

/// Piece types (`NetherFortressPieces$*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PieceKind {
    BridgeStraight,
    BridgeCrossing,
    RoomCrossing,
    StairsRoom,
    MonsterThrone,
    CastleEntrance,
    SmallCorridor,
    SmallCorridorCrossing,
    RightTurn,
    LeftTurn,
    CorridorStairs,
    TBalcony,
    StalkRoom,
    EndFiller,
}

/// `BRIDGE_PIECE_WEIGHTS`: (type, weight, max count, allowed twice in a row).
const BRIDGE_WEIGHTS: [(PieceKind, i32, i32, bool); 6] = [
    (PieceKind::BridgeStraight, 30, 0, true),
    (PieceKind::BridgeCrossing, 10, 4, false),
    (PieceKind::RoomCrossing, 10, 4, false),
    (PieceKind::StairsRoom, 10, 3, false),
    (PieceKind::MonsterThrone, 5, 2, false),
    (PieceKind::CastleEntrance, 5, 1, false),
];

/// `CASTLE_PIECE_WEIGHTS`.
const CASTLE_WEIGHTS: [(PieceKind, i32, i32, bool); 7] = [
    (PieceKind::SmallCorridor, 25, 0, true),
    (PieceKind::SmallCorridorCrossing, 15, 5, false),
    (PieceKind::RightTurn, 5, 10, false),
    (PieceKind::LeftTurn, 5, 10, false),
    (PieceKind::CorridorStairs, 10, 3, true),
    (PieceKind::TBalcony, 7, 2, false),
    (PieceKind::StalkRoom, 5, 2, false),
];

/// `PieceWeight`.
#[derive(Clone, Copy, Debug)]
struct Weight {
    kind: PieceKind,
    weight: i32,
    count: i32,
    max: i32,
    row: bool,
}

impl Weight {
    /// `doPlace` / `isValid`.
    fn can_place(&self) -> bool {
        self.max == 0 || self.count < self.max
    }
}

#[derive(Debug)]
enum Part {
    BridgeStraight,
    BridgeCrossing,
    RoomCrossing,
    StairsRoom,
    MonsterThrone { spawner: AtomicBool },
    CastleEntrance,
    SmallCorridor,
    SmallCorridorCrossing,
    RightTurn { chest: AtomicBool },
    LeftTurn { chest: AtomicBool },
    CorridorStairs,
    TBalcony,
    StalkRoom,
    EndFiller { seed: i32 },
}

/// `NetherFortressPieces.NetherBridgePiece` and its subclasses.
#[derive(Debug)]
struct FPiece {
    base: PieceBase,
    part: Part,
}

/// Piece generation state (the start piece's fields).
struct Gen {
    list: Vec<FPiece>,
    bridge: Vec<Weight>,
    castle: Vec<Weight>,
    previous: Option<PieceKind>,
    pending: Vec<usize>,
}

impl Gen {
    fn collides(&self, b: &BoundingBox) -> bool {
        self.list.iter().any(|p| p.base.bbox.intersects(b))
    }

    /// The piece types' `createPiece`: `None` if the box is too low or collides.
    #[allow(clippy::too_many_arguments)]
    fn create(&self, kind: PieceKind, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32) -> Option<FPiece> {
        let (o, s, id): ((i32, i32), (i32, i32, i32), &'static str) = match kind {
            PieceKind::BridgeStraight => ((-1, -3), (5, 10, 19), "minecraft:nebs"),
            PieceKind::BridgeCrossing => ((-8, -3), (19, 10, 19), "minecraft:nebcr"),
            PieceKind::RoomCrossing => ((-2, 0), (7, 9, 7), "minecraft:nerc"),
            PieceKind::StairsRoom => ((-2, 0), (7, 11, 7), "minecraft:nesr"),
            PieceKind::MonsterThrone => ((-2, 0), (7, 8, 9), "minecraft:nemt"),
            PieceKind::CastleEntrance => ((-5, -3), (13, 14, 13), "minecraft:nece"),
            PieceKind::SmallCorridor => ((-1, 0), (5, 7, 5), "minecraft:nesc"),
            PieceKind::SmallCorridorCrossing => ((-1, 0), (5, 7, 5), "minecraft:nescsc"),
            PieceKind::RightTurn => ((-1, 0), (5, 7, 5), "minecraft:nescrt"),
            PieceKind::LeftTurn => ((-1, 0), (5, 7, 5), "minecraft:nesclt"),
            PieceKind::CorridorStairs => ((-1, -7), (5, 14, 10), "minecraft:neccs"),
            PieceKind::TBalcony => ((-3, 0), (9, 7, 9), "minecraft:nectb"),
            PieceKind::StalkRoom => ((-5, -3), (13, 14, 13), "minecraft:necsr"),
            PieceKind::EndFiller => ((-1, -3), (5, 10, 8), "minecraft:nebef"),
        };
        let b = BoundingBox::orient(x, y, z, o.0, o.1, 0, s.0, s.1, s.2, dir);
        if b.min_y <= 10 || self.collides(&b) {
            return None;
        }
        let part = match kind {
            PieceKind::BridgeStraight => Part::BridgeStraight,
            PieceKind::BridgeCrossing => Part::BridgeCrossing,
            PieceKind::RoomCrossing => Part::RoomCrossing,
            PieceKind::StairsRoom => Part::StairsRoom,
            PieceKind::MonsterThrone => Part::MonsterThrone { spawner: AtomicBool::new(false) },
            PieceKind::CastleEntrance => Part::CastleEntrance,
            PieceKind::SmallCorridor => Part::SmallCorridor,
            PieceKind::SmallCorridorCrossing => Part::SmallCorridorCrossing,
            PieceKind::RightTurn => Part::RightTurn { chest: AtomicBool::new(random.next_int_bounded(3) == 0) },
            PieceKind::LeftTurn => Part::LeftTurn { chest: AtomicBool::new(random.next_int_bounded(3) == 0) },
            PieceKind::CorridorStairs => Part::CorridorStairs,
            PieceKind::TBalcony => Part::TBalcony,
            PieceKind::StalkRoom => Part::StalkRoom,
            PieceKind::EndFiller => Part::EndFiller { seed: random.next_int() },
        };
        let mut base = PieceBase::new(id, depth, b);
        base.set_orientation(Some(dir));
        Some(FPiece { base, part })
    }

    /// `NetherBridgePiece.generatePiece`: up to five weighted draws among the list's
    /// placeable types, else a bridge end filler.
    #[allow(clippy::too_many_arguments)]
    fn generate_piece(&mut self, castle: bool, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32) -> Option<FPiece> {
        let list = if castle { &self.castle } else { &self.bridge };
        let any = list.iter().any(|w| w.max > 0 && w.count < w.max);
        let total: i32 = list.iter().map(|w| w.weight).sum();
        let total = if any { total } else { -1 };
        let allowed = total > 0 && depth <= 30;
        let mut tries = 0;
        while tries < 5 && allowed {
            tries += 1;
            let mut n = random.next_int_bounded(total);
            let len = if castle { self.castle.len() } else { self.bridge.len() };
            for i in 0..len {
                let w = if castle { self.castle[i] } else { self.bridge[i] };
                n -= w.weight;
                if n >= 0 {
                    continue;
                }
                if !w.can_place() || (Some(w.kind) == self.previous && !w.row) {
                    break;
                }
                if let Some(p) = self.create(w.kind, random, x, y, z, dir, depth) {
                    let list = if castle { &mut self.castle } else { &mut self.bridge };
                    list[i].count += 1;
                    self.previous = Some(w.kind);
                    if !list[i].can_place() {
                        list.remove(i);
                    }
                    return Some(p);
                }
            }
        }
        self.create(PieceKind::EndFiller, random, x, y, z, dir, depth)
    }

    /// `generateAndAddPiece`: beyond 112 blocks of the start only an end filler is made (and
    /// dropped).
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32, castle: bool) {
        let start = self.list[0].base.bbox;
        if (x - start.min_x).abs() > 112 || (z - start.min_z).abs() > 112 {
            let _ = self.create(PieceKind::EndFiller, random, x, y, z, dir, depth);
            return;
        }
        if let Some(p) = self.generate_piece(castle, random, x, y, z, dir, depth + 1) {
            self.list.push(p);
            self.pending.push(self.list.len() - 1);
        }
    }

    /// `generateChildForward`.
    fn forward(&mut self, i: usize, random: &mut WorldgenRandom, dx: i32, dy: i32, castle: bool) {
        let (b, d) = (self.list[i].base.bbox, self.list[i].base.gen_depth);
        let Some(o) = self.list[i].base.orientation() else { return };
        let (x, z) = match o {
            Dir::North => (b.min_x + dx, b.min_z - 1),
            Dir::South => (b.min_x + dx, b.max_z + 1),
            Dir::West => (b.min_x - 1, b.min_z + dx),
            _ => (b.max_x + 1, b.min_z + dx),
        };
        self.add(random, x, b.min_y + dy, z, o, d, castle);
    }

    /// `generateChildLeft`.
    fn left(&mut self, i: usize, random: &mut WorldgenRandom, dy: i32, dxz: i32, castle: bool) {
        let (b, d) = (self.list[i].base.bbox, self.list[i].base.gen_depth);
        match self.list[i].base.orientation() {
            Some(Dir::North | Dir::South) => self.add(random, b.min_x - 1, b.min_y + dy, b.min_z + dxz, Dir::West, d, castle),
            Some(Dir::West | Dir::East) => self.add(random, b.min_x + dxz, b.min_y + dy, b.min_z - 1, Dir::North, d, castle),
            _ => {}
        }
    }

    /// `generateChildRight`.
    fn right(&mut self, i: usize, random: &mut WorldgenRandom, dy: i32, dxz: i32, castle: bool) {
        let (b, d) = (self.list[i].base.bbox, self.list[i].base.gen_depth);
        match self.list[i].base.orientation() {
            Some(Dir::North | Dir::South) => self.add(random, b.max_x + 1, b.min_y + dy, b.min_z + dxz, Dir::East, d, castle),
            Some(Dir::West | Dir::East) => self.add(random, b.min_x + dxz, b.min_y + dy, b.max_z + 1, Dir::South, d, castle),
            _ => {}
        }
    }

    /// `addChildren` of the piece at `i`.
    fn add_children(&mut self, i: usize, random: &mut WorldgenRandom) {
        match self.list[i].part {
            Part::BridgeCrossing => {
                self.forward(i, random, 8, 3, false);
                self.left(i, random, 3, 8, false);
                self.right(i, random, 3, 8, false);
            }
            Part::BridgeStraight => self.forward(i, random, 1, 3, false),
            Part::RoomCrossing => {
                self.forward(i, random, 2, 0, false);
                self.left(i, random, 0, 2, false);
                self.right(i, random, 0, 2, false);
            }
            Part::StairsRoom => self.right(i, random, 6, 2, false),
            Part::CastleEntrance => self.forward(i, random, 5, 3, true),
            Part::SmallCorridor | Part::CorridorStairs => self.forward(i, random, 1, 0, true),
            Part::SmallCorridorCrossing => {
                self.forward(i, random, 1, 0, true);
                self.left(i, random, 0, 1, true);
                self.right(i, random, 0, 1, true);
            }
            Part::LeftTurn { .. } => self.left(i, random, 0, 1, true),
            Part::RightTurn { .. } => self.right(i, random, 0, 1, true),
            Part::TBalcony => {
                let dxz = if matches!(self.list[i].base.orientation(), Some(Dir::West | Dir::North)) { 5 } else { 1 };
                let castle = random.next_int_bounded(8) > 0;
                self.left(i, random, 0, dxz, castle);
                let castle = random.next_int_bounded(8) > 0;
                self.right(i, random, 0, dxz, castle);
            }
            Part::StalkRoom => {
                self.forward(i, random, 5, 3, true);
                self.forward(i, random, 5, 11, true);
            }
            Part::MonsterThrone { .. } | Part::EndFiller { .. } => {}
        }
    }
}

fn bricks() -> u16 {
    st("minecraft:nether_bricks")
}

/// A nether brick fence connected on the given sides.
fn fence(sides: &[&str]) -> u16 {
    let props: Vec<(&str, &str)> = sides.iter().map(|s| (*s, "true")).collect();
    with(st("minecraft:nether_brick_fence"), &props)
}

impl FPiece {
    /// `generateBox(..., nether bricks, nether bricks, false)`.
    #[allow(clippy::too_many_arguments)]
    fn bricks_box(&self, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        let s = bricks();
        self.base.generate_box(r, cb, x0, y0, z0, x1, y1, z1, s, s, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_box(&self, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: u16) {
        self.base.generate_box(r, cb, x0, y0, z0, x1, y1, z1, s, s, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn air_box(&self, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        self.fill_box(r, cb, x0, y0, z0, x1, y1, z1, state::AIR);
    }

    fn column(&self, r: &mut Region, cb: &BoundingBox, x: i32, z: i32) {
        self.base.fill_column_down(r, bricks(), x, -1, z, cb);
    }

    /// The pillars under both ends of a bridge (`x` 0..=4 by `z` in 0..=2 and 18 - that).
    fn bridge_legs(&self, r: &mut Region, cb: &BoundingBox, x0: i32, x1: i32, far: i32) {
        for x in x0..=x1 {
            for z in 0..=2 {
                self.column(r, cb, x, z);
                self.column(r, cb, x, far - z);
            }
        }
    }

    /// The legs of the 13-wide castle rooms: the cross under the floor.
    fn castle_legs(&self, r: &mut Region, cb: &BoundingBox) {
        for x in 4..=8 {
            for z in 0..=2 {
                self.column(r, cb, x, z);
                self.column(r, cb, x, 12 - z);
            }
        }
        for x in 0..=2 {
            for z in 4..=8 {
                self.column(r, cb, x, z);
                self.column(r, cb, 12 - x, z);
            }
        }
    }

    fn solid_legs(&self, r: &mut Region, cb: &BoundingBox, x1: i32, z1: i32) {
        for x in 0..=x1 {
            for z in 0..=z1 {
                self.column(r, cb, x, z);
            }
        }
    }

    /// The walls, battlements and floor cross shared by the castle entrance and stalk room.
    fn castle_shell(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        self.bricks_box(r, cb, 0, 3, 0, 12, 4, 12);
        self.air_box(r, cb, 0, 5, 0, 12, 13, 12);
        self.bricks_box(r, cb, 0, 5, 0, 1, 12, 12);
        self.bricks_box(r, cb, 11, 5, 0, 12, 12, 12);
        self.bricks_box(r, cb, 2, 5, 11, 4, 12, 12);
        self.bricks_box(r, cb, 8, 5, 11, 10, 12, 12);
        self.bricks_box(r, cb, 5, 9, 11, 7, 12, 12);
        self.bricks_box(r, cb, 2, 5, 0, 4, 12, 1);
        self.bricks_box(r, cb, 8, 5, 0, 10, 12, 1);
        self.bricks_box(r, cb, 5, 9, 0, 7, 12, 1);
        self.bricks_box(r, cb, 2, 11, 2, 10, 12, 10);
        let _ = b;
    }

    /// The fence-topped battlements of the castle entrance and stalk room.
    fn battlements(&self, r: &mut Region, cb: &BoundingBox, we: u16, ns: u16) {
        let b = &self.base;
        let mut i = 1;
        while i <= 11 {
            self.fill_box(r, cb, i, 10, 0, i, 11, 0, we);
            self.fill_box(r, cb, i, 10, 12, i, 11, 12, we);
            self.fill_box(r, cb, 0, 10, i, 0, 11, i, ns);
            self.fill_box(r, cb, 12, 10, i, 12, 11, i, ns);
            b.place_block(r, bricks(), i, 13, 0, cb);
            b.place_block(r, bricks(), i, 13, 12, cb);
            b.place_block(r, bricks(), 0, 13, i, cb);
            b.place_block(r, bricks(), 12, 13, i, cb);
            if i != 11 {
                b.place_block(r, we, i + 1, 13, 0, cb);
                b.place_block(r, we, i + 1, 13, 12, cb);
                b.place_block(r, ns, 0, 13, i + 1, cb);
                b.place_block(r, ns, 12, 13, i + 1, cb);
            }
            i += 2;
        }
        b.place_block(r, fence(&["north", "east"]), 0, 13, 0, cb);
        b.place_block(r, fence(&["south", "east"]), 0, 13, 12, cb);
        b.place_block(r, fence(&["south", "west"]), 12, 13, 12, cb);
        b.place_block(r, fence(&["north", "west"]), 12, 13, 0, cb);
    }

    /// The floor cross and its supports under the castle entrance and stalk room.
    fn castle_floor(&self, r: &mut Region, cb: &BoundingBox) {
        self.bricks_box(r, cb, 4, 2, 0, 8, 2, 12);
        self.bricks_box(r, cb, 0, 2, 4, 12, 2, 8);
        self.bricks_box(r, cb, 4, 0, 0, 8, 1, 3);
        self.bricks_box(r, cb, 4, 0, 9, 8, 1, 12);
        self.bricks_box(r, cb, 0, 0, 4, 3, 1, 8);
        self.bricks_box(r, cb, 9, 0, 4, 12, 1, 8);
        self.castle_legs(r, cb);
    }
}

impl Piece for FPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        match &self.part {
            Part::EndFiller { seed } => tag.push(("Seed".into(), Tag::Int(*seed))),
            Part::MonsterThrone { spawner } => tag.push(("Mob".into(), bool_tag(spawner.load(Ordering::Relaxed)))),
            Part::LeftTurn { chest } | Part::RightTurn { chest } => tag.push(("Chest".into(), bool_tag(chest.load(Ordering::Relaxed)))),
            _ => {}
        }
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        let b = &self.base;
        let we = fence(&["west", "east"]);
        let ns = fence(&["north", "south"]);
        match &self.part {
            Part::BridgeCrossing => {
                self.bricks_box(r, cb, 7, 3, 0, 11, 4, 18);
                self.bricks_box(r, cb, 0, 3, 7, 18, 4, 11);
                self.air_box(r, cb, 8, 5, 0, 10, 7, 18);
                self.air_box(r, cb, 0, 5, 8, 18, 7, 10);
                self.bricks_box(r, cb, 7, 5, 0, 7, 5, 7);
                self.bricks_box(r, cb, 7, 5, 11, 7, 5, 18);
                self.bricks_box(r, cb, 11, 5, 0, 11, 5, 7);
                self.bricks_box(r, cb, 11, 5, 11, 11, 5, 18);
                self.bricks_box(r, cb, 0, 5, 7, 7, 5, 7);
                self.bricks_box(r, cb, 11, 5, 7, 18, 5, 7);
                self.bricks_box(r, cb, 0, 5, 11, 7, 5, 11);
                self.bricks_box(r, cb, 11, 5, 11, 18, 5, 11);
                self.bricks_box(r, cb, 7, 2, 0, 11, 2, 5);
                self.bricks_box(r, cb, 7, 2, 13, 11, 2, 18);
                self.bricks_box(r, cb, 7, 0, 0, 11, 1, 3);
                self.bricks_box(r, cb, 7, 0, 15, 11, 1, 18);
                self.bridge_legs(r, cb, 7, 11, 18);
                self.bricks_box(r, cb, 0, 2, 7, 5, 2, 11);
                self.bricks_box(r, cb, 13, 2, 7, 18, 2, 11);
                self.bricks_box(r, cb, 0, 0, 7, 3, 1, 11);
                self.bricks_box(r, cb, 15, 0, 7, 18, 1, 11);
                for x in 0..=2 {
                    for z in 7..=11 {
                        self.column(r, cb, x, z);
                        self.column(r, cb, 18 - x, z);
                    }
                }
            }
            Part::BridgeStraight => {
                self.bricks_box(r, cb, 0, 3, 0, 4, 4, 18);
                self.air_box(r, cb, 1, 5, 0, 3, 7, 18);
                self.bricks_box(r, cb, 0, 5, 0, 0, 5, 18);
                self.bricks_box(r, cb, 4, 5, 0, 4, 5, 18);
                self.bricks_box(r, cb, 0, 2, 0, 4, 2, 5);
                self.bricks_box(r, cb, 0, 2, 13, 4, 2, 18);
                self.bricks_box(r, cb, 0, 0, 0, 4, 1, 3);
                self.bricks_box(r, cb, 0, 0, 15, 4, 1, 18);
                self.bridge_legs(r, cb, 0, 4, 18);
                let e = fence(&["north", "south", "east"]);
                let w = fence(&["north", "south", "west"]);
                self.fill_box(r, cb, 0, 1, 1, 0, 4, 1, e);
                self.fill_box(r, cb, 0, 3, 4, 0, 4, 4, e);
                self.fill_box(r, cb, 0, 3, 14, 0, 4, 14, e);
                self.fill_box(r, cb, 0, 1, 17, 0, 4, 17, e);
                self.fill_box(r, cb, 4, 1, 1, 4, 4, 1, w);
                self.fill_box(r, cb, 4, 3, 4, 4, 4, 4, w);
                self.fill_box(r, cb, 4, 3, 14, 4, 4, 14, w);
                self.fill_box(r, cb, 4, 1, 17, 4, 4, 17, w);
            }
            Part::EndFiller { seed } => {
                // `RandomSource.createThreadLocalInstance(seed)`: the legacy LCG.
                let mut rnd = kiln_javamath::random::LegacyRandom::new(*seed as i64);
                for x in 0..=4 {
                    for y in 3..=4 {
                        let z = rnd.next_int_bounded(8);
                        self.bricks_box(r, cb, x, y, 0, x, y, z);
                    }
                }
                let z = rnd.next_int_bounded(8);
                self.bricks_box(r, cb, 0, 5, 0, 0, 5, z);
                let z = rnd.next_int_bounded(8);
                self.bricks_box(r, cb, 4, 5, 0, 4, 5, z);
                for x in 0..=4 {
                    let z = rnd.next_int_bounded(5);
                    self.bricks_box(r, cb, x, 2, 0, x, 2, z);
                }
                for x in 0..=4 {
                    for y in 0..=1 {
                        let z = rnd.next_int_bounded(3);
                        self.bricks_box(r, cb, x, y, 0, x, y, z);
                    }
                }
            }
            Part::RoomCrossing => {
                self.bricks_box(r, cb, 0, 0, 0, 6, 1, 6);
                self.air_box(r, cb, 0, 2, 0, 6, 7, 6);
                self.bricks_box(r, cb, 0, 2, 0, 1, 6, 0);
                self.bricks_box(r, cb, 0, 2, 6, 1, 6, 6);
                self.bricks_box(r, cb, 5, 2, 0, 6, 6, 0);
                self.bricks_box(r, cb, 5, 2, 6, 6, 6, 6);
                self.bricks_box(r, cb, 0, 2, 0, 0, 6, 1);
                self.bricks_box(r, cb, 0, 2, 5, 0, 6, 6);
                self.bricks_box(r, cb, 6, 2, 0, 6, 6, 1);
                self.bricks_box(r, cb, 6, 2, 5, 6, 6, 6);
                self.bricks_box(r, cb, 2, 6, 0, 4, 6, 0);
                self.fill_box(r, cb, 2, 5, 0, 4, 5, 0, we);
                self.bricks_box(r, cb, 2, 6, 6, 4, 6, 6);
                self.fill_box(r, cb, 2, 5, 6, 4, 5, 6, we);
                self.bricks_box(r, cb, 0, 6, 2, 0, 6, 4);
                self.fill_box(r, cb, 0, 5, 2, 0, 5, 4, ns);
                self.bricks_box(r, cb, 6, 6, 2, 6, 6, 4);
                self.fill_box(r, cb, 6, 5, 2, 6, 5, 4, ns);
                self.solid_legs(r, cb, 6, 6);
            }
            Part::StairsRoom => {
                self.bricks_box(r, cb, 0, 0, 0, 6, 1, 6);
                self.air_box(r, cb, 0, 2, 0, 6, 10, 6);
                self.bricks_box(r, cb, 0, 2, 0, 1, 8, 0);
                self.bricks_box(r, cb, 5, 2, 0, 6, 8, 0);
                self.bricks_box(r, cb, 0, 2, 1, 0, 8, 6);
                self.bricks_box(r, cb, 6, 2, 1, 6, 8, 6);
                self.bricks_box(r, cb, 1, 2, 6, 5, 8, 6);
                self.fill_box(r, cb, 0, 3, 2, 0, 5, 4, ns);
                self.fill_box(r, cb, 6, 3, 2, 6, 5, 2, ns);
                self.fill_box(r, cb, 6, 3, 4, 6, 5, 4, ns);
                b.place_block(r, bricks(), 5, 2, 5, cb);
                self.bricks_box(r, cb, 4, 2, 5, 4, 3, 5);
                self.bricks_box(r, cb, 3, 2, 5, 3, 4, 5);
                self.bricks_box(r, cb, 2, 2, 5, 2, 5, 5);
                self.bricks_box(r, cb, 1, 2, 5, 1, 6, 5);
                self.bricks_box(r, cb, 1, 7, 1, 5, 7, 4);
                self.air_box(r, cb, 6, 8, 2, 6, 8, 4);
                self.bricks_box(r, cb, 2, 6, 0, 4, 8, 0);
                self.fill_box(r, cb, 2, 5, 0, 4, 5, 0, we);
                self.solid_legs(r, cb, 6, 6);
            }
            Part::MonsterThrone { spawner } => {
                self.air_box(r, cb, 0, 2, 0, 6, 7, 7);
                self.bricks_box(r, cb, 1, 0, 0, 5, 1, 7);
                self.bricks_box(r, cb, 1, 2, 1, 5, 2, 7);
                self.bricks_box(r, cb, 1, 3, 2, 5, 3, 7);
                self.bricks_box(r, cb, 1, 4, 3, 5, 4, 7);
                self.bricks_box(r, cb, 1, 2, 0, 1, 4, 2);
                self.bricks_box(r, cb, 5, 2, 0, 5, 4, 2);
                self.bricks_box(r, cb, 1, 5, 2, 1, 5, 3);
                self.bricks_box(r, cb, 5, 5, 2, 5, 5, 3);
                self.bricks_box(r, cb, 0, 5, 3, 0, 5, 8);
                self.bricks_box(r, cb, 6, 5, 3, 6, 5, 8);
                self.bricks_box(r, cb, 1, 5, 8, 5, 5, 8);
                b.place_block(r, fence(&["west"]), 1, 6, 3, cb);
                b.place_block(r, fence(&["east"]), 5, 6, 3, cb);
                b.place_block(r, fence(&["east", "north"]), 0, 6, 3, cb);
                b.place_block(r, fence(&["west", "north"]), 6, 6, 3, cb);
                self.fill_box(r, cb, 0, 6, 4, 0, 6, 7, ns);
                self.fill_box(r, cb, 6, 6, 4, 6, 6, 7, ns);
                b.place_block(r, fence(&["east", "south"]), 0, 6, 8, cb);
                b.place_block(r, fence(&["west", "south"]), 6, 6, 8, cb);
                self.fill_box(r, cb, 1, 6, 8, 5, 6, 8, we);
                b.place_block(r, fence(&["east"]), 1, 7, 8, cb);
                self.fill_box(r, cb, 2, 7, 8, 4, 7, 8, we);
                b.place_block(r, fence(&["west"]), 5, 7, 8, cb);
                b.place_block(r, fence(&["east"]), 2, 8, 8, cb);
                b.place_block(r, we, 3, 8, 8, cb);
                b.place_block(r, fence(&["west"]), 4, 8, 8, cb);
                if !spawner.load(Ordering::Relaxed) {
                    let p = b.world_pos(3, 5, 5);
                    if cb.is_inside(p) {
                        spawner.store(true, Ordering::Relaxed);
                        r.set(p, st("minecraft:spawner"), 2);
                        set_spawner_entity(r, p, "minecraft:blaze");
                    }
                }
                self.solid_legs(r, cb, 6, 6);
            }
            Part::CastleEntrance => {
                self.castle_shell(r, cb);
                self.fill_box(r, cb, 5, 8, 0, 7, 8, 0, st("minecraft:nether_brick_fence"));
                self.battlements(r, cb, we, ns);
                let mut z = 3;
                while z <= 9 {
                    self.fill_box(r, cb, 1, 7, z, 1, 8, z, fence(&["north", "south", "west"]));
                    self.fill_box(r, cb, 11, 7, z, 11, 8, z, fence(&["north", "south", "east"]));
                    z += 2;
                }
                self.castle_floor(r, cb);
                self.bricks_box(r, cb, 5, 5, 5, 7, 5, 7);
                self.air_box(r, cb, 6, 1, 6, 6, 4, 6);
                b.place_block(r, bricks(), 6, 0, 6, cb);
                b.place_block(r, state::LAVA, 6, 5, 6, cb);
                let p = b.world_pos(6, 5, 6);
                if cb.is_inside(p) {
                    r.schedule_fluid_tick(p, "minecraft:lava", 0);
                }
            }
            Part::SmallCorridor => {
                self.bricks_box(r, cb, 0, 0, 0, 4, 1, 4);
                self.air_box(r, cb, 0, 2, 0, 4, 5, 4);
                self.bricks_box(r, cb, 0, 2, 0, 0, 5, 4);
                self.bricks_box(r, cb, 4, 2, 0, 4, 5, 4);
                self.fill_box(r, cb, 0, 3, 1, 0, 4, 1, ns);
                self.fill_box(r, cb, 0, 3, 3, 0, 4, 3, ns);
                self.fill_box(r, cb, 4, 3, 1, 4, 4, 1, ns);
                self.fill_box(r, cb, 4, 3, 3, 4, 4, 3, ns);
                self.bricks_box(r, cb, 0, 6, 0, 4, 6, 4);
                self.solid_legs(r, cb, 4, 4);
            }
            Part::SmallCorridorCrossing => {
                self.bricks_box(r, cb, 0, 0, 0, 4, 1, 4);
                self.air_box(r, cb, 0, 2, 0, 4, 5, 4);
                self.bricks_box(r, cb, 0, 2, 0, 0, 5, 0);
                self.bricks_box(r, cb, 4, 2, 0, 4, 5, 0);
                self.bricks_box(r, cb, 0, 2, 4, 0, 5, 4);
                self.bricks_box(r, cb, 4, 2, 4, 4, 5, 4);
                self.bricks_box(r, cb, 0, 6, 0, 4, 6, 4);
                self.solid_legs(r, cb, 4, 4);
            }
            Part::LeftTurn { chest } | Part::RightTurn { chest } => {
                let left = matches!(self.part, Part::LeftTurn { .. });
                self.bricks_box(r, cb, 0, 0, 0, 4, 1, 4);
                self.air_box(r, cb, 0, 2, 0, 4, 5, 4);
                if left {
                    self.bricks_box(r, cb, 4, 2, 0, 4, 5, 4);
                    self.fill_box(r, cb, 4, 3, 1, 4, 4, 1, ns);
                    self.fill_box(r, cb, 4, 3, 3, 4, 4, 3, ns);
                    self.bricks_box(r, cb, 0, 2, 0, 0, 5, 0);
                    self.bricks_box(r, cb, 0, 2, 4, 3, 5, 4);
                } else {
                    self.bricks_box(r, cb, 0, 2, 0, 0, 5, 4);
                    self.fill_box(r, cb, 0, 3, 1, 0, 4, 1, ns);
                    self.fill_box(r, cb, 0, 3, 3, 0, 4, 3, ns);
                    self.bricks_box(r, cb, 4, 2, 0, 4, 5, 0);
                    self.bricks_box(r, cb, 1, 2, 4, 4, 5, 4);
                }
                self.fill_box(r, cb, 1, 3, 4, 1, 4, 4, we);
                self.fill_box(r, cb, 3, 3, 4, 3, 4, 4, we);
                let cx = if left { 3 } else { 1 };
                if chest.load(Ordering::Relaxed) && cb.is_inside(b.world_pos(cx, 2, 3)) {
                    chest.store(false, Ordering::Relaxed);
                    b.create_chest(r, cb, random, cx, 2, 3, "minecraft:chests/nether_bridge");
                }
                self.bricks_box(r, cb, 0, 6, 0, 4, 6, 4);
                self.solid_legs(r, cb, 4, 4);
            }
            Part::CorridorStairs => {
                let stairs = with(st("minecraft:nether_brick_stairs"), &[("facing", "south")]);
                for i in 0..=9 {
                    let lo = 1.max(7 - i);
                    let hi = (lo + 5).max(14 - i).min(13);
                    let z = i;
                    self.bricks_box(r, cb, 0, 0, z, 4, lo, z);
                    self.air_box(r, cb, 1, lo + 1, z, 3, hi - 1, z);
                    if i <= 6 {
                        b.place_block(r, stairs, 1, lo + 1, z, cb);
                        b.place_block(r, stairs, 2, lo + 1, z, cb);
                        b.place_block(r, stairs, 3, lo + 1, z, cb);
                    }
                    self.bricks_box(r, cb, 0, hi, z, 4, hi, z);
                    self.bricks_box(r, cb, 0, lo + 1, z, 0, hi - 1, z);
                    self.bricks_box(r, cb, 4, lo + 1, z, 4, hi - 1, z);
                    if i & 1 == 0 {
                        self.fill_box(r, cb, 0, lo + 2, z, 0, lo + 3, z, ns);
                        self.fill_box(r, cb, 4, lo + 2, z, 4, lo + 3, z, ns);
                    }
                    for x in 0..=4 {
                        self.column(r, cb, x, z);
                    }
                }
            }
            Part::TBalcony => {
                self.bricks_box(r, cb, 0, 0, 0, 8, 1, 8);
                self.air_box(r, cb, 0, 2, 0, 8, 5, 8);
                self.bricks_box(r, cb, 0, 6, 0, 8, 6, 5);
                self.bricks_box(r, cb, 0, 2, 0, 2, 5, 0);
                self.bricks_box(r, cb, 6, 2, 0, 8, 5, 0);
                self.fill_box(r, cb, 1, 3, 0, 1, 4, 0, we);
                self.fill_box(r, cb, 7, 3, 0, 7, 4, 0, we);
                self.bricks_box(r, cb, 0, 2, 4, 8, 2, 8);
                self.air_box(r, cb, 1, 1, 4, 2, 2, 4);
                self.air_box(r, cb, 6, 1, 4, 7, 2, 4);
                self.fill_box(r, cb, 1, 3, 8, 7, 3, 8, we);
                b.place_block(r, fence(&["east", "south"]), 0, 3, 8, cb);
                b.place_block(r, fence(&["west", "south"]), 8, 3, 8, cb);
                self.fill_box(r, cb, 0, 3, 6, 0, 3, 7, ns);
                self.fill_box(r, cb, 8, 3, 6, 8, 3, 7, ns);
                self.bricks_box(r, cb, 0, 3, 4, 0, 5, 5);
                self.bricks_box(r, cb, 8, 3, 4, 8, 5, 5);
                self.bricks_box(r, cb, 1, 3, 5, 2, 5, 5);
                self.bricks_box(r, cb, 6, 3, 5, 7, 5, 5);
                self.fill_box(r, cb, 1, 4, 5, 1, 5, 5, we);
                self.fill_box(r, cb, 7, 4, 5, 7, 5, 5, we);
                for z in 0..=5 {
                    for x in 0..=8 {
                        self.column(r, cb, x, z);
                    }
                }
            }
            Part::StalkRoom => {
                self.castle_shell(r, cb);
                let w = fence(&["north", "south", "west"]);
                let e = fence(&["north", "south", "east"]);
                self.battlements(r, cb, we, ns);
                let mut z = 3;
                while z <= 9 {
                    self.fill_box(r, cb, 1, 7, z, 1, 8, z, w);
                    self.fill_box(r, cb, 11, 7, z, 11, 8, z, e);
                    z += 2;
                }
                let north = with(st("minecraft:nether_brick_stairs"), &[("facing", "north")]);
                for i in 0..=6 {
                    let z = i + 4;
                    for x in 5..=7 {
                        b.place_block(r, north, x, 5 + i, z, cb);
                    }
                    if (5..=8).contains(&z) {
                        self.bricks_box(r, cb, 5, 5, z, 7, i + 4, z);
                    } else if (9..=10).contains(&z) {
                        self.bricks_box(r, cb, 5, 8, z, 7, i + 4, z);
                    }
                    if i >= 1 {
                        self.air_box(r, cb, 5, 6 + i, z, 7, 9 + i, z);
                    }
                }
                for x in 5..=7 {
                    b.place_block(r, north, x, 12, 11, cb);
                }
                self.fill_box(r, cb, 5, 6, 7, 5, 7, 7, e);
                self.fill_box(r, cb, 7, 6, 7, 7, 7, 7, w);
                self.air_box(r, cb, 5, 13, 12, 7, 13, 12);
                self.bricks_box(r, cb, 2, 5, 2, 3, 5, 3);
                self.bricks_box(r, cb, 2, 5, 9, 3, 5, 10);
                self.bricks_box(r, cb, 2, 5, 4, 2, 5, 8);
                self.bricks_box(r, cb, 9, 5, 2, 10, 5, 3);
                self.bricks_box(r, cb, 9, 5, 9, 10, 5, 10);
                self.bricks_box(r, cb, 10, 5, 4, 10, 5, 8);
                let east = with(north, &[("facing", "east")]);
                let west = with(north, &[("facing", "west")]);
                b.place_block(r, west, 4, 5, 2, cb);
                b.place_block(r, west, 4, 5, 3, cb);
                b.place_block(r, west, 4, 5, 9, cb);
                b.place_block(r, west, 4, 5, 10, cb);
                b.place_block(r, east, 8, 5, 2, cb);
                b.place_block(r, east, 8, 5, 3, cb);
                b.place_block(r, east, 8, 5, 9, cb);
                b.place_block(r, east, 8, 5, 10, cb);
                let (sand, wart) = (st("minecraft:soul_sand"), st("minecraft:nether_wart"));
                self.fill_box(r, cb, 3, 4, 4, 4, 4, 8, sand);
                self.fill_box(r, cb, 8, 4, 4, 9, 4, 8, sand);
                self.fill_box(r, cb, 3, 5, 4, 4, 5, 8, wart);
                self.fill_box(r, cb, 8, 5, 4, 9, 5, 8, wart);
                self.castle_floor(r, cb);
            }
        }
    }
}
