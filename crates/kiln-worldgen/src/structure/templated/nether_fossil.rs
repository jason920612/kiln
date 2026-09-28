//! Nether fossils (`NetherFossilStructure`, `NetherFossilPieces`).

use super::TemplatePiece;
use crate::Error;
use crate::block_facts::{Dir, Support, is_face_sturdy};
use crate::blocks::{is_air, is_block};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::providers::{GenContext, HeightProvider};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::processor::IGNORE_STRUCTURE_AND_AIR;
use crate::structure::template::LiquidSettings;
use crate::structure::transform::{Mirror, Rotation, rotate};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;

/// `NetherFossilStructure`.
pub struct NetherFossil {
    height: HeightProvider,
}

pub fn parse(json: &Json) -> Result<NetherFossil, Error> {
    let height = json.get("height").ok_or_else(|| Error::Invalid("nether fossil without height".into()))?;
    Ok(NetherFossil { height: HeightProvider::parse(height)? })
}

impl Kind for NetherFossil {
    /// `findGenerationPoint`: a random column of the chunk, from a random height down to the
    /// first air above soul sand or a sturdy top face (above sea level).
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let x = (ctx.chunk.0 << 4) + ctx.random.next_int_bounded(16);
        let z = (ctx.chunk.1 << 4) + ctx.random.next_int_bounded(16);
        let sea = ctx.generator.sea_level;
        let g = GenContext { min_y: ctx.generator.gen_min_y, height: ctx.generator.gen_height, sea_level: sea };
        let mut y = self.height.sample(&mut ctx.random, g);
        let column = ctx.base_column(x, z);
        let min_y = ctx.generator.gen_min_y;
        let at = |y: i32| usize::try_from(y - min_y).ok().and_then(|i| column.get(i).copied()).unwrap_or(crate::blocks::state::AIR);
        while y > sea {
            let above = at(y);
            y -= 1;
            let below = at(y);
            if is_air(above) && (is_block(below, "minecraft:soul_sand") || is_face_sturdy(below, Dir::Up, Support::Full)) {
                break;
            }
        }
        if y <= sea {
            return None;
        }
        let pos = BlockPos::new(x, y, z);
        Some(Stub {
            pos,
            build: Box::new(move |ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
                let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
                let name = FOSSILS[ctx.random.next_int_bounded(FOSSILS.len() as i32) as usize];
                let tm = ctx.structures.templates.clone();
                out.push(Box::new(FossilPiece(TemplatePiece::new(
                    "minecraft:nefos",
                    &tm,
                    name,
                    rotation,
                    Mirror::None,
                    BlockPos::new(0, 0, 0),
                    vec![IGNORE_STRUCTURE_AND_AIR.clone()],
                    LiquidSettings::ApplyWaterlogging,
                    pos,
                ))));
            }),
        })
    }
}

/// `NetherFossilPieces.FOSSILS`.
const FOSSILS: [&str; 14] = [
    "minecraft:nether_fossils/fossil_1",
    "minecraft:nether_fossils/fossil_2",
    "minecraft:nether_fossils/fossil_3",
    "minecraft:nether_fossils/fossil_4",
    "minecraft:nether_fossils/fossil_5",
    "minecraft:nether_fossils/fossil_6",
    "minecraft:nether_fossils/fossil_7",
    "minecraft:nether_fossils/fossil_8",
    "minecraft:nether_fossils/fossil_9",
    "minecraft:nether_fossils/fossil_10",
    "minecraft:nether_fossils/fossil_11",
    "minecraft:nether_fossils/fossil_12",
    "minecraft:nether_fossils/fossil_13",
    "minecraft:nether_fossils/fossil_14",
];

/// `NetherFossilPieces.NetherFossilPiece`.
#[derive(Debug)]
pub struct FossilPiece(TemplatePiece);

impl Piece for FossilPiece {
    fn base(&self) -> &PieceBase {
        &self.0.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.0.base
    }

    /// `postProcess`: the chunk box grows to the whole template (vanilla encapsulates the box
    /// it is given), then the template and maybe a dried ghast.
    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let t = &self.0;
        let template_box = t.bbox_at(t.position);
        let mut area = *chunk_box;
        area.encapsulate(&template_box);
        t.post_process(t.position, r, random, &area, pivot, &mut |_, _, _, _| {});
        // `placeDriedGhast`: a positional LCG from the level seed at the template's center.
        let c = template_box.center();
        let mut rnd = LegacyRandom::new(r.seed()).fork_positional().at(c.x, c.y, c.z);
        if rnd.next_float() < 0.5 {
            let x = template_box.min_x + rnd.next_int_bounded(template_box.x_span());
            let y = template_box.min_y;
            let z = template_box.min_z + rnd.next_int_bounded(template_box.z_span());
            let p = BlockPos::new(x, y, z);
            if is_air(r.get(p)) && area.is_inside(p) {
                let ghast = crate::blocks::block("minecraft:dried_ghast").expect("dried ghast").default;
                let rotation = Rotation::ALL[rnd.next_int_bounded(4) as usize];
                r.set(p, rotate(ghast, rotation), 2);
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
