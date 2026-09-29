//! Pathfinding: `WalkNodeEvaluator` (and `AmphibiousNodeEvaluator`, `SwimNodeEvaluator`,
//! `FlyNodeEvaluator`), the A* `PathFinder` with vanilla's `BinaryHeap`, `Path`, and
//! `GroundPathNavigation` / `WallClimberNavigation` (spiders) / `AmphibiousPathNavigation`
//! (drowned) / `WaterBoundPathNavigation` (guardians) / `FlyingPathNavigation` (the wither).

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
    PathType::ALL[corrected().get(state as usize).copied().unwrap_or(0) as usize]
}

/// The extracted path types with vanilla's tag checks put back: the extraction ran without
/// tags bound, so trapdoors, speleothems, fences and walls, fire and lit campfires fell through,
/// lava came out as fire and water as open. `getPathTypeFromState` checks in this order.
fn corrected() -> &'static [u8] {
    static T: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let raw = table();
        let n = raw.len() / 2;
        let mut out: Vec<u8> = (0..n).map(|s| raw[s * 2]).collect();
        let tagged = |tag: &str| -> Vec<u16> {
            let Some(ids) = kiln_data::registries::TAGS
                .iter()
                .find(|(r, _)| *r == "minecraft:block")
                .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
                .map(|(_, ids)| *ids)
            else {
                return Vec::new();
            };
            (0..n as u16).filter(|&s| kiln_data::builtin_id("minecraft:block", crate::blocks::block_name(s)).is_some_and(|id| ids.contains(&id))).collect()
        };
        let set = |out: &mut Vec<u8>, states: &[u16], t: PathType, only_if: &dyn Fn(PathType) -> bool| {
            for &s in states {
                let cur = PathType::ALL[out[s as usize] as usize];
                if only_if(cur) {
                    out[s as usize] = t as u8;
                }
            }
        };
        // Later checks first, so earlier ones win where both apply.
        let fluid = |s: u16| crate::physics::fluid_state(s).kind;
        for s in 0..n as u16 {
            let cur = PathType::ALL[out[s as usize] as usize];
            if cur == PathType::Open && fluid(s).is_water() {
                out[s as usize] = PathType::Water as u8;
            }
        }
        let fences: Vec<u16> = tagged("minecraft:fences").into_iter().chain(tagged("minecraft:walls")).collect();
        set(&mut out, &fences, PathType::Fence, &|c| !matches!(c, PathType::DoorOpen | PathType::DoorWoodClosed | PathType::DoorIronClosed | PathType::Rail | PathType::Leaves));
        let mut fire = tagged("minecraft:fire");
        fire.extend(tagged("minecraft:campfires").into_iter().filter(|&s| kiln_data::blocks_types::block_of(s).property(s, "lit") == Some("true")));
        set(&mut out, &fire, PathType::Fire, &|_| true);
        for s in 0..n as u16 {
            if fluid(s).is_lava() {
                out[s as usize] = PathType::Lava as u8;
            }
        }
        set(&mut out, &tagged("minecraft:speleothems"), PathType::DamageCautious, &|_| true);
        set(&mut out, &tagged("minecraft:trapdoors"), PathType::Trapdoor, &|_| true);
        out
    })
}

/// `isPathfindable(LAND)` (lava is not: `LiquidBlock` checks the lava tag).
pub fn pathfindable_land(state: u16) -> bool {
    flags(state) & 1 != 0 && crate::blocks::block_name(state) != "minecraft:lava"
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
    /// `AmphibiousNodeEvaluator`: water is walkable, swimming up and down, land costs more.
    amphibious: bool,
    /// `SwimNodeEvaluator`: water only, in all 6 directions and the level diagonals.
    swim: bool,
    /// `allowBreaching` (dolphins).
    breaching: bool,
    /// `FlyNodeEvaluator`: open air in 26 directions.
    fly: bool,
    /// `PathfindingContext.mobPosition`.
    mob_pos: BlockPos,
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
        // `AmphibiousNodeEvaluator.prepare`: walkable 6 and water border 4 while searching.
        if self.amphibious {
            match t {
                PathType::Walkable => return 6.0,
                PathType::WaterBorder => return 4.0,
                _ => {}
            }
        }
        malus(self.m, t)
    }

    /// `getPathType(context, x, y, z)`: the static type, or the amphibious evaluator's (water
    /// next to a blocked block is a water border).
    fn block_type(&self, x: i32, y: i32, z: i32) -> PathType {
        if self.fly {
            return fly_type(self.level, x, y, z, self.mob_pos);
        }
        if !self.amphibious {
            return path_type_static(self.level, x, y, z);
        }
        amphibious_type(self.level, x, y, z)
    }

    fn max_up_step(&self) -> f32 {
        self.e.max_up_step
    }

    fn cached_type(&mut self, x: i32, y: i32, z: i32) -> PathType {
        let key = BlockPos::new(x, y, z).as_long();
        if let Some(&t) = self.types.get(&key) {
            return t;
        }
        let t = if self.swim { swim_type_of_mob(self.level, x, y, z, self.width, self.height, self.depth) } else { self.type_of_mob(x, y, z) };
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
        let here = self.block_type(x, y, z);
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
        let mob = self.mob_pos;
        for i in 0..self.width {
            for j in 0..self.height {
                for k in 0..self.depth {
                    let (px, py, pz) = (x + i, y + j, z + k);
                    let mut t = self.block_type(px, py, pz);
                    if t == PathType::DoorWoodClosed && self.can_open_doors && self.can_pass_doors {
                        t = PathType::WalkableDoor;
                    }
                    if t == PathType::DoorOpen && !self.can_pass_doors {
                        t = PathType::Blocked;
                    }
                    if t == PathType::Rail
                        && self.block_type(mob.x, mob.y, mob.z) != PathType::Rail
                        && self.block_type(mob.x, mob.y - 1, mob.z) != PathType::Rail
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
        if (self.can_float || self.amphibious) && crate::fluid::fluid_at(self.level, pos).kind.is_water() {
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
        if self.swim {
            // `SwimNodeEvaluator.getStart`: the box's low corner, half up (no type yet).
            let bb = e.bounding_box();
            return Some(self.node(floor(bb.min_x), floor(bb.min_y + 0.5), floor(bb.min_z)));
        }
        if self.fly {
            return Some(self.fly_start());
        }
        let mut y = e.block_position().y;
        let at = |x: f64, y: i32, z: f64| BlockPos::containing(x, y as f64, z);
        if self.amphibious && e.is_in_water() {
            // `AmphibiousNodeEvaluator.getStart`: the block at the box's low corner, half up.
            let bb = e.bounding_box();
            let (x, y, z) = (floor(bb.min_x), floor(bb.min_y + 0.5), floor(bb.min_z));
            return Some(self.start_node(x, y, z));
        }
        let state = self.level.block(at(e.x(), y, e.z()));
        let lava = |s: u16| crate::physics::fluid_state(s).kind.is_lava();
        if e.stands_on_lava && lava(state) {
            // `canStandOnFluid`: the start is the top of the fluid.
            let mut s = state;
            while lava(s) {
                y += 1;
                s = self.level.block(at(e.x(), y, e.z()));
            }
            y -= 1;
        } else if self.can_float && e.fluid.is_in_water() {
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
        if self.swim {
            return self.swim_neighbors(out, cur);
        }
        if self.fly {
            return self.fly_neighbors(out, cur);
        }
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
        if self.amphibious {
            // `AmphibiousNodeEvaluator.getNeighbors`: swimming straight up and down.
            let up = self.accepted_node(x, y + 1, z, (jump - 1).max(0), floor_y, (0, 0), here);
            let down = self.accepted_node(x, y - 1, z, jump, floor_y, (0, 0), here);
            if self.neighbor_valid(up, cur) && self.n(up.unwrap()).kind == PathType::Water {
                out.push(up.unwrap());
            }
            if self.neighbor_valid(down, cur) && self.n(down.unwrap()).kind == PathType::Water && here != PathType::Trapdoor {
                out.push(down.unwrap());
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
        if t == PathType::Walkable || (self.amphibious && t == PathType::Water) {
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
        if t == PathType::Water && !self.can_float && !self.amphibious {
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

    // ------------------------------------------------------------------ SwimNodeEvaluator

    /// `SwimNodeEvaluator.findAcceptedNode` (no breaching): water nodes; a node without fluid
    /// costs 8 more.
    fn swim_accepted(&mut self, x: i32, y: i32, z: i32) -> Option<u32> {
        let t = self.cached_type(x, y, z);
        if !(t == PathType::Water || (self.breaching && t == PathType::Breach)) {
            return None;
        }
        let malus = self.malus(t);
        if malus < 0.0 {
            return None;
        }
        let i = self.node(x, y, z);
        let dry = crate::physics::fluid_state(self.level.block(BlockPos::new(x, y, z))).is_empty();
        let n = self.nm(i);
        n.kind = t;
        n.cost_malus = n.cost_malus.max(malus);
        if dry {
            n.cost_malus += 8.0;
        }
        Some(i)
    }

    /// `SwimNodeEvaluator.getNeighbors`: `Direction.values()` (down, up, north, south, west,
    /// east), then each horizontal direction with its clockwise neighbour.
    fn swim_neighbors(&mut self, out: &mut Vec<u32>, cur: u32) {
        let (x, y, z) = {
            let n = self.n(cur);
            (n.x, n.y, n.z)
        };
        const DIRS: [(i32, i32, i32); 6] = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];
        let mut by_dir: [Option<u32>; 6] = [None; 6];
        for (k, &(dx, dy, dz)) in DIRS.iter().enumerate() {
            let n = self.swim_accepted(x + dx, y + dy, z + dz);
            by_dir[k] = n;
            if let Some(n) = n.filter(|&n| !self.n(n).closed) {
                out.push(n);
            }
        }
        let has_malus = |s: &Self, n: Option<u32>| n.is_some_and(|n| s.n(n).cost_malus >= 0.0);
        // `Plane.HORIZONTAL` with `getClockWise`: north -> east, east -> south, south -> west,
        // west -> north.
        for (d, cw) in [(2usize, 5usize), (5, 3), (3, 4), (4, 2)] {
            if has_malus(self, by_dir[d]) && has_malus(self, by_dir[cw]) {
                let (a, b) = (DIRS[d], DIRS[cw]);
                let n = self.swim_accepted(x + a.0 + b.0, y, z + a.2 + b.2);
                if let Some(n) = n.filter(|&n| !self.n(n).closed) {
                    out.push(n);
                }
            }
        }
    }

    // ------------------------------------------------------------------ FlyNodeEvaluator

    /// `FlyNodeEvaluator.getStart`. Approximation: a mob smaller than a block that cannot start
    /// where it is keeps its own block (vanilla tries 10 random blocks around it).
    fn fly_start(&mut self) -> u32 {
        let e = self.e;
        let y = if self.can_float && e.is_in_water() {
            let mut y = e.block_position().y;
            while crate::blocks::block_name(self.level.block(BlockPos::containing(e.x(), y as f64, e.z()))) == "minecraft:water" {
                y += 1;
            }
            y
        } else {
            floor(e.y() + 0.5)
        };
        let start = BlockPos::containing(e.x(), y as f64, e.z());
        if !self.fly_can_start_at(start) {
            let bb = e.bounding_box();
            if bb.size() >= 1.0 {
                let by = e.block_position().y as f64;
                for (x, z) in [(bb.min_x, bb.min_z), (bb.min_x, bb.max_z), (bb.max_x, bb.min_z), (bb.max_x, bb.max_z)] {
                    let p = BlockPos::containing(x, by, z);
                    if self.fly_can_start_at(p) {
                        return self.start_node(p.x, p.y, p.z);
                    }
                }
            }
        }
        self.start_node(start.x, start.y, start.z)
    }

    fn fly_can_start_at(&mut self, p: BlockPos) -> bool {
        let t = self.cached_type(p.x, p.y, p.z);
        self.malus(t) >= 0.0
    }

    /// `FlyNodeEvaluator.findAcceptedNode`: walkable nodes cost one more.
    fn fly_accepted(&mut self, x: i32, y: i32, z: i32) -> Option<u32> {
        let t = self.cached_type(x, y, z);
        let malus = self.malus(t);
        if malus < 0.0 {
            return None;
        }
        let i = self.node(x, y, z);
        let n = self.nm(i);
        n.kind = t;
        n.cost_malus = n.cost_malus.max(malus);
        if t == PathType::Walkable {
            n.cost_malus += 1.0;
        }
        Some(i)
    }

    fn fly_open(&self, n: Option<u32>) -> bool {
        n.is_some_and(|n| !self.n(n).closed)
    }

    fn fly_has_malus(&self, n: Option<u32>) -> bool {
        n.is_some_and(|n| self.n(n).cost_malus >= 0.0)
    }

    /// A diagonal neighbour of `FlyNodeEvaluator.getNeighbors`: taken when open and every
    /// node it passes (`sides`) can be entered. Returns the node either way.
    fn fly_diagonal(&mut self, out: &mut Vec<u32>, x: i32, y: i32, z: i32, sides: &[Option<u32>]) -> Option<u32> {
        let n = self.fly_accepted(x, y, z);
        if self.fly_open(n) && sides.iter().all(|&m| self.fly_has_malus(m)) {
            out.push(n.unwrap());
        }
        n
    }

    /// `FlyNodeEvaluator.getNeighbors`: the 26 neighbours in vanilla's order.
    fn fly_neighbors(&mut self, out: &mut Vec<u32>, cur: u32) {
        let (x, y, z) = {
            let n = self.n(cur);
            (n.x, n.y, n.z)
        };
        let south = self.fly_accepted(x, y, z + 1);
        let west = self.fly_accepted(x - 1, y, z);
        let east = self.fly_accepted(x + 1, y, z);
        let north = self.fly_accepted(x, y, z - 1);
        let up = self.fly_accepted(x, y + 1, z);
        let down = self.fly_accepted(x, y - 1, z);
        for n in [south, west, east, north, up, down] {
            if self.fly_open(n) {
                out.push(n.unwrap());
            }
        }
        let south_up = self.fly_diagonal(out, x, y + 1, z + 1, &[south, up]);
        let west_up = self.fly_diagonal(out, x - 1, y + 1, z, &[west, up]);
        let east_up = self.fly_diagonal(out, x + 1, y + 1, z, &[east, up]);
        let north_up = self.fly_diagonal(out, x, y + 1, z - 1, &[north, up]);
        let south_down = self.fly_diagonal(out, x, y - 1, z + 1, &[south, down]);
        let west_down = self.fly_diagonal(out, x - 1, y - 1, z, &[west, down]);
        let east_down = self.fly_diagonal(out, x + 1, y - 1, z, &[east, down]);
        let north_down = self.fly_diagonal(out, x, y - 1, z - 1, &[north, down]);
        let north_east = self.fly_diagonal(out, x + 1, y, z - 1, &[north, east]);
        let south_east = self.fly_diagonal(out, x + 1, y, z + 1, &[south, east]);
        let north_west = self.fly_diagonal(out, x - 1, y, z - 1, &[north, west]);
        let south_west = self.fly_diagonal(out, x - 1, y, z + 1, &[south, west]);
        self.fly_diagonal(out, x + 1, y + 1, z - 1, &[north_east, north, east, up, north_up, east_up]);
        self.fly_diagonal(out, x + 1, y + 1, z + 1, &[south_east, south, east, up, south_up, east_up]);
        self.fly_diagonal(out, x - 1, y + 1, z - 1, &[north_west, north, west, up, north_up, west_up]);
        self.fly_diagonal(out, x - 1, y + 1, z + 1, &[south_west, south, west, up, south_up, west_up]);
        self.fly_diagonal(out, x + 1, y - 1, z - 1, &[north_east, north, east, down, north_down, east_down]);
        self.fly_diagonal(out, x + 1, y - 1, z + 1, &[south_east, south, east, down, south_down, east_down]);
        self.fly_diagonal(out, x - 1, y - 1, z - 1, &[north_west, north, west, down, north_down, west_down]);
        self.fly_diagonal(out, x - 1, y - 1, z + 1, &[south_west, south, west, down, south_down, west_down]);
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

    /// `PathFinder.findPath` for several target blocks (the villagers' points of interest): every
    /// target keeps its own best node; the path goes to the reached target with the fewest nodes,
    /// else to the one that came closest (ties: first in the given order).
    fn find_multi(&mut self, targets: &[BlockPos], max_dist: f32, reach: i32, max_visited: i32) -> Option<Path> {
        let start = self.start()?;
        let tnodes: Vec<u32> = targets.iter().map(|t| self.node(floor(t.x as f64), floor(t.y as f64), floor(t.z as f64))).collect();
        let coords: Vec<(i32, i32, i32)> = tnodes.iter().map(|&t| (self.n(t).x, self.n(t).y, self.n(t).z)).collect();
        let mut best_h = vec![f32::MAX; targets.len()];
        let mut best_node: Vec<Option<u32>> = vec![None; targets.len()];
        let mut reached_t = vec![false; targets.len()];
        // `getBestH(node, targets)`: updates each target's best, answers the smallest distance.
        fn best_of(s: &Search, n: u32, tnodes: &[u32], best_h: &mut [f32], best_node: &mut [Option<u32>]) -> f32 {
            let mut m = f32::MAX;
            for (i, &t) in tnodes.iter().enumerate() {
                let d = s.n(n).distance_to(s.n(t));
                if d < best_h[i] {
                    best_h[i] = d;
                    best_node[i] = Some(n);
                }
                m = m.min(d);
            }
            m
        }
        {
            let h = best_of(self, start, &tnodes, &mut best_h, &mut best_node);
            let n = self.nm(start);
            n.g = 0.0;
            n.h = h;
            n.f = h;
        }
        self.heap.clear();
        self.heap_insert(start);
        let mut any_reached = false;
        let mut visited = 0;
        let mut neigh = Vec::with_capacity(8);
        while !self.heap.is_empty() {
            visited += 1;
            if visited >= max_visited {
                break;
            }
            let cur = self.heap_pop();
            self.nm(cur).closed = true;
            for (i, &(tx, ty, tz)) in coords.iter().enumerate() {
                if self.n(cur).distance_manhattan(tx, ty, tz) <= reach as f32 {
                    reached_t[i] = true;
                    any_reached = true;
                }
            }
            if any_reached {
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
                    let h = best_of(self, nb, &tnodes, &mut best_h, &mut best_node) * 1.5;
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
        let build = |s: &Search, i: usize, reached: bool| -> Option<Path> {
            let mut chain = vec![best_node[i]?];
            let mut c = chain[0];
            while let Some(p) = s.n(c).came_from {
                chain.push(p);
                c = p;
            }
            chain.reverse();
            Some(Path::new(chain.iter().map(|&k| s.n(k).clone()).collect(), targets[i], reached))
        };
        let mut best: Option<Path> = None;
        if any_reached {
            for i in (0..targets.len()).filter(|&i| reached_t[i]) {
                let Some(p) = build(self, i, true) else { continue };
                if best.as_ref().is_none_or(|b| p.nodes.len() < b.nodes.len()) {
                    best = Some(p);
                }
            }
        } else {
            for i in 0..targets.len() {
                let Some(p) = build(self, i, false) else { continue };
                let better = match &best {
                    None => true,
                    Some(b) => p.dist_to_target < b.dist_to_target || (p.dist_to_target == b.dist_to_target && p.nodes.len() < b.nodes.len()),
                };
                if better {
                    best = Some(p);
                }
            }
        }
        best
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
    /// `AmphibiousPathNavigation` (drowned).
    pub amphibious: bool,
    /// `WaterBoundPathNavigation` (fish, squids, guardians, dolphins, tadpoles).
    pub water_bound: bool,
    /// `WaterBoundPathNavigation.allowBreaching` (dolphins: paths may leave the water).
    pub allow_breaching: bool,
    /// `FlyingPathNavigation` (the wither).
    pub fly: bool,
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

/// `GroundPathNavigation.canUpdatePath` (always for amphibious navigation; in a liquid for
/// water-bound navigation; unless riding for flying navigation).
fn can_update_path(e: &Entity, m: &MobData) -> bool {
    if m.nav.water_bound {
        // `WaterBoundPathNavigation.canUpdatePath`: in a liquid, unless it may breach.
        return m.nav.allow_breaching || e.is_in_water() || e.is_in_lava();
    }
    if m.nav.fly {
        return (m.nav.can_float && (e.is_in_water() || e.is_in_lava())) || e.vehicle.is_none();
    }
    m.nav.amphibious || e.on_ground || e.is_in_water() || e.is_in_lava()
}

/// Water-bound, flying and amphibious navigation go to the block itself, ground navigation to
/// the surface there.
fn keeps_target_block(m: &MobData) -> bool {
    m.nav.amphibious || m.nav.water_bound || m.nav.fly
}

/// `AmphibiousNodeEvaluator.getPathType`.
pub fn amphibious_type(level: &dyn EntityLevel, x: i32, y: i32, z: i32) -> PathType {
    if type_at(level, x, y, z) != PathType::Water {
        return path_type_static(level, x, y, z);
    }
    // `Direction.values()`: down, up, north, south, west, east.
    for (dx, dy, dz) in [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)] {
        if type_at(level, x + dx, y + dy, z + dz) == PathType::Blocked {
            return PathType::WaterBorder;
        }
    }
    PathType::Water
}

/// `FlyNodeEvaluator.getPathType`: open air over the ground is walkable, over fire or damaging
/// blocks it is fire or damaging; a fence below counts unless the mob stands on it.
pub fn fly_type(level: &dyn EntityLevel, x: i32, y: i32, z: i32, mob_pos: BlockPos) -> PathType {
    use PathType::*;
    let mut t = type_at(level, x, y, z);
    if t == Open && y >= level.min_y() + 1 {
        let below = BlockPos::new(x, y - 1, z);
        t = match type_at(level, x, y - 1, z) {
            Fire | Lava => Fire,
            Damaging => Damaging,
            Cocoa => Cocoa,
            Fence if below != mob_pos => Fence,
            Fence => Open,
            Walkable | Open | Water => Open,
            _ => Walkable,
        };
    }
    if t == Walkable || t == Open {
        t = check_neighbour_blocks(level, x, y, z, t);
    }
    t
}

/// `BlockBehaviour.isPathfindable(WATER)`: water in the block.
fn pathfindable_water(state: u16) -> bool {
    crate::physics::fluid_state(state).kind.is_water()
}

/// `SwimNodeEvaluator.getPathTypeOfMob` (no breaching): water filling the mob's box, air over
/// water a breach, anything else blocked.
pub fn swim_type_of_mob(level: &dyn EntityLevel, x: i32, y: i32, z: i32, w: i32, h: i32, d: i32) -> PathType {
    let mut last = BlockPos::new(x, y, z);
    for xx in x..x + w {
        for yy in y..y + h {
            for zz in z..z + d {
                let p = BlockPos::new(xx, yy, zz);
                last = p;
                let s = level.block(p);
                let f = crate::physics::fluid_state(s);
                if f.is_empty() && pathfindable_water(level.block(p.below())) && kiln_data::blocks_types::is_air(s) {
                    return PathType::Breach;
                }
                if !f.kind.is_water() {
                    return PathType::Blocked;
                }
            }
        }
    }
    if pathfindable_water(level.block(last)) { PathType::Water } else { PathType::Blocked }
}

/// `Block.isFaceFull(getCollisionShape(level, pos), UP)` (`entityCanStandOn`, the empty
/// collision context).
fn top_face_full(level: &dyn EntityLevel, pos: BlockPos) -> bool {
    let (shape, _) = collision::collision_shape(level.block(pos), pos, &CollisionContext::EMPTY);
    if shape.is_empty() {
        return false;
    }
    // The shape's cells at its top edge cover the whole face.
    let xs = shape.coords(Axis::X);
    let ys = shape.coords(Axis::Y);
    let zs = shape.coords(Axis::Z);
    if ys.last().copied() != Some(1.0) || xs.first().copied() != Some(0.0) || xs.last().copied() != Some(1.0) || zs.first().copied() != Some(0.0) || zs.last().copied() != Some(1.0) {
        return false;
    }
    let top = shape.size(Axis::Y) as i32 - 1;
    (0..shape.size(Axis::X) as i32).all(|i| (0..shape.size(Axis::Z) as i32).all(|k| shape.is_full_wide(i, top, k)))
}

/// `BehaviorUtils.getRandomSwimmablePos`: a `DefaultRandomPos` in the water (up to ten more
/// tries while the spot is not swimmable).
pub fn random_swimmable_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, h: i32, v: i32) -> Option<Vec3> {
    let mut p = super::random_pos::default_pos(e, m, level, h, v);
    let mut count = 0;
    while let Some(q) = p {
        let swimmable = crate::physics::fluid_state(level.block(BlockPos::containing(q.x, q.y, q.z))).kind.is_water();
        if swimmable || count >= 10 {
            break;
        }
        count += 1;
        p = super::random_pos::default_pos(e, m, level, h, v);
    }
    p
}

/// `isStableDestination` of the mob's navigation.
pub fn stable_destination(m: &MobData, level: &dyn EntityLevel, pos: BlockPos) -> bool {
    if let Some(stable) = m.kind.ext().and_then(|k| k.stable_destination_for(m, level, pos)) {
        return stable;
    }
    if m.nav.water_bound {
        return !kiln_data::block_props::solid_render(level.block(pos));
    }
    if m.nav.fly {
        return top_face_full(level, pos);
    }
    if m.nav.amphibious {
        return !kiln_data::blocks_types::is_air(level.block(pos.below()));
    }
    is_stable_destination(level, pos)
}

/// `PathNavigation.createPath(Set<BlockPos>, regionOffset, offsetUpward, reach, maxPathLength)`.
fn create_path_raw(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, target: BlockPos, region: i32, up: bool, reach: i32) -> Option<Path> {
    let max_len = max_path_length(m);
    if e.y() < level.min_y() as f64 || !can_update_path(e, m) {
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
        amphibious: m.nav.amphibious,
        swim: m.nav.water_bound,
        breaching: m.nav.allow_breaching,
        fly: m.nav.fly,
        mob_pos: e.block_position(),
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

/// `PathNavigation.createPath(Set<BlockPos>, reach)` (region 8, no upward offset): a path to the
/// best of several blocks; `None` without targets.
pub fn create_path_multi(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, targets: &[BlockPos], reach: i32) -> Option<Path> {
    if targets.is_empty() {
        return None;
    }
    let max_len = max_path_length(m);
    if e.y() < level.min_y() as f64 || !can_update_path(e, m) {
        return None;
    }
    if let Some(p) = &m.nav.path
        && !p.is_done()
        && m.nav.target_pos.is_some_and(|t| targets.contains(&t))
    {
        return Some(p.clone());
    }
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
        amphibious: m.nav.amphibious,
        swim: m.nav.water_bound,
        breaching: m.nav.allow_breaching,
        fly: m.nav.fly,
        mob_pos: e.block_position(),
        heap: Vec::with_capacity(64),
    };
    let path = s.find_multi(targets, max_len, reach, max_visited);
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
    let pos = if keeps_target_block(m) { pos } else { find_surface(level, pos) };
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
    let pos = if keeps_target_block(m) { target } else { find_surface(level, target) };
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
    if m.nav.fly {
        return e.position();
    }
    if m.nav.amphibious || m.nav.water_bound {
        // `AmphibiousPathNavigation.getTempMobPos`: half way up the box.
        return Vec3::new(e.x(), e.y() + e.height as f64 * 0.5, e.z());
    }
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
    if can_update_path(e, m) {
        follow_the_path(e, m, level);
    } else if let Some(path) = &m.nav.path
        && !path.is_done()
    {
        let cur = temp_mob_pos(e, m, level);
        let next = path.next_entity_pos(e.width);
        let advance = if m.nav.fly {
            // `FlyingPathNavigation.tick`: in the next node's block.
            let bp = e.block_position();
            bp.x == floor(next.x) && bp.y == floor(next.y) && bp.z == floor(next.z)
        } else {
            cur.y > next.y && !e.on_ground && floor(cur.x) == floor(next.x) && floor(cur.z) == floor(next.z)
        };
        if advance {
            m.nav.path.as_mut().unwrap().next += 1;
        }
    }
    if m.nav.is_done() {
        return;
    }
    let next = m.nav.path.as_ref().unwrap().next_entity_pos(e.width);
    let bp = BlockPos::containing(next.x, next.y, next.z);
    // `getGroundY`: the node's own height for water-bound and flying navigation.
    let y = if keeps_target_block(m) || kiln_data::blocks_types::is_air(level.block(bp.below())) { next.y } else { floor_level(level, bp) };
    let s = m.nav.speed_modifier;
    m.mov.set_wanted_position(next.x, y, next.z, s);
}

fn recompute_path(e: &Entity, m: &mut MobData, level: &dyn EntityLevel) {
    if level.game_time() - m.nav.time_last_recompute > 20 && can_update_path(e, m) {
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
    // `getMaxVerticalDistanceToWaypoint`: 0.5 for water-bound navigation.
    let close = dx < md && dz < md && dy < if m.nav.water_bound { 0.5 } else { 1.0 };
    let kind = path.nodes[path.next].kind;
    let cut = !matches!(kind, PathType::FireInNeighbor | PathType::DamagingInNeighbor | PathType::WalkableDoor);
    if close || (cut && should_target_next_node_in_direction(e, m, level, path, cur)) {
        m.nav.path.as_mut().unwrap().next += 1;
    }
    do_stuck_detection(e, m, level, cur);
}

fn should_target_next_node_in_direction(e: &Entity, m: &MobData, level: &dyn EntityLevel, path: &Path, cur: Vec3) -> bool {
    if path.next + 1 >= path.nodes.len() {
        return false;
    }
    let bottom = |p: BlockPos| Vec3::new(p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
    let a = bottom(path.next_node_pos());
    if cur.distance_to_sqr(a) >= 4.0 {
        return false;
    }
    // `canMoveDirectly`: false for ground navigation; amphibious navigation in a liquid goes
    // straight when nothing is in the way (`isClearForMovementBetween`).
    if (m.nav.amphibious && (e.is_in_water() || e.is_in_lava())) || m.nav.water_bound || m.nav.fly {
        let to = path.next_entity_pos(e.width);
        let to = Vec3::new(to.x, to.y + e.height as f64 * 0.5, to.z);
        // Flying navigation is blocked by fluids too (`isClearForMovementBetween(.., true)`).
        let blocked = if m.nav.fly { clip_blocks_and_fluids(level, cur, to) } else { super::clip_blocks(level, cur, to) };
        if !blocked {
            return true;
        }
    }
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

/// `Level.clip` with `COLLIDER` shapes and `Fluid.ANY` hits something.
fn clip_blocks_and_fluids(level: &dyn EntityLevel, from: Vec3, to: Vec3) -> bool {
    crate::clip::traverse_blocks(from, to, |p| {
        let s = level.block(p);
        let (shape, _) = collision::collision_shape(s, p, &CollisionContext::EMPTY);
        if crate::clip::shape_clips(&shape, from, to, p) {
            return Some(());
        }
        let f = crate::physics::fluid_state(s);
        if f.is_empty() {
            return None;
        }
        let h = crate::fluid::height(level, p, &f) as f64;
        let fluid = crate::shape::Shape::from_box(&Aabb::new(0.0, 0.0, 0.0, 1.0, h, 1.0))?;
        crate::clip::shape_clips(&fluid, from, to, p).then_some(())
    })
    .is_some()
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
