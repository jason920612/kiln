//! Per-block-state physics facts extracted from vanilla (`gen/physics.bin`, written by
//! `cargo xtask codegen` from tools/ExtractEntityPhysics.java).

use crate::shape::Shape;
use kiln_data::blocks_types::block_of;
use std::collections::HashMap;
use std::sync::OnceLock;

static RAW: &[u8] = include_bytes!("gen/physics.bin");

/// A fluid state's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FluidKind {
    Empty,
    FlowingWater,
    Water,
    FlowingLava,
    Lava,
}

impl FluidKind {
    pub fn is_water(self) -> bool {
        matches!(self, FluidKind::Water | FluidKind::FlowingWater)
    }

    pub fn is_lava(self) -> bool {
        matches!(self, FluidKind::Lava | FluidKind::FlowingLava)
    }

    /// `Fluid.isSame`: flowing and source forms of one fluid.
    pub fn is_same(self, other: FluidKind) -> bool {
        (self.is_water() && other.is_water()) || (self.is_lava() && other.is_lava()) || (self == other)
    }
}

/// A block state's fluid state (`BlockState.getFluidState()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidState {
    pub kind: FluidKind,
    /// 1-8 for flowing fluids, 8 for sources, 0 for empty.
    pub amount: u8,
    pub falling: bool,
    pub source: bool,
}

impl FluidState {
    pub const EMPTY: FluidState = FluidState { kind: FluidKind::Empty, amount: 0, falling: false, source: false };

    pub fn is_empty(&self) -> bool {
        self.kind == FluidKind::Empty
    }

    /// `FluidState.getOwnHeight`: `amount / 9f`.
    pub fn own_height(&self) -> f32 {
        self.amount as f32 / 9.0
    }
}

#[derive(Clone, Copy)]
struct StateEntry {
    collision: u16,
    inside: u16,
    outline: u16,
    fluid: u8,
    amount: u8,
    sturdy: u8,
    flags: u16,
}

const FALLING: u16 = 1 << 0;
const SOURCE: u16 = 1 << 1;
const AIR: u16 = 1 << 2;
const LIQUID: u16 = 1 << 3;
const SOLID: u16 = 1 << 4;
const REPLACEABLE: u16 = 1 << 5;
const OFFSET: u16 = 1 << 6;
const SUFFOCATING: u16 = 1 << 7;
const LARGE: u16 = 1 << 8;
const CUBE: u16 = 1 << 9;
/// The block uses the default `isSuffocating` predicate (`#causes_suffocation` with a full cube).
const SUFFOCATING_DEFAULT: u16 = 1 << 10;

/// Per-block movement factors (`BlockBehaviour.Properties`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockFactors {
    pub friction: f32,
    pub speed: f32,
    pub jump: f32,
    pub bounce: f32,
    pub fall_reduction: f32,
    pub explosion_resistance: f32,
}

struct Table {
    states: Vec<StateEntry>,
    shapes: Vec<Shape>,
    blocks: Vec<BlockFactors>,
    named: HashMap<String, u16>,
    /// Offset blocks: maximum horizontal offset, and whether the outline shape follows it.
    offsets: HashMap<u16, (f32, bool)>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut r = Reader { data: RAW, pos: 0 };
        assert_eq!(r.bytes(4), b"KEP3", "physics.bin: bad magic");
        let (n_states, n_shapes, n_blocks) = (r.u32() as usize, r.u32() as usize, r.u32() as usize);
        let mut shapes = Vec::with_capacity(n_shapes);
        for _ in 0..n_shapes {
            let counts = [r.u8() as usize, r.u8() as usize, r.u8() as usize];
            let coords = counts.map(|n| (0..n).map(|_| r.f64()).collect::<Vec<_>>());
            let cells = counts.iter().map(|n| n.saturating_sub(1)).product::<usize>();
            let bits = r.bytes(cells.div_ceil(8));
            let full = (0..cells).map(|i| bits[i / 8] & (1 << (i % 8)) != 0).collect::<Vec<_>>();
            shapes.push(Shape::new(coords, &full));
        }
        let states = (0..n_states)
            .map(|_| StateEntry {
                collision: r.u16(),
                inside: r.u16(),
                outline: r.u16(),
                fluid: r.u8(),
                amount: r.u8(),
                sturdy: r.u8(),
                flags: r.u16(),
            })
            .collect();
        let blocks = (0..n_blocks)
            .map(|_| BlockFactors { friction: r.f32(), speed: r.f32(), jump: r.f32(), bounce: r.f32(), fall_reduction: r.f32(), explosion_resistance: r.f32() })
            .collect();
        let mut named = HashMap::new();
        for _ in 0..r.u8() {
            let len = r.u8() as usize;
            let name = String::from_utf8(r.bytes(len).to_vec()).expect("physics.bin: shape name");
            named.insert(name, r.u16());
        }
        let mut offsets = HashMap::new();
        for _ in 0..r.u16() {
            let state = r.u16();
            let (h, outline) = (r.f32(), r.u8() != 0);
            offsets.insert(state, (h, outline));
        }
        assert_eq!(r.pos, RAW.len(), "physics.bin: trailing data");
        Table { states, shapes, blocks, named, offsets }
    })
}

struct Reader {
    data: &'static [u8],
    pos: usize,
}

impl Reader {
    fn bytes(&mut self, n: usize) -> &'static [u8] {
        let b = &self.data[self.pos..self.pos + n];
        self.pos += n;
        b
    }
    fn u8(&mut self) -> u8 {
        self.bytes(1)[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.bytes(2).try_into().unwrap())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes(4).try_into().unwrap())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.bytes(4).try_into().unwrap())
    }
    fn f64(&mut self) -> f64 {
        f64::from_le_bytes(self.bytes(8).try_into().unwrap())
    }
}

fn entry(state: u16) -> StateEntry {
    table().states[state as usize]
}

/// The state's collision shape with an empty collision context (`BlockState.getCollisionShape`).
pub fn collision_shape(state: u16) -> &'static Shape {
    &table().shapes[entry(state).collision as usize]
}

/// The state's outline shape (`BlockState.getShape`: what view rays stop at), unshifted for
/// offset blocks (see [`block_offset`]).
pub fn outline_shape(state: u16) -> &'static Shape {
    &table().shapes[entry(state).outline as usize]
}

/// `BlockState.getOffset(pos)` (horizontal) of a block whose outline shape follows it
/// (flowers, bamboo, dripstone).
pub fn outline_offset(state: u16, x: i32, z: i32) -> Option<(f64, f64)> {
    let (_, follows) = *table().offsets.get(&state)?;
    if !follows {
        return None;
    }
    collision_offset(state, x, z)
}

/// `Shapes.empty()`.
pub fn empty_shape() -> &'static Shape {
    &table().shapes[0]
}

/// `Shapes.block()`.
pub fn block_shape() -> &'static Shape {
    &table().shapes[1]
}

/// Whether the collision shape is vanilla's `Shapes.block()` singleton (collision fast path).
pub fn is_full_cube(state: u16) -> bool {
    entry(state).flags & CUBE != 0
}

/// `hasLargeCollisionShape`: the collision shape leaves the unit cube (fences, walls).
pub fn has_large_collision_shape(state: u16) -> bool {
    entry(state).flags & LARGE != 0
}

/// `getEntityInsideCollisionShape`; `None` for `Shapes.block()`.
pub fn inside_shape(state: u16) -> Option<&'static Shape> {
    match entry(state).inside {
        0xffff => None,
        i => Some(&table().shapes[i as usize]),
    }
}

/// A shape used by context-dependent blocks (scaffolding, powder snow).
pub fn named_shape(name: &str) -> &'static Shape {
    let t = table();
    &t.shapes[*t.named.get(name).unwrap_or_else(|| panic!("physics.bin: no shape {name}")) as usize]
}

pub fn fluid_state(state: u16) -> FluidState {
    let e = entry(state);
    let kind = match e.fluid {
        0 => FluidKind::Empty,
        1 => FluidKind::FlowingWater,
        2 => FluidKind::Water,
        3 => FluidKind::FlowingLava,
        _ => FluidKind::Lava,
    };
    FluidState { kind, amount: e.amount, falling: e.flags & FALLING != 0, source: e.flags & SOURCE != 0 }
}

/// `isFaceSturdy(level, pos, direction)` with `SupportType.FULL`.
pub fn is_face_sturdy(state: u16, dir: crate::math::Direction) -> bool {
    entry(state).sturdy & (1 << dir.index()) != 0
}

pub fn is_air(state: u16) -> bool {
    kiln_data::blocks_types::is_air(state)
}

/// The air flag of the generated tables (what `is_air` read before it compared state ids).
#[cfg(test)]
pub(crate) fn entry_is_air(state: u16) -> bool {
    entry(state).flags & AIR != 0
}

/// `BlockState.liquid()`: a liquid block (water or lava, not waterlogged blocks).
pub fn is_liquid(state: u16) -> bool {
    entry(state).flags & LIQUID != 0
}

/// Legacy solidity (`BlockState.isSolid()`).
pub fn is_solid(state: u16) -> bool {
    entry(state).flags & SOLID != 0
}

/// `BlockState.canBeReplaced()`.
pub fn can_be_replaced(state: u16) -> bool {
    entry(state).flags & REPLACEABLE != 0
}

/// Has a random position offset (bamboo, pointed dripstone, flowers).
pub fn has_offset(state: u16) -> bool {
    entry(state).flags & OFFSET != 0
}

/// `BlockState.isSuffocating`: the blocks that set their own predicate (`SUFFOCATING`, read from
/// vanilla), else the default one: `#minecraft:causes_suffocation` with a full-cube collision
/// shape. (The extraction runs before vanilla loads its tags: it only tells which blocks use the
/// default.)
pub fn is_suffocating(state: u16) -> bool {
    let flags = entry(state).flags;
    if flags & SUFFOCATING_DEFAULT != 0 {
        flags & CUBE != 0 && causes_suffocation(state)
    } else {
        flags & SUFFOCATING != 0
    }
}

/// Whether the block of `state` is in `#minecraft:causes_suffocation`.
fn causes_suffocation(state: u16) -> bool {
    static STATES: OnceLock<Vec<bool>> = OnceLock::new();
    let states = STATES.get_or_init(|| {
        let mut out = vec![false; table().states.len()];
        let tags = kiln_data::registries::TAGS.iter().find(|(r, _)| *r == "minecraft:block").map_or(&[][..], |(_, tags)| *tags);
        let ids = tags.iter().find(|(t, _)| *t == "minecraft:causes_suffocation").map_or(&[][..], |(_, ids)| *ids);
        let names = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        for &id in ids {
            if let Some(info) = names.get(id as usize).and_then(|n| kiln_data::blocks_types::block_by_name(n)) {
                for s in &mut out[info.first as usize..=info.last as usize] {
                    *s = true;
                }
            }
        }
        out
    });
    states.get(state as usize).copied().unwrap_or(false)
}

/// `BlockState.getOffset(pos)` for offset blocks with collision (bamboo, pointed dripstone;
/// all horizontal): their collision shape is stored unshifted.
pub fn collision_offset(state: u16, x: i32, z: i32) -> Option<(f64, f64)> {
    if entry(state).flags & OFFSET == 0 {
        return None;
    }
    let (max, _) = *table().offsets.get(&state)?;
    let seed = kiln_javamath::math::get_seed(x, 0, z);
    let clamp = |v: f64| {
        let lo = -max as f64;
        if v < lo { lo } else { crate::math::jmin(v, max as f64) }
    };
    let ox = clamp((((seed & 15) as f32 / 15.0) as f64 - 0.5) * 0.5);
    let oz = clamp((((seed >> 8 & 15) as f32 / 15.0) as f64 - 0.5) * 0.5);
    Some((ox, oz))
}

/// Friction, speed, jump and bounce factors of the state's block.
pub fn block_factors(state: u16) -> BlockFactors {
    static BLOCK_OF_STATE: OnceLock<Vec<u16>> = OnceLock::new();
    let index = BLOCK_OF_STATE.get_or_init(|| {
        let mut v = vec![0u16; kiln_data::blocks::STATE_COUNT as usize];
        for (i, b) in kiln_data::blocks::BLOCKS.iter().enumerate() {
            v[b.first as usize..=b.last as usize].fill(i as u16);
        }
        v
    })[state as usize];
    debug_assert_eq!(kiln_data::blocks::BLOCKS[index as usize].name, block_of(state).name);
    table().blocks[index as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Direction;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn spot_checks() {
        assert!(is_full_cube(d::STONE) && !is_full_cube(d::OAK_SLAB));
        assert!(collision_shape(d::AIR).is_empty());
        assert_eq!(block_factors(d::ICE).friction, 0.98);
        assert_eq!(block_factors(d::BLUE_ICE).friction, 0.989);
        assert_eq!(block_factors(d::SOUL_SAND).speed, 0.4);
        assert_eq!(block_factors(d::HONEY_BLOCK).jump, 0.5);
        assert_eq!(block_factors(d::SLIME_BLOCK).bounce, 1.0);
        assert_eq!(block_factors(d::STONE).friction, 0.6);
        let w = fluid_state(d::WATER);
        assert_eq!((w.kind, w.amount, w.source), (FluidKind::Water, 8, true));
        assert!(fluid_state(d::STONE).is_empty());
        assert!(is_face_sturdy(d::STONE, Direction::Up) && !is_face_sturdy(d::OAK_SLAB, Direction::Up));
        assert!(has_large_collision_shape(d::OAK_FENCE) && !has_large_collision_shape(d::STONE));
        assert!(is_air(d::CAVE_AIR) && is_liquid(d::LAVA) && !is_liquid(d::STONE));
        assert!(!named_shape("scaffolding_stable").is_empty());
    }
}
