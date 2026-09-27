//! Igloos (`IglooStructure`, `IglooPieces`).

use super::TemplatePiece;
use crate::blocks::{is_air, is_block, state};
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::processor::IGNORE_STRUCTURE_BLOCK;
use crate::structure::template::{LiquidSettings, TemplateManager};
use crate::structure::transform::{Mirror, Rotation};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

const TOP: &str = "minecraft:igloo/top";
const MIDDLE: &str = "minecraft:igloo/middle";
const BOTTOM: &str = "minecraft:igloo/bottom";

/// `IglooPieces.PIVOTS` and `OFFSETS`.
fn pivot_offset(name: &str) -> (BlockPos, BlockPos) {
    match name {
        TOP => (BlockPos::new(3, 5, 5), BlockPos::new(0, 0, 0)),
        MIDDLE => (BlockPos::new(1, 3, 1), BlockPos::new(2, -3, 4)),
        _ => (BlockPos::new(3, 6, 7), BlockPos::new(0, -3, -2)),
    }
}

/// `IglooStructure`.
pub struct Igloo;

impl Kind for Igloo {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let pos = ctx.on_top_of_chunk_center(Heightmap::WorldSurfaceWg)?;
        Some(Stub {
            pos,
            build: Box::new(|ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
                let p = BlockPos::new(ctx.chunk.0 << 4, 90, ctx.chunk.1 << 4);
                let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
                let tm = ctx.structures.templates.clone();
                add_pieces(&tm, p, rotation, out, &mut ctx.random);
            }),
        })
    }
}

/// `IglooPieces.addPieces`.
fn add_pieces(tm: &TemplateManager, p: BlockPos, rotation: Rotation, out: &mut Vec<Box<dyn Piece>>, random: &mut WorldgenRandom) {
    if random.next_double() < 0.5 {
        let depth = random.next_int_bounded(8) + 4;
        out.push(Box::new(IglooPiece::new(tm, BOTTOM, p, rotation, depth * 3)));
        for i in 0..depth - 1 {
            out.push(Box::new(IglooPiece::new(tm, MIDDLE, p, rotation, i * 3)));
        }
    }
    out.push(Box::new(IglooPiece::new(tm, TOP, p, rotation, 0)));
}

/// `IglooPieces.IglooPiece`.
#[derive(Debug)]
pub struct IglooPiece(TemplatePiece);

impl IglooPiece {
    fn new(tm: &TemplateManager, name: &str, p: BlockPos, rotation: Rotation, down: i32) -> Self {
        let (pivot, offset) = pivot_offset(name);
        let position = p.offset(offset.x, offset.y - down, offset.z);
        IglooPiece(TemplatePiece::new(
            "minecraft:iglu",
            tm,
            name,
            rotation,
            Mirror::None,
            pivot,
            vec![IGNORE_STRUCTURE_BLOCK.clone()],
            LiquidSettings::IgnoreWaterlogging,
            position,
        ))
    }
}

impl Piece for IglooPiece {
    fn base(&self) -> &PieceBase {
        &self.0.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.0.base
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let t = &self.0;
        let (_, offset) = pivot_offset(&t.name);
        let rel = t.relative(BlockPos::new(3 - offset.x, 0, -offset.z));
        let probe = t.position.offset(rel.x, rel.y, rel.z);
        let h = r.height_at(Heightmap::WorldSurfaceWg, probe.x, probe.z);
        let pos = t.position.offset(0, h - 90 - 1, 0);
        t.post_process(pos, r, random, chunk_box, pivot, &mut |metadata, p, r, random| {
            if metadata != "chest" {
                return;
            }
            r.set(p, state::AIR, 3);
            super::set_loot(r, random, p.below(), "minecraft:chests/igloo_chest", true);
        });
        if t.name == TOP {
            let rel = t.relative(BlockPos::new(3, 0, 5));
            let at = pos.offset(rel.x, rel.y, rel.z);
            let below = r.get(at.below());
            if !is_air(below) && !is_block(below, "minecraft:ladder") {
                r.set(at, state::SNOW_BLOCK, 3);
            }
        }
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        self.0.save_template(tag);
        tag.push(("Rot".into(), Tag::String(self.0.rotation.enum_name().into())));
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.0.base.bbox.shift(dx, dy, dz);
        self.0.position = self.0.position.offset(dx, dy, dz);
    }
}
