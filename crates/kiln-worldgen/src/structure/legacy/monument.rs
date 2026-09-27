//! Ocean monuments (`OceanMonumentStructure`, `OceanMonumentPieces`): one saved building piece
//! whose rooms are rebuilt from the start's random and placed with it.

use super::st;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{same_block, state};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BiomeSet, Loader};
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

const BASE_GRAY: u16 = state::PRISMARINE;
const BASE_LIGHT: u16 = state::PRISMARINE_BRICKS;
const BASE_BLACK: u16 = state::DARK_PRISMARINE;
const DOT_DECO_DATA: u16 = BASE_LIGHT;
const LAMP_BLOCK: u16 = state::SEA_LANTERN;
const FILL_BLOCK: u16 = state::WATER;

/// `OceanMonumentStructure`.
pub struct OceanMonument {
    /// `#required_ocean_monument_surrounding`.
    surrounding: BiomeSet,
}

impl OceanMonument {
    pub fn parse(_json: &Json, l: &Loader) -> Result<Self, Error> {
        Ok(Self { surrounding: l.biomes(&Json::String("#minecraft:required_ocean_monument_surrounding".into()))? })
    }
}

impl Kind for OceanMonument {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let (x, z) = ((ctx.chunk.0 << 4) + 9, (ctx.chunk.1 << 4) + 9);
        let y = ctx.generator.sea_level;
        // `BiomeResolver.getBiomesWithin(x, seaLevel, z, 29)`: every biome, then the check.
        let mut all = true;
        for qz in (z - 29) >> 2..=(z + 29) >> 2 {
            for qx in (x - 29) >> 2..=(x + 29) >> 2 {
                for qy in (y - 29) >> 2..=(y + 29) >> 2 {
                    let b = ctx.biome(qx, qy, qz);
                    all &= self.surrounding.contains(b);
                }
            }
        }
        if !all {
            return None;
        }
        let pos = ctx.on_top_of_chunk_center(Heightmap::OceanFloorWg)?;
        let (x0, z0) = ((ctx.chunk.0 << 4) - 29, (ctx.chunk.1 << 4) - 29);
        Some(Stub {
            pos,
            build: Box::new(move |ctx, pieces| {
                let dir = PieceBase::random_horizontal(&mut ctx.random);
                pieces.push(Box::new(Building::new(&mut ctx.random, x0, z0, dir)));
            }),
        })
    }
}

/// `getRoomIndex(x, y, z)`.
fn room_index(x: i32, y: i32, z: i32) -> i32 {
    y * 25 + z * 5 + x
}

/// `OceanMonumentPieces.RoomDefinition`, in an arena.
#[derive(Clone, Debug)]
struct Def {
    index: i32,
    conn: [Option<usize>; 6],
    opening: [bool; 6],
    claimed: bool,
    is_source: bool,
    scan: i32,
}

impl Def {
    fn new(index: i32) -> Self {
        Self { index, conn: [None; 6], opening: [false; 6], claimed: false, is_source: false, scan: 0 }
    }

    fn has(&self, d: Dir) -> bool {
        self.opening[d as usize]
    }

    fn count_openings(&self) -> i32 {
        self.opening.iter().filter(|&&o| o).count() as i32
    }
}

fn set_connection(defs: &mut [Def], a: usize, d: Dir, b: usize) {
    defs[a].conn[d as usize] = Some(b);
    defs[b].conn[d.opposite() as usize] = Some(a);
}

fn update_openings(def: &mut Def) {
    for i in 0..6 {
        def.opening[i] = def.conn[i].is_some();
    }
}

/// `RoomDefinition.findSource`.
fn find_source(defs: &mut [Def], i: usize, scan: i32) -> bool {
    if defs[i].is_source {
        return true;
    }
    defs[i].scan = scan;
    for d in 0..6 {
        if let Some(c) = defs[i].conn[d]
            && defs[i].opening[d]
            && defs[c].scan != scan
            && find_source(defs, c, scan)
        {
            return true;
        }
    }
    false
}

/// `MonumentBuilding.generateRoomGraph`: the arena, the room order to fit, the source and
/// the core room.
fn room_graph(random: &mut WorldgenRandom) -> (Vec<Def>, Vec<usize>, usize, usize) {
    let mut defs: Vec<Def> = Vec::new();
    let mut grid: [Option<usize>; 75] = [None; 75];
    let mut add = |defs: &mut Vec<Def>, x: i32, y: i32, z: i32| {
        let i = room_index(x, y, z);
        defs.push(Def::new(i));
        grid[i as usize] = Some(defs.len() - 1);
    };
    for x in 0..5 {
        for z in 0..4 {
            add(&mut defs, x, 0, z);
        }
    }
    for x in 0..5 {
        for z in 0..4 {
            add(&mut defs, x, 1, z);
        }
    }
    for x in 1..4 {
        for z in 0..2 {
            add(&mut defs, x, 2, z);
        }
    }
    let source = grid[room_index(2, 0, 0) as usize].expect("source room");
    for x in 0..5 {
        for z in 0..5 {
            for y in 0..3 {
                let Some(a) = grid[room_index(x, y, z) as usize] else { continue };
                for d in Dir::ALL {
                    let (dx, dy, dz) = d.offset();
                    let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                    if !(0..5).contains(&nx) || !(0..5).contains(&nz) || !(0..3).contains(&ny) {
                        continue;
                    }
                    let Some(b) = grid[room_index(nx, ny, nz) as usize] else { continue };
                    if nz == z {
                        set_connection(&mut defs, a, d, b);
                    } else {
                        set_connection(&mut defs, a, d.opposite(), b);
                    }
                }
            }
        }
    }
    let special = |defs: &mut Vec<Def>, index: i32| {
        defs.push(Def::new(index));
        defs.len() - 1
    };
    let penthouse = special(&mut defs, 1003);
    let left = special(&mut defs, 1001);
    let right = special(&mut defs, 1002);
    set_connection(&mut defs, grid[room_index(2, 2, 0) as usize].unwrap(), Dir::Up, penthouse);
    set_connection(&mut defs, grid[room_index(0, 1, 0) as usize].unwrap(), Dir::South, left);
    set_connection(&mut defs, grid[room_index(4, 1, 0) as usize].unwrap(), Dir::South, right);
    for i in [penthouse, left, right] {
        defs[i].claimed = true;
    }
    defs[source].is_source = true;
    let core = grid[room_index(random.next_int_bounded(4), 0, 2) as usize].expect("core room");
    let go = |defs: &[Def], i: usize, path: &[Dir]| path.iter().fold(i, |i, d| defs[i].conn[*d as usize].expect("core neighbours"));
    for path in [
        &[][..],
        &[Dir::East],
        &[Dir::North],
        &[Dir::East, Dir::North],
        &[Dir::Up],
        &[Dir::East, Dir::Up],
        &[Dir::North, Dir::Up],
        &[Dir::East, Dir::North, Dir::Up],
    ] {
        let i = go(&defs, core, path);
        defs[i].claimed = true;
    }
    let mut list: Vec<usize> = Vec::new();
    for g in grid.iter().flatten() {
        update_openings(&mut defs[*g]);
        list.push(*g);
    }
    update_openings(&mut defs[penthouse]);
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        list.swap(i - 1, j);
    }
    let mut scan = 1;
    for &i in &list {
        let (mut done, mut tries) = (0, 0);
        while done < 2 && tries < 5 {
            tries += 1;
            let d = random.next_int_bounded(6) as usize;
            if !defs[i].opening[d] {
                continue;
            }
            let od = Dir::from_index(d).opposite() as usize;
            let c = defs[i].conn[d].expect("opening without room");
            defs[i].opening[d] = false;
            defs[c].opening[od] = false;
            scan += 1;
            let ok = find_source(&mut defs, i, scan) && {
                scan += 1;
                find_source(&mut defs, c, scan)
            };
            if ok {
                done += 1;
            } else {
                defs[i].opening[d] = true;
                defs[c].opening[od] = true;
            }
        }
    }
    list.extend([penthouse, left, right]);
    (defs, list, source, core)
}

/// Room shapes (`OceanMonumentPiece` subclasses); `usize` fields are the room definition.
#[derive(Clone, Copy, Debug)]
enum Room {
    Entry(usize),
    Core,
    Simple(usize, i32),
    SimpleTop(usize),
    DoubleX(usize),
    DoubleY(usize),
    DoubleZ(usize),
    DoubleXY(usize),
    DoubleYZ(usize),
    Wing(i32),
    Penthouse,
}

#[derive(Debug)]
struct Child {
    base: PieceBase,
    room: Room,
}

/// `OceanMonumentPiece(type, genDepth 1, dir, def, w, h, d)`.
#[allow(clippy::too_many_arguments)]
fn room_piece(kind: &'static str, dir: Dir, defs: &[Def], def: usize, w: i32, h: i32, d: i32, room: Room) -> Child {
    let idx = defs[def].index;
    let (x, z, y) = (idx % 5, (idx / 5) % 5, idx / 25);
    let mut b = PieceBase::make_bbox(0, 0, 0, dir, w * 8, h * 4, d * 8);
    match dir {
        Dir::North => b.shift(x * 8, y * 4, -(z + d) * 8 + 1),
        Dir::South => b.shift(x * 8, y * 4, z * 8),
        Dir::West => b.shift(-(z + d) * 8 + 1, y * 4, x * 8),
        _ => b.shift(z * 8, y * 4, x * 8),
    }
    let mut base = PieceBase::new(kind, 1, b);
    base.set_orientation(Some(dir));
    Child { base, room }
}

/// `MonumentRoomFitter`s in vanilla's order: the room fitted to `def`, if any.
fn fit(defs: &mut [Def], dir: Dir, def: usize, random: &mut WorldgenRandom) -> Option<Child> {
    let free = |defs: &[Def], i: usize, d: Dir| defs[i].has(d) && !defs[defs[i].conn[d as usize].unwrap()].claimed;
    let next = |defs: &[Def], i: usize, d: Dir| defs[i].conn[d as usize].unwrap();
    // XY
    if free(defs, def, Dir::East) && free(defs, def, Dir::Up) && free(defs, next(defs, def, Dir::East), Dir::Up) {
        let (e, u) = (next(defs, def, Dir::East), next(defs, def, Dir::Up));
        let eu = next(defs, e, Dir::Up);
        for i in [def, e, u, eu] {
            defs[i].claimed = true;
        }
        return Some(room_piece("minecraft:omdxyr", dir, defs, def, 2, 2, 1, Room::DoubleXY(def)));
    }
    // YZ
    if free(defs, def, Dir::North) && free(defs, def, Dir::Up) && free(defs, next(defs, def, Dir::North), Dir::Up) {
        let (n, u) = (next(defs, def, Dir::North), next(defs, def, Dir::Up));
        let nu = next(defs, n, Dir::Up);
        for i in [def, n, u, nu] {
            defs[i].claimed = true;
        }
        return Some(room_piece("minecraft:omdyzr", dir, defs, def, 1, 2, 2, Room::DoubleYZ(def)));
    }
    // Z
    if free(defs, def, Dir::North) {
        let at = if free(defs, def, Dir::North) { def } else { next(defs, def, Dir::South) };
        let n = next(defs, at, Dir::North);
        defs[at].claimed = true;
        defs[n].claimed = true;
        return Some(room_piece("minecraft:omdzr", dir, defs, at, 1, 1, 2, Room::DoubleZ(at)));
    }
    // X
    if free(defs, def, Dir::East) {
        let e = next(defs, def, Dir::East);
        defs[def].claimed = true;
        defs[e].claimed = true;
        return Some(room_piece("minecraft:omdxr", dir, defs, def, 2, 1, 1, Room::DoubleX(def)));
    }
    // Y
    if free(defs, def, Dir::Up) {
        let u = next(defs, def, Dir::Up);
        defs[def].claimed = true;
        defs[u].claimed = true;
        return Some(room_piece("minecraft:omdyr", dir, defs, def, 1, 2, 1, Room::DoubleY(def)));
    }
    // SimpleTop
    if ![Dir::West, Dir::East, Dir::North, Dir::South, Dir::Up].iter().any(|d| defs[def].has(*d)) {
        defs[def].claimed = true;
        return Some(room_piece("minecraft:omsimplet", dir, defs, def, 1, 1, 1, Room::SimpleTop(def)));
    }
    defs[def].claimed = true;
    let design = random.next_int_bounded(3);
    Some(room_piece("minecraft:omsimple", dir, defs, def, 1, 1, 1, Room::Simple(def, design)))
}

/// `OceanMonumentPieces.MonumentBuilding`.
#[derive(Debug)]
struct Building {
    base: PieceBase,
    defs: Vec<Def>,
    children: Vec<Child>,
}

impl Building {
    fn new(random: &mut WorldgenRandom, x: i32, z: i32, dir: Dir) -> Self {
        let mut base = PieceBase::new("minecraft:omb", 0, PieceBase::make_bbox(x, 39, z, dir, 58, 23, 58));
        base.set_orientation(Some(dir));
        let (mut defs, list, source, core) = room_graph(random);
        defs[source].claimed = true;
        let mut children = vec![
            room_piece("minecraft:omentry", dir, &defs, source, 1, 1, 1, Room::Entry(source)),
            room_piece("minecraft:omcr", dir, &defs, core, 2, 2, 2, Room::Core),
        ];
        for &i in &list {
            if !defs[i].claimed && defs[i].index < 75
                && let Some(c) = fit(&mut defs, dir, i, random)
            {
                children.push(c);
            }
        }
        let o = base.world_pos(9, 0, 22);
        for c in &mut children {
            c.base.bbox.shift(o.x, o.y, o.z);
        }
        let wing_box = |a: (i32, i32, i32), b: (i32, i32, i32)| BoundingBox::from_corners(base.world_pos(a.0, a.1, a.2), base.world_pos(b.0, b.1, b.2));
        let (w1, w2, pent) = (wing_box((1, 1, 1), (23, 8, 21)), wing_box((34, 1, 1), (56, 8, 21)), wing_box((22, 13, 22), (35, 17, 35)));
        let n = random.next_int();
        let other = |kind: &'static str, bbox: BoundingBox, room: Room| {
            let mut base = PieceBase::new(kind, 1, bbox);
            base.set_orientation(Some(dir));
            Child { base, room }
        };
        children.push(other("minecraft:omwr", w1, Room::Wing(n & 1)));
        children.push(other("minecraft:omwr", w2, Room::Wing(n.wrapping_add(1) & 1)));
        children.push(other("minecraft:ompenthouse", pent, Room::Penthouse));
        Self { base, defs, children }
    }

    fn chunk_intersects(&self, cb: &BoundingBox, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        chunk_intersects(&self.base, cb, x0, z0, x1, z1)
    }

    fn wing(&self, r: &mut Region, cb: &BoundingBox, wing: bool, v2: i32) {
        let b = &self.base;
        if !self.chunk_intersects(cb, v2, 0, v2 + 23, 20) {
            return;
        }
        b.generate_box(r, cb, v2, 0, 0, v2 + 24, 0, 20, BASE_GRAY, BASE_GRAY, false);
        water_box(b, r, cb, v2, 1, 0, v2 + 24, 10, 20);
        for v7 in 0..4 {
            b.generate_box(r, cb, v2 + v7, v7 + 1, v7, v2 + v7, v7 + 1, 20, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, (v2 + v7) + 7, v7 + 5, v7 + 7, (v2 + v7) + 7, v7 + 5, 20, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, (v2 + 17) - v7, v7 + 5, v7 + 7, (v2 + 17) - v7, v7 + 5, 20, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, (v2 + 24) - v7, v7 + 1, v7, (v2 + 24) - v7, v7 + 1, 20, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, (v2 + v7) + 1, v7 + 1, v7, (v2 + 23) - v7, v7 + 1, v7, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, (v2 + v7) + 8, v7 + 5, v7 + 7, (v2 + 16) - v7, v7 + 5, v7 + 7, BASE_LIGHT, BASE_LIGHT, false);
        }
        b.generate_box(r, cb, v2 + 4, 4, 4, v2 + 6, 4, 20, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, v2 + 7, 4, 4, v2 + 17, 4, 6, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, v2 + 18, 4, 4, v2 + 20, 4, 20, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, v2 + 11, 8, 11, v2 + 13, 8, 20, BASE_GRAY, BASE_GRAY, false);
        b.place_block(r, DOT_DECO_DATA, v2 + 12, 9, 12, cb);
        b.place_block(r, DOT_DECO_DATA, v2 + 12, 9, 15, cb);
        b.place_block(r, DOT_DECO_DATA, v2 + 12, 9, 18, cb);
        let v7 = v2 + if wing { 19 } else { 5 };
        let v8 = v2 + if wing { 5 } else { 19 };
        for v9 in (5..=20).rev().step_by(3) {
            b.place_block(r, DOT_DECO_DATA, v7, 5, v9, cb);
        }
        for v9 in (7..=19).rev().step_by(3) {
            b.place_block(r, DOT_DECO_DATA, v8, 5, v9, cb);
        }
        for v9 in 0..4 {
            let v10 = if wing { v2 + 24 - (17 - v9 * 3) } else { v2 + 17 - v9 * 3 };
            b.place_block(r, DOT_DECO_DATA, v10, 5, 5, cb);
        }
        b.place_block(r, DOT_DECO_DATA, v8, 5, 5, cb);
        b.generate_box(r, cb, v2 + 11, 1, 12, v2 + 13, 7, 12, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, v2 + 12, 1, 11, v2 + 12, 7, 13, BASE_GRAY, BASE_GRAY, false);
    }

    fn entrance_archs(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        if !self.chunk_intersects(cb, 22, 5, 35, 17) {
            return;
        }
        water_box(b, r, cb, 25, 0, 0, 32, 8, 20);
        for v4 in 0..4 {
            b.generate_box(r, cb, 24, 2, 5 + (v4 * 4), 24, 4, 5 + (v4 * 4), BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, 22, 4, 5 + (v4 * 4), 23, 4, 5 + (v4 * 4), BASE_LIGHT, BASE_LIGHT, false);
            b.place_block(r, BASE_LIGHT, 25, 5, 5 + (v4 * 4), cb);
            b.place_block(r, BASE_LIGHT, 26, 6, 5 + (v4 * 4), cb);
            b.place_block(r, LAMP_BLOCK, 26, 5, 5 + (v4 * 4), cb);
            b.generate_box(r, cb, 33, 2, 5 + (v4 * 4), 33, 4, 5 + (v4 * 4), BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, 34, 4, 5 + (v4 * 4), 35, 4, 5 + (v4 * 4), BASE_LIGHT, BASE_LIGHT, false);
            b.place_block(r, BASE_LIGHT, 32, 5, 5 + (v4 * 4), cb);
            b.place_block(r, BASE_LIGHT, 31, 6, 5 + (v4 * 4), cb);
            b.place_block(r, LAMP_BLOCK, 31, 5, 5 + (v4 * 4), cb);
            b.generate_box(r, cb, 27, 6, 5 + (v4 * 4), 30, 6, 5 + (v4 * 4), BASE_GRAY, BASE_GRAY, false);
        }
    }

    fn entrance_wall(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        if !self.chunk_intersects(cb, 15, 20, 42, 21) {
            return;
        }
        b.generate_box(r, cb, 15, 0, 21, 42, 0, 21, BASE_GRAY, BASE_GRAY, false);
        water_box(b, r, cb, 26, 1, 21, 31, 3, 21);
        b.generate_box(r, cb, 21, 12, 21, 36, 12, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 17, 11, 21, 40, 11, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 16, 10, 21, 41, 10, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 15, 7, 21, 42, 9, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 16, 6, 21, 41, 6, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 17, 5, 21, 40, 5, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 21, 4, 21, 36, 4, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 22, 3, 21, 26, 3, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 31, 3, 21, 35, 3, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 23, 2, 21, 25, 2, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 32, 2, 21, 34, 2, 21, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 28, 4, 20, 29, 4, 21, BASE_LIGHT, BASE_LIGHT, false);
        b.place_block(r, BASE_LIGHT, 27, 3, 21, cb);
        b.place_block(r, BASE_LIGHT, 30, 3, 21, cb);
        b.place_block(r, BASE_LIGHT, 26, 2, 21, cb);
        b.place_block(r, BASE_LIGHT, 31, 2, 21, cb);
        b.place_block(r, BASE_LIGHT, 25, 1, 21, cb);
        b.place_block(r, BASE_LIGHT, 32, 1, 21, cb);
        for v4 in 0..7 {
            b.place_block(r, BASE_BLACK, 28 - v4, 6 + v4, 21, cb);
            b.place_block(r, BASE_BLACK, 29 + v4, 6 + v4, 21, cb);
        }
        for v4 in 0..4 {
            b.place_block(r, BASE_BLACK, 28 - v4, 9 + v4, 21, cb);
            b.place_block(r, BASE_BLACK, 29 + v4, 9 + v4, 21, cb);
        }
        b.place_block(r, BASE_BLACK, 28, 12, 21, cb);
        b.place_block(r, BASE_BLACK, 29, 12, 21, cb);
        for v4 in 0..3 {
            b.place_block(r, BASE_BLACK, 22 - (v4 * 2), 8, 21, cb);
            b.place_block(r, BASE_BLACK, 22 - (v4 * 2), 9, 21, cb);
            b.place_block(r, BASE_BLACK, 35 + (v4 * 2), 8, 21, cb);
            b.place_block(r, BASE_BLACK, 35 + (v4 * 2), 9, 21, cb);
        }
        water_box(b, r, cb, 15, 13, 21, 42, 15, 21);
        water_box(b, r, cb, 15, 1, 21, 15, 6, 21);
        water_box(b, r, cb, 16, 1, 21, 16, 5, 21);
        water_box(b, r, cb, 17, 1, 21, 20, 4, 21);
        water_box(b, r, cb, 21, 1, 21, 21, 3, 21);
        water_box(b, r, cb, 22, 1, 21, 22, 2, 21);
        water_box(b, r, cb, 23, 1, 21, 24, 1, 21);
        water_box(b, r, cb, 42, 1, 21, 42, 6, 21);
        water_box(b, r, cb, 41, 1, 21, 41, 5, 21);
        water_box(b, r, cb, 37, 1, 21, 40, 4, 21);
        water_box(b, r, cb, 36, 1, 21, 36, 3, 21);
        water_box(b, r, cb, 33, 1, 21, 34, 1, 21);
        water_box(b, r, cb, 35, 1, 21, 35, 2, 21);
    }

    fn roof(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        if !self.chunk_intersects(cb, 21, 21, 36, 36) {
            return;
        }
        b.generate_box(r, cb, 21, 0, 22, 36, 0, 36, BASE_GRAY, BASE_GRAY, false);
        water_box(b, r, cb, 21, 1, 22, 36, 23, 36);
        for v4 in 0..4 {
            b.generate_box(r, cb, 21 + v4, 13 + v4, 21 + v4, 36 - v4, 13 + v4, 21 + v4, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, 21 + v4, 13 + v4, 36 - v4, 36 - v4, 13 + v4, 36 - v4, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, 21 + v4, 13 + v4, 22 + v4, 21 + v4, 13 + v4, 35 - v4, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, 36 - v4, 13 + v4, 22 + v4, 36 - v4, 13 + v4, 35 - v4, BASE_LIGHT, BASE_LIGHT, false);
        }
        b.generate_box(r, cb, 25, 16, 25, 32, 16, 32, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 25, 17, 25, 25, 19, 25, BASE_LIGHT, BASE_LIGHT, false);
        b.generate_box(r, cb, 32, 17, 25, 32, 19, 25, BASE_LIGHT, BASE_LIGHT, false);
        b.generate_box(r, cb, 25, 17, 32, 25, 19, 32, BASE_LIGHT, BASE_LIGHT, false);
        b.generate_box(r, cb, 32, 17, 32, 32, 19, 32, BASE_LIGHT, BASE_LIGHT, false);
        b.place_block(r, BASE_LIGHT, 26, 20, 26, cb);
        b.place_block(r, BASE_LIGHT, 27, 21, 27, cb);
        b.place_block(r, LAMP_BLOCK, 27, 20, 27, cb);
        b.place_block(r, BASE_LIGHT, 26, 20, 31, cb);
        b.place_block(r, BASE_LIGHT, 27, 21, 30, cb);
        b.place_block(r, LAMP_BLOCK, 27, 20, 30, cb);
        b.place_block(r, BASE_LIGHT, 31, 20, 31, cb);
        b.place_block(r, BASE_LIGHT, 30, 21, 30, cb);
        b.place_block(r, LAMP_BLOCK, 30, 20, 30, cb);
        b.place_block(r, BASE_LIGHT, 31, 20, 26, cb);
        b.place_block(r, BASE_LIGHT, 30, 21, 27, cb);
        b.place_block(r, LAMP_BLOCK, 30, 20, 27, cb);
        b.generate_box(r, cb, 28, 21, 27, 29, 21, 27, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 27, 21, 28, 27, 21, 29, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 28, 21, 30, 29, 21, 30, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, 30, 21, 28, 30, 21, 29, BASE_GRAY, BASE_GRAY, false);
    }

    fn lower_wall(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        if self.chunk_intersects(cb, 0, 21, 6, 58) {
            b.generate_box(r, cb, 0, 0, 21, 6, 0, 57, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 0, 1, 21, 6, 7, 57);
            b.generate_box(r, cb, 4, 4, 21, 6, 4, 53, BASE_GRAY, BASE_GRAY, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, v4, v4 + 1, 21, v4, v4 + 1, 57 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (23..53).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, 5, 5, v4, cb);
            }
            b.place_block(r, DOT_DECO_DATA, 5, 5, 52, cb);
            for v4 in 0..4 {
                b.generate_box(r, cb, v4, v4 + 1, 21, v4, v4 + 1, 57 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
            b.generate_box(r, cb, 4, 1, 52, 6, 3, 52, BASE_GRAY, BASE_GRAY, false);
            b.generate_box(r, cb, 5, 1, 51, 5, 3, 53, BASE_GRAY, BASE_GRAY, false);
        }
        if self.chunk_intersects(cb, 51, 21, 58, 58) {
            b.generate_box(r, cb, 51, 0, 21, 57, 0, 57, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 51, 1, 21, 57, 7, 57);
            b.generate_box(r, cb, 51, 4, 21, 53, 4, 53, BASE_GRAY, BASE_GRAY, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, 57 - v4, v4 + 1, 21, 57 - v4, v4 + 1, 57 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (23..53).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, 52, 5, v4, cb);
            }
            b.place_block(r, DOT_DECO_DATA, 52, 5, 52, cb);
            b.generate_box(r, cb, 51, 1, 52, 53, 3, 52, BASE_GRAY, BASE_GRAY, false);
            b.generate_box(r, cb, 52, 1, 51, 52, 3, 53, BASE_GRAY, BASE_GRAY, false);
        }
        if self.chunk_intersects(cb, 0, 51, 57, 57) {
            b.generate_box(r, cb, 7, 0, 51, 50, 0, 57, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 7, 1, 51, 50, 10, 57);
            for v4 in 0..4 {
                b.generate_box(r, cb, v4 + 1, v4 + 1, 57 - v4, 56 - v4, v4 + 1, 57 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
        }
    }

    fn middle_wall(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        if self.chunk_intersects(cb, 7, 21, 13, 50) {
            b.generate_box(r, cb, 7, 0, 21, 13, 0, 50, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 7, 1, 21, 13, 10, 50);
            b.generate_box(r, cb, 11, 8, 21, 13, 8, 53, BASE_GRAY, BASE_GRAY, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, v4 + 7, v4 + 5, 21, v4 + 7, v4 + 5, 54, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (21..=45).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, 12, 9, v4, cb);
            }
        }
        if self.chunk_intersects(cb, 44, 21, 50, 54) {
            b.generate_box(r, cb, 44, 0, 21, 50, 0, 50, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 44, 1, 21, 50, 10, 50);
            b.generate_box(r, cb, 44, 8, 21, 46, 8, 53, BASE_GRAY, BASE_GRAY, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, 50 - v4, v4 + 5, 21, 50 - v4, v4 + 5, 54, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (21..=45).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, 45, 9, v4, cb);
            }
        }
        if self.chunk_intersects(cb, 8, 44, 49, 54) {
            b.generate_box(r, cb, 14, 0, 44, 43, 0, 50, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 14, 1, 44, 43, 10, 50);
            for v4 in (12..=45).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, v4, 9, 45, cb);
                b.place_block(r, DOT_DECO_DATA, v4, 9, 52, cb);
                if matches!(v4, 12 | 18 | 24 | 33 | 39 | 45) {
                    b.place_block(r, DOT_DECO_DATA, v4, 9, 47, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 9, 50, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 10, 45, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 10, 46, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 10, 51, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 10, 52, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 11, 47, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 11, 50, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 12, 48, cb);
                    b.place_block(r, DOT_DECO_DATA, v4, 12, 49, cb);
                }
            }
            for v4 in 0..3 {
                b.generate_box(r, cb, 8 + v4, 5 + v4, 54, 49 - v4, 5 + v4, 54, BASE_GRAY, BASE_GRAY, false);
            }
            b.generate_box(r, cb, 11, 8, 54, 46, 8, 54, BASE_LIGHT, BASE_LIGHT, false);
            b.generate_box(r, cb, 14, 8, 44, 43, 8, 53, BASE_GRAY, BASE_GRAY, false);
        }
    }

    fn upper_wall(&self, r: &mut Region, cb: &BoundingBox) {
        let b = &self.base;
        if self.chunk_intersects(cb, 14, 21, 20, 43) {
            b.generate_box(r, cb, 14, 0, 21, 20, 0, 43, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 14, 1, 22, 20, 14, 43);
            b.generate_box(r, cb, 18, 12, 22, 20, 12, 39, BASE_GRAY, BASE_GRAY, false);
            b.generate_box(r, cb, 18, 12, 21, 20, 12, 21, BASE_LIGHT, BASE_LIGHT, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, v4 + 14, v4 + 9, 21, v4 + 14, v4 + 9, 43 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (23..=39).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, 19, 13, v4, cb);
            }
        }
        if self.chunk_intersects(cb, 37, 21, 43, 43) {
            b.generate_box(r, cb, 37, 0, 21, 43, 0, 43, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 37, 1, 22, 43, 14, 43);
            b.generate_box(r, cb, 37, 12, 22, 39, 12, 39, BASE_GRAY, BASE_GRAY, false);
            b.generate_box(r, cb, 37, 12, 21, 39, 12, 21, BASE_LIGHT, BASE_LIGHT, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, 43 - v4, v4 + 9, 21, 43 - v4, v4 + 9, 43 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (23..=39).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, 38, 13, v4, cb);
            }
        }
        if self.chunk_intersects(cb, 15, 37, 42, 43) {
            b.generate_box(r, cb, 21, 0, 37, 36, 0, 43, BASE_GRAY, BASE_GRAY, false);
            water_box(b, r, cb, 21, 1, 37, 36, 14, 43);
            b.generate_box(r, cb, 21, 12, 37, 36, 12, 39, BASE_GRAY, BASE_GRAY, false);
            for v4 in 0..4 {
                b.generate_box(r, cb, 15 + v4, v4 + 9, 43 - v4, 42 - v4, v4 + 9, 43 - v4, BASE_LIGHT, BASE_LIGHT, false);
            }
            for v4 in (21..=36).step_by(3) {
                b.place_block(r, DOT_DECO_DATA, v4, 13, 38, cb);
            }
        }
    }
}

impl Piece for Building {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn save_extra(&self, _tag: &mut Vec<(String, Tag)>) {}

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        let b = &self.base;
        let v8 = r.sea_level().max(64) - b.bbox.min_y;
        water_box(b, r, cb, 0, 0, 0, 58, v8, 58);
        self.wing(r, cb, false, 0);
        self.wing(r, cb, true, 33);
        self.entrance_archs(r, cb);
        self.entrance_wall(r, cb);
        self.roof(r, cb);
        self.lower_wall(r, cb);
        self.middle_wall(r, cb);
        self.upper_wall(r, cb);
        for v9 in 0..7 {
            let mut v10 = 0;
            while v10 < 7 {
                if v10 == 0 && v9 == 3 {
                    v10 = 6;
                }
                let (v11, v12) = (v9 * 9, v10 * 9);
                for v13 in 0..4 {
                    for v14 in 0..4 {
                        b.place_block(r, BASE_LIGHT, v11 + v13, 0, v12 + v14, cb);
                        b.fill_column_down(r, BASE_LIGHT, v11 + v13, -1, v12 + v14, cb);
                    }
                }
                v10 += if v9 == 0 || v9 == 6 { 1 } else { 6 };
            }
        }
        for v9 in 0..5 {
            water_box(b, r, cb, -1 - v9, v9 * 2, -1 - v9, -1 - v9, 23, 58 + v9);
            water_box(b, r, cb, 58 + v9, v9 * 2, -1 - v9, 58 + v9, 23, 58 + v9);
            water_box(b, r, cb, -v9, v9 * 2, -1 - v9, 57 + v9, 23, -1 - v9);
            water_box(b, r, cb, -v9, v9 * 2, 58 + v9, 57 + v9, 23, 58 + v9);
        }
        for c in &self.children {
            if c.base.bbox.intersects(cb) {
                c.place(&self.defs, r, random, cb);
            }
        }
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.shift(dx, dy, dz);
        for c in &mut self.children {
            c.base.bbox.shift(dx, dy, dz);
        }
    }
}

/// `chunkIntersects`: the piece-local rectangle, turned to the world, overlaps `cb`.
fn chunk_intersects(b: &PieceBase, cb: &BoundingBox, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
    let (ax, az, bx, bz) = (b.world_x(x0, z0), b.world_z(x0, z0), b.world_x(x1, z1), b.world_z(x1, z1));
    cb.intersects_xz(ax.min(bx), az.min(bz), ax.max(bx), az.max(bz))
}

/// `generateWaterBox`: water below sea level (and where water is), air above; ice and water
/// stay.
#[allow(clippy::too_many_arguments)]
fn water_box(b: &PieceBase, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            for z in z0..=z1 {
                let s = b.get_block(r, x, y, z, cb);
                let name = kiln_data::blocks_types::block_of(s).name;
                if matches!(name, "minecraft:ice" | "minecraft:packed_ice" | "minecraft:blue_ice") || same_block(s, FILL_BLOCK) {
                    continue;
                }
                if b.world_y(y) < r.sea_level() || s == FILL_BLOCK {
                    b.place_block(r, FILL_BLOCK, x, y, z, cb);
                } else {
                    b.place_block(r, state::AIR, x, y, z, cb);
                }
            }
        }
    }
}

/// `generateBoxOnFillOnly`: `s` where the box holds still water.
#[allow(clippy::too_many_arguments)]
fn fill_only_box(b: &PieceBase, r: &mut Region, cb: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: u16) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            for z in z0..=z1 {
                if b.get_block(r, x, y, z, cb) == FILL_BLOCK {
                    b.place_block(r, s, x, y, z, cb);
                }
            }
        }
    }
}

/// `generateDefaultFloor`.
fn default_floor(b: &PieceBase, r: &mut Region, cb: &BoundingBox, x: i32, z: i32, opening: bool) {
    if opening {
        b.generate_box(r, cb, x, 0, z, x + 2, 0, z + 8 - 1, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, x + 5, 0, z, x + 8 - 1, 0, z + 8 - 1, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, x + 3, 0, z, x + 4, 0, z + 2, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, x + 3, 0, z + 5, x + 4, 0, z + 8 - 1, BASE_GRAY, BASE_GRAY, false);
        b.generate_box(r, cb, x + 3, 0, z + 2, x + 4, 0, z + 2, BASE_LIGHT, BASE_LIGHT, false);
        b.generate_box(r, cb, x + 3, 0, z + 5, x + 4, 0, z + 5, BASE_LIGHT, BASE_LIGHT, false);
        b.generate_box(r, cb, x + 2, 0, z + 3, x + 2, 0, z + 4, BASE_LIGHT, BASE_LIGHT, false);
        b.generate_box(r, cb, x + 5, 0, z + 3, x + 5, 0, z + 4, BASE_LIGHT, BASE_LIGHT, false);
    } else {
        b.generate_box(r, cb, x, 0, z, x + 8 - 1, 0, z + 8 - 1, BASE_GRAY, BASE_GRAY, false);
    }
}

impl Child {
    fn place(&self, defs: &[Def], r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox) {
        let b = &self.base;
        let conn = |i: usize, d: Dir| defs[i].conn[d as usize];
        let up_open = |i: usize| conn(i, Dir::Up).is_some();
        match self.room {
            Room::Entry(d) => {
                let v9 = &defs[d];
                b.generate_box(r, cb, 0, 3, 0, 2, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 3, 0, 7, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 2, 0, 1, 2, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 2, 0, 7, 2, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 1, 0, 0, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 7, 1, 0, 7, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 1, 7, 7, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 0, 2, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 1, 0, 6, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                if v9.has(Dir::North) {
                    water_box(b, r, cb, 3, 1, 7, 4, 2, 7);
                }
                if v9.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 3, 1, 2, 4);
                }
                if v9.has(Dir::East) {
                    water_box(b, r, cb, 6, 1, 3, 7, 2, 4);
                }
            }
            Room::Core => {
                fill_only_box(b, r, cb, 1, 8, 0, 14, 8, 14, BASE_GRAY);
                let v9 = BASE_LIGHT;
                b.generate_box(r, cb, 0, 7, 0, 0, 7, 15, v9, v9, false);
                b.generate_box(r, cb, 15, 7, 0, 15, 7, 15, v9, v9, false);
                b.generate_box(r, cb, 1, 7, 0, 15, 7, 0, v9, v9, false);
                b.generate_box(r, cb, 1, 7, 15, 14, 7, 15, v9, v9, false);
                for v8 in 1..=6 {
                    let v9 = if v8 == 2 || v8 == 6 { BASE_GRAY } else { BASE_LIGHT };
                    for v10 in (0..=15).step_by(15) {
                        b.generate_box(r, cb, v10, v8, 0, v10, v8, 1, v9, v9, false);
                        b.generate_box(r, cb, v10, v8, 6, v10, v8, 9, v9, v9, false);
                        b.generate_box(r, cb, v10, v8, 14, v10, v8, 15, v9, v9, false);
                    }
                    b.generate_box(r, cb, 1, v8, 0, 1, v8, 0, v9, v9, false);
                    b.generate_box(r, cb, 6, v8, 0, 9, v8, 0, v9, v9, false);
                    b.generate_box(r, cb, 14, v8, 0, 14, v8, 0, v9, v9, false);
                    b.generate_box(r, cb, 1, v8, 15, 14, v8, 15, v9, v9, false);
                }
                b.generate_box(r, cb, 6, 3, 6, 9, 6, 9, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 7, 4, 7, 8, 5, 8, st("minecraft:gold_block"), st("minecraft:gold_block"), false);
                for v8 in (3..=6).step_by(3) {
                    for v9 in (6..=9).step_by(3) {
                        b.place_block(r, LAMP_BLOCK, v9, v8, 6, cb);
                        b.place_block(r, LAMP_BLOCK, v9, v8, 9, cb);
                    }
                }
                b.generate_box(r, cb, 5, 1, 6, 5, 2, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 1, 9, 5, 2, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 1, 6, 10, 2, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 1, 9, 10, 2, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 1, 5, 6, 2, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 9, 1, 5, 9, 2, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 1, 10, 6, 2, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 9, 1, 10, 9, 2, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 2, 5, 5, 6, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 2, 10, 5, 6, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 2, 5, 10, 6, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 2, 10, 10, 6, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 7, 1, 5, 7, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 7, 1, 10, 7, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 7, 9, 5, 7, 14, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 7, 9, 10, 7, 14, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 7, 5, 6, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 7, 10, 6, 7, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 9, 7, 5, 14, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 9, 7, 10, 14, 7, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 1, 2, 2, 1, 3, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 3, 1, 2, 3, 1, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 13, 1, 2, 13, 1, 3, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 12, 1, 2, 12, 1, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 1, 12, 2, 1, 13, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 3, 1, 13, 3, 1, 13, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 13, 1, 12, 13, 1, 13, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 12, 1, 13, 12, 1, 13, BASE_LIGHT, BASE_LIGHT, false);
            }
            Room::Simple(d, design) => {
                let v9 = &defs[d];
                if v9.index / 25 > 0 {
                    default_floor(b, r, cb, 0, 0, v9.has(Dir::Down));
                }
                if !up_open(d) {
                    fill_only_box(b, r, cb, 1, 4, 1, 6, 4, 6, BASE_GRAY);
                }
                let v8 = design != 0 && random.next_bool() && !v9.has(Dir::Down) && !v9.has(Dir::Up) && v9.count_openings() > 1;
                if design == 0 {
                    b.generate_box(r, cb, 0, 1, 0, 2, 1, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 3, 0, 2, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 2, 0, 0, 2, 2, BASE_GRAY, BASE_GRAY, false);
                    b.generate_box(r, cb, 1, 2, 0, 2, 2, 0, BASE_GRAY, BASE_GRAY, false);
                    b.place_block(r, LAMP_BLOCK, 1, 2, 1, cb);
                    b.generate_box(r, cb, 5, 1, 0, 7, 1, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 5, 3, 0, 7, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 2, 0, 7, 2, 2, BASE_GRAY, BASE_GRAY, false);
                    b.generate_box(r, cb, 5, 2, 0, 6, 2, 0, BASE_GRAY, BASE_GRAY, false);
                    b.place_block(r, LAMP_BLOCK, 6, 2, 1, cb);
                    b.generate_box(r, cb, 0, 1, 5, 2, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 3, 5, 2, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 2, 5, 0, 2, 7, BASE_GRAY, BASE_GRAY, false);
                    b.generate_box(r, cb, 1, 2, 7, 2, 2, 7, BASE_GRAY, BASE_GRAY, false);
                    b.place_block(r, LAMP_BLOCK, 1, 2, 6, cb);
                    b.generate_box(r, cb, 5, 1, 5, 7, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 5, 3, 5, 7, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 2, 5, 7, 2, 7, BASE_GRAY, BASE_GRAY, false);
                    b.generate_box(r, cb, 5, 2, 7, 6, 2, 7, BASE_GRAY, BASE_GRAY, false);
                    b.place_block(r, LAMP_BLOCK, 6, 2, 6, cb);
                    if v9.has(Dir::South) {
                        b.generate_box(r, cb, 3, 3, 0, 4, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, 3, 3, 0, 4, 3, 1, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 3, 2, 0, 4, 2, 0, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 3, 1, 0, 4, 1, 1, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    if v9.has(Dir::North) {
                        b.generate_box(r, cb, 3, 3, 7, 4, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, 3, 3, 6, 4, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 3, 2, 7, 4, 2, 7, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 3, 1, 6, 4, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    if v9.has(Dir::West) {
                        b.generate_box(r, cb, 0, 3, 3, 0, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, 0, 3, 3, 1, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 0, 2, 3, 0, 2, 4, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 0, 1, 3, 1, 1, 4, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    if v9.has(Dir::East) {
                        b.generate_box(r, cb, 7, 3, 3, 7, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, 6, 3, 3, 7, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 7, 2, 3, 7, 2, 4, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 6, 1, 3, 7, 1, 4, BASE_LIGHT, BASE_LIGHT, false);
                    }
                } else if design == 1 {
                    b.generate_box(r, cb, 2, 1, 2, 2, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 2, 1, 5, 2, 3, 5, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 5, 1, 5, 5, 3, 5, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 5, 1, 2, 5, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.place_block(r, LAMP_BLOCK, 2, 2, 2, cb);
                    b.place_block(r, LAMP_BLOCK, 2, 2, 5, cb);
                    b.place_block(r, LAMP_BLOCK, 5, 2, 5, cb);
                    b.place_block(r, LAMP_BLOCK, 5, 2, 2, cb);
                    b.generate_box(r, cb, 0, 1, 0, 1, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 1, 1, 0, 3, 1, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 1, 7, 1, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 1, 6, 0, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 1, 7, 7, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 1, 6, 7, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 1, 0, 7, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 1, 1, 7, 3, 1, BASE_LIGHT, BASE_LIGHT, false);
                    b.place_block(r, BASE_GRAY, 1, 2, 0, cb);
                    b.place_block(r, BASE_GRAY, 0, 2, 1, cb);
                    b.place_block(r, BASE_GRAY, 1, 2, 7, cb);
                    b.place_block(r, BASE_GRAY, 0, 2, 6, cb);
                    b.place_block(r, BASE_GRAY, 6, 2, 7, cb);
                    b.place_block(r, BASE_GRAY, 7, 2, 6, cb);
                    b.place_block(r, BASE_GRAY, 6, 2, 0, cb);
                    b.place_block(r, BASE_GRAY, 7, 2, 1, cb);
                    if !v9.has(Dir::South) {
                        b.generate_box(r, cb, 1, 3, 0, 6, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 1, 2, 0, 6, 2, 0, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 1, 1, 0, 6, 1, 0, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    if !v9.has(Dir::North) {
                        b.generate_box(r, cb, 1, 3, 7, 6, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 1, 2, 7, 6, 2, 7, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 1, 1, 7, 6, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    if !v9.has(Dir::West) {
                        b.generate_box(r, cb, 0, 3, 1, 0, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 0, 2, 1, 0, 2, 6, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 0, 1, 1, 0, 1, 6, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    if !v9.has(Dir::East) {
                        b.generate_box(r, cb, 7, 3, 1, 7, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 7, 2, 1, 7, 2, 6, BASE_GRAY, BASE_GRAY, false);
                        b.generate_box(r, cb, 7, 1, 1, 7, 1, 6, BASE_LIGHT, BASE_LIGHT, false);
                    }
                } else if design == 2 {
                    b.generate_box(r, cb, 0, 1, 0, 0, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 1, 0, 7, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 1, 0, 6, 1, 0, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 1, 7, 6, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 2, 0, 0, 2, 7, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 7, 2, 0, 7, 2, 7, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 1, 2, 0, 6, 2, 0, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 1, 2, 7, 6, 2, 7, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 0, 3, 0, 0, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 3, 0, 7, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 3, 0, 6, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 3, 7, 6, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 0, 1, 3, 0, 2, 4, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 7, 1, 3, 7, 2, 4, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 3, 1, 0, 4, 2, 0, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 3, 1, 7, 4, 2, 7, BASE_BLACK, BASE_BLACK, false);
                    if v9.has(Dir::South) {
                        water_box(b, r, cb, 3, 1, 0, 4, 2, 0);
                    }
                    if v9.has(Dir::North) {
                        water_box(b, r, cb, 3, 1, 7, 4, 2, 7);
                    }
                    if v9.has(Dir::West) {
                        water_box(b, r, cb, 0, 1, 3, 0, 2, 4);
                    }
                    if v9.has(Dir::East) {
                        water_box(b, r, cb, 7, 1, 3, 7, 2, 4);
                    }
                }
                if v8 {
                    b.generate_box(r, cb, 3, 1, 3, 4, 1, 4, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 3, 2, 3, 4, 2, 4, BASE_GRAY, BASE_GRAY, false);
                    b.generate_box(r, cb, 3, 3, 3, 4, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                }
            }
            Room::SimpleTop(d) => {
                let v9 = &defs[d];
                if v9.index / 25 > 0 {
                    default_floor(b, r, cb, 0, 0, v9.has(Dir::Down));
                }
                if !up_open(d) {
                    fill_only_box(b, r, cb, 1, 4, 1, 6, 4, 6, BASE_GRAY);
                }
                for v8 in 1..=6 {
                    for v9 in 1..=6 {
                        if random.next_int_bounded(3) != 0 {
                            let v10 = 2 + if random.next_int_bounded(4) == 0 { 0 } else { 1 };
                            let v11 = st("minecraft:wet_sponge");
                            b.generate_box(r, cb, v8, v10, v9, v8, 3, v9, v11, v11, false);
                        }
                    }
                }
                b.generate_box(r, cb, 0, 1, 0, 0, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 7, 1, 0, 7, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 0, 6, 1, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 7, 6, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 2, 0, 0, 2, 7, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 7, 2, 0, 7, 2, 7, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 1, 2, 0, 6, 2, 0, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 1, 2, 7, 6, 2, 7, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 0, 3, 0, 0, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 7, 3, 0, 7, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 0, 6, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 7, 6, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 1, 3, 0, 2, 4, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 7, 1, 3, 7, 2, 4, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 3, 1, 0, 4, 2, 0, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 3, 1, 7, 4, 2, 7, BASE_BLACK, BASE_BLACK, false);
                if v9.has(Dir::South) {
                    water_box(b, r, cb, 3, 1, 0, 4, 2, 0);
                }
            }
            Room::DoubleX(d) => {
                let (v9, e) = (&defs[d], conn(d, Dir::East).unwrap());
                let v8 = &defs[e];
                if v9.index / 25 > 0 {
                    default_floor(b, r, cb, 8, 0, v8.has(Dir::Down));
                    default_floor(b, r, cb, 0, 0, v9.has(Dir::Down));
                }
                if !up_open(d) {
                    fill_only_box(b, r, cb, 1, 4, 1, 7, 4, 6, BASE_GRAY);
                }
                if !up_open(e) {
                    fill_only_box(b, r, cb, 8, 4, 1, 14, 4, 6, BASE_GRAY);
                }
                b.generate_box(r, cb, 0, 3, 0, 0, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 15, 3, 0, 15, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 0, 15, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 7, 14, 3, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 2, 0, 0, 2, 7, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 15, 2, 0, 15, 2, 7, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 1, 2, 0, 15, 2, 0, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 1, 2, 7, 14, 2, 7, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 0, 1, 0, 0, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 15, 1, 0, 15, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 0, 15, 1, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 7, 14, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 1, 0, 10, 1, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 2, 0, 9, 2, 3, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 5, 3, 0, 10, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.place_block(r, LAMP_BLOCK, 6, 2, 3, cb);
                b.place_block(r, LAMP_BLOCK, 9, 2, 3, cb);
                if v9.has(Dir::South) {
                    water_box(b, r, cb, 3, 1, 0, 4, 2, 0);
                }
                if v9.has(Dir::North) {
                    water_box(b, r, cb, 3, 1, 7, 4, 2, 7);
                }
                if v9.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 3, 0, 2, 4);
                }
                if v8.has(Dir::South) {
                    water_box(b, r, cb, 11, 1, 0, 12, 2, 0);
                }
                if v8.has(Dir::North) {
                    water_box(b, r, cb, 11, 1, 7, 12, 2, 7);
                }
                if v8.has(Dir::East) {
                    water_box(b, r, cb, 15, 1, 3, 15, 2, 4);
                }
            }
            Room::DoubleY(d) => {
                let u = conn(d, Dir::Up).unwrap();
                if defs[d].index / 25 > 0 {
                    default_floor(b, r, cb, 0, 0, defs[d].has(Dir::Down));
                }
                if !up_open(u) {
                    fill_only_box(b, r, cb, 1, 8, 1, 6, 8, 6, BASE_GRAY);
                }
                b.generate_box(r, cb, 0, 4, 0, 0, 4, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 7, 4, 0, 7, 4, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 4, 0, 6, 4, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 4, 7, 6, 4, 7, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 4, 1, 2, 4, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 4, 2, 1, 4, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 4, 1, 5, 4, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 4, 2, 6, 4, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 4, 5, 2, 4, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 4, 5, 1, 4, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 4, 5, 5, 4, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 4, 5, 6, 4, 5, BASE_LIGHT, BASE_LIGHT, false);
                for (v9, v10) in [(&defs[d], 1), (&defs[u], 5)] {
                    let v11 = 0;
                    if v9.has(Dir::South) {
                        b.generate_box(r, cb, 2, v10, v11, 2, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 5, v10, v11, 5, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 3, v10 + 2, v11, 4, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, 0, v10, v11, 7, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 0, v10 + 1, v11, 7, v10 + 1, v11, BASE_GRAY, BASE_GRAY, false);
                    }
                    let v11 = 7;
                    if v9.has(Dir::North) {
                        b.generate_box(r, cb, 2, v10, v11, 2, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 5, v10, v11, 5, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 3, v10 + 2, v11, 4, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, 0, v10, v11, 7, v10 + 2, v11, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, 0, v10 + 1, v11, 7, v10 + 1, v11, BASE_GRAY, BASE_GRAY, false);
                    }
                    let v12 = 0;
                    if v9.has(Dir::West) {
                        b.generate_box(r, cb, v12, v10, 2, v12, v10 + 2, 2, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v12, v10, 5, v12, v10 + 2, 5, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v12, v10 + 2, 3, v12, v10 + 2, 4, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, v12, v10, 0, v12, v10 + 2, 7, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v12, v10 + 1, 0, v12, v10 + 1, 7, BASE_GRAY, BASE_GRAY, false);
                    }
                    let v12 = 7;
                    if v9.has(Dir::East) {
                        b.generate_box(r, cb, v12, v10, 2, v12, v10 + 2, 2, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v12, v10, 5, v12, v10 + 2, 5, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v12, v10 + 2, 3, v12, v10 + 2, 4, BASE_LIGHT, BASE_LIGHT, false);
                    } else {
                        b.generate_box(r, cb, v12, v10, 0, v12, v10 + 2, 7, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v12, v10 + 1, 0, v12, v10 + 1, 7, BASE_GRAY, BASE_GRAY, false);
                    }
                }
            }
            Room::DoubleZ(d) => {
                let (v9, n) = (&defs[d], conn(d, Dir::North).unwrap());
                let v8 = &defs[n];
                if v9.index / 25 > 0 {
                    default_floor(b, r, cb, 0, 8, v8.has(Dir::Down));
                    default_floor(b, r, cb, 0, 0, v9.has(Dir::Down));
                }
                if !up_open(d) {
                    fill_only_box(b, r, cb, 1, 4, 1, 6, 4, 7, BASE_GRAY);
                }
                if !up_open(n) {
                    fill_only_box(b, r, cb, 1, 4, 8, 6, 4, 14, BASE_GRAY);
                }
                b.generate_box(r, cb, 0, 3, 0, 0, 3, 15, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 7, 3, 0, 7, 3, 15, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 0, 7, 3, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 15, 6, 3, 15, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, 2, 0, 0, 2, 15, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 7, 2, 0, 7, 2, 15, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 1, 2, 0, 7, 2, 0, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 1, 2, 15, 6, 2, 15, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 0, 1, 0, 0, 1, 15, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 7, 1, 0, 7, 1, 15, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 0, 7, 1, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 15, 6, 1, 15, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 1, 1, 1, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 1, 1, 6, 1, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 1, 1, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 3, 1, 6, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 1, 13, 1, 1, 14, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 1, 13, 6, 1, 14, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 3, 13, 1, 3, 14, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, 3, 13, 6, 3, 14, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 1, 6, 2, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 1, 6, 5, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 1, 9, 2, 3, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 1, 9, 5, 3, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 3, 2, 6, 4, 2, 6, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 3, 2, 9, 4, 2, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 2, 2, 7, 2, 2, 8, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 2, 7, 5, 2, 8, BASE_LIGHT, BASE_LIGHT, false);
                b.place_block(r, LAMP_BLOCK, 2, 2, 5, cb);
                b.place_block(r, LAMP_BLOCK, 5, 2, 5, cb);
                b.place_block(r, LAMP_BLOCK, 2, 2, 10, cb);
                b.place_block(r, LAMP_BLOCK, 5, 2, 10, cb);
                b.place_block(r, BASE_LIGHT, 2, 3, 5, cb);
                b.place_block(r, BASE_LIGHT, 5, 3, 5, cb);
                b.place_block(r, BASE_LIGHT, 2, 3, 10, cb);
                b.place_block(r, BASE_LIGHT, 5, 3, 10, cb);
                if v9.has(Dir::South) {
                    water_box(b, r, cb, 3, 1, 0, 4, 2, 0);
                }
                if v9.has(Dir::East) {
                    water_box(b, r, cb, 7, 1, 3, 7, 2, 4);
                }
                if v9.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 3, 0, 2, 4);
                }
                if v8.has(Dir::North) {
                    water_box(b, r, cb, 3, 1, 15, 4, 2, 15);
                }
                if v8.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 11, 0, 2, 12);
                }
                if v8.has(Dir::East) {
                    water_box(b, r, cb, 7, 1, 11, 7, 2, 12);
                }
            }
            Room::DoubleXY(d) => {
                let e = conn(d, Dir::East).unwrap();
                let (v8, v9) = (&defs[e], &defs[d]);
                let (v10, v11) = (&defs[conn(d, Dir::Up).unwrap()], &defs[conn(e, Dir::Up).unwrap()]);
                if v9.index / 25 > 0 {
                    default_floor(b, r, cb, 8, 0, v8.has(Dir::Down));
                    default_floor(b, r, cb, 0, 0, v9.has(Dir::Down));
                }
                if v10.conn[Dir::Up as usize].is_none() {
                    fill_only_box(b, r, cb, 1, 8, 1, 7, 8, 6, BASE_GRAY);
                }
                if v11.conn[Dir::Up as usize].is_none() {
                    fill_only_box(b, r, cb, 8, 8, 1, 14, 8, 6, BASE_GRAY);
                }
                for v12 in 1..=7 {
                    let v13 = if v12 == 2 || v12 == 6 { BASE_GRAY } else { BASE_LIGHT };
                    b.generate_box(r, cb, 0, v12, 0, 0, v12, 7, v13, v13, false);
                    b.generate_box(r, cb, 15, v12, 0, 15, v12, 7, v13, v13, false);
                    b.generate_box(r, cb, 1, v12, 0, 15, v12, 0, v13, v13, false);
                    b.generate_box(r, cb, 1, v12, 7, 14, v12, 7, v13, v13, false);
                }
                b.generate_box(r, cb, 2, 1, 3, 2, 7, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 3, 1, 2, 4, 7, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 3, 1, 5, 4, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 13, 1, 3, 13, 7, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 11, 1, 2, 12, 7, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 11, 1, 5, 12, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 1, 3, 5, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 1, 3, 10, 3, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 7, 2, 10, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 5, 2, 5, 7, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 5, 2, 10, 7, 2, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 5, 5, 5, 5, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 10, 5, 5, 10, 7, 5, BASE_LIGHT, BASE_LIGHT, false);
                b.place_block(r, BASE_LIGHT, 6, 6, 2, cb);
                b.place_block(r, BASE_LIGHT, 9, 6, 2, cb);
                b.place_block(r, BASE_LIGHT, 6, 6, 5, cb);
                b.place_block(r, BASE_LIGHT, 9, 6, 5, cb);
                b.generate_box(r, cb, 5, 4, 3, 6, 4, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 9, 4, 3, 10, 4, 4, BASE_LIGHT, BASE_LIGHT, false);
                b.place_block(r, LAMP_BLOCK, 5, 4, 2, cb);
                b.place_block(r, LAMP_BLOCK, 5, 4, 5, cb);
                b.place_block(r, LAMP_BLOCK, 10, 4, 2, cb);
                b.place_block(r, LAMP_BLOCK, 10, 4, 5, cb);
                if v9.has(Dir::South) {
                    water_box(b, r, cb, 3, 1, 0, 4, 2, 0);
                }
                if v9.has(Dir::North) {
                    water_box(b, r, cb, 3, 1, 7, 4, 2, 7);
                }
                if v9.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 3, 0, 2, 4);
                }
                if v8.has(Dir::South) {
                    water_box(b, r, cb, 11, 1, 0, 12, 2, 0);
                }
                if v8.has(Dir::North) {
                    water_box(b, r, cb, 11, 1, 7, 12, 2, 7);
                }
                if v8.has(Dir::East) {
                    water_box(b, r, cb, 15, 1, 3, 15, 2, 4);
                }
                if v10.has(Dir::South) {
                    water_box(b, r, cb, 3, 5, 0, 4, 6, 0);
                }
                if v10.has(Dir::North) {
                    water_box(b, r, cb, 3, 5, 7, 4, 6, 7);
                }
                if v10.has(Dir::West) {
                    water_box(b, r, cb, 0, 5, 3, 0, 6, 4);
                }
                if v11.has(Dir::South) {
                    water_box(b, r, cb, 11, 5, 0, 12, 6, 0);
                }
                if v11.has(Dir::North) {
                    water_box(b, r, cb, 11, 5, 7, 12, 6, 7);
                }
                if v11.has(Dir::East) {
                    water_box(b, r, cb, 15, 5, 3, 15, 6, 4);
                }
            }
            Room::DoubleYZ(d) => {
                let n = conn(d, Dir::North).unwrap();
                let (v8, v9) = (&defs[n], &defs[d]);
                let (v10, v11) = (&defs[conn(n, Dir::Up).unwrap()], &defs[conn(d, Dir::Up).unwrap()]);
                if v9.index / 25 > 0 {
                    default_floor(b, r, cb, 0, 8, v8.has(Dir::Down));
                    default_floor(b, r, cb, 0, 0, v9.has(Dir::Down));
                }
                if v11.conn[Dir::Up as usize].is_none() {
                    fill_only_box(b, r, cb, 1, 8, 1, 6, 8, 7, BASE_GRAY);
                }
                if v10.conn[Dir::Up as usize].is_none() {
                    fill_only_box(b, r, cb, 1, 8, 8, 6, 8, 14, BASE_GRAY);
                }
                for v12 in 1..=7 {
                    let v13 = if v12 == 2 || v12 == 6 { BASE_GRAY } else { BASE_LIGHT };
                    b.generate_box(r, cb, 0, v12, 0, 0, v12, 15, v13, v13, false);
                    b.generate_box(r, cb, 7, v12, 0, 7, v12, 15, v13, v13, false);
                    b.generate_box(r, cb, 1, v12, 0, 6, v12, 0, v13, v13, false);
                    b.generate_box(r, cb, 1, v12, 15, 6, v12, 15, v13, v13, false);
                }
                for v12 in 1..=7 {
                    let v13 = if v12 == 2 || v12 == 6 { LAMP_BLOCK } else { BASE_BLACK };
                    b.generate_box(r, cb, 3, v12, 7, 4, v12, 8, v13, v13, false);
                }
                if v9.has(Dir::South) {
                    water_box(b, r, cb, 3, 1, 0, 4, 2, 0);
                }
                if v9.has(Dir::East) {
                    water_box(b, r, cb, 7, 1, 3, 7, 2, 4);
                }
                if v9.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 3, 0, 2, 4);
                }
                if v8.has(Dir::North) {
                    water_box(b, r, cb, 3, 1, 15, 4, 2, 15);
                }
                if v8.has(Dir::West) {
                    water_box(b, r, cb, 0, 1, 11, 0, 2, 12);
                }
                if v8.has(Dir::East) {
                    water_box(b, r, cb, 7, 1, 11, 7, 2, 12);
                }
                if v11.has(Dir::South) {
                    water_box(b, r, cb, 3, 5, 0, 4, 6, 0);
                }
                if v11.has(Dir::East) {
                    water_box(b, r, cb, 7, 5, 3, 7, 6, 4);
                    b.generate_box(r, cb, 5, 4, 2, 6, 4, 5, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 1, 2, 6, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 1, 5, 6, 3, 5, BASE_LIGHT, BASE_LIGHT, false);
                }
                if v11.has(Dir::West) {
                    water_box(b, r, cb, 0, 5, 3, 0, 6, 4);
                    b.generate_box(r, cb, 1, 4, 2, 2, 4, 5, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 1, 2, 1, 3, 2, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 1, 5, 1, 3, 5, BASE_LIGHT, BASE_LIGHT, false);
                }
                if v10.has(Dir::North) {
                    water_box(b, r, cb, 3, 5, 15, 4, 6, 15);
                }
                if v10.has(Dir::West) {
                    water_box(b, r, cb, 0, 5, 11, 0, 6, 12);
                    b.generate_box(r, cb, 1, 4, 10, 2, 4, 13, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 1, 10, 1, 3, 10, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 1, 1, 13, 1, 3, 13, BASE_LIGHT, BASE_LIGHT, false);
                }
                if v10.has(Dir::East) {
                    water_box(b, r, cb, 7, 5, 11, 7, 6, 12);
                    b.generate_box(r, cb, 5, 4, 10, 6, 4, 13, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 1, 10, 6, 3, 10, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 1, 13, 6, 3, 13, BASE_LIGHT, BASE_LIGHT, false);
                }
            }
            Room::Wing(design) => {
                if design == 0 {
                    for v8 in 0..4 {
                        b.generate_box(r, cb, 10 - v8, 3 - v8, 20 - v8, 12 + v8, 3 - v8, 20, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    b.generate_box(r, cb, 7, 0, 6, 15, 0, 16, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 6, 0, 6, 6, 3, 20, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 16, 0, 6, 16, 3, 20, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 1, 7, 7, 1, 20, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 15, 1, 7, 15, 1, 20, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 7, 1, 6, 9, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 13, 1, 6, 15, 3, 6, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 8, 1, 7, 9, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 13, 1, 7, 14, 1, 7, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 9, 0, 5, 13, 0, 5, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 10, 0, 7, 12, 0, 7, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 8, 0, 10, 8, 0, 12, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 14, 0, 10, 14, 0, 12, BASE_BLACK, BASE_BLACK, false);
                    for v8 in (7..=18).rev().step_by(3) {
                        b.place_block(r, LAMP_BLOCK, 6, 3, v8, cb);
                        b.place_block(r, LAMP_BLOCK, 16, 3, v8, cb);
                    }
                    b.place_block(r, LAMP_BLOCK, 10, 0, 10, cb);
                    b.place_block(r, LAMP_BLOCK, 12, 0, 10, cb);
                    b.place_block(r, LAMP_BLOCK, 10, 0, 12, cb);
                    b.place_block(r, LAMP_BLOCK, 12, 0, 12, cb);
                    b.place_block(r, LAMP_BLOCK, 8, 3, 6, cb);
                    b.place_block(r, LAMP_BLOCK, 14, 3, 6, cb);
                    b.place_block(r, BASE_LIGHT, 4, 2, 4, cb);
                    b.place_block(r, LAMP_BLOCK, 4, 1, 4, cb);
                    b.place_block(r, BASE_LIGHT, 4, 0, 4, cb);
                    b.place_block(r, BASE_LIGHT, 18, 2, 4, cb);
                    b.place_block(r, LAMP_BLOCK, 18, 1, 4, cb);
                    b.place_block(r, BASE_LIGHT, 18, 0, 4, cb);
                    b.place_block(r, BASE_LIGHT, 4, 2, 18, cb);
                    b.place_block(r, LAMP_BLOCK, 4, 1, 18, cb);
                    b.place_block(r, BASE_LIGHT, 4, 0, 18, cb);
                    b.place_block(r, BASE_LIGHT, 18, 2, 18, cb);
                    b.place_block(r, LAMP_BLOCK, 18, 1, 18, cb);
                    b.place_block(r, BASE_LIGHT, 18, 0, 18, cb);
                    b.place_block(r, BASE_LIGHT, 9, 7, 20, cb);
                    b.place_block(r, BASE_LIGHT, 13, 7, 20, cb);
                    b.generate_box(r, cb, 6, 0, 21, 7, 4, 21, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 15, 0, 21, 16, 4, 21, BASE_LIGHT, BASE_LIGHT, false);
                } else if design == 1 {
                    b.generate_box(r, cb, 9, 3, 18, 13, 3, 20, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 9, 0, 18, 9, 2, 18, BASE_LIGHT, BASE_LIGHT, false);
                    b.generate_box(r, cb, 13, 0, 18, 13, 2, 18, BASE_LIGHT, BASE_LIGHT, false);
                    for v8 in [9, 13] {
                        b.place_block(r, BASE_LIGHT, v8, 6, 20, cb);
                        b.place_block(r, LAMP_BLOCK, v8, 5, 20, cb);
                        b.place_block(r, BASE_LIGHT, v8, 4, 20, cb);
                    }
                    b.generate_box(r, cb, 7, 3, 7, 15, 3, 14, BASE_LIGHT, BASE_LIGHT, false);
                    for v8 in [10, 12] {
                        b.generate_box(r, cb, v8, 0, 10, v8, 6, 10, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v8, 0, 12, v8, 6, 12, BASE_LIGHT, BASE_LIGHT, false);
                        b.place_block(r, LAMP_BLOCK, v8, 0, 10, cb);
                        b.place_block(r, LAMP_BLOCK, v8, 0, 12, cb);
                        b.place_block(r, LAMP_BLOCK, v8, 4, 10, cb);
                        b.place_block(r, LAMP_BLOCK, v8, 4, 12, cb);
                    }
                    for v8 in [8, 14] {
                        b.generate_box(r, cb, v8, 0, 7, v8, 2, 7, BASE_LIGHT, BASE_LIGHT, false);
                        b.generate_box(r, cb, v8, 0, 14, v8, 2, 14, BASE_LIGHT, BASE_LIGHT, false);
                    }
                    b.generate_box(r, cb, 8, 3, 8, 8, 3, 13, BASE_BLACK, BASE_BLACK, false);
                    b.generate_box(r, cb, 14, 3, 8, 14, 3, 13, BASE_BLACK, BASE_BLACK, false);
                }
            }
            Room::Penthouse => {
                b.generate_box(r, cb, 2, -1, 2, 11, -1, 11, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 0, -1, 0, 1, -1, 11, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 12, -1, 0, 13, -1, 11, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 2, -1, 0, 11, -1, 1, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 2, -1, 12, 11, -1, 13, BASE_GRAY, BASE_GRAY, false);
                b.generate_box(r, cb, 0, 0, 0, 0, 0, 13, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 13, 0, 0, 13, 0, 13, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 0, 0, 12, 0, 0, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 1, 0, 13, 12, 0, 13, BASE_LIGHT, BASE_LIGHT, false);
                for v8 in (2..=11).step_by(3) {
                    b.place_block(r, LAMP_BLOCK, 0, 0, v8, cb);
                    b.place_block(r, LAMP_BLOCK, 13, 0, v8, cb);
                    b.place_block(r, LAMP_BLOCK, v8, 0, 0, cb);
                }
                b.generate_box(r, cb, 2, 0, 3, 4, 0, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 9, 0, 3, 11, 0, 9, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 4, 0, 9, 9, 0, 11, BASE_LIGHT, BASE_LIGHT, false);
                b.place_block(r, BASE_LIGHT, 5, 0, 8, cb);
                b.place_block(r, BASE_LIGHT, 8, 0, 8, cb);
                b.place_block(r, BASE_LIGHT, 10, 0, 10, cb);
                b.place_block(r, BASE_LIGHT, 3, 0, 10, cb);
                b.generate_box(r, cb, 3, 0, 3, 3, 0, 7, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 10, 0, 3, 10, 0, 7, BASE_BLACK, BASE_BLACK, false);
                b.generate_box(r, cb, 6, 0, 10, 7, 0, 10, BASE_BLACK, BASE_BLACK, false);
                for v8 in [3, 10] {
                    for v10 in (2..=8).step_by(3) {
                        b.generate_box(r, cb, v8, 0, v10, v8, 2, v10, BASE_LIGHT, BASE_LIGHT, false);
                    }
                }
                b.generate_box(r, cb, 5, 0, 10, 5, 2, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 8, 0, 10, 8, 2, 10, BASE_LIGHT, BASE_LIGHT, false);
                b.generate_box(r, cb, 6, -1, 7, 7, -1, 8, BASE_BLACK, BASE_BLACK, false);
                water_box(b, r, cb, 6, -1, 3, 7, -1, 4);
            }
        }
    }
}
