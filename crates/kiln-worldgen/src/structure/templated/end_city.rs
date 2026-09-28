//! End cities (`EndCityStructure`, `EndCityPieces`): house towers, towers, bridges, fat towers
//! and the ship, grown recursively from the start with collision checks; shulker sentries and
//! the ship's elytra item frame are entities placed at data markers.

use super::TemplatePiece;
use crate::block_facts::Dir;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::processor::{IGNORE_STRUCTURE_AND_AIR, IGNORE_STRUCTURE_BLOCK};
use crate::structure::template::{LiquidSettings, TemplateManager, transform};
use crate::structure::transform::{Mirror, Rotation};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `EndCityPieces.MAX_GEN_DEPTH`.
const MAX_GEN_DEPTH: i32 = 8;

/// `EndCityStructure`.
pub struct EndCity;

impl Kind for EndCity {
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
            build: Box::new(move |ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
                let tm = ctx.structures.templates.clone();
                let mut b = Builder { tm: &tm, ship_created: false };
                let mut pieces = Vec::new();
                b.start_house_tower(pos, rotation, &mut pieces, &mut ctx.random);
                out.extend(pieces.into_iter().map(|p| Box::new(p) as Box<dyn Piece>));
            }),
        })
    }
}

/// `EndCityPieces.SectionGenerator` implementations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    HouseTower,
    Tower,
    TowerBridge,
    FatTower,
}

/// `TOWER_BRIDGES`.
const TOWER_BRIDGES: [(Rotation, (i32, i32, i32)); 4] = [
    (Rotation::None, (1, -1, 0)),
    (Rotation::Clockwise90, (6, -1, 1)),
    (Rotation::CounterClockwise90, (0, -1, 5)),
    (Rotation::Clockwise180, (5, -1, 6)),
];

/// `FAT_TOWER_BRIDGES`.
const FAT_TOWER_BRIDGES: [(Rotation, (i32, i32, i32)); 4] = [
    (Rotation::None, (4, -1, 0)),
    (Rotation::Clockwise90, (12, -1, 4)),
    (Rotation::CounterClockwise90, (0, -1, 8)),
    (Rotation::Clockwise180, (8, -1, 12)),
];

struct Builder<'a> {
    tm: &'a TemplateManager,
    /// `TOWER_BRIDGE_GENERATOR.shipCreated` (reset by `init` for each city).
    ship_created: bool,
}

impl Builder<'_> {
    /// `EndCityPieces.startHouseTower`.
    fn start_house_tower(&mut self, pos: BlockPos, rotation: Rotation, pieces: &mut Vec<EndCityPiece>, random: &mut WorldgenRandom) {
        self.ship_created = false;
        let base = EndCityPiece::new(self.tm, "base_floor", pos, rotation, true);
        pieces.push(base);
        let mut last = self.add(pieces, (-1, 0, -1), "second_floor_1", rotation, false);
        last = self.add_to(pieces, last, (-1, 4, -1), "third_floor_1", rotation, false);
        last = self.add_to(pieces, last, (-1, 8, -1), "third_roof", rotation, true);
        let parent = pieces[last].clone();
        self.recursive_children(Section::Tower, 1, &parent, None, pieces, random);
    }

    /// `addHelper(list, addPiece(tm, <last of list>, ...))`: returns the new piece's index.
    fn add(&self, list: &mut Vec<EndCityPiece>, offset: (i32, i32, i32), name: &str, rotation: Rotation, overwrite: bool) -> usize {
        let prev = list.len() - 1;
        self.add_to(list, prev, offset, name, rotation, overwrite)
    }

    /// `addHelper(list, addPiece(tm, list[prev], offset, name, rotation, overwrite))`.
    fn add_to(&self, list: &mut Vec<EndCityPiece>, prev: usize, offset: (i32, i32, i32), name: &str, rotation: Rotation, overwrite: bool) -> usize {
        let p = add_piece(self.tm, &list[prev].clone(), offset, name, rotation, overwrite);
        list.push(p);
        list.len() - 1
    }

    /// `EndCityPieces.recursiveChildren`.
    fn recursive_children(
        &mut self,
        section: Section,
        depth: i32,
        parent: &EndCityPiece,
        pos: Option<BlockPos>,
        pieces: &mut Vec<EndCityPiece>,
        random: &mut WorldgenRandom,
    ) -> bool {
        if depth > MAX_GEN_DEPTH {
            return false;
        }
        let mut children = Vec::new();
        if !self.generate(section, depth, parent, pos, &mut children, random) {
            return false;
        }
        let id = random.next_int();
        for c in &mut children {
            c.t.base.gen_depth = id;
            if let Some(hit) = pieces.iter().find(|p| p.t.base.bbox.intersects(&c.t.base.bbox))
                && hit.t.base.gen_depth != parent.t.base.gen_depth
            {
                return false;
            }
        }
        pieces.extend(children);
        true
    }

    fn generate(
        &mut self,
        section: Section,
        depth: i32,
        parent: &EndCityPiece,
        pos: Option<BlockPos>,
        out: &mut Vec<EndCityPiece>,
        random: &mut WorldgenRandom,
    ) -> bool {
        let rotation = parent.t.rotation;
        match section {
            Section::HouseTower => {
                if depth > MAX_GEN_DEPTH {
                    return false;
                }
                let at = pos.expect("house tower position");
                out.push(add_piece(self.tm, parent, (at.x, at.y, at.z), "base_floor", rotation, true));
                match random.next_int_bounded(3) {
                    0 => {
                        self.add(out, (-1, 4, -1), "base_roof", rotation, true);
                    }
                    1 => {
                        self.add(out, (-1, 0, -1), "second_floor_2", rotation, false);
                        let last = self.add(out, (-1, 8, -1), "second_roof", rotation, false);
                        let p = out[last].clone();
                        self.recursive_children(Section::Tower, depth + 1, &p, None, out, random);
                    }
                    2 => {
                        self.add(out, (-1, 0, -1), "second_floor_2", rotation, false);
                        self.add(out, (-1, 4, -1), "third_floor_2", rotation, false);
                        let last = self.add(out, (-1, 8, -1), "third_roof", rotation, true);
                        let p = out[last].clone();
                        self.recursive_children(Section::Tower, depth + 1, &p, None, out, random);
                    }
                    _ => {}
                }
                true
            }
            Section::Tower => {
                let x = 3 + random.next_int_bounded(2);
                let z = 3 + random.next_int_bounded(2);
                out.push(add_piece(self.tm, parent, (x, -3, z), "tower_base", rotation, true));
                let mut last = self.add(out, (0, 7, 0), "tower_piece", rotation, true);
                let mut bridge_parent = (random.next_int_bounded(3) == 0).then_some(last);
                let floors = 1 + random.next_int_bounded(3);
                for i in 0..floors {
                    last = self.add(out, (0, 4, 0), "tower_piece", rotation, true);
                    if i < floors - 1 && random.next_bool() {
                        bridge_parent = Some(last);
                    }
                }
                if let Some(bp) = bridge_parent {
                    for (r, off) in TOWER_BRIDGES {
                        if random.next_bool() {
                            let b = self.add_to(out, bp, off, "bridge_end", rotation.then(r), true);
                            let p = out[b].clone();
                            self.recursive_children(Section::TowerBridge, depth + 1, &p, None, out, random);
                        }
                    }
                    self.add_to(out, last, (-1, 4, -1), "tower_top", rotation, true);
                } else if depth == 7 {
                    self.add_to(out, last, (-1, 4, -1), "tower_top", rotation, true);
                } else {
                    let p = out[last].clone();
                    return self.recursive_children(Section::FatTower, depth + 1, &p, None, out, random);
                }
                true
            }
            Section::TowerBridge => {
                let n = random.next_int_bounded(4) + 1;
                out.push(add_piece(self.tm, parent, (0, 0, -4), "bridge_piece", rotation, true));
                let mut last = out.len() - 1;
                out[last].t.base.gen_depth = -1;
                let mut y = 0;
                for _ in 0..n {
                    if random.next_bool() {
                        last = self.add_to(out, last, (0, y, -4), "bridge_piece", rotation, true);
                        y = 0;
                    } else {
                        last = if random.next_bool() {
                            self.add_to(out, last, (0, y, -4), "bridge_steep_stairs", rotation, true)
                        } else {
                            self.add_to(out, last, (0, y, -8), "bridge_gentle_stairs", rotation, true)
                        };
                        y = 4;
                    }
                }
                if self.ship_created || random.next_int_bounded(10 - depth) != 0 {
                    let p = out[last].clone();
                    if !self.recursive_children(Section::HouseTower, depth + 1, &p, Some(BlockPos::new(-3, y + 1, -11)), out, random) {
                        return false;
                    }
                } else {
                    let x = -8 + random.next_int_bounded(8);
                    let z = -70 + random.next_int_bounded(10);
                    self.add_to(out, last, (x, y, z), "ship", rotation, true);
                    self.ship_created = true;
                }
                let end = self.add_to(out, last, (4, y, 0), "bridge_end", rotation.then(Rotation::Clockwise180), true);
                out[end].t.base.gen_depth = -1;
                true
            }
            Section::FatTower => {
                out.push(add_piece(self.tm, parent, (-3, 4, -3), "fat_tower_base", rotation, true));
                let mut last = self.add(out, (0, 4, 0), "fat_tower_middle", rotation, true);
                for _ in 0..2 {
                    if random.next_int_bounded(3) == 0 {
                        break;
                    }
                    last = self.add_to(out, last, (0, 8, 0), "fat_tower_middle", rotation, true);
                    for (r, off) in FAT_TOWER_BRIDGES {
                        if random.next_bool() {
                            let b = self.add_to(out, last, off, "bridge_end", rotation.then(r), true);
                            let p = out[b].clone();
                            self.recursive_children(Section::TowerBridge, depth + 1, &p, None, out, random);
                        }
                    }
                }
                self.add_to(out, last, (-2, 8, -2), "fat_tower_top", rotation, true);
                true
            }
        }
    }
}

/// `EndCityPieces.addPiece`: a piece at `previous`'s template position, moved by `offset`
/// turned into `previous`'s frame (`calculateConnectedPosition` with the origin).
fn add_piece(tm: &TemplateManager, previous: &EndCityPiece, offset: (i32, i32, i32), name: &str, rotation: Rotation, overwrite: bool) -> EndCityPiece {
    let mut p = EndCityPiece::new(tm, name, previous.t.position, rotation, overwrite);
    let t = &previous.t;
    let a = transform(BlockPos::new(offset.0, offset.1, offset.2), t.mirror, t.rotation, t.pivot);
    let b = transform(BlockPos::new(0, 0, 0), p.t.mirror, p.t.rotation, p.t.pivot);
    p.shift(a.x - b.x, a.y - b.y, a.z - b.z);
    p
}

/// `EndCityPieces.EndCityPiece`.
#[derive(Clone, Debug)]
pub struct EndCityPiece {
    t: TemplatePiece,
    /// The template name without `end_city/` (as saved).
    short: String,
    /// `OW`: only structure blocks are ignored (else air too).
    overwrite: bool,
}

impl EndCityPiece {
    fn new(tm: &TemplateManager, name: &str, position: BlockPos, rotation: Rotation, overwrite: bool) -> Self {
        let processor = if overwrite { IGNORE_STRUCTURE_BLOCK.clone() } else { IGNORE_STRUCTURE_AND_AIR.clone() };
        let t = TemplatePiece::new(
            "minecraft:ecp",
            tm,
            &format!("minecraft:end_city/{name}"),
            rotation,
            Mirror::None,
            BlockPos::new(0, 0, 0),
            vec![processor],
            LiquidSettings::ApplyWaterlogging,
            position,
        );
        EndCityPiece { t, short: name.to_string(), overwrite }
    }
}

impl Piece for EndCityPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let rotation = self.t.rotation;
        self.t.post_process(self.t.position, r, random, chunk_box, pivot, &mut |metadata, p, r, random| {
            handle_data_marker(metadata, p, r, random, chunk_box, rotation);
        });
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        tag.push(("TPX".into(), Tag::Int(self.t.position.x)));
        tag.push(("TPY".into(), Tag::Int(self.t.position.y)));
        tag.push(("TPZ".into(), Tag::Int(self.t.position.z)));
        tag.push(("Template".into(), Tag::String(self.short.clone())));
        tag.push(("Rot".into(), Tag::String(self.t.rotation.enum_name().into())));
        tag.push(("OW".into(), Tag::Byte(self.overwrite as i8)));
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.base.bbox.shift(dx, dy, dz);
        self.t.position = self.t.position.offset(dx, dy, dz);
    }
}

/// `EndCityPiece.handleDataMarker`.
fn handle_data_marker(metadata: &str, p: BlockPos, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, rotation: Rotation) {
    if metadata.starts_with("Chest") {
        let chest = p.below();
        if chunk_box.is_inside(chest) {
            super::set_loot(r, random, chest, "minecraft:chests/end_city_treasure", false);
        }
        return;
    }
    // `Level.isInSpawnableBounds`: always true inside a generated End chunk.
    if !chunk_box.is_inside(p) {
        return;
    }
    if metadata.starts_with("Sentry") {
        // Vanilla's yaw is `Math.random() * 2pi` from `LivingEntity`'s constructor (not
        // reproducible); 0 here.
        let (x, y, z) = (p.x as f64 + 0.5, p.y as f64, p.z as f64 + 0.5);
        let extra = vec![
            ("AttachFace".to_string(), Tag::Byte(0)),
            ("Peek".to_string(), Tag::Byte(0)),
            ("Color".to_string(), Tag::Byte(16)),
        ];
        r.add_entity(x, z, crate::feature::entity_tag("minecraft:shulker", [x, y, z], 0.0, extra));
    } else if metadata.starts_with("Elytra") {
        let facing = rotation.rotate(Dir::South);
        let (dx, _, dz) = facing.offset();
        let (x, y, z) = (p.x as f64 + 0.5 - dx as f64 * 0.46875, p.y as f64 + 0.5, p.z as f64 + 0.5 - dz as f64 * 0.46875);
        // Keys in the order vanilla's `CompoundTag` writes them.
        let item = Tag::Compound(vec![("count".into(), Tag::Int(1)), ("id".into(), Tag::String("minecraft:elytra".into()))]);
        let extra = vec![
            ("Facing".to_string(), Tag::Byte(facing as i8)),
            ("Item".to_string(), item),
            ("ItemRotation".to_string(), Tag::Byte(0)),
            ("ItemDropChance".to_string(), Tag::Float(1.0)),
            ("block_pos".to_string(), Tag::IntArray(vec![p.x, p.y, p.z])),
        ];
        let yaw = (facing.index_2d() * 90) as f32;
        r.add_entity(x, z, crate::feature::entity_tag("minecraft:item_frame", [x, y, z], yaw, extra));
    }
}
