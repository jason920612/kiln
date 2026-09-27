//! Features that reshape terrain: lakes, springs, disks, underwater magma, snow and ice, icebergs, blue ice, geodes, monster rooms, block blobs.

mod blob;
mod disk;
mod geode;
mod ice;
mod iceberg;
mod lake;
mod magma;
mod monster_room;
mod spring;

use crate::Error;
use crate::biome::BiomeInfo;
use crate::blocks::is_air;
use crate::feature::Features;
use crate::json::Json;
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::sets::Loader;
use crate::simplex::biome_temperature;

/// The feature types of this family.
#[derive(Debug)]
pub enum Kind {
    Lake(lake::Lake),
    Spring(spring::Spring),
    Disk(disk::Disk),
    UnderwaterMagma(magma::UnderwaterMagma),
    FreezeTopLayer,
    MonsterRoom,
    Geode(Box<geode::Geode>),
    Iceberg(u16),
    BlueIce,
    BlockBlob(blob::BlockBlob),
}

/// Parses a feature of this family (`ty` without the `minecraft:` prefix); `None` if the
/// type is not one of them.
pub fn parse(ty: &str, json: &Json, _f: &mut Features, l: &Loader) -> Option<Result<Kind, Error>> {
    Some(match ty {
        "lake" => lake::Lake::parse(json, l).map(Kind::Lake),
        "spring_feature" => spring::Spring::parse(json, l).map(Kind::Spring),
        "disk" => disk::Disk::parse(json, l).map(Kind::Disk),
        "underwater_magma" => magma::UnderwaterMagma::parse(json).map(Kind::UnderwaterMagma),
        "freeze_top_layer" => Ok(Kind::FreezeTopLayer),
        "monster_room" => Ok(Kind::MonsterRoom),
        "geode" => geode::Geode::parse(json, l).map(|g| Kind::Geode(Box::new(g))),
        "iceberg" => field(json, "state").and_then(crate::blocks::block_state).map(Kind::Iceberg),
        "blue_ice" => Ok(Kind::BlueIce),
        "block_blob" => blob::BlockBlob::parse(json, l).map(Kind::BlockBlob),
        _ => return None,
    })
}

impl Kind {
    pub fn place(&self, _f: &Features, r: &mut Region, random: &mut WorldgenRandom, p: BlockPos) -> bool {
        match self {
            Kind::Lake(f) => f.place(r, random, p),
            Kind::Spring(f) => f.place(r, p),
            Kind::Disk(f) => f.place(r, random, p),
            Kind::UnderwaterMagma(f) => f.place(r, random, p),
            Kind::FreezeTopLayer => ice::freeze_top_layer(r, p),
            Kind::MonsterRoom => monster_room::place(r, random, p),
            Kind::Geode(f) => f.place(r, random, p),
            Kind::Iceberg(s) => iceberg::place(*s, r, random, p),
            Kind::BlueIce => ice::blue_ice(r, random, p),
            Kind::BlockBlob(f) => f.place(r, random, p),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Kind::Lake(_) => "minecraft:lake",
            Kind::Spring(_) => "minecraft:spring_feature",
            Kind::Disk(_) => "minecraft:disk",
            Kind::UnderwaterMagma(_) => "minecraft:underwater_magma",
            Kind::FreezeTopLayer => "minecraft:freeze_top_layer",
            Kind::MonsterRoom => "minecraft:monster_room",
            Kind::Geode(_) => "minecraft:geode",
            Kind::Iceberg(_) => "minecraft:iceberg",
            Kind::BlueIce => "minecraft:blue_ice",
            Kind::BlockBlob(_) => "minecraft:block_blob",
        }
    }

    /// Placed features this feature places (for [`Features::is_supported`]).
    pub fn nested(&self) -> Vec<usize> {
        Vec::new()
    }
}

fn field<'a>(json: &'a Json, key: &str) -> Result<&'a Json, Error> {
    json.get(key).ok_or_else(|| Error::Invalid(format!("missing {key}")))
}

/// An optional field (`optionalFieldOf(key, default)`), parsed when present.
fn opt<T>(json: &Json, key: &str, default: T, parse: impl FnOnce(&Json) -> Option<T>) -> Result<T, Error> {
    match json.get(key) {
        None => Ok(default),
        Some(v) => parse(v).ok_or_else(|| Error::Invalid(format!("bad {key}"))),
    }
}

/// `Mth.ceil(float)`.
fn ceil(f: f32) -> i32 {
    let i = f as i32;
    if f > i as f32 { i + 1 } else { i }
}

/// `BlockPos.betweenClosed(a, b)` in vanilla's order: x fastest, then y, then z.
fn between_closed(a: BlockPos, b: BlockPos) -> impl Iterator<Item = BlockPos> {
    let (x0, y0, z0) = (a.x.min(b.x), a.y.min(b.y), a.z.min(b.z));
    let (x1, y1, z1) = (a.x.max(b.x), a.y.max(b.y), a.z.max(b.z));
    (z0..=z1).flat_map(move |z| (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| BlockPos::new(x, y, z))))
}

/// `Feature.markAboveForPostProcessing`: the (up to two) non-air blocks above `p`.
fn mark_above_for_post_processing(r: &mut Region, p: BlockPos) {
    let mut q = p;
    for _ in 0..2 {
        q = q.above();
        if r.is_air(q) {
            return;
        }
        r.mark_post_processing(q);
    }
}

/// `Feature.safeSetBlock` with `!state.is(set)`-style predicates.
fn safe_set(r: &mut Region, p: BlockPos, s: u16, can_replace: impl Fn(u16) -> bool) {
    if can_replace(r.get(p)) {
        r.set(p, s, 2);
    }
}

fn biome_info<'r>(r: &'r mut Region, p: BlockPos) -> &'r BiomeInfo {
    let b = r.biome(p);
    &r.generator.biomes[b as usize]
}

/// `Biome.getTemperature(pos, seaLevel) >= 0.15` (`warmEnoughToRain`) for the biome at `at`.
fn warm_enough_to_rain(b: &BiomeInfo, sea_level: i32, p: BlockPos) -> bool {
    biome_temperature(b.temperature, b.frozen, sea_level, p.x, p.y, p.z) >= 0.15
}

/// `Biome.shouldFreeze(level, pos, false)` for the biome looked up at `at`. Block light is 0
/// during generation.
fn should_freeze(r: &mut Region, at: BlockPos, p: BlockPos) -> bool {
    let sea = r.sea_level();
    if warm_enough_to_rain(biome_info(r, at), sea, p) || r.is_outside_build_height(p.y) {
        return false;
    }
    let s = r.get(p);
    crate::block_facts::fluid(s).kind == crate::block_facts::FluidKind::Water && crate::blocks::is_water(s)
}

/// `Biome.shouldSnow(level, pos)` for the biome looked up at `at`.
fn should_snow(r: &mut Region, at: BlockPos, p: BlockPos) -> bool {
    let sea = r.sea_level();
    let b = biome_info(r, at);
    if !b.has_precipitation || warm_enough_to_rain(b, sea, p) || r.is_outside_build_height(p.y) {
        return false;
    }
    let s = r.get(p);
    (is_air(s) || crate::blocks::is_block(s, "minecraft:snow"))
        && crate::survive::can_survive(crate::blocks::state::SNOW, r, p)
}
