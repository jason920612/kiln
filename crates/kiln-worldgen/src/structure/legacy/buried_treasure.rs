//! Buried treasure (`BuriedTreasureStructure`, `BuriedTreasurePieces`).

use super::st;
use crate::block_facts::Dir;
use crate::blocks::{is_air, is_lava, is_water};
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext, create_chest_at};
use crate::structure::{GenCtx, Stub};
use kiln_data::blocks_types::block_of;
use kiln_proto::nbt::Tag;

pub struct BuriedTreasure;

impl Kind for BuriedTreasure {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let pos = ctx.on_top_of_chunk_center(Heightmap::OceanFloorWg)?;
        let (x, z) = ((ctx.chunk.0 << 4) + 9, (ctx.chunk.1 << 4) + 9);
        Some(Stub {
            pos,
            build: Box::new(move |_, pieces| {
                let b = BoundingBox::new(x, 90, z, x, 90, z);
                pieces.push(Box::new(TreasurePiece { base: PieceBase::new("minecraft:btp", 0, b) }));
            }),
        })
    }
}

/// `BuriedTreasurePieces.BuriedTreasurePiece`.
#[derive(Debug)]
struct TreasurePiece {
    base: PieceBase,
}

fn liquid(s: u16) -> bool {
    is_water(s) || is_lava(s)
}

impl Piece for TreasurePiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn save_extra(&self, _tag: &mut Vec<(String, Tag)>) {}

    /// Sinks to the first stone-like block below the ocean floor, seals the chest's sides and
    /// places it (vanilla also moves its own box there, which only matters after placement).
    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), _pivot: BlockPos) {
        let (x, z) = (self.base.bbox.min_x, self.base.bbox.min_z);
        let mut p = BlockPos::new(x, r.height_at(Heightmap::OceanFloorWg, x, z), z);
        while p.y > r.min_y() {
            let here = r.get(p);
            let below = r.get(p.below());
            let name = block_of(below).name;
            if matches!(name, "minecraft:sandstone" | "minecraft:stone" | "minecraft:andesite" | "minecraft:granite" | "minecraft:diorite") {
                let fill = if is_air(here) || liquid(here) { st("minecraft:sand") } else { here };
                for d in Dir::ALL {
                    let n = p.relative(d);
                    let s = r.get(n);
                    if is_air(s) || liquid(s) {
                        let under = r.get(n.below());
                        if (is_air(under) || liquid(under)) && d != Dir::Up {
                            r.set(n, below, 3);
                        } else {
                            r.set(n, fill, 3);
                        }
                    }
                }
                create_chest_at(r, chunk_box, random, p, "minecraft:chests/buried_treasure", None);
                return;
            }
            p = p.below();
        }
    }
}
