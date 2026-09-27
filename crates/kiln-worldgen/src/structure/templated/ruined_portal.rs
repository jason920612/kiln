//! Ruined portals (`RuinedPortalStructure`, `RuinedPortalPiece`).

use super::TemplatePiece;
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{block, is_air, is_block, is_lava, state, with_prop};
use crate::json::Json;
use crate::pos::BlockPos;
use crate::predicate::RuleTest;
use crate::proto::{Heightmap, heightmap_bit, heightmap_flags};
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::{BlockSet, Loader};
use crate::structure::bbox::BoundingBox;
use crate::structure::kinds::Kind;
use crate::structure::piece::{Piece, PieceBase, PlaceContext};
use crate::structure::processor::{IGNORE_STRUCTURE_AND_AIR, IGNORE_STRUCTURE_BLOCK, Processor, ProcessorRule};
use crate::structure::template::{LiquidSettings, bounding_box};
use crate::structure::transform::{Mirror, Rotation};
use crate::structure::{GenCtx, Stub};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// `RuinedPortalPiece.VerticalPlacement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vertical {
    OnLandSurface,
    PartlyBuried,
    OnOceanFloor,
    InMountain,
    Underground,
    InNether,
}

impl Vertical {
    const NAMES: [&str; 6] = ["on_land_surface", "partly_buried", "on_ocean_floor", "in_mountain", "underground", "in_nether"];
    const ALL: [Vertical; 6] =
        [Vertical::OnLandSurface, Vertical::PartlyBuried, Vertical::OnOceanFloor, Vertical::InMountain, Vertical::Underground, Vertical::InNether];

    fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    /// `RuinedPortalPiece.getHeightMapType`.
    fn heightmap(self) -> Heightmap {
        if self == Vertical::OnOceanFloor { Heightmap::OceanFloorWg } else { Heightmap::WorldSurfaceWg }
    }
}

/// `RuinedPortalStructure.Setup`.
#[derive(Clone, Debug)]
struct Setup {
    placement: Vertical,
    air_pocket_probability: f32,
    mossiness: f32,
    overgrown: bool,
    vines: bool,
    can_be_cold: bool,
    replace_with_blackstone: bool,
    weight: f32,
}

/// `RuinedPortalStructure`.
pub struct RuinedPortal {
    setups: Vec<Setup>,
    /// `#features_cannot_replace`, for the protected-blocks processor and netherrack spread.
    cannot_replace: Arc<BlockSet>,
    stairs: Arc<BlockSet>,
    slabs: Arc<BlockSet>,
    walls: Arc<BlockSet>,
}

pub fn parse(json: &Json, l: &Loader) -> Result<RuinedPortal, Error> {
    let setups = json
        .get("setups")
        .and_then(Json::as_array)
        .ok_or_else(|| Error::Invalid("ruined portal without setups".into()))?
        .iter()
        .map(|s| {
            let f = |k: &str| s.get(k).and_then(Json::as_f32).ok_or_else(|| Error::Invalid(format!("setup without {k}")));
            let b = |k: &str| s.get(k).and_then(Json::as_bool).ok_or_else(|| Error::Invalid(format!("setup without {k}")));
            let name = s.get("placement").and_then(Json::as_str).unwrap_or("");
            let i = Vertical::NAMES.iter().position(|n| *n == name).ok_or_else(|| Error::Invalid(format!("bad placement {name}")))?;
            Ok(Setup {
                placement: Vertical::ALL[i],
                air_pocket_probability: f("air_pocket_probability")?,
                mossiness: f("mossiness")?,
                overgrown: b("overgrown")?,
                vines: b("vines")?,
                can_be_cold: b("can_be_cold")?,
                replace_with_blackstone: b("replace_with_blackstone")?,
                weight: f("weight")?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(RuinedPortal {
        setups,
        cannot_replace: l.block_tag("minecraft:features_cannot_replace")?,
        stairs: l.block_tag("minecraft:stairs")?,
        slabs: l.block_tag("minecraft:slabs")?,
        walls: l.block_tag("minecraft:walls")?,
    })
}

const PORTALS: [&str; 10] = [
    "portal_1", "portal_2", "portal_3", "portal_4", "portal_5", "portal_6", "portal_7", "portal_8", "portal_9", "portal_10",
];
const GIANT_PORTALS: [&str; 3] = ["giant_portal_1", "giant_portal_2", "giant_portal_3"];

/// `Mth.randomBetweenInclusive`.
fn between(random: &mut WorldgenRandom, a: i32, b: i32) -> i32 {
    random.next_int_bounded(b - a + 1) + a
}

/// `RuinedPortalStructure.getRandomWithinInterval`.
fn within(random: &mut WorldgenRandom, a: i32, b: i32) -> i32 {
    if a < b { between(random, a, b) } else { b }
}

/// `RuinedPortalStructure.sample`.
fn sample(random: &mut WorldgenRandom, p: f32) -> bool {
    if p == 0.0 {
        false
    } else if p == 1.0 {
        true
    } else {
        random.next_float() < p
    }
}

impl Kind for RuinedPortal {
    /// `RuinedPortalStructure.findGenerationPoint`.
    fn find<'k>(&'k self, ctx: &mut GenCtx) -> Option<Stub<'k>> {
        let setup = if self.setups.len() > 1 {
            let total: f32 = self.setups.iter().map(|s| s.weight).sum();
            let mut f = ctx.random.next_float();
            let mut chosen = None;
            for s in &self.setups {
                f -= s.weight / total;
                if f < 0.0 {
                    chosen = Some(s);
                    break;
                }
            }
            chosen?.clone()
        } else {
            self.setups[0].clone()
        };
        let air_pocket = sample(&mut ctx.random, setup.air_pocket_probability);
        let name = if ctx.random.next_float() < 0.05 {
            GIANT_PORTALS[ctx.random.next_int_bounded(3) as usize]
        } else {
            PORTALS[ctx.random.next_int_bounded(10) as usize]
        };
        let name = format!("minecraft:ruined_portal/{name}");
        let template = ctx.structures.templates.get(&name);
        let rotation = Rotation::ALL[ctx.random.next_int_bounded(4) as usize];
        let mirror = if ctx.random.next_float() < 0.5 { Mirror::None } else { Mirror::FrontBack };
        let pivot = BlockPos::new(template.size[0] / 2, 0, template.size[2] / 2);
        let origin = BlockPos::new(ctx.chunk.0 << 4, 0, ctx.chunk.1 << 4);
        let b = bounding_box(origin, rotation, pivot, mirror, template.size);
        let center = b.center();
        let surface = ctx.first_free_height(center.x, center.z, setup.placement.heightmap()) - 1;
        let y = find_suitable_y(ctx, setup.placement, air_pocket, surface, b.y_span(), &b);
        let pos = BlockPos::new(origin.x, y, origin.z);
        Some(Stub {
            pos,
            build: Box::new(move |ctx: &mut GenCtx, out: &mut Vec<Box<dyn Piece>>| {
                let cold = setup.can_be_cold && {
                    let b = ctx.biome(pos.x >> 2, pos.y >> 2, pos.z >> 2);
                    let info = &ctx.generator.biomes[b as usize];
                    crate::simplex::cold_enough_to_snow(info.temperature, info.frozen, ctx.generator.sea_level, pos.x, pos.y, pos.z)
                };
                let props = Properties {
                    cold,
                    mossiness: setup.mossiness,
                    air_pocket,
                    overgrown: setup.overgrown,
                    vines: setup.vines,
                    replace_with_blackstone: setup.replace_with_blackstone,
                };
                let tm = ctx.structures.templates.clone();
                let processors = self.processors(setup.placement, &props);
                let t = TemplatePiece::new("minecraft:rupo", &tm, &name, rotation, mirror, pivot, processors, LiquidSettings::ApplyWaterlogging, pos);
                out.push(Box::new(RuinedPortalPiece { t, placement: setup.placement, props, cannot_replace: self.cannot_replace.clone() }));
            }),
        })
    }
}

/// `RuinedPortalStructure.findSuitableY`.
fn find_suitable_y(ctx: &mut GenCtx, placement: Vertical, air_pocket: bool, surface: i32, y_span: i32, b: &BoundingBox) -> i32 {
    let min = ctx.min_y() + 15;
    let random = &mut ctx.random;
    let start = match placement {
        Vertical::InNether => {
            if air_pocket {
                between(random, 32, 100)
            } else if random.next_float() < 0.5 {
                between(random, 27, 29)
            } else {
                between(random, 29, 100)
            }
        }
        Vertical::InMountain => within(random, 70, surface - y_span),
        Vertical::Underground => within(random, min, surface - y_span),
        Vertical::PartlyBuried => surface - y_span + between(random, 2, 8),
        _ => surface,
    };
    let corners = [(b.min_x, b.min_z), (b.max_x, b.min_z), (b.min_x, b.max_z), (b.max_x, b.max_z)];
    let columns: Vec<Vec<u16>> = corners.iter().map(|&(x, z)| ctx.base_column(x, z)).collect();
    let map = if placement == Vertical::OnOceanFloor { Heightmap::OceanFloorWg } else { Heightmap::WorldSurfaceWg };
    let bit = heightmap_bit(map);
    let min_y = ctx.min_y();
    let mut y = start;
    while y > min {
        let mut count = 0;
        for c in &columns {
            let s = usize::try_from(y - min_y).ok().and_then(|i| c.get(i).copied()).unwrap_or(state::AIR);
            if heightmap_flags(s) & bit != 0 {
                count += 1;
                if count == 3 {
                    return y;
                }
            }
        }
        y -= 1;
    }
    y
}

/// `RuinedPortalPiece.Properties`.
#[derive(Clone, Copy, Debug)]
struct Properties {
    cold: bool,
    mossiness: f32,
    air_pocket: bool,
    overgrown: bool,
    vines: bool,
    replace_with_blackstone: bool,
}

fn single(name: &str) -> Arc<BlockSet> {
    let mut s = BlockSet::empty();
    s.insert_block(name).expect("vanilla block");
    Arc::new(s)
}

/// `getBlockReplaceRule(from, probability, to)` / `getBlockReplaceRule(from, to)`.
fn replace(from: &str, probability: Option<f32>, to: &str) -> ProcessorRule {
    let input = match probability {
        Some(p) => RuleTest::RandomBlockMatch { blocks: single(from), probability: p },
        None => RuleTest::BlockMatch(single(from)),
    };
    ProcessorRule::new(input, RuleTest::AlwaysTrue, block(to).expect("vanilla block").default, None)
}

impl RuinedPortal {
    /// `RuinedPortalPiece.makeSettings` processors.
    fn processors(&self, placement: Vertical, props: &Properties) -> Vec<Processor> {
        let ignore = if props.air_pocket { IGNORE_STRUCTURE_BLOCK.clone() } else { IGNORE_STRUCTURE_AND_AIR.clone() };
        let mut rules = vec![replace("minecraft:gold_block", Some(0.3), "minecraft:air")];
        rules.push(if placement == Vertical::OnOceanFloor {
            replace("minecraft:lava", None, "minecraft:magma_block")
        } else if props.cold {
            replace("minecraft:lava", None, "minecraft:netherrack")
        } else {
            replace("minecraft:lava", Some(0.2), "minecraft:magma_block")
        });
        if !props.cold {
            rules.push(replace("minecraft:netherrack", Some(0.07), "minecraft:magma_block"));
        }
        let mut out = vec![
            ignore,
            Processor::Rule(rules),
            Processor::BlockAge { mossiness: props.mossiness, stairs: self.stairs.clone(), slabs: self.slabs.clone(), walls: self.walls.clone() },
            Processor::ProtectedBlocks(self.cannot_replace.clone()),
            Processor::LavaSubmergedBlock,
        ];
        if props.replace_with_blackstone {
            out.push(Processor::BlackstoneReplace);
        }
        out
    }
}

/// `RuinedPortalPiece`.
#[derive(Debug)]
pub struct RuinedPortalPiece {
    t: TemplatePiece,
    placement: Vertical,
    props: Properties,
    cannot_replace: Arc<BlockSet>,
}

/// `Block.isFaceFull(state.getCollisionShape(), dir)`, from the collision boxes at 1/16
/// resolution.
fn collision_face_full(s: u16, dir: Dir) -> bool {
    let boxes = kiln_data::block_props::collision(s);
    let (axis, positive) = match dir {
        Dir::Down => (1, false),
        Dir::Up => (1, true),
        Dir::North => (2, false),
        Dir::South => (2, true),
        Dir::West => (0, false),
        Dir::East => (0, true),
    };
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let face = if positive { 1.0 } else { 0.0 };
    (0..16).all(|i| {
        (0..16).all(|j| {
            let (pu, pv) = ((i as f32 + 0.5) / 16.0, (j as f32 + 0.5) / 16.0);
            boxes.iter().any(|b| {
                let on_face = if positive { b[axis + 3] >= face } else { b[axis] <= face };
                on_face && b[u] <= pu && pu <= b[u + 3] && b[v] <= pv && pv <= b[v + 3]
            })
        })
    })
}

impl RuinedPortalPiece {
    /// `canBlockBeReplacedByNetherrackOrMagma`.
    fn replaceable(&self, s: u16) -> bool {
        !is_air(s)
            && !is_block(s, "minecraft:obsidian")
            && !self.cannot_replace.contains(s)
            && (self.placement == Vertical::InNether || !is_lava(s))
    }

    /// `placeNetherrackOrMagma`.
    fn netherrack_or_magma(&self, random: &mut WorldgenRandom, r: &mut Region, p: BlockPos) {
        if !self.replaceable(r.get(p)) {
            return;
        }
        if !self.props.cold && random.next_float() < 0.07 {
            r.set(p, state::MAGMA_BLOCK, 3);
        } else {
            r.set(p, state::NETHERRACK, 3);
        }
    }

    /// `addNetherrackDripColumn`.
    fn drip_column(&self, random: &mut WorldgenRandom, r: &mut Region, p: BlockPos) {
        let mut at = p;
        self.netherrack_or_magma(random, r, at);
        let mut left = 8;
        while left > 0 && random.next_float() < 0.5 {
            at = at.below();
            left -= 1;
            self.netherrack_or_magma(random, r, at);
        }
    }

    /// `spreadNetherrack`.
    fn spread_netherrack(&self, random: &mut WorldgenRandom, r: &mut Region) {
        const CHANCES: [f32; 14] = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.9, 0.9, 0.8, 0.7, 0.6, 0.4, 0.2];
        let b = self.t.base.bbox;
        let surface = matches!(self.placement, Vertical::OnLandSurface | Vertical::OnOceanFloor);
        let c = b.center();
        let n = CHANCES.len() as i32;
        let size = (b.x_span() + b.z_span()) / 2;
        let offset = random.next_int_bounded(1.max(8 - size / 2));
        for x in c.x - n..=c.x + n {
            for z in c.z - n..=c.z + n {
                let d = (x - c.x).abs() + (z - c.z).abs();
                let i = 0.max(d + offset);
                if i >= n || !(random.next_double() < CHANCES[i as usize] as f64) {
                    continue;
                }
                let sy = r.height_at(self.placement.heightmap(), x, z) - 1;
                let y = if surface { sy } else { b.min_y.min(sy) };
                let at = BlockPos::new(x, y, z);
                if (y - b.min_y).abs() <= 3 && self.replaceable(r.get(at)) {
                    self.netherrack_or_magma(random, r, at);
                    if self.props.overgrown {
                        self.leaves_above(random, r, at);
                    }
                    self.drip_column(random, r, at.below());
                }
            }
        }
    }

    /// `maybeAddLeavesAbove`.
    fn leaves_above(&self, random: &mut WorldgenRandom, r: &mut Region, p: BlockPos) {
        if random.next_float() < 0.5 && is_block(r.get(p), "minecraft:netherrack") && is_air(r.get(p.above())) {
            r.set(p.above(), with_prop(state::JUNGLE_LEAVES, "persistent", "true"), 3);
        }
    }

    /// `maybeAddVines`.
    fn vines(&self, random: &mut WorldgenRandom, r: &mut Region, p: BlockPos) {
        let s = r.get(p);
        if is_air(s) || is_block(s, "minecraft:vine") {
            return;
        }
        let dir = Dir::HORIZONTAL[random.next_int_bounded(4) as usize];
        let n = p.relative(dir);
        if !is_air(r.get(n)) || !collision_face_full(s, dir) {
            return;
        }
        r.set(n, with_prop(state::VINE, dir.opposite().name(), "true"), 3);
    }
}

impl Piece for RuinedPortalPiece {
    fn base(&self) -> &PieceBase {
        &self.t.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.t.base
    }

    fn place(&self, _cx: &PlaceContext, r: &mut Region, random: &mut WorldgenRandom, chunk_box: &BoundingBox, _chunk: (i32, i32), pivot: BlockPos) {
        let b = self.t.bbox_at(self.t.position);
        if !chunk_box.is_inside(b.center()) {
            return;
        }
        let mut area = *chunk_box;
        area.encapsulate(&b);
        self.t.post_process(self.t.position, r, random, &area, pivot, &mut |_, _, _, _| {});
        self.spread_netherrack(random, r);
        for x in b.min_x + 1..b.max_x {
            for z in b.min_z + 1..b.max_z {
                let p = BlockPos::new(x, b.min_y, z);
                if is_block(r.get(p), "minecraft:netherrack") {
                    self.drip_column(random, r, p.below());
                }
            }
        }
        if self.props.vines || self.props.overgrown {
            for z in b.min_z..=b.max_z {
                for y in b.min_y..=b.max_y {
                    for x in b.min_x..=b.max_x {
                        let p = BlockPos::new(x, y, z);
                        if self.props.vines {
                            self.vines(random, r, p);
                        }
                        if self.props.overgrown {
                            self.leaves_above(random, r, p);
                        }
                    }
                }
            }
        }
    }

    fn save_extra(&self, tag: &mut Vec<(String, Tag)>) {
        self.t.save_template(tag);
        tag.push(("Rotation".into(), Tag::String(self.t.rotation.enum_name().into())));
        tag.push(("Mirror".into(), Tag::String(self.t.mirror.enum_name().into())));
        tag.push(("VerticalPlacement".into(), Tag::String(self.placement.name().into())));
        let p = self.props;
        tag.push((
            "Properties".into(),
            Tag::Compound(vec![
                ("cold".into(), Tag::Byte(p.cold as i8)),
                ("mossiness".into(), Tag::Float(p.mossiness)),
                ("air_pocket".into(), Tag::Byte(p.air_pocket as i8)),
                ("overgrown".into(), Tag::Byte(p.overgrown as i8)),
                ("vines".into(), Tag::Byte(p.vines as i8)),
                ("replace_with_blackstone".into(), Tag::Byte(p.replace_with_blackstone as i8)),
            ]),
        ));
    }

    fn shift(&mut self, dx: i32, dy: i32, dz: i32) {
        self.t.base.bbox.shift(dx, dy, dz);
        self.t.position = self.t.position.offset(dx, dy, dz);
    }
}
