//! Woodland mansions (`WoodlandMansionStructure`, `WoodlandMansionPieces`): a room grid laid out
//! on three floors, built from `woodland_mansion/*` templates.

use super::st;
use crate::block_facts::Dir;
use crate::blocks::{is_air, with_prop};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext, create_chest_at};
use crate::structure::processor::IGNORE_STRUCTURE_BLOCK;
use crate::structure::template::{PlaceSettings, Template, TemplateManager, zero_position_with_transform};
use crate::structure::transform::{Mirror, Rotation};
use crate::structure::{GenCtx, Start, Stub};
use kiln_data::block_props::liquid;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// `WoodlandMansionStructure`.
pub struct Mansion;

impl Kind for Mansion {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let (x, z) = ((ctx.chunk.0 << 4) + 7, (ctx.chunk.1 << 4) + 7);
        let (lo, hi) = (ctx.min_y() - 1, ctx.max_y());
        if !ctx.could_exist_in_column(x, z, lo, hi) {
            return None;
        }
        let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
        let pos = ctx.lowest_y_in_5x5(rotation);
        if pos.y < 60 {
            return None;
        }
        Some(Stub {
            pos,
            build: Box::new(move |ctx, out| {
                let tm = ctx.structures.templates.clone();
                let grid = Grid::new(&mut ctx.random);
                let mut placer = Placer { tm: &tm, random: &mut ctx.random, start_x: 0, start_y: 0, list: Vec::new() };
                placer.create(pos, rotation, &grid);
                *out = placer.list.into_iter().map(|p| Box::new(p) as Box<dyn Piece>).collect();
            }),
        })
    }

    /// Fills below the mansion's lowest layer with cobblestone down to the ground.
    fn after_place(&self, _cx: &PlaceContext, r: &mut Region, _random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), start: &Start) {
        let Some(all) = crate::structure::pieces_bbox(&start.pieces) else { return };
        let (min_y, y0) = (r.min_y(), all.min_y);
        for x in cb.min_x..=cb.max_x {
            for z in cb.min_z..=cb.max_z {
                let p = BlockPos::new(x, y0, z);
                if r.is_air(p) || !all.is_inside(p) || !start.pieces.iter().any(|q| q.base().bbox.is_inside(p)) {
                    continue;
                }
                let mut y = y0 - 1;
                while y > min_y {
                    let q = p.at_y(y);
                    let s = r.get(q);
                    if !is_air(s) && !liquid(s) {
                        break;
                    }
                    r.set(q, st("minecraft:cobblestone"), 2);
                    y -= 1;
                }
            }
        }
    }
}

const CORRIDOR: i32 = 1;
const ROOM: i32 = 2;
const START_ROOM: i32 = 3;
const TEST_ROOM: i32 = 4;
const BLOCKED: i32 = 5;
const ROOM_1X1: i32 = 65536;
const ROOM_1X2: i32 = 131072;
const ROOM_2X2: i32 = 262144;
const ORIGIN_FLAG: i32 = 1048576;
const DOOR_FLAG: i32 = 2097152;
const STAIRS_FLAG: i32 = 4194304;
const CORRIDOR_FLAG: i32 = 8388608;
const TYPE_MASK: i32 = 983040;
const ID_MASK: i32 = 65535;

/// `WoodlandMansionPieces.SimpleGrid`.
#[derive(Clone)]
struct SimpleGrid {
    grid: Vec<Vec<i32>>,
    width: i32,
    height: i32,
    outside: i32,
}

impl SimpleGrid {
    fn new(width: i32, height: i32, outside: i32) -> Self {
        Self { grid: vec![vec![0; height as usize]; width as usize], width, height, outside }
    }

    fn set(&mut self, x: i32, y: i32, v: i32) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            self.grid[x as usize][y as usize] = v;
        }
    }

    fn set_area(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, v: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, v);
            }
        }
    }

    fn get(&self, x: i32, y: i32) -> i32 {
        if x >= 0 && x < self.width && y >= 0 && y < self.height { self.grid[x as usize][y as usize] } else { self.outside }
    }

    fn setif(&mut self, x: i32, y: i32, if_value: i32, v: i32) {
        if self.get(x, y) == if_value {
            self.set(x, y, v);
        }
    }

    fn edges_to(&self, x: i32, y: i32, v: i32) -> bool {
        self.get(x - 1, y) == v || self.get(x + 1, y) == v || self.get(x, y + 1) == v || self.get(x, y - 1) == v
    }
}

/// `MansionGrid.isHouse`.
fn is_house(g: &SimpleGrid, x: i32, y: i32) -> bool {
    matches!(g.get(x, y), CORRIDOR | ROOM | START_ROOM | TEST_ROOM)
}

fn step(d: Dir) -> (i32, i32) {
    let (x, _, z) = d.offset();
    (x, z)
}

/// `WoodlandMansionPieces.MansionGrid`.
struct Grid {
    base: SimpleGrid,
    third: SimpleGrid,
    floors: [SimpleGrid; 3],
    entrance_x: i32,
    entrance_y: i32,
}

impl Grid {
    fn new(random: &mut WorldgenRandom) -> Self {
        let (ex, ey) = (7, 4);
        let mut base = SimpleGrid::new(11, 11, BLOCKED);
        base.set_area(ex, ey, ex + 1, ey + 1, START_ROOM);
        base.set_area(ex - 1, ey, ex - 1, ey + 1, ROOM);
        base.set_area(ex + 2, ey - 2, ex + 3, ey + 3, BLOCKED);
        base.set_area(ex + 1, ey - 2, ex + 1, ey - 1, CORRIDOR);
        base.set_area(ex + 1, ey + 2, ex + 1, ey + 3, CORRIDOR);
        base.set(ex - 1, ey - 1, CORRIDOR);
        base.set(ex - 1, ey + 2, CORRIDOR);
        base.set_area(0, 0, 11, 1, BLOCKED);
        base.set_area(0, 9, 11, 11, BLOCKED);
        recursive_corridor(random, &mut base, ex, ey - 2, Dir::West, 6);
        recursive_corridor(random, &mut base, ex, ey + 3, Dir::West, 6);
        recursive_corridor(random, &mut base, ex - 2, ey - 1, Dir::West, 3);
        recursive_corridor(random, &mut base, ex - 2, ey + 2, Dir::West, 3);
        while clean_edges(&mut base) {}
        let mut floors = [SimpleGrid::new(11, 11, BLOCKED), SimpleGrid::new(11, 11, BLOCKED), SimpleGrid::new(11, 11, BLOCKED)];
        identify_rooms(random, &base, &mut floors[0]);
        identify_rooms(random, &base, &mut floors[1]);
        floors[0].set_area(ex + 1, ey, ex + 1, ey + 1, CORRIDOR_FLAG);
        floors[1].set_area(ex + 1, ey, ex + 1, ey + 1, CORRIDOR_FLAG);
        let third = SimpleGrid::new(base.width, base.height, BLOCKED);
        let mut g = Grid { base, third, floors, entrance_x: ex, entrance_y: ey };
        g.setup_third_floor(random);
        let third = g.third.clone();
        identify_rooms(random, &third, &mut g.floors[2]);
        g
    }

    /// `isRoomId`.
    fn is_room_id(&self, x: i32, y: i32, floor: usize, id: i32) -> bool {
        self.floors[floor].get(x, y) & ID_MASK == id
    }

    /// `get1x2RoomDirection`.
    fn room_1x2_direction(&self, x: i32, y: i32, floor: usize, id: i32) -> Option<Dir> {
        Dir::HORIZONTAL.into_iter().find(|&d| {
            let (dx, dz) = step(d);
            self.is_room_id(x + dx, y + dz, floor, id)
        })
    }

    /// `setupThirdFloor`.
    fn setup_third_floor(&mut self, random: &mut WorldgenRandom) {
        let mut stairs: Vec<(i32, i32)> = Vec::new();
        for y in 0..self.third.height {
            for x in 0..self.third.width {
                let v = self.floors[1].get(x, y);
                if v & TYPE_MASK == ROOM_1X2 && v & DOOR_FLAG == DOOR_FLAG {
                    stairs.push((x, y));
                }
            }
        }
        let (w, h) = (self.third.width, self.third.height);
        if stairs.is_empty() {
            self.third.set_area(0, 0, w, h, BLOCKED);
            return;
        }
        let (sx, sy) = stairs[random.next_int_bounded(stairs.len() as i32) as usize];
        let v = self.floors[1].get(sx, sy);
        self.floors[1].set(sx, sy, v | STAIRS_FLAG);
        let d = self.room_1x2_direction(sx, sy, 1, v & ID_MASK).expect("1x2 room pair");
        let (dx, dz) = step(d);
        let (ox, oy) = (sx + dx, sy + dz);
        for y in 0..h {
            for x in 0..w {
                if !is_house(&self.base, x, y) {
                    self.third.set(x, y, BLOCKED);
                } else if x == sx && y == sy {
                    self.third.set(x, y, START_ROOM);
                } else if x == ox && y == oy {
                    self.third.set(x, y, START_ROOM);
                    self.floors[2].set(x, y, CORRIDOR_FLAG);
                }
            }
        }
        let free: Vec<Dir> = Dir::HORIZONTAL
            .into_iter()
            .filter(|&d| {
                let (dx, dz) = step(d);
                self.third.get(ox + dx, oy + dz) == 0
            })
            .collect();
        if free.is_empty() {
            self.third.set_area(0, 0, w, h, BLOCKED);
            self.floors[1].set(sx, sy, v);
            return;
        }
        let d = free[random.next_int_bounded(free.len() as i32) as usize];
        let (dx, dz) = step(d);
        recursive_corridor(random, &mut self.third, ox + dx, oy + dz, d, 4);
        while clean_edges(&mut self.third) {}
    }
}

/// `MansionGrid.recursiveCorridor`.
fn recursive_corridor(random: &mut WorldgenRandom, g: &mut SimpleGrid, x: i32, y: i32, d: Dir, depth: i32) {
    if depth <= 0 {
        return;
    }
    let (sx, sz) = step(d);
    g.set(x, y, CORRIDOR);
    g.setif(x + sx, y + sz, 0, CORRIDOR);
    for _ in 0..8 {
        let n = Dir::from_2d(random.next_int_bounded(4));
        if n == d.opposite() || (n == Dir::East && random.next_bool()) {
            continue;
        }
        let (nx, nz) = step(n);
        let (ax, ay) = (x + sx, y + sz);
        if g.get(ax + nx, ay + nz) == 0 && g.get(ax + nx * 2, ay + nz * 2) == 0 {
            recursive_corridor(random, g, ax + nx, ay + nz, n, depth - 1);
            break;
        }
    }
    let (cw, ccw) = (step(d.clockwise()), step(d.counter_clockwise()));
    g.setif(x + cw.0, y + cw.1, 0, ROOM);
    g.setif(x + ccw.0, y + ccw.1, 0, ROOM);
    g.setif(x + sx + cw.0, y + sz + cw.1, 0, ROOM);
    g.setif(x + sx + ccw.0, y + sz + ccw.1, 0, ROOM);
    g.setif(x + sx * 2, y + sz * 2, 0, ROOM);
    g.setif(x + cw.0 * 2, y + cw.1 * 2, 0, ROOM);
    g.setif(x + ccw.0 * 2, y + ccw.1 * 2, 0, ROOM);
}

/// `MansionGrid.cleanEdges`.
fn clean_edges(g: &mut SimpleGrid) -> bool {
    let mut changed = false;
    for y in 0..g.height {
        for x in 0..g.width {
            if g.get(x, y) != 0 {
                continue;
            }
            let h = |dx: i32, dy: i32| is_house(g, x + dx, y + dy) as i32;
            let sides = h(1, 0) + h(-1, 0) + h(0, 1) + h(0, -1);
            if sides >= 3 {
                g.set(x, y, ROOM);
                changed = true;
            } else if sides == 2 {
                let diagonals = h(1, 1) + h(-1, 1) + h(1, -1) + h(-1, -1);
                if diagonals <= 1 {
                    g.set(x, y, ROOM);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// `MansionGrid.identifyRooms`.
fn identify_rooms(random: &mut WorldgenRandom, g: &SimpleGrid, rooms: &mut SimpleGrid) {
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for y in 0..g.height {
        for x in 0..g.width {
            if g.get(x, y) == ROOM {
                cells.push((x, y));
            }
        }
    }
    for i in (2..=cells.len()).rev() {
        let j = random.next_int_bounded(i as i32) as usize;
        cells.swap(i - 1, j);
    }
    let mut id = 10;
    for (x, y) in cells {
        if rooms.get(x, y) != 0 {
            continue;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (x, x, y, y);
        let free = |dx: i32, dy: i32| rooms.get(x + dx, y + dy) == 0 && g.get(x + dx, y + dy) == ROOM;
        let mut ty = ROOM_1X1;
        if free(1, 0) && free(0, 1) && free(1, 1) {
            x1 += 1;
            y1 += 1;
            ty = ROOM_2X2;
        } else if free(-1, 0) && free(0, 1) && free(-1, 1) {
            x0 -= 1;
            y1 += 1;
            ty = ROOM_2X2;
        } else if free(-1, 0) && free(0, -1) && free(-1, -1) {
            x0 -= 1;
            y0 -= 1;
            ty = ROOM_2X2;
        } else if free(1, 0) {
            x1 += 1;
            ty = ROOM_1X2;
        } else if free(0, 1) {
            y1 += 1;
            ty = ROOM_1X2;
        } else if free(-1, 0) {
            x0 -= 1;
            ty = ROOM_1X2;
        } else if free(0, -1) {
            y0 -= 1;
            ty = ROOM_1X2;
        }
        let mut dx = if random.next_bool() { x0 } else { x1 };
        let mut dy = if random.next_bool() { y0 } else { y1 };
        let mut door = DOOR_FLAG;
        if !g.edges_to(dx, dy, CORRIDOR) {
            dx = if dx == x0 { x1 } else { x0 };
            dy = if dy == y0 { y1 } else { y0 };
            if !g.edges_to(dx, dy, CORRIDOR) {
                dy = if dy == y0 { y1 } else { y0 };
                if !g.edges_to(dx, dy, CORRIDOR) {
                    dx = if dx == x0 { x1 } else { x0 };
                    dy = if dy == y0 { y1 } else { y0 };
                    if !g.edges_to(dx, dy, CORRIDOR) {
                        door = 0;
                        dx = x0;
                        dy = y0;
                    }
                }
            }
        }
        for yy in y0..=y1 {
            for xx in x0..=x1 {
                if xx == dx && yy == dy {
                    rooms.set(xx, yy, ORIGIN_FLAG | door | ty | id);
                } else {
                    rooms.set(xx, yy, ty | id);
                }
            }
        }
        id += 1;
    }
}

/// `WoodlandMansionPieces.PlacementData`.
struct Placement {
    rotation: Rotation,
    position: BlockPos,
    wall: &'static str,
}

/// `FloorRoomCollection`s: 0 first floor, 1 second, 2 third (as the second).
fn room_1x1(floor: usize, random: &mut WorldgenRandom) -> String {
    format!("1x1_{}{}", if floor == 0 { "a" } else { "b" }, random.next_int_bounded(5) + 1)
}

fn room_1x1_secret(random: &mut WorldgenRandom) -> String {
    format!("1x1_as{}", random.next_int_bounded(4) + 1)
}

fn room_1x2_side(floor: usize, random: &mut WorldgenRandom, stairs: bool) -> String {
    if floor == 0 {
        format!("1x2_a{}", random.next_int_bounded(9) + 1)
    } else if stairs {
        "1x2_c_stairs".into()
    } else {
        format!("1x2_c{}", random.next_int_bounded(4) + 1)
    }
}

fn room_1x2_front(floor: usize, random: &mut WorldgenRandom, stairs: bool) -> String {
    if floor == 0 {
        format!("1x2_b{}", random.next_int_bounded(5) + 1)
    } else if stairs {
        "1x2_d_stairs".into()
    } else {
        format!("1x2_d{}", random.next_int_bounded(5) + 1)
    }
}

fn room_1x2_secret(floor: usize, random: &mut WorldgenRandom) -> String {
    if floor == 0 { format!("1x2_s{}", random.next_int_bounded(2) + 1) } else { format!("1x2_se{}", random.next_int_bounded(1) + 1) }
}

fn room_2x2(floor: usize, random: &mut WorldgenRandom) -> String {
    if floor == 0 { format!("2x2_a{}", random.next_int_bounded(4) + 1) } else { format!("2x2_b{}", random.next_int_bounded(5) + 1) }
}

/// `BlockPos.rotate(rotation)`.
fn rotate_pos(p: BlockPos, r: Rotation) -> BlockPos {
    match r {
        Rotation::None => p,
        Rotation::Clockwise90 => BlockPos::new(-p.z, p.y, p.x),
        Rotation::Clockwise180 => BlockPos::new(-p.x, p.y, -p.z),
        Rotation::CounterClockwise90 => BlockPos::new(p.z, p.y, -p.x),
    }
}

/// `WoodlandMansionPieces.MansionPiecePlacer`.
struct Placer<'a> {
    tm: &'a TemplateManager,
    random: &'a mut WorldgenRandom,
    start_x: i32,
    start_y: i32,
    list: Vec<MansionPiece>,
}

impl Placer<'_> {
    fn add(&mut self, name: &str, pos: BlockPos, rotation: Rotation, mirror: Mirror) {
        self.list.push(MansionPiece::new(self.tm, name, pos, rotation, mirror));
    }

    /// The grid cell's corner at `(x, y)`.
    fn cell(&self, origin: BlockPos, r: Rotation, x: i32, y: i32, dx: i32) -> BlockPos {
        origin.relative_n(r.rotate(Dir::South), 8 + (y - self.start_y) * 8).relative_n(r.rotate(Dir::East), dx + (x - self.start_x) * 8)
    }

    /// `createMansion`.
    fn create(&mut self, pos: BlockPos, r: Rotation, grid: &Grid) {
        let mut ground = Placement { rotation: r, position: pos, wall: "wall_flat" };
        let west = r.rotate(Dir::West);
        self.add("entrance", ground.position.relative_n(west, 9), r, Mirror::None);
        ground.position = ground.position.relative_n(r.rotate(Dir::South), 16);
        let mut second = Placement { rotation: ground.rotation, position: ground.position.above_n(8), wall: "wall_window" };
        let (base, third) = (&grid.base, &grid.third);
        self.start_x = grid.entrance_x + 1;
        self.start_y = grid.entrance_y + 1;
        let (ex, ey) = (grid.entrance_x + 1, grid.entrance_y);
        let (sx, sy) = (self.start_x, self.start_y);
        self.traverse_outer_walls(&mut ground, base, Dir::South, sx, sy, ex, ey);
        self.traverse_outer_walls(&mut second, base, Dir::South, sx, sy, ex, ey);
        let mut top = Placement { rotation: ground.rotation, position: ground.position.above_n(19), wall: "wall_window" };
        'find: for y in 0..third.height {
            for x in (0..third.width).rev() {
                if is_house(third, x, y) {
                    top.position = top.position.relative_n(r.rotate(Dir::South), 8 + (y - self.start_y) * 8);
                    top.position = top.position.relative_n(r.rotate(Dir::East), (x - self.start_x) * 8);
                    self.traverse_wall_piece(&mut top);
                    self.traverse_outer_walls(&mut top, third, Dir::South, x, y, x, y);
                    break 'find;
                }
            }
        }
        self.create_roof(pos.above_n(16), r, base, Some(third));
        self.create_roof(pos.above_n(27), r, third, None);
        for floor in 0..3 {
            let origin = pos.above_n(8 * floor as i32 + if floor == 2 { 3 } else { 0 });
            let rooms = &grid.floors[floor];
            let g = if floor == 2 { third } else { base };
            let (carpet_south, carpet_west) = if floor == 0 { ("carpet_south_1", "carpet_west_1") } else { ("carpet_south_2", "carpet_west_2") };
            for y in 0..g.height {
                for x in 0..g.width {
                    if g.get(x, y) != CORRIDOR {
                        continue;
                    }
                    let c = self.cell(origin, r, x, y, 0);
                    self.add("corridor_floor", c, r, Mirror::None);
                    let open = |dx: i32, dy: i32| g.get(x + dx, y + dy) == CORRIDOR || rooms.get(x + dx, y + dy) & CORRIDOR_FLAG == CORRIDOR_FLAG;
                    if open(0, -1) {
                        self.add("carpet_north", c.relative_n(r.rotate(Dir::East), 1).above(), r, Mirror::None);
                    }
                    if open(1, 0) {
                        self.add("carpet_east", c.relative_n(r.rotate(Dir::South), 1).relative_n(r.rotate(Dir::East), 5).above(), r, Mirror::None);
                    }
                    if open(0, 1) {
                        self.add(carpet_south, c.relative_n(r.rotate(Dir::South), 5).relative_n(r.rotate(Dir::West), 1), r, Mirror::None);
                    }
                    if open(-1, 0) {
                        self.add(carpet_west, c.relative_n(r.rotate(Dir::West), 1).relative_n(r.rotate(Dir::North), 1), r, Mirror::None);
                    }
                }
            }
            let (wall, door) = if floor == 0 { ("indoors_wall_1", "indoors_door_1") } else { ("indoors_wall_2", "indoors_door_2") };
            for y in 0..g.height {
                for x in 0..g.width {
                    let stair_room = floor == 2 && g.get(x, y) == START_ROOM;
                    if g.get(x, y) != ROOM && !stair_room {
                        continue;
                    }
                    let v = rooms.get(x, y);
                    let (ty, id) = (v & TYPE_MASK, v & ID_MASK);
                    let stair_room = stair_room && v & CORRIDOR_FLAG == CORRIDOR_FLAG;
                    let mut doors: Vec<Dir> = Vec::new();
                    if v & DOOR_FLAG == DOOR_FLAG {
                        for d in Dir::HORIZONTAL {
                            let (dx, dz) = step(d);
                            if g.get(x + dx, y + dz) == CORRIDOR {
                                doors.push(d);
                            }
                        }
                    }
                    let entry = if !doors.is_empty() {
                        Some(doors[self.random.next_int_bounded(doors.len() as i32) as usize])
                    } else if v & ORIGIN_FLAG == ORIGIN_FLAG {
                        Some(Dir::Up)
                    } else {
                        None
                    };
                    let c = self.cell(origin, r, x, y, -1);
                    let pick = |d: Dir| if entry == Some(d) { door } else { wall };
                    if is_house(g, x - 1, y) && !grid.is_room_id(x - 1, y, floor, id) {
                        self.add(pick(Dir::West), c, r, Mirror::None);
                    }
                    if g.get(x + 1, y) == CORRIDOR && !stair_room {
                        self.add(pick(Dir::East), c.relative_n(r.rotate(Dir::East), 8), r, Mirror::None);
                    }
                    if is_house(g, x, y + 1) && !grid.is_room_id(x, y + 1, floor, id) {
                        let p = c.relative_n(r.rotate(Dir::South), 7).relative_n(r.rotate(Dir::East), 7);
                        self.add(pick(Dir::South), p, r.then(Rotation::Clockwise90), Mirror::None);
                    }
                    if g.get(x, y - 1) == CORRIDOR && !stair_room {
                        let p = c.relative_n(r.rotate(Dir::North), 1).relative_n(r.rotate(Dir::East), 7);
                        self.add(pick(Dir::North), p, r.then(Rotation::Clockwise90), Mirror::None);
                    }
                    if ty == ROOM_1X1 {
                        self.add_room_1x1(c, r, entry, floor);
                    } else if ty == ROOM_1X2 && let Some(e) = entry {
                        let d = grid.room_1x2_direction(x, y, floor, id);
                        self.add_room_1x2(c, r, d, e, floor, v & STAIRS_FLAG == STAIRS_FLAG);
                    } else if ty == ROOM_2X2 && let Some(e) = entry.filter(|&e| e != Dir::Up) {
                        let mut d = e.clockwise();
                        let (dx, dz) = step(d);
                        if !grid.is_room_id(x + dx, y + dz, floor, id) {
                            d = d.opposite();
                        }
                        self.add_room_2x2(c, r, d, e, floor);
                    } else if ty == ROOM_2X2 && entry == Some(Dir::Up) {
                        self.add("2x2_s1", c.relative_n(r.rotate(Dir::East), 1), r, Mirror::None);
                    }
                }
            }
        }
    }

    /// `traverseOuterWalls`.
    #[allow(clippy::too_many_arguments)]
    fn traverse_outer_walls(&mut self, data: &mut Placement, g: &SimpleGrid, d: Dir, sx: i32, sy: i32, ex: i32, ey: i32) {
        let (mut x, mut y, mut d) = (sx, sy, d);
        let first = d;
        loop {
            let (dx, dz) = step(d);
            if !is_house(g, x + dx, y + dz) {
                self.traverse_turn(data);
                d = d.clockwise();
                if x != ex || y != ey || first != d {
                    self.traverse_wall_piece(data);
                }
            } else {
                let (cx, cz) = step(d.counter_clockwise());
                if is_house(g, x + dx + cx, y + dz + cz) {
                    self.traverse_inner_turn(data);
                    x += dx;
                    y += dz;
                    d = d.counter_clockwise();
                } else {
                    x += dx;
                    y += dz;
                    if x != ex || y != ey || first != d {
                        self.traverse_wall_piece(data);
                    }
                }
            }
            if x == ex && y == ey && first == d {
                break;
            }
        }
    }

    /// `traverseWallPiece`.
    fn traverse_wall_piece(&mut self, data: &mut Placement) {
        let r = data.rotation;
        self.add(data.wall, data.position.relative_n(r.rotate(Dir::East), 7), r, Mirror::None);
        data.position = data.position.relative_n(r.rotate(Dir::South), 8);
    }

    /// `traverseTurn`.
    fn traverse_turn(&mut self, data: &mut Placement) {
        let r = data.rotation;
        data.position = data.position.relative_n(r.rotate(Dir::South), -1);
        self.add("wall_corner", data.position, r, Mirror::None);
        data.position = data.position.relative_n(r.rotate(Dir::South), -7);
        data.position = data.position.relative_n(r.rotate(Dir::West), -6);
        data.rotation = r.then(Rotation::Clockwise90);
    }

    /// `traverseInnerTurn`.
    fn traverse_inner_turn(&mut self, data: &mut Placement) {
        let r = data.rotation;
        data.position = data.position.relative_n(r.rotate(Dir::South), 6);
        data.position = data.position.relative_n(r.rotate(Dir::East), 8);
        data.rotation = r.then(Rotation::CounterClockwise90);
    }

    /// `createRoof`.
    fn create_roof(&mut self, pos: BlockPos, r: Rotation, g: &SimpleGrid, upper: Option<&SimpleGrid>) {
        let (east, south, west, north) = (r.rotate(Dir::East), r.rotate(Dir::South), r.rotate(Dir::West), r.rotate(Dir::North));
        let covered = |x: i32, y: i32| upper.is_some_and(|u| is_house(u, x, y));
        for y in 0..g.height {
            for x in 0..g.width {
                let c = self.cell(pos, r, x, y, 0);
                if !is_house(g, x, y) || covered(x, y) {
                    continue;
                }
                self.add("roof", c.above_n(3), r, Mirror::None);
                if !is_house(g, x + 1, y) {
                    self.add("roof_front", c.relative_n(east, 6), r, Mirror::None);
                }
                if !is_house(g, x - 1, y) {
                    self.add("roof_front", c.relative_n(east, 0).relative_n(south, 7), r.then(Rotation::Clockwise180), Mirror::None);
                }
                if !is_house(g, x, y - 1) {
                    self.add("roof_front", c.relative_n(west, 1), r.then(Rotation::CounterClockwise90), Mirror::None);
                }
                if !is_house(g, x, y + 1) {
                    self.add("roof_front", c.relative_n(east, 6).relative_n(south, 6), r.then(Rotation::Clockwise90), Mirror::None);
                }
            }
        }
        if let Some(u) = upper {
            for y in 0..g.height {
                for x in 0..g.width {
                    let c = self.cell(pos, r, x, y, 0);
                    if !is_house(g, x, y) || !is_house(u, x, y) {
                        continue;
                    }
                    if !is_house(g, x + 1, y) {
                        self.add("small_wall", c.relative_n(east, 7), r, Mirror::None);
                    }
                    if !is_house(g, x - 1, y) {
                        self.add("small_wall", c.relative_n(west, 1).relative_n(south, 6), r.then(Rotation::Clockwise180), Mirror::None);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("small_wall", c.relative_n(west, 0).relative_n(north, 1), r.then(Rotation::CounterClockwise90), Mirror::None);
                    }
                    if !is_house(g, x, y + 1) {
                        self.add("small_wall", c.relative_n(east, 6).relative_n(south, 7), r.then(Rotation::Clockwise90), Mirror::None);
                    }
                    if !is_house(g, x + 1, y) {
                        if !is_house(g, x, y - 1) {
                            self.add("small_wall_corner", c.relative_n(east, 7).relative_n(north, 2), r, Mirror::None);
                        }
                        if !is_house(g, x, y + 1) {
                            self.add("small_wall_corner", c.relative_n(east, 8).relative_n(south, 7), r.then(Rotation::Clockwise90), Mirror::None);
                        }
                    }
                    if !is_house(g, x - 1, y) {
                        if !is_house(g, x, y - 1) {
                            self.add("small_wall_corner", c.relative_n(west, 2).relative_n(north, 1), r.then(Rotation::CounterClockwise90), Mirror::None);
                        }
                        if !is_house(g, x, y + 1) {
                            self.add("small_wall_corner", c.relative_n(west, 1).relative_n(south, 8), r.then(Rotation::Clockwise180), Mirror::None);
                        }
                    }
                }
            }
        }
        for y in 0..g.height {
            for x in 0..g.width {
                let c = self.cell(pos, r, x, y, 0);
                if !is_house(g, x, y) || covered(x, y) {
                    continue;
                }
                if !is_house(g, x + 1, y) {
                    let e = c.relative_n(east, 6);
                    if !is_house(g, x, y + 1) {
                        self.add("roof_corner", e.relative_n(south, 6), r, Mirror::None);
                    } else if is_house(g, x + 1, y + 1) {
                        self.add("roof_inner_corner", e.relative_n(south, 5), r, Mirror::None);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("roof_corner", e, r.then(Rotation::CounterClockwise90), Mirror::None);
                    } else if is_house(g, x + 1, y - 1) {
                        self.add("roof_inner_corner", c.relative_n(east, 9).relative_n(north, 2), r.then(Rotation::Clockwise90), Mirror::None);
                    }
                }
                if !is_house(g, x - 1, y) {
                    let w = c.relative_n(east, 0).relative_n(south, 0);
                    if !is_house(g, x, y + 1) {
                        self.add("roof_corner", w.relative_n(south, 6), r.then(Rotation::Clockwise90), Mirror::None);
                    } else if is_house(g, x - 1, y + 1) {
                        self.add("roof_inner_corner", w.relative_n(south, 8).relative_n(west, 3), r.then(Rotation::CounterClockwise90), Mirror::None);
                    }
                    if !is_house(g, x, y - 1) {
                        self.add("roof_corner", w, r.then(Rotation::Clockwise180), Mirror::None);
                    } else if is_house(g, x - 1, y - 1) {
                        self.add("roof_inner_corner", w.relative_n(south, 1), r.then(Rotation::Clockwise180), Mirror::None);
                    }
                }
            }
        }
    }

    /// `addRoom1x1`.
    fn add_room_1x1(&mut self, c: BlockPos, r: Rotation, entry: Option<Dir>, floor: usize) {
        let mut rot = Rotation::None;
        let mut name = room_1x1(floor, self.random);
        match entry {
            Some(Dir::East) => {}
            Some(Dir::North) => rot = rot.then(Rotation::CounterClockwise90),
            Some(Dir::West) => rot = rot.then(Rotation::Clockwise180),
            Some(Dir::South) => rot = rot.then(Rotation::Clockwise90),
            _ => name = room_1x1_secret(self.random),
        }
        let zero = zero_position_with_transform(BlockPos::new(1, 0, 0), Mirror::None, rot, 7, 7);
        let rot = rot.then(r);
        let zero = rotate_pos(zero, r);
        self.add(&name, c.offset(zero.x, 0, zero.z), rot, Mirror::None);
    }

    /// `addRoom1x2`: `d` is where the second cell lies, `e` the entrance side.
    fn add_room_1x2(&mut self, c: BlockPos, r: Rotation, d: Option<Dir>, e: Dir, floor: usize, stairs: bool) {
        let (east, south, west, north) = (r.rotate(Dir::East), r.rotate(Dir::South), r.rotate(Dir::West), r.rotate(Dir::North));
        let cw = r.then(Rotation::Clockwise90);
        let (name, p, rot, m) = match (e, d) {
            (Dir::East, Some(Dir::South)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 1), r, Mirror::None),
            (Dir::East, Some(Dir::North)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 1).relative_n(south, 6), r, Mirror::LeftRight),
            (Dir::West, Some(Dir::North)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 7).relative_n(south, 6), r.then(Rotation::Clockwise180), Mirror::None),
            (Dir::West, Some(Dir::South)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 7), r, Mirror::FrontBack),
            (Dir::South, Some(Dir::East)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 1), cw, Mirror::LeftRight),
            (Dir::South, Some(Dir::West)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 7), cw, Mirror::None),
            (Dir::North, Some(Dir::West)) => (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 7).relative_n(south, 6), cw, Mirror::FrontBack),
            (Dir::North, Some(Dir::East)) => {
                (room_1x2_side(floor, self.random, stairs), c.relative_n(east, 1).relative_n(south, 6), r.then(Rotation::CounterClockwise90), Mirror::None)
            }
            (Dir::South, Some(Dir::North)) => (room_1x2_front(floor, self.random, stairs), c.relative_n(east, 1).relative_n(north, 8), r, Mirror::None),
            (Dir::North, Some(Dir::South)) => {
                (room_1x2_front(floor, self.random, stairs), c.relative_n(east, 7).relative_n(south, 14), r.then(Rotation::Clockwise180), Mirror::None)
            }
            (Dir::West, Some(Dir::East)) => (room_1x2_front(floor, self.random, stairs), c.relative_n(east, 15), cw, Mirror::None),
            (Dir::East, Some(Dir::West)) => {
                (room_1x2_front(floor, self.random, stairs), c.relative_n(west, 7).relative_n(south, 6), r.then(Rotation::CounterClockwise90), Mirror::None)
            }
            (Dir::Up, Some(Dir::East)) => (room_1x2_secret(floor, self.random), c.relative_n(east, 15), cw, Mirror::None),
            (Dir::Up, Some(Dir::South)) => (room_1x2_secret(floor, self.random), c.relative_n(east, 1).relative_n(north, 0), r, Mirror::None),
            _ => return,
        };
        self.add(&name, p, rot, m);
    }

    /// `addRoom2x2`: `d` is the room's side direction, `e` the entrance side.
    fn add_room_2x2(&mut self, c: BlockPos, r: Rotation, d: Dir, e: Dir, floor: usize) {
        let (mut ox, mut oz, mut rot, mut m) = (0, 0, r, Mirror::None);
        match (e, d) {
            (Dir::East, Dir::South) => ox = -7,
            (Dir::East, Dir::North) => (ox, oz, m) = (-7, 6, Mirror::LeftRight),
            (Dir::North, Dir::East) => (ox, oz, rot) = (1, 14, r.then(Rotation::CounterClockwise90)),
            (Dir::North, Dir::West) => (ox, oz, rot, m) = (7, 14, r.then(Rotation::CounterClockwise90), Mirror::LeftRight),
            (Dir::South, Dir::West) => (ox, oz, rot) = (7, -8, r.then(Rotation::Clockwise90)),
            (Dir::South, Dir::East) => (ox, oz, rot, m) = (1, -8, r.then(Rotation::Clockwise90), Mirror::LeftRight),
            (Dir::West, Dir::North) => (ox, oz, rot) = (15, 6, r.then(Rotation::Clockwise180)),
            (Dir::West, Dir::South) => (ox, m) = (15, Mirror::FrontBack),
            _ => {}
        }
        let p = c.relative_n(r.rotate(Dir::East), ox).relative_n(r.rotate(Dir::South), oz);
        let name = room_2x2(floor, self.random);
        self.add(&name, p, rot, m);
    }
}

/// `WoodlandMansionPieces.WoodlandMansionPiece` (a `TemplateStructurePiece`).
#[derive(Debug)]
struct MansionPiece {
    base: PieceBase,
    name: String,
    template: Arc<Template>,
    position: BlockPos,
}

impl MansionPiece {
    fn new(tm: &TemplateManager, name: &str, position: BlockPos, rotation: Rotation, mirror: Mirror) -> Self {
        let template = tm.get(&format!("minecraft:woodland_mansion/{name}"));
        let mut settings = PlaceSettings::with_rotation(rotation);
        settings.mirror = mirror;
        let bbox = template.bounding_box(&settings, position);
        let mut base = PieceBase::new("minecraft:wmp", 0, bbox);
        base.set_orientation(Some(Dir::North));
        base.rotation = rotation;
        base.mirror = mirror;
        Self { base, name: name.to_string(), template, position }
    }

    /// `makeSettings`.
    fn settings(&self, cb: &BoundingBox) -> PlaceSettings<'static> {
        let mut s = PlaceSettings::with_rotation(self.base.rotation);
        s.mirror = self.base.mirror;
        s.ignore_entities = true;
        s.processors.push(&IGNORE_STRUCTURE_BLOCK);
        s.bbox = Some(*cb);
        s
    }

    /// `handleDataMarker`.
    fn data_marker(&self, marker: &str, p: BlockPos, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox) {
        if marker.starts_with("Chest") {
            let rot = self.base.rotation;
            let mut s = st("minecraft:chest");
            let facing = match marker {
                "ChestWest" => Some(Dir::West),
                "ChestEast" => Some(Dir::East),
                "ChestSouth" => Some(Dir::South),
                "ChestNorth" => Some(Dir::North),
                _ => None,
            };
            if let Some(d) = facing {
                s = with_prop(s, "facing", rot.rotate(d).name());
            }
            create_chest_at(r, cb, random, p, "minecraft:chests/woodland_mansion", Some(s));
            return;
        }
        // Mobs are not spawned; their markers are cleared as vanilla does once it adds them.
        match marker {
            "Mage" | "Warrior" => {}
            "Group of Allays" => {
                r.level_random().next_int_bounded(3);
            }
            _ => return,
        }
        r.set(p, crate::blocks::state::AIR, 2);
    }
}

impl Piece for MansionPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.shift(dx, dy, dz);
        self.position = self.position.offset(dx, dy, dz);
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        tag.push(("TPX".into(), Tag::Int(self.position.x)));
        tag.push(("TPY".into(), Tag::Int(self.position.y)));
        tag.push(("TPZ".into(), Tag::Int(self.position.z)));
        tag.push(("Template".into(), Tag::String(self.name.clone())));
        tag.push(("Rot".into(), Tag::String(self.base.rotation.enum_name().into())));
        tag.push(("Mi".into(), Tag::String(self.base.mirror.enum_name().into())));
    }

    /// `TemplateStructurePiece.postProcess`.
    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, cb: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let mut settings = self.settings(cb);
        if !self.template.place_in_world(r, self.position, pivot, &mut settings, random, 2) {
            return;
        }
        for m in self.template.markers(self.position, &mut settings, true) {
            let Some(Tag::Compound(fields)) = m.nbt.as_deref() else { continue };
            let text = |k: &str| fields.iter().find(|(n, _)| n == k).and_then(|(_, v)| if let Tag::String(s) = v { Some(s.as_str()) } else { None });
            if text("mode") == Some("DATA") {
                let marker = text("metadata").unwrap_or("").to_string();
                self.data_marker(&marker, m.pos, r, random, cb);
            }
        }
    }
}
