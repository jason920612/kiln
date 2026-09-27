//! Per-state block facts world generation needs beyond `kiln_data::block_props`: legacy
//! solidity, sturdy faces per support type, fluid states and block classes, extracted from the
//! vanilla game by `tools/ExtractWorldgenBlocks.java` into `gen/block_facts.bin`.

use std::sync::OnceLock;

static RAW: &[u8] = include_bytes!("gen/block_facts.bin");

/// `SupportType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Full = 0,
    Center = 1,
    Rigid = 2,
}

/// `Direction` in 3D data value order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

impl Dir {
    pub const ALL: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];
    /// `Direction.Plane.HORIZONTAL` order.
    pub const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

    pub fn offset(self) -> (i32, i32, i32) {
        match self {
            Dir::Down => (0, -1, 0),
            Dir::Up => (0, 1, 0),
            Dir::North => (0, 0, -1),
            Dir::South => (0, 0, 1),
            Dir::West => (-1, 0, 0),
            Dir::East => (1, 0, 0),
        }
    }

    pub fn opposite(self) -> Dir {
        Dir::ALL[self as usize ^ 1]
    }

    /// `Direction.from3DDataValue`.
    pub fn from_index(i: usize) -> Dir {
        Dir::ALL[i % 6]
    }

    /// `Direction.from2DDataValue` (south, west, north, east).
    pub fn from_2d(i: i32) -> Dir {
        [Dir::South, Dir::West, Dir::North, Dir::East][i.rem_euclid(4) as usize]
    }

    /// `Direction.get2DDataValue`.
    pub fn index_2d(self) -> i32 {
        match self {
            Dir::South => 0,
            Dir::West => 1,
            Dir::North => 2,
            Dir::East => 3,
            _ => -1,
        }
    }

    pub fn name(self) -> &'static str {
        ["down", "up", "north", "south", "west", "east"][self as usize]
    }

    pub fn by_name(name: &str) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.name() == name)
    }

    pub fn is_horizontal(self) -> bool {
        !matches!(self, Dir::Down | Dir::Up)
    }

    /// `Direction.getClockWise` (horizontal directions).
    pub fn clockwise(self) -> Dir {
        match self {
            Dir::North => Dir::East,
            Dir::East => Dir::South,
            Dir::South => Dir::West,
            Dir::West => Dir::North,
            d => d,
        }
    }

    pub fn counter_clockwise(self) -> Dir {
        self.clockwise().opposite()
    }
}

/// Fluid types in `BuiltInRegistries.FLUID` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidKind {
    Empty = 0,
    FlowingWater = 1,
    Water = 2,
    FlowingLava = 3,
    Lava = 4,
}

/// A block state's `FluidState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fluid {
    pub kind: FluidKind,
    pub amount: u8,
    pub source: bool,
}

impl Fluid {
    pub fn is_empty(self) -> bool {
        self.kind == FluidKind::Empty
    }

    /// `FluidState.is(FluidTags.WATER)`.
    pub fn is_water(self) -> bool {
        matches!(self.kind, FluidKind::Water | FluidKind::FlowingWater)
    }

    /// `FluidState.is(FluidTags.LAVA)`.
    pub fn is_lava(self) -> bool {
        matches!(self.kind, FluidKind::Lava | FluidKind::FlowingLava)
    }

    /// `FluidState.isSourceOfType(Fluids.WATER)`.
    pub fn is_water_source(self) -> bool {
        self.kind == FluidKind::Water && self.source
    }

    /// Registry name of the fluid type.
    pub fn name(self) -> &'static str {
        ["minecraft:empty", "minecraft:flowing_water", "minecraft:water", "minecraft:flowing_lava", "minecraft:lava"]
            [self.kind as usize]
    }
}

struct Table {
    flags: Vec<u32>,
    fluids: Vec<(u8, u8)>,
    /// Class chain of each block (registry order), outermost class first.
    classes: Vec<String>,
}

const SOLID: u32 = 1;
const COLLISION_FULL_UP: u32 = 1 << 19;
const REDSTONE_CONDUCTOR: u32 = 1 << 20;
const FLUID_SOURCE: u32 = 1 << 22;

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let u32_at = |p: usize| u32::from_le_bytes(RAW[p..p + 4].try_into().unwrap());
        assert_eq!(&RAW[..4], b"KWBF", "block_facts.bin: bad magic");
        assert_eq!(u32_at(4), 1, "block_facts.bin: unsupported version");
        let n = u32_at(8) as usize;
        assert_eq!(n as u32, kiln_data::blocks::STATE_COUNT, "block_facts.bin: state count differs from kiln-data");
        let mut p = 12;
        let mut flags = Vec::with_capacity(n);
        let mut fluids = Vec::with_capacity(n);
        for _ in 0..n {
            flags.push(u32_at(p));
            fluids.push((RAW[p + 4], RAW[p + 5]));
            p += 6;
        }
        let blocks = u32_at(p) as usize;
        p += 4;
        let mut classes = Vec::with_capacity(blocks);
        for _ in 0..blocks {
            let len = u32_at(p) as usize;
            p += 4;
            classes.push(String::from_utf8(RAW[p..p + len].to_vec()).expect("utf-8 class name"));
            p += len;
        }
        assert_eq!(p, RAW.len(), "block_facts.bin: trailing data");
        assert_eq!(classes.len(), kiln_data::blocks::BLOCKS.len(), "block_facts.bin: block count differs from kiln-data");
        Table { flags, fluids, classes }
    })
}

/// `BlockState.isSolid()` (the deprecated legacy solidity).
pub fn is_solid(state: u16) -> bool {
    table().flags[state as usize] & SOLID != 0
}

/// `BlockState.isFaceSturdy(level, pos, dir, support)` for the state's unoffset shape.
pub fn is_face_sturdy(state: u16, dir: Dir, support: Support) -> bool {
    table().flags[state as usize] & (1 << (1 + dir as u32 * 3 + support as u32)) != 0
}

/// `Block.isFaceFull(state.getCollisionShape(level, pos), UP)`.
pub fn collision_top_full(state: u16) -> bool {
    table().flags[state as usize] & COLLISION_FULL_UP != 0
}

/// `BlockState.isRedstoneConductor(level, pos)`.
pub fn is_redstone_conductor(state: u16) -> bool {
    table().flags[state as usize] & REDSTONE_CONDUCTOR != 0
}

/// `BlockState.getFluidState()`.
pub fn fluid(state: u16) -> Fluid {
    let t = table();
    let (kind, amount) = t.fluids[state as usize];
    let kind = match kind {
        1 => FluidKind::FlowingWater,
        2 => FluidKind::Water,
        3 => FluidKind::FlowingLava,
        4 => FluidKind::Lava,
        _ => FluidKind::Empty,
    };
    Fluid { kind, amount, source: t.flags[state as usize] & FLUID_SOURCE != 0 }
}

/// The class chain of a state's block, e.g. `"SaplingBlock<VegetationBlock<Block"`.
pub fn class_chain(state: u16) -> &'static str {
    let blocks = kiln_data::blocks::BLOCKS;
    &table().classes[blocks.partition_point(|b| b.first <= state) - 1]
}

/// Whether a state's block is an instance of the vanilla class `class` (simple name).
pub fn is_instance(state: u16, class: &str) -> bool {
    class_chain(state).split('<').any(|c| c == class)
}

/// The block's own class (simple name).
pub fn block_class(state: u16) -> &'static str {
    class_chain(state).split('<').next().unwrap_or("Block")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn spot_checks() {
        assert!(is_solid(d::STONE) && !is_solid(d::AIR) && !is_solid(d::SHORT_GRASS));
        assert!(is_face_sturdy(d::STONE, Dir::Up, Support::Full));
        assert!(!is_face_sturdy(d::AIR, Dir::Up, Support::Center));
        assert_eq!(fluid(d::WATER), Fluid { kind: FluidKind::Water, amount: 8, source: true });
        assert!(fluid(d::STONE).is_empty());
        assert!(fluid(d::SEAGRASS).is_water_source());
        assert!(is_instance(d::OAK_SAPLING, "SaplingBlock"));
        assert!(is_instance(d::TALL_GRASS, "DoublePlantBlock"));
        assert!(collision_top_full(d::STONE) && !collision_top_full(d::AIR));
        assert!(is_redstone_conductor(d::STONE) && !is_redstone_conductor(d::GLASS));
    }
}
