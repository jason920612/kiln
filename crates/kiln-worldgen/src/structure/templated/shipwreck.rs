//! Shipwrecks (`ShipwreckStructure`, `ShipwreckPieces`).

use super::TemplatePiece;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::processor::IGNORE_STRUCTURE_AND_AIR;
use crate::structure::template::LiquidSettings;
use crate::structure::transform::{Mirror, Rotation};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::sync::Mutex;

const BEACHED: [&str; 11] = [
    "with_mast",
    "sideways_full",
    "sideways_fronthalf",
    "sideways_backhalf",
    "rightsideup_full",
    "rightsideup_fronthalf",
    "rightsideup_backhalf",
    "with_mast_degraded",
    "rightsideup_full_degraded",
    "rightsideup_fronthalf_degraded",
    "rightsideup_backhalf_degraded",
];

const OCEAN: [&str; 20] = [
    "with_mast",
    "upsidedown_full",
    "upsidedown_fronthalf",
    "upsidedown_backhalf",
    "sideways_full",
    "sideways_fronthalf",
    "sideways_backhalf",
    "rightsideup_full",
    "rightsideup_fronthalf",
    "rightsideup_backhalf",
    "with_mast_degraded",
    "upsidedown_full_degraded",
    "upsidedown_fronthalf_degraded",
    "upsidedown_backhalf_degraded",
    "sideways_full_degraded",
    "sideways_fronthalf_degraded",
    "sideways_backhalf_degraded",
    "rightsideup_full_degraded",
    "rightsideup_fronthalf_degraded",
    "rightsideup_backhalf_degraded",
];

/// `ShipwreckStructure`.
pub struct Shipwreck {
    pub beached: bool,
}

impl Kind for Shipwreck {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let map = if self.beached { Heightmap::WorldSurfaceWg } else { Heightmap::OceanFloorWg };
        let pos = ctx.on_top_of_chunk_center(map)?;
        let beached = self.beached;
        Some(Stub {
            pos,
            build: Box::new(move |ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
                let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
                let p = BlockPos::new(ctx.chunk.0 << 4, 90, ctx.chunk.1 << 4);
                let names: &[&str] = if beached { &BEACHED } else { &OCEAN };
                let name = format!("minecraft:shipwreck/{}", names[ctx.random.next_int_bounded(names.len() as i32) as usize]);
                let tm = ctx.structures.templates.clone();
                let mut piece = ShipwreckPiece {
                    t: TemplatePiece::new(
                        "minecraft:shipwreck",
                        &tm,
                        &name,
                        rotation,
                        Mirror::None,
                        BlockPos::new(4, 0, 15),
                        vec![IGNORE_STRUCTURE_AND_AIR.clone()],
                        LiquidSettings::ApplyWaterlogging,
                        p,
                    ),
                    beached,
                    height_adjusted: false,
                    placed: Mutex::new(None),
                };
                if piece.too_big() {
                    let b = piece.t.base.bbox;
                    let y = if beached {
                        // Vanilla passes the span where `getLowestY` takes z and the other way round.
                        let lowest = ctx.lowest_y_at(b.min_x, b.x_span(), b.min_z, b.z_span());
                        piece.beached_y(lowest, &mut ctx.random)
                    } else {
                        ctx.mean_first_occupied_height(b.min_x, b.x_span(), b.min_z, b.z_span())
                    };
                    piece.adjust(y);
                }
                out.push(Box::new(piece));
            }),
        })
    }
}

/// `ShipwreckPieces.ShipwreckPiece`.
#[derive(Debug)]
pub struct ShipwreckPiece {
    t: TemplatePiece,
    beached: bool,
    height_adjusted: bool,
    /// The height chosen by the first placement (`adjustPositionHeight` during `postProcess`
    /// changes the shared piece).
    placed: Mutex<Option<i32>>,
}

impl ShipwreckPiece {
    /// `isTooBigToFitInWorldGenRegion`.
    fn too_big(&self) -> bool {
        self.t.template.size[0] > 32 || self.t.template.size[1] > 32
    }

    /// `calculateBeachedPosition`.
    fn beached_y(&self, y: i32, random: &mut WorldgenRandom) -> i32 {
        y - self.t.template.size[1] / 2 - random.next_int_bounded(3)
    }

    /// `adjustPositionHeight`.
    fn adjust(&mut self, y: i32) {
        self.height_adjusted = true;
        self.t.position = BlockPos::new(self.t.position.x, y, self.t.position.z);
        self.t.base.bbox = self.t.bbox_at(self.t.position);
    }
}

impl Piece for ShipwreckPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let mut pos = self.t.position;
        if !self.height_adjusted && !self.too_big() {
            let mut placed = self.placed.lock().unwrap();
            let y = match *placed {
                Some(y) => y,
                None => {
                    let size = self.t.template.size;
                    let map = if self.beached { Heightmap::WorldSurfaceWg } else { Heightmap::OceanFloorWg };
                    let (mut lowest, mut sum) = (r.max_y() + 1, 0);
                    let area = size[0] * size[2];
                    if area == 0 {
                        sum = r.height_at(map, pos.x, pos.z);
                    } else {
                        for z in pos.z..pos.z + size[2] {
                            for x in pos.x..pos.x + size[0] {
                                let h = r.height_at(map, x, z);
                                sum += h;
                                lowest = lowest.min(h);
                            }
                        }
                        sum /= area;
                    }
                    let y = if self.beached { self.beached_y(lowest, random) } else { sum };
                    *placed = Some(y);
                    y
                }
            };
            pos = BlockPos::new(pos.x, y, pos.z);
        }
        self.t.post_process(pos, r, random, chunk_box, pivot, &mut |metadata, p, r, random| {
            let table = match metadata {
                "map_chest" => "minecraft:chests/shipwreck_map",
                "treasure_chest" => "minecraft:chests/shipwreck_treasure",
                "supply_chest" => "minecraft:chests/shipwreck_supply",
                _ => return,
            };
            super::set_loot(r, random, p.below(), table, false);
        });
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        self.t.save_template(tag);
        tag.push(("isBeached".into(), Tag::Byte(self.beached as i8)));
        tag.push(("Rot".into(), Tag::String(self.t.rotation.enum_name().into())));
        tag.push(("height_adjusted".into(), Tag::Byte(self.height_adjusted as i8)));
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.base.bbox.shift(dx, dy, dz);
        self.t.position = self.t.position.offset(dx, dy, dz);
    }
}
