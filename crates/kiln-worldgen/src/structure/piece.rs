//! Structure pieces (`StructurePiece`): the saved parts of a structure start, placed chunk by
//! chunk during FEATURES. [`PieceBase`] carries the common state and vanilla's placement
//! helpers in piece-local coordinates.

use super::bbox::BoundingBox;
use super::transform::{Mirror, Rotation, mirror, rotate};
use crate::block_facts::{Dir, fluid};
use crate::blocks::{is_air, state};
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use kiln_data::blocks_types::block_of;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// What pieces may need while placing, besides the region (`StructureManager`, the chunk
/// generator, the template manager).
pub struct PlaceContext<'a> {
    pub structures: &'a super::Structures,
    pub generator: &'a crate::generator::Generator,
    /// Placed features, for feature pool elements.
    pub features: Option<&'a crate::feature::Features>,
}

/// A structure piece.
pub trait Piece: Send + Sync + std::fmt::Debug {
    fn base(&self) -> &PieceBase;

    fn base_mut(&mut self) -> &mut PieceBase;

    /// `postProcess`: places the part of the piece inside `chunk_box`.
    fn place(
        &self,
        cx: &PlaceContext,
        r: &mut Region,
        random: &mut WorldgenRandom,
        chunk_box: &BoundingBox,
        chunk: (i32, i32),
        pivot: BlockPos,
    );

    /// `addAdditionalSaveData`: the piece type's own NBT fields.
    fn save_extra(&self, tag: &mut Vec<(String, Tag)>);

    /// `StructurePiece.createTag`.
    fn save(&self) -> Tag {
        let b = self.base();
        let mut tag = vec![
            ("id".to_string(), Tag::String(b.kind.to_string())),
            ("BB".to_string(), b.bbox.to_tag()),
            ("O".to_string(), Tag::Int(b.orientation.map_or(-1, Dir::index_2d))),
            ("GD".to_string(), Tag::Int(b.gen_depth)),
        ];
        self.save_extra(&mut tag);
        Tag::Compound(tag)
    }

    /// `StructurePiece.move`.
    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base_mut().bbox.shift(dx, dy, dz);
    }

    /// The piece as a `PoolElementStructurePiece` (jigsaw pieces), for the beardifier.
    fn as_pool_element(&self) -> Option<&super::jigsaw::piece::PoolElementPiece> {
        None
    }
}

/// `StructurePiece`'s own fields.
#[derive(Clone, Debug)]
pub struct PieceBase {
    /// Piece type (`StructurePieceType` registry id).
    pub kind: &'static str,
    pub gen_depth: i32,
    pub bbox: BoundingBox,
    orientation: Option<Dir>,
    pub mirror: Mirror,
    pub rotation: Rotation,
    /// `canBeReplaced` override: whether `placeBlock` may replace the state there.
    pub can_replace: Option<fn(u16) -> bool>,
}

/// Blocks whose shape depends on neighbours: placing one marks it for post-processing.
const SHAPE_CHECK_BLOCKS: [&str; 12] = [
    "minecraft:nether_brick_fence",
    "minecraft:torch",
    "minecraft:wall_torch",
    "minecraft:oak_fence",
    "minecraft:spruce_fence",
    "minecraft:dark_oak_fence",
    "minecraft:pale_oak_fence",
    "minecraft:acacia_fence",
    "minecraft:birch_fence",
    "minecraft:jungle_fence",
    "minecraft:ladder",
    "minecraft:iron_bars",
];

impl PieceBase {
    pub fn new(kind: &'static str, gen_depth: i32, bbox: BoundingBox) -> Self {
        Self { kind, gen_depth, bbox, orientation: None, mirror: Mirror::None, rotation: Rotation::None, can_replace: None }
    }

    pub fn orientation(&self) -> Option<Dir> {
        self.orientation
    }

    /// `setOrientation`: also sets the mirror and rotation block states get.
    pub fn set_orientation(&mut self, d: Option<Dir>) {
        self.orientation = d;
        (self.mirror, self.rotation) = match d {
            Some(Dir::South) => (Mirror::LeftRight, Rotation::None),
            Some(Dir::West) => (Mirror::LeftRight, Rotation::Clockwise90),
            Some(Dir::East) => (Mirror::None, Rotation::Clockwise90),
            _ => (Mirror::None, Rotation::None),
        };
    }

    /// `makeBoundingBox`.
    pub fn make_bbox(x: i32, y: i32, z: i32, d: Dir, w: i32, h: i32, depth: i32) -> BoundingBox {
        if matches!(d, Dir::North | Dir::South) {
            BoundingBox::new(x, y, z, x + w - 1, y + h - 1, z + depth - 1)
        } else {
            BoundingBox::new(x, y, z, x + depth - 1, y + h - 1, z + w - 1)
        }
    }

    /// `getRandomHorizontalDirection`.
    pub fn random_horizontal(random: &mut WorldgenRandom) -> Dir {
        Dir::HORIZONTAL[random.next_int_bounded(4) as usize]
    }

    pub fn world_x(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Dir::North | Dir::South) => self.bbox.min_x + x,
            Some(Dir::West) => self.bbox.max_x - z,
            Some(Dir::East) => self.bbox.min_x + z,
            _ => x,
        }
    }

    pub fn world_y(&self, y: i32) -> i32 {
        if self.orientation.is_none() { y } else { y + self.bbox.min_y }
    }

    pub fn world_z(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Dir::North) => self.bbox.max_z - z,
            Some(Dir::South) => self.bbox.min_z + z,
            Some(Dir::West | Dir::East) => self.bbox.min_z + x,
            _ => z,
        }
    }

    pub fn world_pos(&self, x: i32, y: i32, z: i32) -> BlockPos {
        BlockPos::new(self.world_x(x, z), self.world_y(y), self.world_z(x, z))
    }

    /// `placeBlock`: sets a block given in piece coordinates, turned by the piece's mirror and
    /// rotation, if inside `bbox`.
    pub fn place_block(&self, r: &mut Region, s: u16, x: i32, y: i32, z: i32, bbox: &BoundingBox) {
        let p = self.world_pos(x, y, z);
        if !bbox.is_inside(p) {
            return;
        }
        if let Some(can_replace) = self.can_replace
            && !can_replace(r.get(p))
        {
            return;
        }
        let mut s = s;
        if self.mirror != Mirror::None {
            s = mirror(s, self.mirror);
        }
        if self.rotation != Rotation::None {
            s = rotate(s, self.rotation);
        }
        r.set(p, s, 2);
        let f = fluid(r.get(p));
        if !f.is_empty() {
            r.schedule_fluid_tick(p, f.name(), 0);
        }
        if SHAPE_CHECK_BLOCKS.contains(&block_of(s).name) {
            r.mark_post_processing(p);
        }
    }

    /// `getBlock`: the block at piece coordinates, air outside `bbox`.
    pub fn get_block(&self, r: &mut Region, x: i32, y: i32, z: i32, bbox: &BoundingBox) -> u16 {
        let p = self.world_pos(x, y, z);
        if bbox.is_inside(p) { r.get(p) } else { state::AIR }
    }

    /// `isInterior`: the block above is below the ocean floor.
    pub fn is_interior(&self, r: &mut Region, x: i32, y: i32, z: i32, bbox: &BoundingBox) -> bool {
        let p = self.world_pos(x, y + 1, z);
        bbox.is_inside(p) && p.y < r.height_at(Heightmap::OceanFloorWg, p.x, p.z)
    }

    pub fn generate_air_box(&self, r: &mut Region, bbox: &BoundingBox, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    self.place_block(r, state::AIR, x, y, z, bbox);
                }
            }
        }
    }

    /// `generateBox`: `edge` on the box's faces, `inner` inside; `skip_air` leaves air alone.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_box(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        edge: u16,
        inner: u16,
        skip_air: bool,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if skip_air && is_air(self.get_block(r, x, y, z, bbox)) {
                        continue;
                    }
                    let on_edge = y == y0 || y == y1 || x == x0 || x == x1 || z == z0 || z == z1;
                    self.place_block(r, if on_edge { edge } else { inner }, x, y, z, bbox);
                }
            }
        }
    }

    /// `generateBox` with a `BlockSelector`: `select(random, x, y, z, on_edge)` gives each state.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_box_with(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        skip_air: bool,
        random: &mut WorldgenRandom,
        select: &mut dyn FnMut(&mut WorldgenRandom, i32, i32, i32, bool) -> u16,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if skip_air && is_air(self.get_block(r, x, y, z, bbox)) {
                        continue;
                    }
                    let on_edge = y == y0 || y == y1 || x == x0 || x == x1 || z == z0 || z == z1;
                    let s = select(random, x, y, z, on_edge);
                    self.place_block(r, s, x, y, z, bbox);
                }
            }
        }
    }

    /// `generateMaybeBox`.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_maybe_box(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        random: &mut WorldgenRandom,
        chance: f32,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        edge: u16,
        inner: u16,
        skip_air: bool,
        only_interior: bool,
    ) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if random.next_float() > chance {
                        continue;
                    }
                    if skip_air && is_air(self.get_block(r, x, y, z, bbox)) {
                        continue;
                    }
                    if only_interior && !self.is_interior(r, x, y, z, bbox) {
                        continue;
                    }
                    let on_edge = y == y0 || y == y1 || x == x0 || x == x1 || z == z0 || z == z1;
                    self.place_block(r, if on_edge { edge } else { inner }, x, y, z, bbox);
                }
            }
        }
    }

    /// `maybeGenerateBlock`.
    #[allow(clippy::too_many_arguments)]
    pub fn maybe_generate_block(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        random: &mut WorldgenRandom,
        chance: f32,
        x: i32,
        y: i32,
        z: i32,
        s: u16,
    ) {
        if random.next_float() < chance {
            self.place_block(r, s, x, y, z, bbox);
        }
    }

    /// `generateUpperHalfSphere`.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_upper_half_sphere(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        s: u16,
        skip_air: bool,
    ) {
        let (sx, sy, sz) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32, (z1 - z0 + 1) as f32);
        let (cx, cz) = (x0 as f32 + sx / 2.0, z0 as f32 + sz / 2.0);
        for y in y0..=y1 {
            let fy = (y - y0) as f32 / sy;
            for x in x0..=x1 {
                let fx = (x as f32 - cx) / (sx * 0.5);
                for z in z0..=z1 {
                    let fz = (z as f32 - cz) / (sz * 0.5);
                    if skip_air && is_air(self.get_block(r, x, y, z, bbox)) {
                        continue;
                    }
                    if !(fx * fx + fy * fy + fz * fz > 1.05) {
                        self.place_block(r, s, x, y, z, bbox);
                    }
                }
            }
        }
    }

    /// `fillColumnDown`.
    pub fn fill_column_down(&self, r: &mut Region, s: u16, x: i32, y: i32, z: i32, bbox: &BoundingBox) {
        let mut p = self.world_pos(x, y, z);
        if !bbox.is_inside(p) {
            return;
        }
        while is_replaceable_by_structures(r.get(p)) && p.y > r.min_y() + 1 {
            r.set(p, s, 2);
            p = p.below();
        }
    }

    /// `createChest` at piece coordinates: a chest (facing chosen by `reorient` unless given)
    /// with a loot table and seed.
    pub fn create_chest(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        random: &mut WorldgenRandom,
        x: i32,
        y: i32,
        z: i32,
        loot_table: &str,
    ) -> bool {
        let p = self.world_pos(x, y, z);
        create_chest_at(r, bbox, random, p, loot_table, None)
    }

    /// `createDispenser`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_dispenser(
        &self,
        r: &mut Region,
        bbox: &BoundingBox,
        random: &mut WorldgenRandom,
        x: i32,
        y: i32,
        z: i32,
        facing: Dir,
        loot_table: &str,
    ) -> bool {
        let p = self.world_pos(x, y, z);
        let dispenser = crate::blocks::block("minecraft:dispenser").expect("dispenser").default;
        if !bbox.is_inside(p) || crate::blocks::same_block(r.get(p), dispenser) {
            return false;
        }
        self.place_block(r, crate::blocks::with_prop(dispenser, "facing", facing.name()), x, y, z, bbox);
        let seed = random.next_long();
        set_loot_table(r, p, loot_table, seed);
        true
    }
}

/// `StructurePiece.isReplaceableByStructures`.
pub fn is_replaceable_by_structures(s: u16) -> bool {
    is_air(s)
        || matches!(block_of(s).name, "minecraft:water" | "minecraft:lava" | "minecraft:glow_lichen" | "minecraft:seagrass" | "minecraft:tall_seagrass")
}

/// Writes a loot table and seed into the block entity at `p` (`setLootTable`).
pub fn set_loot_table(r: &mut Region, p: BlockPos, table: &str, seed: i64) {
    if let Some(Tag::Compound(fields)) = r.block_entity_mut(p) {
        fields.retain(|(k, _)| k != "LootTable" && k != "LootTableSeed");
        fields.push(("LootTable".to_string(), Tag::String(table.to_string())));
        fields.push(("LootTableSeed".to_string(), Tag::Long(seed)));
    }
}

/// `StructurePiece.createChest(level, box, random, pos, table, state)`.
pub fn create_chest_at(r: &mut Region, bbox: &BoundingBox, random: &mut WorldgenRandom, p: BlockPos, table: &str, s: Option<u16>) -> bool {
    let chest = crate::blocks::block("minecraft:chest").expect("chest").default;
    if !bbox.is_inside(p) || crate::blocks::same_block(r.get(p), chest) {
        return false;
    }
    let s = s.unwrap_or_else(|| reorient(r, p, chest));
    r.set(p, s, 2);
    let seed = random.next_long();
    set_loot_table(r, p, table, seed);
    true
}

/// `StructurePiece.reorient`: faces a chest away from a solid neighbour.
pub fn reorient(r: &mut Region, p: BlockPos, s: u16) -> u16 {
    use crate::blocks::{prop, same_block, with_prop};
    use kiln_data::block_props::solid_render;
    let chest = crate::blocks::block("minecraft:chest").expect("chest").default;
    let mut solid: Option<Dir> = None;
    for d in Dir::HORIZONTAL {
        let n = r.get(p.relative(d));
        if same_block(n, chest) {
            return s;
        }
        if solid_render(n) {
            if solid.is_some() {
                solid = None;
                break;
            }
            solid = Some(d);
        }
    }
    if let Some(d) = solid {
        return with_prop(s, "facing", d.opposite().name());
    }
    let mut d = prop(s, "facing").and_then(Dir::by_name).unwrap_or(Dir::North);
    if solid_render(r.get(p.relative(d))) {
        d = d.opposite();
    }
    if solid_render(r.get(p.relative(d))) {
        d = d.clockwise();
    }
    if solid_render(r.get(p.relative(d))) {
        d = d.opposite();
    }
    with_prop(s, "facing", d.name())
}
