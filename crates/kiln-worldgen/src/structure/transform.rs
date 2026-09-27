//! `Rotation` and `Mirror`, and turning block states with them (`BlockState.rotate`/`mirror`).
//!
//! Vanilla implements rotation per block class; the properties those overrides touch follow a
//! few patterns (facing, axis, 16-step rotation, per-side connections, rail and stair shapes,
//! door hinges, chest halves), which is what this implements.

use crate::block_facts::{Dir, block_class};
use crate::blocks::{has_prop, prop, with_prop};
use kiln_data::blocks_types::block_of;

/// `Rotation`, in vanilla's ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Rotation {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    CounterClockwise90,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::CounterClockwise90];

    /// `Rotation.rotate(Direction)`.
    pub fn rotate(self, d: Dir) -> Dir {
        if !d.is_horizontal() {
            return d;
        }
        match self {
            Rotation::None => d,
            Rotation::Clockwise90 => d.clockwise(),
            Rotation::Clockwise180 => d.opposite(),
            Rotation::CounterClockwise90 => d.counter_clockwise(),
        }
    }

    /// `Rotation.rotate(rotation, positionCount)`.
    pub fn rotate_index(self, r: i32, count: i32) -> i32 {
        match self {
            Rotation::None => r,
            Rotation::Clockwise90 => (r + count / 4) % count,
            Rotation::Clockwise180 => (r + count / 2) % count,
            Rotation::CounterClockwise90 => (r + count * 3 / 4) % count,
        }
    }

    /// `Rotation.getRotated`: this rotation followed by `other`.
    pub fn then(self, other: Rotation) -> Rotation {
        Rotation::ALL[(self as usize + other as usize) % 4]
    }

    pub fn name(self) -> &'static str {
        ["none", "clockwise_90", "180", "counterclockwise_90"][self as usize]
    }

    /// `Rotation.valueOf` names as saved in structure piece NBT (`NONE`...).
    pub fn enum_name(self) -> &'static str {
        ["NONE", "CLOCKWISE_90", "CLOCKWISE_180", "COUNTERCLOCKWISE_90"][self as usize]
    }
}

/// `Mirror`, in vanilla's ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Mirror {
    #[default]
    None,
    LeftRight,
    FrontBack,
}

impl Mirror {
    /// `Mirror.mirror(Direction)`.
    pub fn mirror(self, d: Dir) -> Dir {
        match (self, d) {
            (Mirror::FrontBack, Dir::East | Dir::West) | (Mirror::LeftRight, Dir::North | Dir::South) => d.opposite(),
            _ => d,
        }
    }

    /// `Mirror.getRotation(Direction)`.
    pub fn rotation(self, d: Dir) -> Rotation {
        match (self, d) {
            (Mirror::LeftRight, Dir::North | Dir::South) | (Mirror::FrontBack, Dir::East | Dir::West) => Rotation::Clockwise180,
            _ => Rotation::None,
        }
    }

    /// `Mirror.mirror(rotation, rotationCount)`.
    pub fn mirror_index(self, r: i32, count: i32) -> i32 {
        let half = count / 2;
        let r = if r > half { r - count } else { r };
        match self {
            Mirror::LeftRight => (half - r + count) % count,
            Mirror::FrontBack => (count - r) % count,
            Mirror::None => r,
        }
    }

    pub fn enum_name(self) -> &'static str {
        ["NONE", "LEFT_RIGHT", "FRONT_BACK"][self as usize]
    }
}

fn dir_prop(s: u16, name: &str) -> Option<Dir> {
    prop(s, name).and_then(Dir::by_name)
}

/// Remaps the four side properties (`north`... or `NORTH`-keyed values) of connection blocks.
fn map_sides(s: u16, f: impl Fn(Dir) -> Dir) -> u16 {
    let values: Vec<(Dir, &'static str)> =
        Dir::HORIZONTAL.iter().filter_map(|&d| prop(s, d.name()).map(|v| (d, v))).collect();
    if values.len() != 4 {
        return s;
    }
    let mut out = s;
    for (d, v) in values {
        out = with_prop(out, f(d).name(), v);
    }
    out
}

/// Remaps all six side properties (huge mushroom blocks, multiface blocks with `up`/`down`).
fn map_all_sides(s: u16, f: impl Fn(Dir) -> Dir) -> u16 {
    let values: Vec<(Dir, &'static str)> = Dir::ALL.iter().filter_map(|&d| prop(s, d.name()).map(|v| (d, v))).collect();
    let mut out = s;
    for (d, v) in values {
        out = with_prop(out, f(d).name(), v);
    }
    out
}

/// A rail shape's two ends (`ascending_x` rises toward x).
fn rail_ends(shape: &str) -> Option<(Dir, Dir, bool)> {
    Some(match shape {
        "north_south" => (Dir::North, Dir::South, false),
        "east_west" => (Dir::East, Dir::West, false),
        "ascending_east" => (Dir::East, Dir::West, true),
        "ascending_west" => (Dir::West, Dir::East, true),
        "ascending_north" => (Dir::North, Dir::South, true),
        "ascending_south" => (Dir::South, Dir::North, true),
        "south_east" => (Dir::South, Dir::East, false),
        "south_west" => (Dir::South, Dir::West, false),
        "north_west" => (Dir::North, Dir::West, false),
        "north_east" => (Dir::North, Dir::East, false),
        _ => return None,
    })
}

fn rail_shape(a: Dir, b: Dir, ascending: bool) -> &'static str {
    if ascending {
        return match a {
            Dir::East => "ascending_east",
            Dir::West => "ascending_west",
            Dir::North => "ascending_north",
            _ => "ascending_south",
        };
    }
    let has = |d: Dir| a == d || b == d;
    match (has(Dir::North), has(Dir::South), has(Dir::East), has(Dir::West)) {
        (true, true, _, _) => "north_south",
        (_, _, true, true) => "east_west",
        (false, true, true, false) => "south_east",
        (false, true, false, true) => "south_west",
        (true, false, false, true) => "north_west",
        _ => "north_east",
    }
}

/// Remaps a `FrontAndTop` `orientation` (jigsaws, crafters).
fn map_orientation(s: u16, f: impl Fn(Dir) -> Dir) -> u16 {
    let Some((front, top)) = prop(s, "orientation").and_then(|v| v.split_once('_')) else { return s };
    let (Some(front), Some(top)) = (Dir::by_name(front), Dir::by_name(top)) else { return s };
    with_prop(s, "orientation", &format!("{}_{}", f(front).name(), f(top).name()))
}

fn map_rail(s: u16, f: impl Fn(Dir) -> Dir) -> u16 {
    let Some((a, b, up)) = prop(s, "shape").and_then(rail_ends) else { return s };
    with_prop(s, "shape", rail_shape(f(a), f(b), up))
}

/// Block classes overriding `BlockBehaviour.rotate` (the others ignore rotation).
const ROTATES: [&str; 65] = [
    "AbstractFurnaceBlock",
    "AmethystClusterBlock",
    "AnvilBlock",
    "AttachedStemBlock",
    "BannerBlock",
    "BarrelBlock",
    "BaseCoralWallFanBlock",
    "BeehiveBlock",
    "BellBlock",
    "CalibratedSculkSensorBlock",
    "CampfireBlock",
    "CeilingHangingSignBlock",
    "ChestBlock",
    "ChiseledBookShelfBlock",
    "CommandBlock",
    "CopperGolemStatueBlock",
    "CrafterBlock",
    "CreakingHeartBlock",
    "CrossCollisionBlock",
    "DecoratedPotBlock",
    "DetectorRailBlock",
    "DispenserBlock",
    "DoorBlock",
    "EndPortalFrameBlock",
    "EnderChestBlock",
    "FlowerBedBlock",
    "GrindstoneBlock",
    "HopperBlock",
    "HorizontalDirectionalBlock",
    "HugeMushroomBlock",
    "InfestedRotatedPillarBlock",
    "JigsawBlock",
    "LadderBlock",
    "LeafLitterBlock",
    "LecternBlock",
    "MossyCarpetBlock",
    "MultifaceBlock",
    "NetherPortalBlock",
    "ObserverBlock",
    "PoweredRailBlock",
    "RailBlock",
    "RedstoneWallTorchBlock",
    "RedstoneWireBlock",
    "RodBlock",
    "RotatedPillarBlock",
    "ShelfBlock",
    "ShulkerBoxBlock",
    "SkullBlock",
    "SmallDripleafBlock",
    "StairBlock",
    "StandingSignBlock",
    "StonecutterBlock",
    "TripWireBlock",
    "TripWireHookBlock",
    "VaultBlock",
    "VineBlock",
    "WallBannerBlock",
    "WallBlock",
    "WallHangingSignBlock",
    "WallSignBlock",
    "WallSkullBlock",
    "WallTorchBlock",
    "PistonBaseBlock",
    "PistonHeadBlock",
    "MovingPistonBlock",
];

/// Block classes overriding `BlockBehaviour.mirror` (the others ignore mirroring).
const MIRRORS: [&str; 60] = [
    "AbstractFurnaceBlock",
    "AmethystClusterBlock",
    "AttachedStemBlock",
    "BannerBlock",
    "BarrelBlock",
    "BaseCoralWallFanBlock",
    "BeehiveBlock",
    "BellBlock",
    "CalibratedSculkSensorBlock",
    "CampfireBlock",
    "CeilingHangingSignBlock",
    "ChestBlock",
    "ChiseledBookShelfBlock",
    "CommandBlock",
    "CopperGolemStatueBlock",
    "CrafterBlock",
    "CrossCollisionBlock",
    "DecoratedPotBlock",
    "DetectorRailBlock",
    "DispenserBlock",
    "DoorBlock",
    "EndPortalFrameBlock",
    "EnderChestBlock",
    "FlowerBedBlock",
    "GrindstoneBlock",
    "HopperBlock",
    "HorizontalDirectionalBlock",
    "HugeMushroomBlock",
    "JigsawBlock",
    "LadderBlock",
    "LeafLitterBlock",
    "LecternBlock",
    "MossyCarpetBlock",
    "MultifaceBlock",
    "ObserverBlock",
    "PoweredRailBlock",
    "RailBlock",
    "RedstoneWallTorchBlock",
    "RedstoneWireBlock",
    "RodBlock",
    "ShelfBlock",
    "ShulkerBoxBlock",
    "SkullBlock",
    "SmallDripleafBlock",
    "StairBlock",
    "StandingSignBlock",
    "StonecutterBlock",
    "TripWireBlock",
    "TripWireHookBlock",
    "VaultBlock",
    "VineBlock",
    "WallBannerBlock",
    "WallBlock",
    "WallHangingSignBlock",
    "WallSignBlock",
    "WallSkullBlock",
    "WallTorchBlock",
    "PistonBaseBlock",
    "PistonHeadBlock",
    "MovingPistonBlock",
];

/// Whether a state's class chain includes one of `classes`.
fn overrides(s: u16, classes: &[&str]) -> bool {
    crate::block_facts::class_chain(s).split('<').any(|c| classes.contains(&c))
}

/// `BlockState.rotate(rotation)`.
pub fn rotate(s: u16, r: Rotation) -> u16 {
    if r == Rotation::None || !overrides(s, &ROTATES) {
        return s;
    }
    let class = block_class(s);
    let info = block_of(s);
    let values = |name: &str| info.properties.iter().find(|p| p.name == name).map_or(0, |p| p.values.len());
    let mut out = s;
    if values("axis") >= 2 && matches!(r, Rotation::Clockwise90 | Rotation::CounterClockwise90) {
        out = match prop(s, "axis") {
            Some("x") => with_prop(out, "axis", "z"),
            Some("z") => with_prop(out, "axis", "x"),
            _ => out,
        };
    }
    if let Some(d) = dir_prop(s, "facing") {
        out = with_prop(out, "facing", r.rotate(d).name());
    }
    if values("rotation") == 16 {
        let v: i32 = prop(s, "rotation").and_then(|v| v.parse().ok()).unwrap_or(0);
        out = with_prop(out, "rotation", &r.rotate_index(v, 16).to_string());
    }
    if class.ends_with("RailBlock") && has_prop(s, "shape") {
        out = map_rail(out, |d| r.rotate(d));
    }
    out = map_orientation(out, |d| r.rotate(d));
    if has_prop(s, "up") && has_prop(s, "down") {
        out = map_all_sides(out, |d| r.rotate(d));
    } else {
        out = map_sides(out, |d| r.rotate(d));
    }
    out
}

/// `BlockState.mirror(mirror)`.
pub fn mirror(s: u16, m: Mirror) -> u16 {
    if m == Mirror::None || !overrides(s, &MIRRORS) {
        return s;
    }
    let class = block_class(s);
    let info = block_of(s);
    let values = |name: &str| info.properties.iter().find(|p| p.name == name).map_or(0, |p| p.values.len());
    if class == "StairBlock" {
        let Some(facing) = dir_prop(s, "facing") else { return s };
        let shape = prop(s, "shape").unwrap_or("straight");
        let z = matches!(facing, Dir::North | Dir::South);
        let flipped = |to: &str| with_prop(rotate(s, Rotation::Clockwise180), "shape", to);
        return match (m, z) {
            (Mirror::LeftRight, true) => match shape {
                "outer_left" => flipped("outer_right"),
                "inner_right" => flipped("inner_left"),
                "inner_left" => flipped("inner_right"),
                "outer_right" => flipped("outer_left"),
                _ => rotate(s, Rotation::Clockwise180),
            },
            (Mirror::FrontBack, false) => match shape {
                "outer_left" => flipped("outer_right"),
                "inner_right" => flipped("inner_right"),
                "inner_left" => flipped("inner_left"),
                "outer_right" => flipped("outer_left"),
                _ => rotate(s, Rotation::Clockwise180),
            },
            _ => s,
        };
    }
    if class == "DoorBlock" {
        let facing = dir_prop(s, "facing").unwrap_or(Dir::North);
        let turned = rotate(s, m.rotation(facing));
        let hinge = if prop(turned, "hinge") == Some("left") { "right" } else { "left" };
        return with_prop(turned, "hinge", hinge);
    }
    if class == "ChestBlock" || class == "TrappedChestBlock" || class.ends_with("CopperChestBlock") {
        let facing = dir_prop(s, "facing").unwrap_or(Dir::North);
        return rotate(s, m.rotation(facing));
    }
    let mut out = s;
    if let Some(d) = dir_prop(s, "facing") {
        out = with_prop(out, "facing", m.mirror(d).name());
    }
    if values("rotation") == 16 {
        let v: i32 = prop(s, "rotation").and_then(|v| v.parse().ok()).unwrap_or(0);
        out = with_prop(out, "rotation", &m.mirror_index(v, 16).to_string());
    }
    if class.ends_with("RailBlock") && has_prop(s, "shape") {
        out = map_rail(out, |d| m.mirror(d));
    }
    out = map_orientation(out, |d| m.mirror(d));
    if has_prop(s, "up") && has_prop(s, "down") {
        out = map_all_sides(out, |d| m.mirror(d));
    } else {
        out = map_sides(out, |d| m.mirror(d));
    }
    out
}
