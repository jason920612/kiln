//! `PoolElementStructurePiece` and `JigsawJunction`.

use super::pool::{PoolElement, Projection};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::template::LiquidSettings;
use crate::structure::transform::Rotation;
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// `JigsawJunction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Junction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
    pub delta_y: i32,
    pub dest_projection: Projection,
}

impl Junction {
    fn save(&self) -> Tag {
        Tag::Compound(vec![
            ("source_x".into(), Tag::Int(self.source_x)),
            ("source_ground_y".into(), Tag::Int(self.source_ground_y)),
            ("source_z".into(), Tag::Int(self.source_z)),
            ("delta_y".into(), Tag::Int(self.delta_y)),
            ("dest_proj".into(), Tag::String(self.dest_projection.name().into())),
        ])
    }
}

/// `PoolElementStructurePiece`.
#[derive(Debug)]
pub struct PoolElementPiece {
    pub base: PieceBase,
    pub element: Arc<PoolElement>,
    pub position: BlockPos,
    pub ground_level_delta: i32,
    pub rotation: Rotation,
    pub junctions: Vec<Junction>,
    pub liquid: LiquidSettings,
}

impl PoolElementPiece {
    pub fn new(element: Arc<PoolElement>, position: BlockPos, ground_level_delta: i32, rotation: Rotation, bbox: BoundingBox, liquid: LiquidSettings) -> Self {
        Self { base: PieceBase::new("minecraft:jigsaw", 0, bbox), element, position, ground_level_delta, rotation, junctions: Vec::new(), liquid }
    }

    /// `move`.
    pub fn shift_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.shift(dx, dy, dz);
        self.position = self.position.offset(dx, dy, dz);
    }
}

impl Piece for PoolElementPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn place(&self, cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        self.element.place(cx, r, self.position, pivot, self.rotation, chunk_box, random, self.liquid, false);
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        tag.push(("PosX".into(), Tag::Int(self.position.x)));
        tag.push(("PosY".into(), Tag::Int(self.position.y)));
        tag.push(("PosZ".into(), Tag::Int(self.position.z)));
        tag.push(("ground_level_delta".into(), Tag::Int(self.ground_level_delta)));
        tag.push(("pool_element".into(), self.element.nbt.clone()));
        tag.push(("rotation".into(), Tag::String(self.rotation.enum_name().into())));
        tag.push(("junctions".into(), Tag::List(self.junctions.iter().map(Junction::save).collect())));
        if self.liquid != LiquidSettings::ApplyWaterlogging {
            tag.push(("liquid_settings".into(), Tag::String(self.liquid.name().into())));
        }
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.shift_by(dx, dy, dz);
    }

    fn as_pool_element(&self) -> Option<&PoolElementPiece> {
        Some(self)
    }
}
