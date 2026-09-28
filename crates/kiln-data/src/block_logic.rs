//! Facts block behaviour needs, extracted from the vanilla game (`cargo xtask extract`):
//! per state face sturdiness, redstone flags and context-free signal strengths, fluid state,
//! piston push reaction and wall cover tests; per block its Java class, superclasses,
//! interfaces and a few constructor parameters.

use crate::blocks::{BLOCKS, STATE_COUNT};
use std::sync::OnceLock;

#[path = "gen/block_classes.rs"]
#[allow(clippy::needless_update)]
mod classes;
pub use classes::{BlockClass, BlockClassInfo, BlockParams, interface};

#[path = "gen/block_items.rs"]
#[allow(clippy::type_complexity)]
mod items;

/// The block a block item places and, for standing-and-wall items (torches, signs, heads,
/// banners, fans), the wall block with the direction the standing block attaches toward.
pub fn block_item(item: &str) -> Option<(&'static str, Option<(&'static str, &'static str)>)> {
    let i = items::BLOCK_ITEMS.binary_search_by(|(name, _, _)| (*name).cmp(item)).ok()?;
    let (_, block, wall) = items::BLOCK_ITEMS[i];
    Some((block, wall))
}

#[path = "gen/flammability.rs"]
mod flammability;

/// `FireBlock`'s (ignite odds, burn odds) of the state's block, before the waterlogged check
/// (0, 0 for blocks fire does not burn).
pub fn flammability(state: u16) -> (i32, i32) {
    static BY_BLOCK: OnceLock<Vec<(u8, u8)>> = OnceLock::new();
    let t = BY_BLOCK.get_or_init(|| {
        let mut t = vec![(0, 0); BLOCKS.len()];
        for &(name, ignite, burn) in flammability::FLAMMABILITY {
            let i = BLOCKS.iter().position(|b| b.name == name).expect("flammable block exists");
            t[i] = (ignite, burn);
        }
        t
    });
    let (i, b) = t[block_index(state)];
    (i32::from(i), i32::from(b))
}

static RAW: &[u8] = include_bytes!("gen/block_logic.bin");

struct Table {
    states: Vec<u64>,
    signals: Vec<[u8; 12]>,
    /// Block index (into `BLOCKS`) of every state.
    block_of: Vec<u16>,
    /// `BLOCK_CLASSES` entry of every block index.
    class_of: Vec<u16>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        assert_eq!(&RAW[..4], b"KBL1", "block_logic.bin: bad magic");
        let n = u32::from_le_bytes(RAW[4..8].try_into().unwrap()) as usize;
        assert_eq!(n, STATE_COUNT as usize, "block_logic.bin: state count");
        let rows = u16::from_le_bytes(RAW[8..10].try_into().unwrap()) as usize;
        let mut p = 10;
        let states = (0..n)
            .map(|_| {
                let v = u64::from_le_bytes(RAW[p..p + 8].try_into().unwrap());
                p += 8;
                v
            })
            .collect();
        let signals = (0..rows)
            .map(|_| {
                let r: [u8; 12] = RAW[p..p + 12].try_into().unwrap();
                p += 12;
                r
            })
            .collect();
        assert_eq!(p, RAW.len(), "block_logic.bin: trailing data");
        let mut block_of = vec![0u16; n];
        for (i, b) in BLOCKS.iter().enumerate() {
            block_of[b.first as usize..=b.last as usize].fill(i as u16);
        }
        let names = crate::builtin_entries("minecraft:block").expect("block registry");
        let class_of = BLOCKS
            .iter()
            .map(|b| names.iter().position(|n| *n == b.name).expect("block in registry") as u16)
            .collect();
        Table { states, signals, block_of, class_of }
    })
}

fn word(state: u16) -> u64 {
    table().states[state as usize]
}

/// Index of the state's block in `blocks::BLOCKS`.
pub fn block_index(state: u16) -> usize {
    table().block_of[state as usize] as usize
}

/// Class facts of a block, by index in `blocks::BLOCKS`.
pub fn class_info(block_index: usize) -> &'static BlockClassInfo {
    &classes::BLOCK_CLASSES[table().class_of[block_index] as usize]
}

/// The block's own Java class.
pub fn block_class(state: u16) -> BlockClass {
    class_info(block_index(state)).classes[0]
}

/// Whether the block's class is `class` or extends it.
pub fn is_instance(state: u16, class: BlockClass) -> bool {
    class_info(block_index(state)).classes.contains(&class)
}

pub fn implements(state: u16, interface: u32) -> bool {
    class_info(block_index(state)).interfaces & interface != 0
}

pub fn params(state: u16) -> &'static BlockParams {
    &class_info(block_index(state)).params
}

/// `SupportType`: what a face must provide to hold something up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Full = 0,
    Center = 1,
    Rigid = 2,
}

/// `BlockState.isFaceSturdy` for the face toward `dir` (0 down .. 5 east).
pub fn face_sturdy(state: u16, dir: u8, support: Support) -> bool {
    word(state) >> (support as u32 * 6 + dir as u32) & 1 != 0
}

/// `PushReaction` in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushReaction {
    PushPull,
    Push,
    Popped,
    Immoveable,
    IgnoreEntity,
}

pub fn push_reaction(state: u16) -> PushReaction {
    match word(state) >> 18 & 7 {
        0 => PushReaction::PushPull,
        1 => PushReaction::Push,
        2 => PushReaction::Popped,
        3 => PushReaction::Immoveable,
        _ => PushReaction::IgnoreEntity,
    }
}

/// Whether this state, above a wall, covers the wall's post test shape (`WallBlock`).
pub fn wall_post_covered(state: u16) -> bool {
    word(state) >> 21 & 1 != 0
}

/// Whether this state, above a wall, covers the wall's side test shape toward `side`
/// (0 north, 1 east, 2 south, 3 west).
pub fn wall_side_covered(state: u16, side: u8) -> bool {
    word(state) >> (22 + side as u32) & 1 != 0
}

fn flag(state: u16, bit: u32) -> bool {
    word(state) >> (32 + bit) & 1 != 0
}

pub fn is_signal_source(state: u16) -> bool {
    flag(state, 0)
}

pub fn has_analog_output(state: u16) -> bool {
    flag(state, 1)
}

/// `isRedstoneConductor` (context-free).
pub fn is_redstone_conductor(state: u16) -> bool {
    flag(state, 2)
}

/// `BlockState.isSolid` (the legacy solidity flag).
pub fn is_solid(state: u16) -> bool {
    flag(state, 3)
}

pub fn ignited_by_lava(state: u16) -> bool {
    flag(state, 4)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FluidKind {
    Empty,
    Water,
    Lava,
}

/// A fluid state: kind, source or flowing, falling, amount (8 for sources, 1..=8 flowing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fluid {
    pub kind: FluidKind,
    pub source: bool,
    pub falling: bool,
    pub amount: u8,
}

impl Fluid {
    pub const EMPTY: Fluid = Fluid { kind: FluidKind::Empty, source: false, falling: false, amount: 0 };

    pub fn is_empty(self) -> bool {
        self.kind == FluidKind::Empty
    }
}

/// The fluid state of a block state (`BlockState.getFluidState`).
pub fn fluid(state: u16) -> Fluid {
    let b = (word(state) >> 40) as u8;
    let kind = match b & 3 {
        1 => FluidKind::Water,
        2 => FluidKind::Lava,
        _ => FluidKind::Empty,
    };
    Fluid { kind, source: b & 4 != 0, falling: b & 8 != 0, amount: b >> 4 }
}

/// The note block instrument a block gives (`BlockState.instrument`): its index in the
/// note block's `instrument` values, whether it works above a note block (mob heads), and
/// whether it is tunable.
pub fn instrument(state: u16) -> (u8, bool, bool) {
    let b = (word(state) >> 56) as u8;
    (b & 31, b & 32 != 0, b & 64 != 0)
}

fn signals(state: u16) -> &'static [u8; 12] {
    &table().signals[(word(state) >> 48 & 0xff) as usize]
}

/// `getSignal` toward `dir` computed without a level: exact for sources whose output depends
/// only on their state (levers, buttons, torches, repeaters, observers, plates, ...), zero for
/// ones that need the world (redstone wire, comparators, trapped chests).
pub fn weak_signal(state: u16, dir: u8) -> u8 {
    signals(state)[dir as usize]
}

/// `getDirectSignal` toward `dir`, computed like [`weak_signal`].
pub fn strong_signal(state: u16, dir: u8) -> u8 {
    signals(state)[6 + dir as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::default_state as d;
    use crate::blocks_types::block_by_name;

    #[test]
    fn spot_checks_against_known_vanilla_values() {
        assert!(face_sturdy(d::STONE, 1, Support::Full) && face_sturdy(d::STONE, 0, Support::Rigid));
        assert!(!face_sturdy(d::AIR, 1, Support::Center));
        let fence = block_by_name("minecraft:oak_fence").unwrap().default;
        assert!(face_sturdy(fence, 1, Support::Center) && !face_sturdy(fence, 1, Support::Full));
        assert!(is_redstone_conductor(d::STONE) && !is_redstone_conductor(d::GLASS));
        assert!(is_signal_source(d::REDSTONE_BLOCK) && weak_signal(d::REDSTONE_BLOCK, 3) == 15);
        assert!(is_solid(d::STONE) && !is_solid(d::AIR));
        assert_eq!(fluid(d::WATER), Fluid { kind: FluidKind::Water, source: true, falling: false, amount: 8 });
        let water = block_by_name("minecraft:water").unwrap();
        let flowing = water.with_property(water.default, "level", "3").unwrap();
        assert_eq!(fluid(flowing), Fluid { kind: FluidKind::Water, source: false, falling: false, amount: 5 });
        let falling = water.with_property(water.default, "level", "9").unwrap();
        assert!(fluid(falling).falling && fluid(falling).amount == 8);
        assert_eq!(push_reaction(d::OBSIDIAN), PushReaction::Immoveable);
        assert_eq!(push_reaction(d::TORCH), PushReaction::Popped);
        assert!(wall_post_covered(d::STONE) && !wall_post_covered(d::AIR));
        assert_eq!(block_class(fence), BlockClass::FenceBlock);
        assert!(is_instance(d::STONE_BUTTON, BlockClass::FaceAttachedHorizontalDirectionalBlock));
        assert_eq!(params(d::STONE_BUTTON).ticks_to_stay_pressed, 20);
        assert_eq!(params(d::OAK_BUTTON).ticks_to_stay_pressed, 30);
        assert!(implements(d::OAK_STAIRS, interface::SIMPLE_WATERLOGGED_BLOCK));
        assert_eq!(params(d::OAK_STAIRS).base_state, d::OAK_PLANKS as i32);
        assert!(!params(d::IRON_DOOR).open_by_hand && params(d::OAK_DOOR).open_by_hand);
        let lever = block_by_name("minecraft:lever").unwrap();
        let on = lever.with_property(lever.default, "powered", "true").unwrap();
        let on = lever.with_property(on, "face", "floor").unwrap();
        // A floor lever strongly powers the block below it (getDirectSignal toward UP).
        assert_eq!((strong_signal(on, 1), strong_signal(on, 0), weak_signal(on, 4)), (15, 0, 15));
    }
}
