//! Pathfinding: `WalkNodeEvaluator`, the A* `PathFinder` with vanilla's `BinaryHeap`, `Path`,
//! and `GroundPathNavigation` / `WallClimberNavigation` (spiders).

use super::MobData;
use super::mth;
use crate::collision::{self, CollisionContext};
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{Aabb, Axis, BlockPos, Vec3, floor};
use std::collections::HashMap;

// ---------------------------------------------------------------------- block facts

static RAW: &[u8] = include_bytes!("../gen/mob_blocks.bin");

fn table() -> &'static [u8] {
    static T: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        assert_eq!(&RAW[..4], b"KMB1", "mob_blocks.bin: bad magic");
        let n = u32::from_le_bytes(RAW[4..8].try_into().unwrap()) as usize;
        assert_eq!(RAW.len(), 8 + 2 * n, "mob_blocks.bin: size");
        &RAW[8..]
    })
}

fn flags(state: u16) -> u8 {
    table().get(state as usize * 2 + 1).copied().unwrap_or(0)
}

/// `WalkNodeEvaluator.getPathTypeFromState` (a function of the state alone).
pub fn path_type_from_state(state: u16) -> PathType {
    let t = PathType::ALL[table().get(state as usize * 2).copied().unwrap_or(0) as usize];
    // The table was extracted without bound fluid tags: vanilla's `FluidTags.LAVA` and
    // `FluidTags.WATER` checks make lava `LAVA` and (pathfindable) water `WATER`.
    let f = crate::physics::fluid_state(state);
    if f.kind.is_lava() {
        return PathType::Lava;
    }
    if t == PathType::Open && f.kind.is_water() {
        return PathType::Water;
    }
    t
}

/// `isPathfindable(LAND)`.
pub fn pathfindable_land(state: u16) -> bool {
    flags(state) & 1 != 0
}

/// `isValidSpawn` on top of `state` for a monster (`zombie`) or an animal (`pig`).
pub fn valid_spawn(state: u16, animal: bool) -> bool {
    flags(state) & if animal { 4 } else { 2 } != 0
}

/// `NaturalSpawner.isValidEmptySpawnBlock` for a monster or an animal.
pub fn valid_empty_spawn(state: u16, animal: bool) -> bool {
    flags(state) & if animal { 16 } else { 8 } != 0
}

/// `isCollisionShapeFullBlock`.
pub fn collision_full_block(state: u16) -> bool {
    flags(state) & 32 != 0
}

// ---------------------------------------------------------------------- path types

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathType {
    Blocked,
    Open,
    Walkable,
    WalkableDoor,
    Trapdoor,
    PowderSnow,
    OnTopOfPowderSnow,
    Fence,
    Lava,
    Water,
    WaterBorder,
    Rail,
    UnpassableRail,
    FireInNeighbor,
    Fire,
    DamagingInNeighbor,
    Damaging,
    DoorOpen,
    DoorWoodClosed,
    DoorIronClosed,
    Breach,
    Leaves,
    StickyHoney,
    Cocoa,
    DamageCautious,
    OnTopOfTrapdoor,
    BigMobsCloseToDanger,
}

impl PathType {
    pub const ALL: [PathType; 27] = {
        use PathType::*;
        [
            Blocked, Open, Walkable, WalkableDoor, Trapdoor, PowderSnow, OnTopOfPowderSnow, Fence, Lava, Water, WaterBorder, Rail,
            UnpassableRail, FireInNeighbor, Fire, DamagingInNeighbor, Damaging, DoorOpen, DoorWoodClosed, DoorIronClosed, Breach,
            Leaves, StickyHoney, Cocoa, DamageCautious, OnTopOfTrapdoor, BigMobsCloseToDanger,
        ]
    };

    /// `PathType.getMalus`.
    pub fn malus(self) -> f32 {
        use PathType::*;
        match self {
            Blocked | PowderSnow | Fence | Lava | UnpassableRail | Damaging | DoorWoodClosed | DoorIronClosed | Leaves => -1.0,
            Water | WaterBorder | FireInNeighbor | DamagingInNeighbor | StickyHoney => 8.0,
            Fire => 16.0,
            Breach | BigMobsCloseToDanger => 4.0,
            _ => 0.0,
        }
    }
}

/// `Mob.getPathfindingMalus`.
pub fn malus(m: &MobData, t: PathType) -> f32 {
    m.maluses.iter().find(|(k, _)| *k == t).map_or(t.malus(), |&(_, v)| v)
}

fn type_at(level: &dyn EntityLevel, x: i32, y: i32, z: i32) -> PathType {
    path_type_from_state(level.block(BlockPos::new(x, y, z)))
}

/// `WalkNodeEvaluator.getPathTypeStatic`.
pub fn path_type_static(level: &dyn EntityLevel, x: i32, y: i32, z: i32) -> PathType {
    let t = type_at(level, x, y, z);
    if t != PathType::Open || y < level.min_y() + 1 {
        return t;
    }
    use PathType::*;
    match type_at(level, x, y - 1, z) {
        Open | Water | Lava | Walkable => Open,
        Fire => Fire,
        Damaging => Damaging,
        StickyHoney => StickyHoney,
        PowderSnow => OnTopOfPowderSnow,
        DamageCautious => DamageCautious,
        Trapdoor => OnTopOfTrapdoor,
        _ => check_neighbour_blocks(level, x, y, z, Walkable),
    }
}

/// `WalkNodeEvaluator.checkNeighbourBlocks`.
fn check_neighbour_blocks(level: &dyn EntityLevel, x: i32, y: i32, z: i32, default: PathType) -> PathType {
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                if dx == 0 && dz == 0 {
                    continue;
                }
                match type_at(level, x + dx, y + dy, z + dz) {
                    PathType::Damaging => return PathType::DamagingInNeighbor,
                    PathType::Fire | PathType::Lava => return PathType::FireInNeighbor,
                    PathType::Water => return PathType::WaterBorder,
                    PathType::DamageCautious => return PathType::DamageCautious,
                    _ => {}
                }
            }
        }
    }
    default
}

/// `getCollisionShape(level, pos)` with the empty context: its top, if any.
fn shape_top(level: &dyn EntityLevel, pos: BlockPos) -> Option<f64> {
    let (shape, _) = collision::collision_shape(level.block(pos), pos, &CollisionContext::EMPTY);
    (!shape.is_empty()).then(|| shape.max(Axis::Y, 0.0))
}

/// `WalkNodeEvaluator.getFloorLevel(BlockGetter, pos)`.
pub fn floor_level(level: &dyn EntityLevel, pos: BlockPos) -> f64 {
    let below = pos.below();
    below.y as f64 + shape_top(level, below).unwrap_or(0.0)
}

// ---------------------------------------------------------------------- nodes

#[derive(Clone, Debug)]
pub struct Node {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    hash: i32,
    heap_idx: i32,
    pub g: f32,
    pub h: f32,
    pub f: f32,
    came_from: Option<u32>,
    pub closed: bool,
    pub walked_distance: f32,
    pub cost_malus: f32,
    pub kind: PathType,
}

/// `Node.createHash`.
pub fn node_hash(x: i32, y: i32, z: i32) -> i32 {
    (y & 255) | ((x & 32767) << 8) | ((z & 32767) << 24) | if x < 0 { i32::MIN } else { 0 } | if z < 0 { 32768 } else { 0 }
}

impl Node {
    fn new(x: i32, y: i32, z: i32) -> Node {
        Node {
            x,
            y,
            z,
            hash: node_hash(x, y, z),
            heap_idx: -1,
            g: 0.0,
            h: 0.0,
            f: 0.0,
            came_from: None,
            closed: false,
            walked_distance: 0.0,
            cost_malus: 0.0,
            kind: PathType::Blocked,
        }
    }

    fn distance_to(&self, o: &Node) -> f32 {
        let (a, b, c) = ((o.x - self.x) as f32, (o.y - self.y) as f32, (o.z - self.z) as f32);
        mth::sqrt_f(a * a + b * b + c * c)
    }

    fn distance_manhattan(&self, x: i32, y: i32, z: i32) -> f32 {
        (x - self.x).abs() as f32 + (y - self.y).abs() as f32 + (z - self.z).abs() as f32
    }

    pub fn pos(&self) -> BlockPos {
        BlockPos::new(self.x, self.y, self.z)
    }

    fn same(&self, o: &Node) -> bool {
        self.hash == o.hash && self.x == o.x && self.y == o.y && self.z == o.z
    }
}

/// A finished path: its nodes (copies) and where it is.
#[derive(Clone, Debug)]
pub struct Path {
    pub nodes: Vec<Node>,
    pub next: usize,
    pub target: BlockPos,
    pub dist_to_target: f32,
    pub reached: bool,
}

impl Path {
    fn new(nodes: Vec<Node>, target: BlockPos, reached: bool) -> Path {
        let dist = nodes.last().map_or(f32::MAX, |n| n.distance_manhattan(target.x, target.y, target.z));
        Path { nodes, next: 0, target, dist_to_target: dist, reached }
    }

    pub fn is_done(&self) -> bool {
        self.next >= self.nodes.len()
    }

    pub fn end_node(&self) -> Option<&Node> {
        self.nodes.last()
    }

    /// `getEntityPosAtNode`.
    pub fn entity_pos_at(&self, width: f32, i: usize) -> Vec3 {
        let n = &self.nodes[i];
        let off = ((width + 1.0) as i32) as f64 * 0.5;
        Vec3::new(n.x as f64 + off, n.y as f64, n.z as f64 + off)
    }

    pub fn next_entity_pos(&self, width: f32) -> Vec3 {
        self.entity_pos_at(width, self.next)
    }

    pub fn next_node_pos(&self) -> BlockPos {
        self.nodes[self.next].pos()
    }

    /// `sameAs`: the same nodes (by `Node.equals`).
    pub fn same_as(&self, o: Option<&Path>) -> bool {
        o.is_some_and(|o| o.nodes.len() == self.nodes.len() && self.nodes.iter().zip(&o.nodes).all(|(a, b)| a.same(b)))
    }
}

// ---------------------------------------------------------------------- evaluator + A*

/// The per-search state of `WalkNodeEvaluator` (its node pool and caches) and `PathFinder`.
struct Search<'a> {
    level: &'a dyn EntityLevel,
    e: &'a Entity,
    m: &'a MobData,
    nodes: Vec<Node>,
    by_hash: HashMap<i32, u32>,
    types: HashMap<i64, PathType>,
    collisions: HashMap<[u64; 6], bool>,
    width: i32,
    height: i32,
    depth: i32,
    can_float: bool,
    can_open_doors: bool,
    can_pass_doors: bool,
    can_walk_over_fences: bool,
    heap: Vec<u32>,
}

impl<'a> Search<'a> {
    fn node(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let h = node_hash(x, y, z);
        if let Some(&i) = self.by_hash.get(&h) {
            return i;
        }
        let i = self.nodes.len() as u32;
        self.nodes.push(Node::new(x, y, z));
        self.by_hash.insert(h, i);
        i
    }

    fn n(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    fn nm(&mut self, i: u32) -> &mut Node {
        &mut self.nodes[i as usize]
    }

    fn malus(&self, t: PathType) -> f32 {
        malus(self.m, t)
    }

    fn max_up_step(&self) -> f32 {
        self.e.max_up_step
    }

    fn cached_type(&mut self, x: i32, y: i32, z: i32) -> PathType {
        let key = BlockPos::new(x, y, z).as_long();
        if let Some(&t) = self.types.get(&key) {
            return t;
        }
        let t = self.type_of_mob(x, y, z);
        self.types.insert(key, t);
        t
    }

    /// `getPathTypeOfMob`.
    fn type_of_mob(&mut self, x: i32, y: i32, z: i32) -> PathType {
        let set = self.types_within_bb(x, y, z);
        if set.len() == 1 {
            return set[0];
        }
        if set.contains(&PathType::Fence) {
            return PathType::Fence;
        }
        if set.contains(&PathType::UnpassableRail) {
            return PathType::UnpassableRail;
        }
        let mut best = PathType::Blocked;
        let mut best_malus = self.malus(best);
        for &t in &set {
            let v = self.malus(t);
            if v < 0.0 {
                return t;
            }
            if v >= best_malus {
                best_malus = v;
                best = t;
            }
        }
        let here = path_type_static(self.level, x, y, z);
        if self.width > 1 {
            if self.malus(here) < best_malus && self.malus(PathType::BigMobsCloseToDanger) < best_malus {
                return PathType::BigMobsCloseToDanger;
            }
            return best;
        }
        if here == PathType::Open && best != PathType::Open && best_malus == 0.0 {
            return PathType::Open;
        }
        best
    }

    /// `getPathTypeWithinMobBB`: an `EnumSet` (ordinal order).
    fn types_within_bb(&mut self, x: i32, y: i32, z: i32) -> Vec<PathType> {
        let mut set: Vec<PathType> = Vec::with_capacity(2);
        let mob = self.e.block_position();
        for i in 0..self.width {
            for j in 0..self.height {
                for k in 0..self.depth {
                    let (px, py, pz) = (x + i, y + j, z + k);
                    let mut t = path_type_static(self.level, px, py, pz);
                    if t == PathType::DoorWoodClosed && self.can_open_doors && self.can_pass_doors {
                        t = PathType::WalkableDoor;
                    }
                    if t == PathType::DoorOpen && !self.can_pass_doors {
                        t = PathType::Blocked;
                    }
                    if t == PathType::Rail
                        && path_type_static(self.level, mob.x, mob.y, mob.z) != PathType::Rail
                        && path_type_static(self.level, mob.x, mob.y - 1, mob.z) != PathType::Rail
                    {
                        t = PathType::UnpassableRail;
                    }
                    if let Err(pos) = set.binary_search(&t) {
                        set.insert(pos, t);
                    }
                }
            }
        }
        set
    }

    fn floor_level(&self, pos: BlockPos) -> f64 {
        if self.can_float && crate::fluid::fluid_at(self.level, pos).kind.is_water() {
            return pos.y as f64 + 0.5;
        }
        floor_level(self.level, pos)
    }

    fn has_collisions(&mut self, b: &Aabb) -> bool {
        let key = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z].map(f64::to_bits);
        if let Some(&v) = self.collisions.get(&key) {
            return v;
        }
        let ctx = self.e.collision_context();
        let v = !collision::no_collision(self.level, &ctx, self.e.id, b);
        self.collisions.insert(key, v);
        v
    }

    /// `getStart`.
    fn start(&mut self) -> Option<u32> {
        let e = self.e;
        let mut y = e.block_position().y;
        let at = |x: f64, y: i32, z: f64| BlockPos::containing(x, y as f64, z);
        let state = self.level.block(at(e.x(), y, e.z()));
        if self.can_float && e.fluid.is_in_water() {
            let mut s = state;
            while crate::physics::fluid_state(s).kind.is_water() {
                y += 1;
                s = self.level.block(at(e.x(), y, e.z()));
            }
            y -= 1;
        } else if e.on_ground {
            y = floor(e.y() + 0.5);
        } else {
            let mut p = at(e.x(), 0, e.z()).at_y(floor(e.y() + 1.0));
            while p.y > self.level.min_y() {
                y = p.y;
                p = p.below();
                let s = self.level.block(p);
                if !(kiln_data::blocks_types::is_air(s) || pathfindable_land(s)) {
                    break;
                }
            }
        }
        let bp = e.block_position();
        if !self.can_start_at(bp.x, y, bp.z) {
            let bb = e.bounding_box();
            for (x, z) in [(bb.min_x, bb.min_z), (bb.min_x, bb.max_z), (bb.max_x, bb.min_z), (bb.max_x, bb.max_z)] {
                let p = at(x, y, z);
                if self.can_start_at(p.x, p.y, p.z) {
                    return Some(self.start_node(p.x, p.y, p.z));
                }
            }
        }
        Some(self.start_node(bp.x, y, bp.z))
    }

    fn start_node(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let i = self.node(x, y, z);
        let t = self.cached_type(x, y, z);
        let m = self.malus(t);
        let n = self.nm(i);
        n.kind = t;
        n.cost_malus = m;
        i
    }

    fn can_start_at(&mut self, x: i32, y: i32, z: i32) -> bool {
        let t = self.cached_type(x, y, z);
        t != PathType::Open && self.malus(t) >= 0.0
    }

    /// `getNeighbors`.
    fn neighbors(&mut self, out: &mut Vec<u32>, cur: u32) {
        out.clear();
        let (x, y, z) = { let n = self.n(cur); (n.x, n.y, n.z) };
        let above = self.cached_type(x, y + 1, z);
        let here = self.cached_type(x, y, z);
        let mut jump = 0;
        if self.malus(above) >= 0.0 && here != PathType::StickyHoney {
            jump = floor(1f32.max(self.max_up_step()) as f64);
        }
        let floor_y = self.floor_level(BlockPos::new(x, y, z));
        // Direction.Plane.HORIZONTAL: north, east, south, west; 2D data values s=0 w=1 n=2 e=3.
        const HORIZ: [(i32, i32, usize); 4] = [(0, -1, 2), (1, 0, 3), (0, 1, 0), (-1, 0, 1)];
        let mut reusable: [Option<u32>; 4] = [None; 4];
        for &(dx, dz, d2) in &HORIZ {
            let n = self.accepted_node(x + dx, y, z + dz, jump, floor_y, (dx, dz), here);
            reusable[d2] = n;
            if self.neighbor_valid(n, cur) {
                out.push(n.unwrap());
            }
        }
        // `getClockWise`: north->east, east->south, south->west, west->north.
        const CW: [usize; 4] = [3, 0, 1, 2]; // indexed by the 2D value of the direction
        for &(dx, dz, d2) in &HORIZ {
            let cw = CW[d2];
            let (cx, cz) = match cw {
                0 => (0, 1),
                1 => (-1, 0),
                2 => (0, -1),
                _ => (1, 0),
            };
            if self.diagonal_valid3(cur, reusable[d2], reusable[cw]) {
                let n = self.accepted_node(x + dx + cx, y, z + dz + cz, jump, floor_y, (dx, dz), here);
                if self.diagonal_valid(n) {
                    out.push(n.unwrap());
                }
            }
        }
    }

    fn neighbor_valid(&self, n: Option<u32>, cur: u32) -> bool {
        let Some(n) = n else { return false };
        let n = self.n(n);
        !n.closed && (n.cost_malus >= 0.0 || self.n(cur).cost_malus < 0.0)
    }

    fn diagonal_valid3(&self, root: u32, a: Option<u32>, b: Option<u32>) -> bool {
        let (Some(a), Some(b)) = (a, b) else { return false };
        let (r, a, b) = (self.n(root), self.n(a), self.n(b));
        if b.y > r.y || a.y > r.y {
            return false;
        }
        if a.kind == PathType::WalkableDoor || b.kind == PathType::WalkableDoor {
            return false;
        }
        if self.e.width > 1.0 && (a.cost_malus > 0.0 || b.cost_malus > 0.0) {
            return false;
        }
        let fences = b.kind == PathType::Fence && a.kind == PathType::Fence && (self.e.width as f64) < 0.5;
        (b.y < r.y || b.cost_malus >= 0.0 || fences) && (a.y < r.y || a.cost_malus >= 0.0 || fences)
    }

    fn diagonal_valid(&self, n: Option<u32>) -> bool {
        let Some(n) = n else { return false };
        let n = self.n(n);
        !n.closed && n.kind != PathType::WalkableDoor && n.cost_malus >= 0.0
    }

    fn partial_collision(t: PathType) -> bool {
        matches!(t, PathType::Fence | PathType::DoorWoodClosed | PathType::DoorIronClosed)
    }

    fn jump_height(&self) -> f64 {
        1.125f64.max(self.max_up_step() as f64)
    }

    /// `findAcceptedNode`.
    #[allow(clippy::too_many_arguments)]
    fn accepted_node(&mut self, x: i32, y: i32, z: i32, jump: i32, floor_y: f64, dir: (i32, i32), here: PathType) -> Option<u32> {
        let mut node = None;
        let fl = self.floor_level(BlockPos::new(x, y, z));
        if fl - floor_y > self.jump_height() {
            return None;
        }
        let t = self.cached_type(x, y, z);
        let malus = self.malus(t);
        if malus >= 0.0 {
            node = Some(self.update_cost(x, y, z, t, malus));
        }
        if Self::partial_collision(here)
            && let Some(n) = node
            && self.n(n).cost_malus >= 0.0
            && !self.reach_without_collision(n)
        {
            node = None;
        }
        if t == PathType::Walkable {
            return node;
        }
        let blocked = node.is_none_or(|n| self.n(n).cost_malus < 0.0);
        if blocked
            && jump > 0
            && (t != PathType::Fence || self.can_walk_over_fences)
            && t != PathType::UnpassableRail
            && t != PathType::Trapdoor
            && t != PathType::PowderSnow
        {
            return self.try_jump_on(x, y, z, jump, floor_y, dir, here);
        }
        if t == PathType::Water && !self.can_float {
            return self.first_non_water_below(x, y, z, node);
        }
        if t == PathType::Open {
            return Some(self.first_ground_below(x, y, z));
        }
        if Self::partial_collision(t) && node.is_none() {
            let i = self.node(x, y, z);
            let n = self.nm(i);
            n.closed = true;
            n.kind = t;
            n.cost_malus = t.malus();
            return Some(i);
        }
        node
    }

    fn update_cost(&mut self, x: i32, y: i32, z: i32, t: PathType, malus: f32) -> u32 {
        let i = self.node(x, y, z);
        let n = self.nm(i);
        n.kind = t;
        n.cost_malus = n.cost_malus.max(malus);
        i
    }

    fn blocked_node(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let i = self.node(x, y, z);
        let n = self.nm(i);
        n.kind = PathType::Blocked;
        n.cost_malus = -1.0;
        i
    }

    fn reach_without_collision(&mut self, n: u32) -> bool {
        let mut bb = self.e.bounding_box();
        let (nx, ny, nz) = { let n = self.n(n); (n.x, n.y, n.z) };
        let mut v = Vec3::new(
            nx as f64 - self.e.x() + bb.x_size() / 2.0,
            ny as f64 - self.e.y() + bb.y_size() / 2.0,
            nz as f64 - self.e.z() + bb.z_size() / 2.0,
        );
        let steps = mth::ceil(v.length() / bb.size());
        v = v.scale((1.0 / steps as f32) as f64);
        for _ in 1..=steps {
            bb = bb.offset_vec(v);
            if self.has_collisions(&bb) {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn try_jump_on(&mut self, x: i32, y: i32, z: i32, jump: i32, floor_y: f64, dir: (i32, i32), here: PathType) -> Option<u32> {
        let n = self.accepted_node(x, y + 1, z, jump - 1, floor_y, dir, here)?;
        if self.e.width >= 1.0 {
            return Some(n);
        }
        let (kind, nx, ny, nz) = { let n = self.n(n); (n.kind, n.x, n.y, n.z) };
        if kind != PathType::Open && kind != PathType::Walkable {
            return Some(n);
        }
        let cx = (x - dir.0) as f64 + 0.5;
        let cz = (z - dir.1) as f64 + 0.5;
        let w = self.e.width as f64 / 2.0;
        let lo = self.floor_level(BlockPos::containing(cx, (y + 1) as f64, cz)) + 0.001;
        let hi = self.e.height as f64 + self.floor_level(BlockPos::new(nx, ny, nz)) - 0.002;
        let b = Aabb::new(cx - w, lo, cz - w, cx + w, hi, cz + w);
        if self.has_collisions(&b) { None } else { Some(n) }
    }

    fn first_non_water_below(&mut self, x: i32, mut y: i32, z: i32, mut node: Option<u32>) -> Option<u32> {
        y -= 1;
        while y > self.level.min_y() {
            let t = self.cached_type(x, y, z);
            if t != PathType::Water {
                return node;
            }
            let m = self.malus(t);
            node = Some(self.update_cost(x, y, z, t, m));
            y -= 1;
        }
        node
    }

    fn first_ground_below(&mut self, x: i32, y: i32, z: i32) -> u32 {
        let max_fall = 3; // `getMaxFallDistance`: floor(0 + 3)
        let mut i = y - 1;
        while i >= self.level.min_y() {
            if y - i > max_fall {
                return self.blocked_node(x, i, z);
            }
            let t = self.cached_type(x, i, z);
            let m = self.malus(t);
            if t != PathType::Open {
                if m >= 0.0 {
                    return self.update_cost(x, i, z, t, m);
                }
                return self.blocked_node(x, i, z);
            }
            i -= 1;
        }
        self.blocked_node(x, y, z)
    }

    // ------------------------------------------------------------------ BinaryHeap

    fn heap_insert(&mut self, i: u32) {
        let idx = self.heap.len();
        self.heap.push(i);
        self.nm(i).heap_idx = idx as i32;
        self.up_heap(idx);
    }

    fn heap_pop(&mut self) -> u32 {
        let top = self.heap[0];
        let last = self.heap.pop().unwrap();
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.nm(last).heap_idx = 0;
            self.down_heap(0);
        }
        self.nm(top).heap_idx = -1;
        top
    }

    fn change_cost(&mut self, i: u32, f: f32) {
        let old = self.n(i).f;
        self.nm(i).f = f;
        let idx = self.n(i).heap_idx as usize;
        if f < old { self.up_heap(idx) } else { self.down_heap(idx) }
    }

    fn up_heap(&mut self, mut idx: usize) {
        let node = self.heap[idx];
        let f = self.n(node).f;
        while idx > 0 {
            let parent = (idx - 1) >> 1;
            let p = self.heap[parent];
            if f >= self.n(p).f {
                break;
            }
            self.heap[idx] = p;
            self.nm(p).heap_idx = idx as i32;
            idx = parent;
        }
        self.heap[idx] = node;
        self.nm(node).heap_idx = idx as i32;
    }

    fn down_heap(&mut self, mut idx: usize) {
        let node = self.heap[idx];
        let f = self.n(node).f;
        let size = self.heap.len();
        loop {
            let l = 1 + (idx << 1);
            let r = l + 1;
            if l >= size {
                break;
            }
            let ln = self.heap[l];
            let lf = self.n(ln).f;
            let (rn, rf) = if r >= size { (None, f32::INFINITY) } else { (Some(self.heap[r]), self.n(self.heap[r]).f) };
            if lf < rf {
                if lf >= f {
                    break;
                }
                self.heap[idx] = ln;
                self.nm(ln).heap_idx = idx as i32;
                idx = l;
            } else {
                if rf >= f {
                    break;
                }
                let rn = rn.unwrap();
                self.heap[idx] = rn;
                self.nm(rn).heap_idx = idx as i32;
                idx = r;
            }
        }
        self.heap[idx] = node;
        self.nm(node).heap_idx = idx as i32;
    }

    // ------------------------------------------------------------------ A*

    /// `PathFinder.findPath` for one target block.
    fn find(&mut self, target: BlockPos, max_dist: f32, reach: i32, max_visited: i32) -> Option<Path> {
        let start = self.start()?;
        let t = self.node(floor(target.x as f64), floor(target.y as f64), floor(target.z as f64));
        let (tx, ty, tz) = { let n = self.n(t); (n.x, n.y, n.z) };
        let mut best_h = f32::MAX;
        let mut best_node: Option<u32> = None;
        let update_best = |s: &Search, n: u32, best_h: &mut f32, best_node: &mut Option<u32>| {
            let d = s.n(n).distance_to(s.n(t));
            if d < *best_h {
                *best_h = d;
                *best_node = Some(n);
            }
            d
        };
        {
            let h = update_best(self, start, &mut best_h, &mut best_node);
            let n = self.nm(start);
            n.g = 0.0;
            n.h = h;
            n.f = h;
        }
        self.heap.clear();
        self.heap_insert(start);
        let mut reached = false;
        let mut visited = 0;
        let mut neigh = Vec::with_capacity(8);
        while !self.heap.is_empty() {
            visited += 1;
            if visited >= max_visited {
                break;
            }
            let cur = self.heap_pop();
            self.nm(cur).closed = true;
            if self.n(cur).distance_manhattan(tx, ty, tz) <= reach as f32 {
                reached = true;
            }
            if reached {
                break;
            }
            if self.n(cur).distance_to(self.n(start)) >= max_dist {
                continue;
            }
            self.neighbors(&mut neigh, cur);
            for &nb in &neigh {
                let d = self.n(cur).distance_to(self.n(nb));
                let walked = self.n(cur).walked_distance + d;
                self.nm(nb).walked_distance = walked;
                let g = self.n(cur).g + d + self.n(nb).cost_malus;
                if walked < max_dist && (self.n(nb).heap_idx < 0 || g < self.n(nb).g) {
                    self.nm(nb).came_from = Some(cur);
                    self.nm(nb).g = g;
                    let h = update_best(self, nb, &mut best_h, &mut best_node) * 1.5;
                    self.nm(nb).h = h;
                    if self.n(nb).heap_idx >= 0 {
                        self.change_cost(nb, g + h);
                    } else {
                        self.nm(nb).f = g + h;
                        self.heap_insert(nb);
                    }
                }
            }
        }
        let best = best_node?;
        let mut chain = vec![best];
        let mut c = best;
        while let Some(p) = self.n(c).came_from {
            chain.push(p);
            c = p;
        }
        chain.reverse();
        let nodes = chain.iter().map(|&i| self.n(i).clone()).collect();
        Some(Path::new(nodes, target, reached))
    }
}

// ---------------------------------------------------------------------- navigation

#[derive(Clone, Debug, Default)]
pub struct Navigation {
    pub path: Option<Path>,
    pub speed_modifier: f64,
    pub tick: i32,
    last_stuck_check: i32,
    last_stuck_check_pos: Vec3,
    timeout_cached_node: BlockPos,
    timeout_timer: i64,
    last_timeout_check: i64,
    timeout_limit: f64,
    max_distance_to_waypoint: f32,
    pub is_stuck: bool,
    time_last_recompute: i64,
    pub target_pos: Option<BlockPos>,
    reach_range: i32,
    pub max_visited_nodes_multiplier: f32,
    has_delayed_recomputation: bool,
    pub can_float: bool,
    pub can_open_doors: bool,
    pub can_pass_doors: bool,
    pub can_walk_over_fences: bool,
    pub avoid_sun: bool,
    pub required_path_length: f32,
    /// `WallClimberNavigation.pathToPosition` (spiders).
    pub climber: bool,
    pub path_to_position: Option<BlockPos>,
}

impl Navigation {
    pub fn new(climber: bool) -> Navigation {
        Navigation {
            max_distance_to_waypoint: 0.5,
            max_visited_nodes_multiplier: 1.0,
            required_path_length: 16.0,
            can_pass_doors: true,
            climber,
            ..Default::default()
        }
    }

    pub fn is_done(&self) -> bool {
        self.path.as_ref().is_none_or(Path::is_done)
    }

    pub fn stop(&mut self) {
        self.path = None;
    }

    fn reset_stuck_timeout(&mut self) {
        self.timeout_cached_node = BlockPos::default();
        self.timeout_timer = 0;
        self.timeout_limit = 0.0;
        self.is_stuck = false;
    }
}

fn max_path_length(m: &MobData) -> f32 {
    (m.attrs.value(super::attributes::Attr::FollowRange) as f32).max(m.nav.required_path_length)
}

/// `GroundPathNavigation.canUpdatePath`.
fn can_update_path(e: &Entity) -> bool {
    e.on_ground || e.is_in_water() || e.is_in_lava()
}

/// `PathNavigation.createPath(Set<BlockPos>, regionOffset, offsetUpward, reach, maxPathLength)`.
fn create_path_raw(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, target: BlockPos, region: i32, up: bool, reach: i32) -> Option<Path> {
    let max_len = max_path_length(m);
    if e.y() < level.min_y() as f64 || !can_update_path(e) {
        return None;
    }
    if let Some(p) = &m.nav.path
        && !p.is_done()
        && m.nav.target_pos == Some(target)
    {
        return Some(p.clone());
    }
    let _ = (region, up);
    // `updatePathfinderMaxVisitedNodes` (the follow range is at least 16 for every mob here, so
    // the constructor's value agrees).
    let max_visited = (floor((max_len * 16.0) as f64) as f32 * m.nav.max_visited_nodes_multiplier) as i32;
    let mut s = Search {
        level,
        e,
        m,
        nodes: Vec::with_capacity(256),
        by_hash: HashMap::with_capacity(256),
        types: HashMap::with_capacity(256),
        collisions: HashMap::new(),
        width: floor((e.width + 1.0) as f64),
        height: floor((e.height + 1.0) as f64),
        depth: floor((e.width + 1.0) as f64),
        can_float: m.nav.can_float,
        can_open_doors: m.nav.can_open_doors,
        can_pass_doors: m.nav.can_pass_doors,
        can_walk_over_fences: m.nav.can_walk_over_fences,
        heap: Vec::with_capacity(64),
    };
    let path = s.find(target, max_len, reach, max_visited);
    if let Some(p) = &path {
        m.nav.target_pos = Some(p.target);
        m.nav.reach_range = reach;
        m.nav.reset_stuck_timeout();
    }
    path
}

/// `createPath(BlockPos, reach)`: ground navigation first finds the surface.
pub fn create_path(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, pos: BlockPos, reach: i32) -> Option<Path> {
    if m.nav.climber {
        m.nav.path_to_position = Some(pos);
    }
    if !level.is_loaded(pos) {
        return None;
    }
    let pos = find_surface(level, pos);
    create_path_raw(e, m, level, pos, 8, false, reach)
}

/// `createPath(Entity, reach)`.
pub fn create_path_to_entity(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, target: BlockPos, reach: i32) -> Option<Path> {
    if m.nav.climber {
        m.nav.path_to_position = Some(target);
    }
    if !level.is_loaded(target) {
        return None;
    }
    let pos = find_surface(level, target);
    create_path_raw(e, m, level, pos, 16, true, reach)
}

/// `GroundPathNavigation.findSurfacePosition`.
fn find_surface(level: &dyn EntityLevel, mut pos: BlockPos) -> BlockPos {
    let air = |p: BlockPos| kiln_data::blocks_types::is_air(level.block(p));
    let solid = |p: BlockPos| kiln_data::block_logic::is_solid(level.block(p));
    if air(pos) {
        let mut p = pos.below();
        while p.y >= level.min_y() && air(p) {
            p = p.below();
        }
        if p.y >= level.min_y() {
            return p.above();
        }
        p = pos.at_y(pos.y + 1);
        while p.y <= level.max_y() && air(p) {
            p = p.above();
        }
        pos = p;
    }
    if solid(pos) {
        let mut p = pos.above();
        while p.y <= level.max_y() && solid(p) {
            p = p.above();
        }
        return p;
    }
    pos
}

/// `moveTo(Path, speed)`.
pub fn move_to_path(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, path: Option<Path>, speed: f64) -> bool {
    let Some(path) = path else {
        m.nav.path = None;
        return false;
    };
    if !path.same_as(m.nav.path.as_ref()) {
        m.nav.path = Some(path);
    }
    if m.nav.is_done() {
        return false;
    }
    trim_path(e, m, level);
    if m.nav.path.as_ref().is_none_or(|p| p.nodes.is_empty()) {
        return false;
    }
    m.nav.speed_modifier = speed;
    let pos = temp_mob_pos(e, m, level);
    m.nav.last_stuck_check = m.nav.tick;
    m.nav.last_stuck_check_pos = pos;
    true
}

/// `moveTo(x, y, z, speed)`.
pub fn move_to(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, x: f64, y: f64, z: f64, speed: f64) -> bool {
    let p = create_path(e, m, level, BlockPos::containing(x, y, z), 1);
    move_to_path(e, m, level, p, speed)
}

/// `moveTo(Entity, speed)`.
pub fn move_to_entity(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, target: BlockPos, speed: f64) -> bool {
    let p = create_path_to_entity(e, m, level, target, 1);
    if m.nav.climber && p.is_none() {
        m.nav.path_to_position = Some(target);
        m.nav.speed_modifier = speed;
        return true;
    }
    p.is_some() && move_to_path(e, m, level, p, speed)
}

fn trim_path(_e: &Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let Some(path) = m.nav.path.as_mut() else { return };
    for i in 0..path.nodes.len() {
        let n = path.nodes[i].clone();
        let state = level.block(n.pos());
        if crate::blocks::has_tag(state, crate::blocks::Tag::Cauldrons) {
            let mut moved = n.clone();
            moved.y += 1;
            moved.hash = node_hash(moved.x, moved.y, moved.z);
            path.nodes[i] = moved;
            if i + 1 < path.nodes.len() && n.y >= path.nodes[i + 1].y {
                let next = &path.nodes[i + 1];
                let mut m2 = n.clone();
                m2.x = next.x;
                m2.y = n.y + 1;
                m2.z = next.z;
                m2.hash = node_hash(m2.x, m2.y, m2.z);
                path.nodes[i + 1] = m2;
            }
        }
    }
    if m.nav.avoid_sun {
        let e = _e;
        if level.can_see_sky(BlockPos::containing(e.x(), e.y() + 0.5, e.z())) {
            return;
        }
        let path = m.nav.path.as_mut().unwrap();
        for i in 0..path.nodes.len() {
            if level.can_see_sky(path.nodes[i].pos()) {
                path.nodes.truncate(i);
                return;
            }
        }
    }
}

/// `GroundPathNavigation.getTempMobPos`.
fn temp_mob_pos(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Vec3 {
    Vec3::new(e.x(), surface_y(e, m, level) as f64, e.z())
}

fn surface_y(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> i32 {
    if !(e.fluid.is_in_water() && m.nav.can_float) {
        return floor(e.y() + 0.5);
    }
    let mut y = e.block_position().y;
    let mut s = level.block(BlockPos::containing(e.x(), y as f64, e.z()));
    let mut n = 0;
    while crate::physics::fluid_state(s).kind.is_water() {
        y += 1;
        s = level.block(BlockPos::containing(e.x(), y as f64, e.z()));
        n += 1;
        if n > 16 {
            return e.block_position().y;
        }
    }
    y
}

/// `PathNavigation.tick` (and `WallClimberNavigation.tick`).
pub fn tick(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) {
    if m.nav.climber && m.nav.is_done() {
        if let Some(p) = m.nav.path_to_position {
            let close = |q: BlockPos| {
                let c = Vec3::new(q.x as f64 + 0.5, q.y as f64 + 0.5, q.z as f64 + 0.5);
                c.distance_to_sqr(e.position()) < (e.width as f64) * (e.width as f64)
            };
            if close(p) || (e.y() > p.y as f64 && close(BlockPos::containing(p.x as f64, e.y(), p.z as f64))) {
                m.nav.path_to_position = None;
            } else {
                let s = m.nav.speed_modifier;
                m.mov.set_wanted_position(p.x as f64, p.y as f64, p.z as f64, s);
            }
        }
        return;
    }
    m.nav.tick += 1;
    if m.nav.has_delayed_recomputation {
        recompute_path(e, m, level);
    }
    if m.nav.is_done() {
        return;
    }
    if can_update_path(e) {
        follow_the_path(e, m, level);
    } else if let Some(path) = &m.nav.path
        && !path.is_done()
    {
        let cur = temp_mob_pos(e, m, level);
        let next = path.next_entity_pos(e.width);
        if cur.y > next.y && !e.on_ground && floor(cur.x) == floor(next.x) && floor(cur.z) == floor(next.z) {
            m.nav.path.as_mut().unwrap().next += 1;
        }
    }
    if m.nav.is_done() {
        return;
    }
    let next = m.nav.path.as_ref().unwrap().next_entity_pos(e.width);
    let bp = BlockPos::containing(next.x, next.y, next.z);
    let y = if kiln_data::blocks_types::is_air(level.block(bp.below())) { next.y } else { floor_level(level, bp) };
    let s = m.nav.speed_modifier;
    m.mov.set_wanted_position(next.x, y, next.z, s);
}

fn recompute_path(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) {
    if level.game_time() - m.nav.time_last_recompute > 20 && can_update_path(e) {
        if let Some(t) = m.nav.target_pos {
            m.nav.path = None;
            let reach = m.nav.reach_range;
            m.nav.path = create_path_raw(e, m, level, t, 8, false, reach);
            m.nav.time_last_recompute = level.game_time();
            m.nav.has_delayed_recomputation = false;
        }
    } else {
        m.nav.has_delayed_recomputation = true;
    }
}

fn follow_the_path(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let cur = temp_mob_pos(e, m, level);
    m.nav.max_distance_to_waypoint = if e.width > 0.75 { e.width / 2.0 } else { 0.75 - e.width / 2.0 };
    let path = m.nav.path.as_ref().unwrap();
    let next = path.next_node_pos();
    let dx = (e.x() - (next.x as f64 + 0.5)).abs();
    let dy = (e.y() - next.y as f64).abs();
    let dz = (e.z() - (next.z as f64 + 0.5)).abs();
    let md = m.nav.max_distance_to_waypoint as f64;
    let close = dx < md && dz < md && dy < 1.0;
    let kind = path.nodes[path.next].kind;
    let cut = !matches!(kind, PathType::FireInNeighbor | PathType::DamagingInNeighbor | PathType::WalkableDoor);
    if close || (cut && should_target_next_node_in_direction(e, path, cur)) {
        m.nav.path.as_mut().unwrap().next += 1;
    }
    do_stuck_detection(e, m, level, cur);
}

fn should_target_next_node_in_direction(e: &Entity, path: &Path, cur: Vec3) -> bool {
    if path.next + 1 >= path.nodes.len() {
        return false;
    }
    let bottom = |p: BlockPos| Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
    let a = bottom(path.next_node_pos());
    if cur.distance_to_sqr(a) >= 4.0 {
        return false;
    }
    // `canMoveDirectly` is false for ground navigation.
    let b = bottom(path.nodes[path.next + 1].pos());
    let (va, vb) = (a - cur, b - cur);
    let (la, lb) = (va.length_sqr(), vb.length_sqr());
    let closer = lb < la;
    let near = la < 0.5;
    if closer || near {
        let (na, nb) = (va.normalize(), vb.normalize());
        return nb.x * na.x + nb.y * na.y + nb.z * na.z < 0.0;
    }
    let _ = e;
    false
}

fn do_stuck_detection(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, cur: Vec3) {
    let nav = &mut m.nav;
    if nav.tick - nav.last_stuck_check > 100 {
        let speed = if m.speed >= 1.0 { m.speed } else { m.speed * m.speed };
        let d = speed * 100.0 * 0.25;
        if cur.distance_to_sqr(nav.last_stuck_check_pos) < (d * d) as f64 {
            nav.is_stuck = true;
            nav.path = None;
        } else {
            nav.is_stuck = false;
        }
        nav.last_stuck_check = nav.tick;
        nav.last_stuck_check_pos = cur;
    }
    if let Some(path) = &nav.path
        && !path.is_done()
    {
        let next = path.next_node_pos();
        let now = level.game_time();
        if next == nav.timeout_cached_node {
            nav.timeout_timer += now - nav.last_timeout_check;
        } else {
            nav.timeout_cached_node = next;
            let d = cur.distance_to_sqr(Vec3::new(next.x as f64 + 0.5, next.y as f64, next.z as f64 + 0.5)).sqrt();
            nav.timeout_limit = if m.speed > 0.0 { d / m.speed as f64 * 20.0 } else { 0.0 };
        }
        if nav.timeout_limit > 0.0 && nav.timeout_timer as f64 > nav.timeout_limit * 3.0 {
            nav.reset_stuck_timeout();
            nav.path = None;
        }
        nav.last_timeout_check = now;
    }
    let _ = e;
}

/// `PathNavigation.isStableDestination`.
pub fn is_stable_destination(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    kiln_data::block_props::solid_render(level.block(pos.below()))
}

/// `shouldRecomputePath(pos)`.
pub fn should_recompute_path(e: &Entity, m: &MobData, pos: BlockPos) -> bool {
    if m.nav.has_delayed_recomputation {
        return false;
    }
    let Some(path) = &m.nav.path else { return false };
    if path.is_done() || path.nodes.is_empty() {
        return false;
    }
    let end = path.end_node().unwrap();
    let mid = Vec3::new((end.x as f64 + e.x()) / 2.0, (end.y as f64 + e.y()) / 2.0, (end.z as f64 + e.z()) / 2.0);
    let c = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    let r = (path.nodes.len() - path.next) as f64;
    c.distance_to_sqr(mid) < r * r
}

/// `recomputePath` from a block change nearby (`ServerLevel.sendBlockUpdated`).
pub fn on_block_changed(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, pos: BlockPos) {
    if should_recompute_path(e, m, pos) {
        recompute_path(e, m, level);
    }
}
