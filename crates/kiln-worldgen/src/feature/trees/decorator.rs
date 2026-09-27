//! `TreeDecorator`s: blocks added around a placed tree (or fallen log).

use super::jset::JHashSet;
use super::tree::lowest_trunk_or_root;
use super::trunk::{random_horizontal, shuffle};
use super::{field, float_or, int_or};
use crate::Error;
use crate::block_facts::Dir;
use crate::blocks::{is_air, is_block, state, with_prop};
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::proto::Heightmap;
use crate::providers::float;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::state_provider::StateProvider;
use crate::vtags;
use kiln_data::block_props::{replaceable, solid_render};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use std::collections::HashSet;

#[derive(Debug)]
pub enum Decorator {
    PlaceOnGround { tries: i32, radius: i32, height: i32, provider: StateProvider },
    Beehive { probability: f32 },
    ShelfMushroom { probability: f32 },
    LeaveVine { probability: f32 },
    TrunkVine,
    AttachedToLeaves { probability: f32, exclusion_xz: i32, exclusion_y: i32, provider: StateProvider, required_empty: i32, directions: Vec<Dir> },
    AlterGround { provider: StateProvider },
    /// `PaleMossDecorator`; `patch` is an inline placed feature wrapping `pale_moss_patch`.
    PaleMoss { leaves: f32, trunk: f32, ground: f32, patch: usize },
    Cocoa { probability: f32 },
    CreakingHeart { probability: f32 },
    AttachedToLogs { probability: f32, provider: StateProvider, directions: Vec<Dir> },
}

/// Placed features the decorators place.
pub fn nested(list: &[Decorator]) -> Vec<usize> {
    list.iter().filter_map(|d| if let Decorator::PaleMoss { patch, .. } = d { Some(*patch) } else { None }).collect()
}

fn directions(json: &Json) -> Result<Vec<Dir>, Error> {
    field(json, "directions")?
        .as_array()
        .ok_or_else(|| Error::Invalid("directions must be a list".into()))?
        .iter()
        .map(|d| d.as_str().and_then(Dir::by_name).ok_or_else(|| Error::Invalid(format!("bad direction {d:?}"))))
        .collect()
}

impl Decorator {
    pub fn parse(json: &Json, f: &mut Features, l: &Loader) -> Result<Decorator, Error> {
        let ty = json.get("type").and_then(Json::as_str).unwrap_or("");
        let provider = |k: &str| StateProvider::parse(field(json, k)?, l);
        Ok(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
            "place_on_ground" => Decorator::PlaceOnGround {
                tries: int_or(json, "tries", 128),
                radius: int_or(json, "radius", 2),
                height: int_or(json, "height", 1),
                provider: provider("block_state_provider")?,
            },
            "beehive" => Decorator::Beehive { probability: float(json, "probability")? },
            "shelf_mushroom" => Decorator::ShelfMushroom { probability: float(json, "probability")? },
            "leave_vine" => Decorator::LeaveVine { probability: float(json, "probability")? },
            "trunk_vine" => Decorator::TrunkVine,
            "attached_to_leaves" => Decorator::AttachedToLeaves {
                probability: float(json, "probability")?,
                exclusion_xz: int_or(json, "exclusion_radius_xz", 0),
                exclusion_y: int_or(json, "exclusion_radius_y", 0),
                provider: provider("block_provider")?,
                required_empty: int_or(json, "required_empty_blocks", 0),
                directions: directions(json)?,
            },
            "alter_ground" => Decorator::AlterGround { provider: provider("provider")? },
            "pale_moss" => Decorator::PaleMoss {
                leaves: float_or(json, "leaves_probability", 0.0),
                trunk: float_or(json, "trunk_probability", 0.0),
                ground: float_or(json, "ground_probability", 0.0),
                patch: f.placed_ref(&Json::parse(r#"{"feature":"minecraft:pale_moss_patch"}"#).expect("valid json"), l)?,
            },
            "cocoa" => Decorator::Cocoa { probability: float(json, "probability")? },
            "creaking_heart" => Decorator::CreakingHeart { probability: float(json, "probability")? },
            "attached_to_logs" => Decorator::AttachedToLogs {
                probability: float(json, "probability")?,
                provider: provider("block_provider")?,
                directions: directions(json)?,
            },
            t => return Err(Error::Invalid(format!("unknown tree decorator {t}"))),
        })
    }

    /// `TreeDecorator.place`.
    pub fn place(&self, d: &mut DecoCtx) {
        match self {
            Decorator::PlaceOnGround { tries, radius, height, provider } => place_on_ground(d, *tries, *radius, *height, provider),
            Decorator::Beehive { probability } => beehive(d, *probability),
            Decorator::ShelfMushroom { probability } => shelf_mushroom(d, *probability),
            Decorator::LeaveVine { probability } => {
                for i in 0..d.leaves.len() {
                    let p = d.leaves[i];
                    for (side, face) in [(Dir::West, "east"), (Dir::East, "west"), (Dir::North, "south"), (Dir::South, "north")] {
                        if d.random.next_float() < *probability {
                            let q = p.relative(side);
                            if d.is_air(q) {
                                hanging_vine(d, q, face);
                            }
                        }
                    }
                }
            }
            Decorator::TrunkVine => {
                for i in 0..d.logs.len() {
                    let p = d.logs[i];
                    for (side, face) in [(Dir::West, "east"), (Dir::East, "west"), (Dir::North, "south"), (Dir::South, "north")] {
                        if d.random.next_int_bounded(3) > 0 {
                            let q = p.relative(side);
                            if d.is_air(q) {
                                d.place_vine(q, face);
                            }
                        }
                    }
                }
            }
            Decorator::AttachedToLeaves { probability, exclusion_xz, exclusion_y, provider, required_empty, directions } => {
                let mut excluded: HashSet<BlockPos> = HashSet::new();
                let mut leaves = d.leaves.clone();
                shuffle(&mut leaves, d.random);
                for p in leaves {
                    let dir = directions[d.random.next_int_bounded(directions.len() as i32) as usize];
                    let q = p.relative(dir);
                    if excluded.contains(&q) {
                        continue;
                    }
                    if d.random.next_float() < *probability && (1..=*required_empty).all(|i| d.is_air(p.relative_n(dir, i))) {
                        for x in -exclusion_xz..=*exclusion_xz {
                            for y in -exclusion_y..=*exclusion_y {
                                for z in -exclusion_xz..=*exclusion_xz {
                                    excluded.insert(q.offset(x, y, z));
                                }
                            }
                        }
                        let s = provider.state(d.r, d.random, q);
                        d.set_block(q, s);
                    }
                }
            }
            Decorator::AlterGround { provider } => {
                let lowest = lowest_trunk_or_root(&d.logs, &d.roots);
                let Some(first) = lowest.first() else { return };
                let y = first.y;
                for p in lowest.iter().filter(|p| p.y == y) {
                    alter_ground_at(d, provider, *p);
                }
            }
            Decorator::PaleMoss { leaves, trunk, ground, patch } => pale_moss(d, *leaves, *trunk, *ground, *patch),
            Decorator::Cocoa { probability } => {
                if d.random.next_float() >= *probability {
                    return;
                }
                let Some(first) = d.logs.first() else { return };
                let y = first.y;
                let logs: Vec<BlockPos> = d.logs.iter().copied().filter(|p| p.y - y <= 2).collect();
                for p in logs {
                    for dir in Dir::HORIZONTAL {
                        if d.random.next_float() <= 0.25 {
                            let (ox, _, oz) = dir.opposite().offset();
                            let q = p.offset(ox, 0, oz);
                            if d.is_air(q) {
                                let age = d.random.next_int_bounded(3).to_string();
                                let s = with_prop(with_prop(state::COCOA, "age", &age), "facing", dir.name());
                                d.set_block(q, s);
                            }
                        }
                    }
                }
            }
            Decorator::CreakingHeart { probability } => {
                if d.logs.is_empty() || d.random.next_float() >= *probability {
                    return;
                }
                let mut logs = d.logs.clone();
                shuffle(&mut logs, d.random);
                let found = logs.into_iter().find(|p| Dir::ALL.iter().all(|dir| vtags::is(d.r.get(p.relative(*dir)), "logs")));
                if let Some(p) = found {
                    let s = with_prop(with_prop(state::CREAKING_HEART, "creaking_heart_state", "dormant"), "natural", "true");
                    d.set_block(p, s);
                }
            }
            Decorator::AttachedToLogs { probability, provider, directions } => {
                let mut logs = d.logs.clone();
                shuffle(&mut logs, d.random);
                for p in logs {
                    let dir = directions[d.random.next_int_bounded(directions.len() as i32) as usize];
                    let q = p.relative(dir);
                    if d.random.next_float() <= *probability && d.is_air(q) {
                        let s = provider.state(d.r, d.random, q);
                        d.set_block(q, s);
                    }
                }
            }
        }
    }
}

/// `TreeDecorator.Context`: the tree's positions (each sorted by y) and the decoration setter.
pub struct DecoCtx<'c, 'r> {
    f: &'c Features,
    pub r: &'c mut Region<'r>,
    pub random: &'c mut WorldgenRandom,
    pub logs: Vec<BlockPos>,
    pub leaves: Vec<BlockPos>,
    pub roots: Vec<BlockPos>,
    decor: Option<&'c mut JHashSet>,
}

impl<'c, 'r> DecoCtx<'c, 'r> {
    pub fn new(
        f: &'c Features,
        r: &'c mut Region<'r>,
        random: &'c mut WorldgenRandom,
        logs: &JHashSet,
        leaves: &JHashSet,
        roots: &JHashSet,
        decor: Option<&'c mut JHashSet>,
    ) -> Self {
        DecoCtx { f, r, random, logs: logs.sorted_by_y(), leaves: leaves.sorted_by_y(), roots: roots.sorted_by_y(), decor }
    }

    /// The decoration setter: `setBlock(pos, state, 19)`, recorded for the leaf update.
    pub fn set_block(&mut self, p: BlockPos, s: u16) {
        if let Some(decor) = &mut self.decor {
            decor.insert(p);
        }
        self.r.set(p, s, 19);
    }

    pub fn is_air(&mut self, p: BlockPos) -> bool {
        is_air(self.r.get(p))
    }

    /// `Context.placeVine`.
    fn place_vine(&mut self, p: BlockPos, face: &str) {
        self.set_block(p, with_prop(state::VINE, face, "true"));
    }

    /// `Context.isWaterOrWaterNearby`.
    fn is_water_or_water_nearby(&mut self, p: BlockPos) -> bool {
        [p, p.offset(1, 0, 0), p.offset(-1, 0, 0), p.offset(0, 0, -1), p.offset(0, 0, 1)]
            .into_iter()
            .any(|q| is_block(self.r.get(q), "minecraft:water"))
    }
}

/// `LeaveVineDecorator.addHangingVine`.
fn hanging_vine(d: &mut DecoCtx, p: BlockPos, face: &str) {
    d.place_vine(p, face);
    let mut p = p.below();
    let mut n = 4;
    while d.is_air(p) && n > 0 {
        d.place_vine(p, face);
        p = p.below();
        n -= 1;
    }
}

/// `PlaceOnGroundDecorator.place`.
fn place_on_ground(d: &mut DecoCtx, tries: i32, radius: i32, height: i32, provider: &StateProvider) {
    let lowest = lowest_trunk_or_root(&d.logs, &d.roots);
    let Some(&first) = lowest.first() else { return };
    let y = first.y;
    let (mut x0, mut x1, mut z0, mut z1) = (first.x, first.x, first.z, first.z);
    for p in lowest.iter().filter(|p| p.y == y) {
        x0 = x0.min(p.x);
        x1 = x1.max(p.x);
        z0 = z0.min(p.z);
        z1 = z1.max(p.z);
    }
    let (x0, x1, y0, y1, z0, z1) = (x0 - radius, x1 + radius, y - height, y + height, z0 - radius, z1 + radius);
    for _ in 0..tries {
        let x = d.random.next_int_between(x0, x1);
        let py = d.random.next_int_between(y0, y1);
        let z = d.random.next_int_between(z0, z1);
        let p = BlockPos::new(x, py, z);
        let above = p.above();
        let s = d.r.get(above);
        if (is_air(s) || is_block(s, "minecraft:vine"))
            && solid_render(d.r.get(p))
            && d.r.height_at(Heightmap::MotionBlockingNoLeaves, p.x, p.z) <= above.y
        {
            let s = provider.state(d.r, d.random, above);
            d.set_block(above, s);
        }
    }
}

/// `BeehiveDecorator.place`.
fn beehive(d: &mut DecoCtx, probability: f32) {
    if d.logs.is_empty() || d.random.next_float() >= probability {
        return;
    }
    let first_log = d.logs[0].y;
    let y = match d.leaves.first() {
        Some(l) => (l.y - 1).max(first_log + 1),
        None => (first_log + 1 + d.random.next_int_bounded(3)).min(d.logs[d.logs.len() - 1].y),
    };
    let mut spots: Vec<BlockPos> = Vec::new();
    for p in d.logs.iter().filter(|p| p.y == y) {
        for dir in [Dir::East, Dir::South, Dir::West] {
            spots.push(p.relative(dir));
        }
    }
    if spots.is_empty() {
        return;
    }
    shuffle(&mut spots, d.random);
    let Some(p) = spots.into_iter().find(|p| d.is_air(*p) && d.is_air(p.relative(Dir::South))) else { return };
    d.set_block(p, with_prop(state::BEE_NEST, "facing", "south"));
    if d.r.block_entity_mut(p).is_some() {
        let n = 2 + d.random.next_int_bounded(2);
        let bees: Vec<Tag> = (0..n)
            .map(|_| {
                let ticks = d.random.next_int_bounded(599);
                Tag::Compound(vec![
                    ("entity_data".into(), Tag::Compound(vec![("id".into(), Tag::String("minecraft:bee".into()))])),
                    ("ticks_in_hive".into(), Tag::Int(ticks)),
                    ("min_ticks_in_hive".into(), Tag::Int(600)),
                ])
            })
            .collect();
        if let Some(be) = d.r.block_entity_mut(p) {
            *be = Tag::Compound(vec![("id".into(), Tag::String("minecraft:beehive".into())), ("bees".into(), Tag::List(bees))]);
        }
    }
}

/// `AlterGroundDecorator.placeCircle`.
fn alter_ground_at(d: &mut DecoCtx, provider: &StateProvider, p: BlockPos) {
    circle(d, provider, p.offset(-1, 0, -1));
    circle(d, provider, p.offset(2, 0, -1));
    circle(d, provider, p.offset(-1, 0, 2));
    circle(d, provider, p.offset(2, 0, 2));
    for _ in 0..5 {
        let i = d.random.next_int_bounded(64);
        let (dx, dz) = (i % 8, i / 8);
        if dx == 0 || dx == 7 || dz == 0 || dz == 7 {
            circle(d, provider, p.offset(-3 + dx, 0, -3 + dz));
        }
    }
}

fn circle(d: &mut DecoCtx, provider: &StateProvider, p: BlockPos) {
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            if dx.abs() != 2 || dz.abs() != 2 {
                alter_block(d, provider, p.offset(dx, 0, dz));
            }
        }
    }
}

/// `AlterGroundDecorator.placeBlockAt`.
fn alter_block(d: &mut DecoCtx, provider: &StateProvider, p: BlockPos) {
    for dy in (-3..=2).rev() {
        let q = p.above_n(dy);
        if let Some(s) = provider.optional_state(d.r, d.random, q) {
            d.set_block(q, s);
            return;
        }
        if !d.is_air(q) && dy < 0 {
            return;
        }
    }
}

/// `PaleMossDecorator.place`.
fn pale_moss(d: &mut DecoCtx, leaves: f32, trunk: f32, ground: f32, patch: usize) {
    let mut logs = d.logs.clone();
    shuffle(&mut logs, d.random);
    let Some(&lowest) = logs.iter().min_by_key(|p| p.y) else { return };
    if d.random.next_float() < ground {
        d.f.place_placed(patch, d.r, d.random, lowest.above(), false);
    }
    for i in 0..d.logs.len() {
        let p = d.logs[i];
        if d.random.next_float() < trunk {
            let q = p.below();
            if d.is_air(q) {
                moss_hanger(d, q);
            }
        }
    }
    for i in 0..d.leaves.len() {
        let p = d.leaves[i];
        if d.random.next_float() < leaves {
            let q = p.below();
            if d.is_air(q) {
                moss_hanger(d, q);
            }
        }
    }
}

/// `PaleMossDecorator.addMossHanger`.
fn moss_hanger(d: &mut DecoCtx, p: BlockPos) {
    let mut p = p;
    while d.is_air(p.below()) && (d.random.next_float() as f64) >= 0.5 {
        d.set_block(p, with_prop(state::PALE_HANGING_MOSS, "tip", "false"));
        p = p.below();
    }
    d.set_block(p, with_prop(state::PALE_HANGING_MOSS, "tip", "true"));
}

/// `ShelfMushroomDecorator.place`.
fn shelf_mushroom(d: &mut DecoCtx, probability: f32) {
    if d.random.next_float() >= probability || d.logs.is_empty() {
        return;
    }
    let logs = d.logs.clone();
    let (first, last) = (logs[0], logs[logs.len() - 1]);
    if first.y == last.y {
        let sides = if first.x != last.x { [Dir::North, Dir::South] } else { [Dir::East, Dir::West] };
        for p in &logs {
            for dir in sides {
                if d.random.next_float() > 0.25 {
                    continue;
                }
                let q = p.offset(dir.offset().0, 0, dir.offset().2);
                if !shelf_replaceable(d, q) || adjacent_shelf(d, q) || adjacent_shelf(d, *p) {
                    continue;
                }
                place_shelf(d, q, dir);
            }
        }
    } else {
        let a = random_horizontal(d.random);
        let sides = [a, a.clockwise()];
        let base = first.y;
        for p in &logs {
            let dy = p.y - base;
            if !(1..=4).contains(&dy) {
                continue;
            }
            for dir in sides {
                if d.random.next_float() > 0.25 {
                    continue;
                }
                let q = p.offset(dir.offset().0, 0, dir.offset().2);
                if !shelf_replaceable(d, q) || is_block(d.r.get(q.below()), "minecraft:shelf_mushroom") {
                    continue;
                }
                place_shelf(d, q, dir);
                break;
            }
        }
    }
}

/// `ShelfMushroomDecorator.isBlockReplaceableWithShelfMushroom`.
fn shelf_replaceable(d: &mut DecoCtx, p: BlockPos) -> bool {
    replaceable(d.r.get(p)) && !d.is_water_or_water_nearby(p)
}

/// `ShelfMushroomDecorator.hasHorizontallyAdjacentShelfMushroom`.
fn adjacent_shelf(d: &mut DecoCtx, p: BlockPos) -> bool {
    Dir::HORIZONTAL.iter().any(|dir| is_block(d.r.get(p.relative(*dir)), "minecraft:shelf_mushroom"))
}

fn place_shelf(d: &mut DecoCtx, p: BlockPos, dir: Dir) {
    let age = d.random.next_int_bounded(2).to_string();
    d.set_block(p, with_prop(with_prop(state::SHELF_MUSHROOM, "age", &age), "facing", dir.name()));
}
