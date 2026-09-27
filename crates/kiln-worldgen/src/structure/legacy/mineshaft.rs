//! Mineshafts (`MineshaftStructure`, `MineshaftPieces`).

use super::{bool_tag, collides, move_below_sea_level, offset_vertically, set_spawner_entity, st, with};
use crate::block_facts::{Dir, Support, is_face_sturdy, is_instance};
use crate::blocks::{is_air, is_lava, same_block, state};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BiomeSet, Loader};
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext, is_replaceable_by_structures};
use crate::structure::{GenCtx, Stub};
use crate::Error;
use kiln_data::block_props::{liquid, solid_render};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const LOOT: &str = "minecraft:chests/abandoned_mineshaft";

/// `MineshaftStructure.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsType {
    Normal,
    Mesa,
}

impl MsType {
    fn wood(self) -> u16 {
        st(if self == MsType::Normal { "minecraft:oak_log" } else { "minecraft:dark_oak_log" })
    }

    fn planks(self) -> u16 {
        st(if self == MsType::Normal { "minecraft:oak_planks" } else { "minecraft:dark_oak_planks" })
    }

    fn fence(self) -> u16 {
        st(if self == MsType::Normal { "minecraft:oak_fence" } else { "minecraft:dark_oak_fence" })
    }

    /// `MineShaftPiece.canBeReplaced`: not the type's planks, log or fence, nor a chain.
    fn can_replace(self) -> fn(u16) -> bool {
        fn keep(s: u16, t: MsType) -> bool {
            !(same_block(s, t.planks()) || same_block(s, t.wood()) || same_block(s, t.fence()) || same_block(s, st("minecraft:iron_chain")))
        }
        match self {
            MsType::Normal => |s| keep(s, MsType::Normal),
            MsType::Mesa => |s| keep(s, MsType::Mesa),
        }
    }
}

/// `MineshaftStructure`.
pub struct Mineshaft {
    ty: MsType,
    blocking: Arc<BiomeSet>,
}

impl Mineshaft {
    pub fn parse(json: &Json, l: &Loader) -> Result<Self, Error> {
        let ty = match json.get("mineshaft_type").and_then(Json::as_str).unwrap_or("normal") {
            "normal" => MsType::Normal,
            "mesa" => MsType::Mesa,
            t => return Err(Error::Invalid(format!("unknown mineshaft type {t}"))),
        };
        let blocking = Arc::new(l.biomes(&Json::String("#minecraft:mineshaft_blocking".into()))?);
        Ok(Self { ty, blocking })
    }
}

impl Kind for Mineshaft {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        ctx.random.next_double();
        let (cx, cz) = (ctx.chunk.0 << 4, ctx.chunk.1 << 4);
        let cfg = Cfg { ty: self.ty, blocking: self.blocking.clone() };
        let room = MsPiece::room(&cfg, &mut ctx.random, cx + 2, cz + 2);
        let root = room.base.bbox;
        let mut list = vec![room];
        add_children(0, &mut list, &mut ctx.random, &cfg, root);
        let mut pieces: Vec<Box<dyn Piece>> = list.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect();
        let sea = ctx.generator.sea_level;
        let dy = if self.ty == MsType::Mesa {
            let c = crate::structure::pieces_bbox(&pieces)?.center();
            let h = ctx.first_free_height(c.x, c.z, Heightmap::WorldSurfaceWg);
            let y = if h <= sea { sea } else { ctx.random.next_int_between(sea, h) };
            let dy = y - c.y;
            offset_vertically(&mut pieces, dy);
            dy
        } else {
            let min_y = ctx.min_y();
            move_below_sea_level(&mut pieces, sea, min_y, &mut ctx.random, 10)
        };
        Some(Stub { pos: BlockPos::new(cx + 8, 50 + dy, cz), build: Box::new(move |_, out| *out = pieces) })
    }
}

struct Cfg {
    ty: MsType,
    blocking: Arc<BiomeSet>,
}

#[derive(Debug)]
enum Part {
    Room { entrances: Vec<BoundingBox> },
    Corridor { rails: bool, spider: bool, placed_spider: AtomicBool, sections: i32 },
    Crossing { dir: Dir, two_floors: bool },
    Stairs,
}

#[derive(Clone, Copy)]
enum Shape {
    Room,
    Corridor,
    Crossing(Dir, bool),
    Stairs,
}

impl Part {
    fn shape(&self) -> Shape {
        match self {
            Part::Room { .. } => Shape::Room,
            Part::Corridor { .. } => Shape::Corridor,
            Part::Crossing { dir, two_floors } => Shape::Crossing(*dir, *two_floors),
            Part::Stairs => Shape::Stairs,
        }
    }
}

/// `MineshaftPieces.MineShaftPiece` and its subclasses.
#[derive(Debug)]
struct MsPiece {
    base: PieceBase,
    ty: MsType,
    blocking: Arc<BiomeSet>,
    part: Part,
}

impl MsPiece {
    fn new(cfg: &Cfg, kind: &'static str, depth: i32, bbox: BoundingBox, part: Part) -> Self {
        let mut base = PieceBase::new(kind, depth, bbox);
        base.can_replace = Some(cfg.ty.can_replace());
        Self { base, ty: cfg.ty, blocking: cfg.blocking.clone(), part }
    }

    /// `MineShaftRoom(0, random, x, z, type)`.
    fn room(cfg: &Cfg, random: &mut WorldgenRandom, x: i32, z: i32) -> Self {
        let x1 = x + 7 + random.next_int_bounded(6);
        let y1 = 54 + random.next_int_bounded(6);
        let z1 = z + 7 + random.next_int_bounded(6);
        Self::new(cfg, "minecraft:msroom", 0, BoundingBox::new(x, 50, z, x1, y1, z1), Part::Room { entrances: Vec::new() })
    }
}

/// `MineshaftPieces.createRandomShaftPiece`.
fn create_random(list: &[MsPiece], random: &mut WorldgenRandom, cfg: &Cfg, x: i32, y: i32, z: i32, dir: Dir, depth: i32) -> Option<MsPiece> {
    let boxes = || list.iter().map(|p| &p.base.bbox);
    let roll = random.next_int_bounded(100);
    if roll >= 80 {
        let h = if random.next_int_bounded(4) == 0 { 6 } else { 2 };
        let b = match dir {
            Dir::South => BoundingBox::new(-1, 0, 0, 3, h, 4),
            Dir::West => BoundingBox::new(-4, 0, -1, 0, h, 3),
            Dir::East => BoundingBox::new(0, 0, -1, 4, h, 3),
            _ => BoundingBox::new(-1, 0, -4, 3, h, 0),
        }
        .moved(x, y, z);
        if collides(boxes(), &b) {
            return None;
        }
        let two_floors = b.y_span() > 3;
        return Some(MsPiece::new(cfg, "minecraft:mscrossing", depth, b, Part::Crossing { dir, two_floors }));
    }
    if roll >= 70 {
        let b = match dir {
            Dir::South => BoundingBox::new(0, -5, 0, 2, 2, 8),
            Dir::West => BoundingBox::new(-8, -5, 0, 0, 2, 2),
            Dir::East => BoundingBox::new(0, -5, 0, 8, 2, 2),
            _ => BoundingBox::new(0, -5, -8, 2, 2, 0),
        }
        .moved(x, y, z);
        if collides(boxes(), &b) {
            return None;
        }
        let mut p = MsPiece::new(cfg, "minecraft:msstairs", depth, b, Part::Stairs);
        p.base.set_orientation(Some(dir));
        return Some(p);
    }
    let mut n = random.next_int_bounded(3) + 2;
    while n > 0 {
        let len = n * 5;
        let b = match dir {
            Dir::South => BoundingBox::new(0, 0, 0, 2, 2, len - 1),
            Dir::West => BoundingBox::new(-(len - 1), 0, 0, 0, 2, 2),
            Dir::East => BoundingBox::new(0, 0, 0, len - 1, 2, 2),
            _ => BoundingBox::new(0, 0, -(len - 1), 2, 2, 0),
        }
        .moved(x, y, z);
        if collides(boxes(), &b) {
            n -= 1;
            continue;
        }
        let rails = random.next_int_bounded(3) == 0;
        let spider = !rails && random.next_int_bounded(23) == 0;
        let sections = if matches!(dir, Dir::North | Dir::South) { b.z_span() / 5 } else { b.x_span() / 5 };
        let part = Part::Corridor { rails, spider, placed_spider: AtomicBool::new(false), sections };
        let mut p = MsPiece::new(cfg, "minecraft:mscorridor", depth, b, part);
        p.base.set_orientation(Some(dir));
        return Some(p);
    }
    None
}

/// `MineshaftPieces.generateAndAddPiece`: adds a piece and its children depth-first; returns
/// the new piece's index.
#[allow(clippy::too_many_arguments)]
fn generate_and_add(list: &mut Vec<MsPiece>, random: &mut WorldgenRandom, cfg: &Cfg, root: BoundingBox, x: i32, y: i32, z: i32, dir: Dir, depth: i32) -> Option<usize> {
    if depth > 8 || (x - root.min_x).abs() > 80 || (z - root.min_z).abs() > 80 {
        return None;
    }
    let p = create_random(list, random, cfg, x, y, z, dir, depth + 1)?;
    list.push(p);
    let i = list.len() - 1;
    add_children(i, list, random, cfg, root);
    Some(i)
}

/// `addChildren` of the piece at `i`.
fn add_children(i: usize, list: &mut Vec<MsPiece>, random: &mut WorldgenRandom, cfg: &Cfg, root: BoundingBox) {
    let b = list[i].base.bbox;
    let depth = list[i].base.gen_depth;
    let add = |list: &mut Vec<MsPiece>, random: &mut WorldgenRandom, x: i32, y: i32, z: i32, dir: Dir, depth: i32| {
        generate_and_add(list, random, cfg, root, x, y, z, dir, depth).map(|j| list[j].base.bbox)
    };
    let kind = list[i].part.shape();
    match kind {
        Shape::Room => {
            let y_room = (b.y_span() - 3 - 1).max(1);
            let mut entrances = Vec::new();
            let mut side = |list: &mut Vec<MsPiece>, random: &mut WorldgenRandom, dir: Dir| {
                let span = if matches!(dir, Dir::North | Dir::South) { b.x_span() } else { b.z_span() };
                let mut o = 0;
                while o < span {
                    o += random.next_int_bounded(span);
                    if o + 3 > span {
                        break;
                    }
                    let y = b.min_y + random.next_int_bounded(y_room) + 1;
                    let (x, z) = match dir {
                        Dir::North => (b.min_x + o, b.min_z - 1),
                        Dir::South => (b.min_x + o, b.max_z + 1),
                        Dir::West => (b.min_x - 1, b.min_z + o),
                        _ => (b.max_x + 1, b.min_z + o),
                    };
                    if let Some(c) = add(list, random, x, y, z, dir, depth) {
                        entrances.push(match dir {
                            Dir::North => BoundingBox::new(c.min_x, c.min_y, b.min_z, c.max_x, c.max_y, b.min_z + 1),
                            Dir::South => BoundingBox::new(c.min_x, c.min_y, b.max_z - 1, c.max_x, c.max_y, b.max_z),
                            Dir::West => BoundingBox::new(b.min_x, c.min_y, c.min_z, b.min_x + 1, c.max_y, c.max_z),
                            _ => BoundingBox::new(b.max_x - 1, c.min_y, c.min_z, b.max_x, c.max_y, c.max_z),
                        });
                    }
                    o += 4;
                }
            };
            for dir in [Dir::North, Dir::South, Dir::West, Dir::East] {
                side(list, random, dir);
            }
            if let Part::Room { entrances: e } = &mut list[i].part {
                *e = entrances;
            }
        }
        Shape::Corridor => {
            let roll = random.next_int_bounded(4);
            let Some(o) = list[i].base.orientation() else { return };
            match o {
                Dir::South => {
                    if roll <= 1 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x, y, b.max_z + 1, o, depth);
                    } else if roll == 2 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x - 1, y, b.max_z - 3, Dir::West, depth);
                    } else {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.max_x + 1, y, b.max_z - 3, Dir::East, depth);
                    }
                }
                Dir::West => {
                    if roll <= 1 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x - 1, y, b.min_z, o, depth);
                    } else if roll == 2 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x, y, b.min_z - 1, Dir::North, depth);
                    } else {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x, y, b.max_z + 1, Dir::South, depth);
                    }
                }
                Dir::East => {
                    if roll <= 1 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.max_x + 1, y, b.min_z, o, depth);
                    } else if roll == 2 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.max_x - 3, y, b.min_z - 1, Dir::North, depth);
                    } else {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.max_x - 3, y, b.max_z + 1, Dir::South, depth);
                    }
                }
                _ => {
                    if roll <= 1 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x, y, b.min_z - 1, o, depth);
                    } else if roll == 2 {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.min_x - 1, y, b.min_z, Dir::West, depth);
                    } else {
                        let y = b.min_y - 1 + random.next_int_bounded(3);
                        add(list, random, b.max_x + 1, y, b.min_z, Dir::East, depth);
                    }
                }
            }
            if depth < 8 {
                if matches!(o, Dir::North | Dir::South) {
                    let mut z = b.min_z + 3;
                    while z + 3 <= b.max_z {
                        match random.next_int_bounded(5) {
                            0 => {
                                add(list, random, b.min_x - 1, b.min_y, z, Dir::West, depth + 1);
                            }
                            1 => {
                                add(list, random, b.max_x + 1, b.min_y, z, Dir::East, depth + 1);
                            }
                            _ => {}
                        }
                        z += 5;
                    }
                } else {
                    let mut x = b.min_x + 3;
                    while x + 3 <= b.max_x {
                        match random.next_int_bounded(5) {
                            0 => {
                                add(list, random, x, b.min_y, b.min_z - 1, Dir::North, depth + 1);
                            }
                            1 => {
                                add(list, random, x, b.min_y, b.max_z + 1, Dir::South, depth + 1);
                            }
                            _ => {}
                        }
                        x += 5;
                    }
                }
            }
        }
        Shape::Crossing(dir, two_floors) => {
            let (n, s, w, e) = (
                (b.min_x + 1, b.min_z - 1, Dir::North),
                (b.min_x + 1, b.max_z + 1, Dir::South),
                (b.min_x - 1, b.min_z + 1, Dir::West),
                (b.max_x + 1, b.min_z + 1, Dir::East),
            );
            let exits = match dir {
                Dir::South => [s, w, e],
                Dir::West => [n, s, w],
                Dir::East => [n, s, e],
                _ => [n, w, e],
            };
            for (x, z, d) in exits {
                add(list, random, x, b.min_y, z, d, depth);
            }
            if two_floors {
                for (x, z, d) in [n, w, e, s] {
                    if random.next_bool() {
                        add(list, random, x, b.min_y + 3 + 1, z, d, depth);
                    }
                }
            }
        }
        Shape::Stairs => {
            let (x, z, d) = match list[i].base.orientation() {
                Some(Dir::South) => (b.min_x, b.max_z + 1, Dir::South),
                Some(Dir::West) => (b.min_x - 1, b.min_z, Dir::West),
                Some(Dir::East) => (b.max_x + 1, b.min_z, Dir::East),
                Some(_) => (b.min_x, b.min_z - 1, Dir::North),
                None => return,
            };
            add(list, random, x, b.min_y, z, d, depth);
        }
    }
}

impl MsPiece {
    /// `isInInvalidLocation`: in a blocking biome, or liquid on the box's faces.
    fn invalid_location(&self, r: &mut Region, cb: &BoundingBox) -> bool {
        let b = &self.base.bbox;
        let (x0, y0, z0) = ((b.min_x - 1).max(cb.min_x), (b.min_y - 1).max(cb.min_y), (b.min_z - 1).max(cb.min_z));
        let (x1, y1, z1) = ((b.max_x + 1).min(cb.max_x), (b.max_y + 1).min(cb.max_y), (b.max_z + 1).min(cb.max_z));
        let center = BlockPos::new((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2);
        if self.blocking.contains(r.biome(center)) {
            return true;
        }
        let mut wet = |x: i32, y: i32, z: i32| liquid(r.get(BlockPos::new(x, y, z)));
        for x in x0..=x1 {
            for z in z0..=z1 {
                if wet(x, y0, z) || wet(x, y1, z) {
                    return true;
                }
            }
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                if wet(x, y, z0) || wet(x, y, z1) {
                    return true;
                }
            }
        }
        for z in z0..=z1 {
            for y in y0..=y1 {
                if wet(x0, y, z) || wet(x1, y, z) {
                    return true;
                }
            }
        }
        false
    }

    /// `setPlanksBlock`: fills under the floor where nothing sturdy is.
    fn set_planks(&self, r: &mut Region, cb: &BoundingBox, s: u16, x: i32, y: i32, z: i32) {
        if !self.base.is_interior(r, x, y, z, cb) {
            return;
        }
        let p = self.base.world_pos(x, y, z);
        if !is_face_sturdy(r.get(p), Dir::Up, Support::Full) {
            r.set(p, s, 2);
        }
    }

    fn air_box(&self, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        self.base.generate_box(r, cb, x0, y0, z0, x1, y1, z1, state::CAVE_AIR, state::CAVE_AIR, false);
    }

    fn place_room(&self, r: &mut Region, cb: &BoundingBox, entrances: &[BoundingBox]) {
        let b = self.base.bbox;
        self.air_box(r, cb, b.min_x, b.min_y + 1, b.min_z, b.max_x, (b.min_y + 3).min(b.max_y), b.max_z);
        for e in entrances {
            self.air_box(r, cb, e.min_x, e.max_y - 2, e.min_z, e.max_x, e.max_y, e.max_z);
        }
        self.base.generate_upper_half_sphere(r, cb, b.min_x, b.min_y + 4, b.min_z, b.max_x, b.max_y, b.max_z, state::CAVE_AIR, false);
    }

    fn place_stairs(&self, r: &mut Region, cb: &BoundingBox) {
        self.air_box(r, cb, 0, 5, 0, 2, 7, 1);
        self.air_box(r, cb, 0, 0, 7, 2, 2, 8);
        for i in 0..5 {
            self.air_box(r, cb, 0, 5 - i - if i < 4 { 1 } else { 0 }, 2 + i, 2, 7 - i, 2 + i);
        }
    }

    fn place_crossing(&self, r: &mut Region, cb: &BoundingBox, two_floors: bool) {
        let b = self.base.bbox;
        let planks = self.ty.planks();
        if two_floors {
            self.air_box(r, cb, b.min_x + 1, b.min_y, b.min_z, b.max_x - 1, b.min_y + 3 - 1, b.max_z);
            self.air_box(r, cb, b.min_x, b.min_y, b.min_z + 1, b.max_x, b.min_y + 3 - 1, b.max_z - 1);
            self.air_box(r, cb, b.min_x + 1, b.max_y - 2, b.min_z, b.max_x - 1, b.max_y, b.max_z);
            self.air_box(r, cb, b.min_x, b.max_y - 2, b.min_z + 1, b.max_x, b.max_y, b.max_z - 1);
            self.air_box(r, cb, b.min_x + 1, b.min_y + 3, b.min_z + 1, b.max_x - 1, b.min_y + 3, b.max_z - 1);
        } else {
            self.air_box(r, cb, b.min_x + 1, b.min_y, b.min_z, b.max_x - 1, b.max_y, b.max_z);
            self.air_box(r, cb, b.min_x, b.min_y, b.min_z + 1, b.max_x, b.max_y, b.max_z - 1);
        }
        for (x, z) in [(b.min_x + 1, b.min_z + 1), (b.min_x + 1, b.max_z - 1), (b.max_x - 1, b.min_z + 1), (b.max_x - 1, b.max_z - 1)] {
            if !is_air(self.base.get_block(r, x, b.max_y + 1, z, cb)) {
                self.base.generate_box(r, cb, x, b.min_y, z, x, b.max_y, z, planks, state::CAVE_AIR, false);
            }
        }
        for x in b.min_x..=b.max_x {
            for z in b.min_z..=b.max_z {
                self.set_planks(r, cb, planks, x, b.min_y - 1, z);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn place_corridor(&self, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, rails: bool, spider: bool, placed_spider: &AtomicBool, sections: i32) {
        let base = &self.base;
        let len = sections * 5 - 1;
        let planks = self.ty.planks();
        self.air_box(r, cb, 0, 0, 0, 2, 1, len);
        base.generate_maybe_box(r, cb, random, 0.8, 0, 2, 0, 2, 2, len, state::CAVE_AIR, state::CAVE_AIR, false, false);
        if spider {
            base.generate_maybe_box(r, cb, random, 0.6, 0, 0, 0, 2, 1, len, st("minecraft:cobweb"), state::CAVE_AIR, false, true);
        }
        for i in 0..sections {
            let z = 2 + i * 5;
            self.place_support(r, cb, 0, 0, z, 2, 2, random);
            for (chance, x, dz) in [(0.1, 0, -1), (0.1, 2, -1), (0.1, 0, 1), (0.1, 2, 1), (0.05, 0, -2), (0.05, 2, -2), (0.05, 0, 2), (0.05, 2, 2)] {
                self.maybe_cobweb(r, cb, random, chance, x, 2, z + dz);
            }
            if random.next_int_bounded(100) == 0 {
                self.minecart_chest(r, cb, random, 2, 0, z - 1);
            }
            if random.next_int_bounded(100) == 0 {
                self.minecart_chest(r, cb, random, 0, 0, z + 1);
            }
            if spider && !placed_spider.load(Ordering::Relaxed) {
                let zz = z - 1 + random.next_int_bounded(3);
                let p = base.world_pos(1, 0, zz);
                if cb.is_inside(p) && base.is_interior(r, 1, 0, zz, cb) {
                    placed_spider.store(true, Ordering::Relaxed);
                    r.set(p, st("minecraft:spawner"), 2);
                    set_spawner_entity(r, p, "minecraft:cave_spider");
                }
            }
        }
        for x in 0..=2 {
            for z in 0..=len {
                self.set_planks(r, cb, planks, x, -1, z);
            }
        }
        self.double_support(r, cb, 0, -1, 2);
        if sections > 1 {
            self.double_support(r, cb, 0, -1, len - 2);
        }
        if rails {
            let rail = with(st("minecraft:rail"), &[("shape", "north_south")]);
            for z in 0..=len {
                let s = base.get_block(r, 1, -1, z, cb);
                if !is_air(s) && solid_render(s) {
                    let chance = if base.is_interior(r, 1, 0, z, cb) { 0.7 } else { 0.9 };
                    base.maybe_generate_block(r, cb, random, chance, 1, 0, z, rail);
                }
            }
        }
    }

    /// `MineShaftCorridor.placeSupport`.
    #[allow(clippy::too_many_arguments)]
    fn place_support(&self, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z: i32, y1: i32, x1: i32, random: &mut WorldgenRandom) {
        let base = &self.base;
        if (x0..=x1).any(|x| is_air(base.get_block(r, x, y1 + 1, z, cb))) {
            return;
        }
        let (planks, fence) = (self.ty.planks(), self.ty.fence());
        base.generate_box(r, cb, x0, y0, z, x0, y1 - 1, z, with(fence, &[("west", "true")]), state::CAVE_AIR, false);
        base.generate_box(r, cb, x1, y0, z, x1, y1 - 1, z, with(fence, &[("east", "true")]), state::CAVE_AIR, false);
        if random.next_int_bounded(4) == 0 {
            base.generate_box(r, cb, x0, y1, z, x0, y1, z, planks, state::CAVE_AIR, false);
            base.generate_box(r, cb, x1, y1, z, x1, y1, z, planks, state::CAVE_AIR, false);
        } else {
            base.generate_box(r, cb, x0, y1, z, x1, y1, z, planks, state::CAVE_AIR, false);
            let torch = st("minecraft:wall_torch");
            base.maybe_generate_block(r, cb, random, 0.05, x0 + 1, y1, z - 1, with(torch, &[("facing", "south")]));
            base.maybe_generate_block(r, cb, random, 0.05, x0 + 1, y1, z + 1, with(torch, &[("facing", "north")]));
        }
    }

    /// `maybePlaceCobWeb`.
    #[allow(clippy::too_many_arguments)]
    fn maybe_cobweb(&self, r: &mut Region, cb: &BoundingBox, random: &mut WorldgenRandom, chance: f32, x: i32, y: i32, z: i32) {
        if self.base.is_interior(r, x, y, z, cb) && random.next_float() < chance && self.sturdy_neighbours(r, cb, x, y, z, 2) {
            self.base.place_block(r, st("minecraft:cobweb"), x, y, z, cb);
        }
    }

    /// `hasSturdyNeighbours`.
    fn sturdy_neighbours(&self, r: &mut Region, cb: &BoundingBox, x: i32, y: i32, z: i32, needed: i32) -> bool {
        let p = self.base.world_pos(x, y, z);
        let mut count = 0;
        for d in Dir::ALL {
            let n = p.relative(d);
            if cb.is_inside(n) && is_face_sturdy(r.get(n), d.opposite(), Support::Full) {
                count += 1;
                if count >= needed {
                    return true;
                }
            }
        }
        false
    }

    /// `MineShaftCorridor.createChest`: a rail with a chest minecart on it. Kiln places the
    /// rail and draws the loot seed; the minecart entity is not spawned.
    fn minecart_chest(&self, r: &mut Region, cb: &BoundingBox, random: &mut WorldgenRandom, x: i32, y: i32, z: i32) -> bool {
        let p = self.base.world_pos(x, y, z);
        if cb.is_inside(p) && is_air(r.get(p)) && !is_air(r.get(p.below())) {
            let shape = if random.next_bool() { "north_south" } else { "east_west" };
            self.base.place_block(r, with(st("minecraft:rail"), &[("shape", shape)]), x, y, z, cb);
            let _loot_seed = (LOOT, random.next_long());
            return true;
        }
        false
    }

    /// `placeDoubleLowerOrUpperSupport`.
    fn double_support(&self, r: &mut Region, cb: &BoundingBox, x: i32, y: i32, z: i32) {
        let (wood, planks) = (self.ty.wood(), self.ty.planks());
        if same_block(self.base.get_block(r, x, y, z, cb), planks) {
            self.pillar_down_or_chain_up(r, cb, wood, x, y, z);
        }
        if same_block(self.base.get_block(r, x + 2, y, z, cb), planks) {
            self.pillar_down_or_chain_up(r, cb, wood, x + 2, y, z);
        }
    }

    /// `fillPillarDownOrChainUp`.
    fn pillar_down_or_chain_up(&self, r: &mut Region, cb: &BoundingBox, s: u16, x: i32, y: i32, z: i32) {
        let p = self.base.world_pos(x, y, z);
        if !cb.is_inside(p) {
            return;
        }
        let y0 = p.y;
        let (mut down, mut up) = (true, true);
        let mut i = 1;
        while down || up {
            if down {
                let q = p.at_y(y0 - i);
                let here = r.get(q);
                let open = is_replaceable_by_structures(here) && !is_lava(here);
                if !open && is_face_sturdy(here, Dir::Up, Support::Full) {
                    fill_between(r, s, q, y0 - i + 1, y0);
                    return;
                }
                down = i <= 20 && open && q.y > r.min_y() + 1;
            }
            if up {
                let q = p.at_y(y0 + i);
                let here = r.get(q);
                let open = is_replaceable_by_structures(here);
                if !open && is_face_sturdy(here, Dir::Down, Support::Center) && !is_instance(here, "FallingBlock") {
                    r.set(q.at_y(y0 + 1), self.ty.fence(), 2);
                    fill_between(r, st("minecraft:iron_chain"), q, y0 + 2, y0 + i);
                    return;
                }
                up = i <= 50 && open && q.y < r.max_y();
            }
            i += 1;
        }
    }
}

/// `fillColumnBetween`: `s` from `from` up to below `to`.
fn fill_between(r: &mut Region, s: u16, p: BlockPos, from: i32, to: i32) {
    for y in from..to {
        r.set(p.at_y(y), s, 2);
    }
}

impl Piece for MsPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        tag.push(("MST".into(), Tag::Int(self.ty as i32)));
        match &self.part {
            Part::Room { entrances } => tag.push(("Entrances".into(), Tag::List(entrances.iter().map(BoundingBox::to_tag).collect()))),
            Part::Corridor { rails, spider, placed_spider, sections } => {
                tag.push(("hr".into(), bool_tag(*rails)));
                tag.push(("sc".into(), bool_tag(*spider)));
                tag.push(("hps".into(), bool_tag(placed_spider.load(Ordering::Relaxed))));
                tag.push(("Num".into(), Tag::Int(*sections)));
            }
            Part::Crossing { dir, two_floors } => {
                tag.push(("tf".into(), bool_tag(*two_floors)));
                tag.push(("D".into(), Tag::Byte(dir.index_2d() as i8)));
            }
            Part::Stairs => {}
        }
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.shift(dx, dy, dz);
        if let Part::Room { entrances } = &mut self.part {
            for e in entrances {
                e.shift(dx, dy, dz);
            }
        }
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        if self.invalid_location(r, cb) {
            return;
        }
        match &self.part {
            Part::Room { entrances } => self.place_room(r, cb, entrances),
            Part::Corridor { rails, spider, placed_spider, sections } => {
                self.place_corridor(r, random, cb, *rails, *spider, placed_spider, *sections)
            }
            Part::Crossing { two_floors, .. } => self.place_crossing(r, cb, *two_floors),
            Part::Stairs => self.place_stairs(r, cb),
        }
    }
}
