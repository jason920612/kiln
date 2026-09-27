//! Ocean ruins (`OceanRuinStructure`, `OceanRuinPieces`).

use super::TemplatePiece;
use crate::blocks::{block, state};
use crate::pos::BlockPos;
use crate::predicate::RuleTest;
use crate::proto::Heightmap;
use crate::providers::IntProvider;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::processor::{IGNORE_STRUCTURE_AND_AIR, Processor, ProcessorRule};
use crate::structure::template::{LiquidSettings, TemplateManager, transform};
use crate::structure::transform::{Mirror, Rotation};
use crate::structure::{GenCtx, Stub};
use crate::Error;
use crate::json::Json;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// `OceanRuinStructure.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temp {
    Warm,
    Cold,
}

/// `OceanRuinStructure`.
pub struct OceanRuin {
    temp: Temp,
    large_probability: f32,
    cluster_probability: f32,
    /// `COLD_SUSPICIOUS_BLOCK_PROCESSOR` / `WARM_SUSPICIOUS_BLOCK_PROCESSOR`.
    archaeology: Processor,
}

/// `OceanRuinPieces.archyRuleProcessor(from, to, lootTable)`.
fn archy(from: &str, to: &str, table: &str) -> Result<Processor, Error> {
    let mut set = BlockSet::empty();
    set.insert_block(from)?;
    Ok(Processor::Capped {
        delegate: Box::new(Processor::Rule(vec![ProcessorRule::new(
            RuleTest::BlockMatch(Arc::new(set)),
            RuleTest::AlwaysTrue,
            block(to)?.default,
            Some(table.to_string()),
        )])),
        limit: IntProvider::Constant(5),
    })
}

pub fn parse(json: &Json, _l: &Loader) -> Result<OceanRuin, Error> {
    let temp = match json.get("biome_temp").and_then(Json::as_str) {
        Some("cold") => Temp::Cold,
        Some("warm") => Temp::Warm,
        t => return Err(Error::Invalid(format!("bad biome_temp {t:?}"))),
    };
    let f = |k: &str| json.get(k).and_then(Json::as_f32).ok_or_else(|| Error::Invalid(format!("ocean ruin without {k}")));
    let archaeology = match temp {
        Temp::Cold => archy("minecraft:gravel", "minecraft:suspicious_gravel", "minecraft:archaeology/ocean_ruin_cold")?,
        Temp::Warm => archy("minecraft:sand", "minecraft:suspicious_sand", "minecraft:archaeology/ocean_ruin_warm")?,
    };
    Ok(OceanRuin { temp, large_probability: f("large_probability")?, cluster_probability: f("cluster_probability")?, archaeology })
}

const WARM: [&str; 8] = ["warm_1", "warm_2", "warm_3", "warm_4", "warm_5", "warm_6", "warm_7", "warm_8"];
const BRICK: [&str; 8] = ["brick_1", "brick_2", "brick_3", "brick_4", "brick_5", "brick_6", "brick_7", "brick_8"];
const CRACKED: [&str; 8] = ["cracked_1", "cracked_2", "cracked_3", "cracked_4", "cracked_5", "cracked_6", "cracked_7", "cracked_8"];
const MOSSY: [&str; 8] = ["mossy_1", "mossy_2", "mossy_3", "mossy_4", "mossy_5", "mossy_6", "mossy_7", "mossy_8"];
const BIG_BRICK: [&str; 4] = ["big_brick_1", "big_brick_2", "big_brick_3", "big_brick_8"];
const BIG_MOSSY: [&str; 4] = ["big_mossy_1", "big_mossy_2", "big_mossy_3", "big_mossy_8"];
const BIG_CRACKED: [&str; 4] = ["big_cracked_1", "big_cracked_2", "big_cracked_3", "big_cracked_8"];
const BIG_WARM: [&str; 4] = ["big_warm_4", "big_warm_5", "big_warm_6", "big_warm_7"];

/// `Mth.nextInt(random, min, max)`.
fn next_int(random: &mut WorldgenRandom, min: i32, max: i32) -> i32 {
    if min >= max { min } else { random.next_int_bounded(max - min + 1) + min }
}

impl Kind for OceanRuin {
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let pos = ctx.on_top_of_chunk_center(Heightmap::OceanFloorWg)?;
        Some(Stub {
            pos,
            build: Box::new(move |ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
                let p = BlockPos::new(ctx.chunk.0 << 4, 90, ctx.chunk.1 << 4);
                let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
                let tm = ctx.structures.templates.clone();
                self.add_pieces(&tm, p, rotation, out, &mut ctx.random);
            }),
        })
    }
}

impl OceanRuin {
    /// `OceanRuinPieces.addPieces`.
    fn add_pieces(&self, tm: &TemplateManager, p: BlockPos, rotation: Rotation, out: &mut Vec<Box<dyn Piece>>, random: &mut WorldgenRandom) {
        let large = random.next_float() <= self.large_probability;
        let integrity = if large { 0.9 } else { 0.8 };
        self.add_piece(tm, p, rotation, out, random, large, integrity);
        if large && random.next_float() <= self.cluster_probability {
            self.add_cluster(tm, random, rotation, p, out);
        }
    }

    /// `OceanRuinPieces.addClusterRuins`.
    fn add_cluster(&self, tm: &TemplateManager, random: &mut WorldgenRandom, rotation: Rotation, p: BlockPos, out: &mut Vec<Box<dyn Piece>>) {
        let origin = BlockPos::new(p.x, 90, p.z);
        let corner = transform(BlockPos::new(15, 0, 15), Mirror::None, rotation, BlockPos::new(0, 0, 0)).offset(origin.x, origin.y, origin.z);
        let big = BoundingBox::from_corners(origin, corner);
        let start = BlockPos::new(origin.x.min(corner.x), origin.y, origin.z.min(corner.z));
        let mut positions = all_positions(random, start);
        let count = next_int(random, 4, 8);
        for _ in 0..count {
            if positions.is_empty() {
                continue;
            }
            let i = random.next_int_bounded(positions.len() as i32) as usize;
            let at = positions.remove(i);
            let r = Rotation::ALL[random.next_int_bounded(4) as usize];
            let far = transform(BlockPos::new(5, 0, 6), Mirror::None, r, BlockPos::new(0, 0, 0)).offset(at.x, at.y, at.z);
            if BoundingBox::from_corners(at, far).intersects(&big) {
                continue;
            }
            self.add_piece(tm, at, r, out, random, false, 0.8);
        }
    }

    /// `OceanRuinPieces.addPiece`.
    #[allow(clippy::too_many_arguments)]
    fn add_piece(
        &self,
        tm: &TemplateManager,
        p: BlockPos,
        rotation: Rotation,
        out: &mut Vec<Box<dyn Piece>>,
        random: &mut WorldgenRandom,
        large: bool,
        integrity: f32,
    ) {
        let name = |n: &str| format!("minecraft:underwater_ruin/{n}");
        match self.temp {
            Temp::Warm => {
                let n = if large { BIG_WARM[random.next_int_bounded(4) as usize] } else { WARM[random.next_int_bounded(8) as usize] };
                out.push(Box::new(OceanRuinPiece::new(tm, &name(n), p, rotation, integrity, self, large)));
            }
            Temp::Cold => {
                let (brick, cracked, mossy): (&[&str], &[&str], &[&str]) =
                    if large { (&BIG_BRICK, &BIG_CRACKED, &BIG_MOSSY) } else { (&BRICK, &CRACKED, &MOSSY) };
                let i = random.next_int_bounded(brick.len() as i32) as usize;
                out.push(Box::new(OceanRuinPiece::new(tm, &name(brick[i]), p, rotation, integrity, self, large)));
                out.push(Box::new(OceanRuinPiece::new(tm, &name(cracked[i]), p, rotation, 0.7, self, large)));
                out.push(Box::new(OceanRuinPiece::new(tm, &name(mossy[i]), p, rotation, 0.5, self, large)));
            }
        }
    }
}

/// `OceanRuinPieces.allPositions`.
fn all_positions(random: &mut WorldgenRandom, p: BlockPos) -> Vec<BlockPos> {
    let mut out = Vec::with_capacity(8);
    let mut add = |random: &mut WorldgenRandom, x0: i32, xr: (i32, i32), z0: i32, zr: (i32, i32)| {
        let x = x0 + next_int(random, xr.0, xr.1);
        let z = z0 + next_int(random, zr.0, zr.1);
        out.push(p.offset(x, 0, z));
    };
    add(random, -16, (1, 8), 16, (1, 7));
    add(random, -16, (1, 8), 0, (1, 7));
    add(random, -16, (1, 8), -16, (4, 8));
    add(random, 0, (1, 7), 16, (1, 7));
    add(random, 0, (1, 7), -16, (4, 6));
    add(random, 16, (1, 7), 16, (3, 8));
    add(random, 16, (1, 7), 0, (1, 7));
    add(random, 16, (1, 7), -16, (4, 8));
    out
}

/// `OceanRuinPieces.OceanRuinPiece`.
#[derive(Debug)]
pub struct OceanRuinPiece {
    t: TemplatePiece,
    temp: Temp,
    integrity: f32,
    large: bool,
}

impl OceanRuinPiece {
    fn new(tm: &TemplateManager, name: &str, p: BlockPos, rotation: Rotation, integrity: f32, s: &OceanRuin, large: bool) -> Self {
        let processors = vec![Processor::BlockRot { rottable: None, integrity }, IGNORE_STRUCTURE_AND_AIR.clone(), s.archaeology.clone()];
        let t = TemplatePiece::new("minecraft:orp", tm, name, rotation, Mirror::None, BlockPos::new(0, 0, 0), processors, LiquidSettings::ApplyWaterlogging, p);
        OceanRuinPiece { t, temp: s.temp, integrity, large }
    }

    /// `OceanRuinPiece.getHeight(templatePos, level, corner)`.
    fn height(&self, r: &mut Region, tp: BlockPos, corner: BlockPos) -> i32 {
        let mut y = tp.y;
        let mut lowest = 512;
        let top = y - 1;
        let mut low_count = 0;
        for z in tp.z.min(corner.z)..=tp.z.max(corner.z) {
            for x in tp.x.min(corner.x)..=tp.x.max(corner.x) {
                let mut cy = tp.y - 1;
                loop {
                    let s = r.get(BlockPos::new(x, cy, z));
                    let submerged = crate::blocks::is_air(s)
                        || crate::block_facts::fluid(s).is_water()
                        || crate::blocks::is_block(s, "minecraft:ice")
                        || crate::blocks::is_block(s, "minecraft:packed_ice")
                        || crate::blocks::is_block(s, "minecraft:blue_ice")
                        || crate::blocks::is_block(s, "minecraft:frosted_ice");
                    if !(submerged && cy > r.min_y() + 1) {
                        break;
                    }
                    cy -= 1;
                }
                lowest = lowest.min(cy);
                if cy < top - 2 {
                    low_count += 1;
                }
            }
        }
        let dx = (tp.x - corner.x).abs();
        if top - lowest > 2 && low_count > dx - 2 {
            y = lowest + 1;
        }
        y
    }
}

impl Piece for OceanRuinPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let t = &self.t;
        let h = r.height_at(Heightmap::OceanFloorWg, t.position.x, t.position.z);
        let tp = BlockPos::new(t.position.x, h, t.position.z);
        let size = t.template.size;
        let c = transform(BlockPos::new(size[0] - 1, 0, size[2] - 1), Mirror::None, t.rotation, BlockPos::new(0, 0, 0));
        let corner = tp.offset(c.x, c.y, c.z);
        let pos = BlockPos::new(tp.x, self.height(r, tp, corner), tp.z);
        let large = self.large;
        t.post_process(pos, r, random, chunk_box, pivot, &mut |metadata, p, r, random| match metadata {
            "chest" => {
                let water = crate::block_facts::fluid(r.get(p)).is_water();
                r.set(p, crate::blocks::with_prop(state::CHEST, "waterlogged", if water { "true" } else { "false" }), 2);
                let table = if large { "minecraft:chests/underwater_ruin_big" } else { "minecraft:chests/underwater_ruin_small" };
                super::set_loot(r, random, p, table, true);
            }
            "drowned" => {
                let s = if p.y > r.sea_level() { state::AIR } else { state::WATER };
                r.set(p, s, 2);
            }
            _ => {}
        });
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        self.t.save_template(tag);
        tag.push(("Rot".into(), Tag::String(self.t.rotation.enum_name().into())));
        tag.push(("Integrity".into(), Tag::Float(self.integrity)));
        tag.push(("BiomeType".into(), Tag::String(if self.temp == Temp::Warm { "WARM" } else { "COLD" }.into())));
        tag.push(("IsLarge".into(), Tag::Byte(self.large as i8)));
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.base.bbox.shift(dx, dy, dz);
        self.t.position = self.t.position.offset(dx, dy, dz);
    }
}
