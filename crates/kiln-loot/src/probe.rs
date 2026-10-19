//! What location predicates (`LocationPredicate`, `FluidPredicate`) ask of the world around a
//! position, answered by whoever holds the chunks ([`WorldProbe`]).

use crate::json::Json;
use crate::predicate::{self, world::LocationPredicate};
use kiln_item::component::DoubleBounds;

/// World queries for location predicates.
pub trait WorldProbe {
    fn block(&self, pos: [i32; 3]) -> Option<u16>;
    /// `minecraft:worldgen/biome` name at `pos`.
    fn biome(&self, pos: [i32; 3]) -> Option<&'static str>;
    /// `getMaxLocalRawBrightness`: block light or sky light less the sky's darkening.
    fn light(&self, _pos: [i32; 3]) -> Option<i32> {
        None
    }
    /// `Level.canSeeSky`.
    fn can_see_sky(&self, _pos: [i32; 3]) -> Option<bool> {
        None
    }
    /// The structures (`minecraft:worldgen/structure` ids) with a piece at `pos`
    /// (`StructureManager.getStructureWithPieceAt`), when the chunks are at hand.
    fn structures_at(&self, _pos: [i32; 3]) -> Option<Vec<String>> {
        None
    }
}

fn bounds(b: &DoubleBounds, v: f64) -> bool {
    b.min.is_none_or(|m| m <= v) && b.max.is_none_or(|m| v <= m)
}


/// `LocationPredicate.matches`: position, dimension, then (where the chunks are at hand)
/// biome, structure, light, block, fluid and sky access. Smoke from campfires is not
/// evaluated (a predicate asking fails).
pub fn location_matches(l: &LocationPredicate, pos: [f64; 3], dim: &str, world: Option<&dyn WorldProbe>) -> bool {
    if let Some(p) = &l.position
        && !(bounds(&p.x, pos[0]) && bounds(&p.y, pos[1]) && bounds(&p.z, pos[2]))
    {
        return false;
    }
    if let Some(d) = &l.dimension
        && d.as_str() != dim
    {
        return false;
    }
    let block_pos = pos.map(|c| c.floor() as i32);
    if let Some(biomes) = &l.biomes {
        let Some(b) = world.and_then(|w| w.biome(block_pos)) else { return false };
        if !biomes.contains_str(b) {
            return false;
        }
    }
    if let Some(structures) = &l.structures {
        let Some(found) = world.and_then(|w| w.structures_at(block_pos)) else { return false };
        if !found.iter().any(|s| structures.contains_str(s)) {
            return false;
        }
    }
    if l.smokey.is_some() {
        return false;
    }
    if let Some(light) = &l.light {
        let Some(v) = world.and_then(|w| w.light(block_pos)) else { return false };
        if !predicate::item::int_bounds(light, v) {
            return false;
        }
    }
    if let Some(block) = &l.block {
        let Some(state) = world.and_then(|w| w.block(block_pos)) else { return false };
        if !block.matches_state(state) {
            return false;
        }
    }
    if let Some(fluid) = &l.fluid {
        let Some(state) = world.and_then(|w| w.block(block_pos)) else { return false };
        if !fluid_matches(fluid, state) {
            return false;
        }
    }
    if let Some(sky) = l.can_see_sky {
        let Some(v) = world.and_then(|w| w.can_see_sky(block_pos)) else { return false };
        if v != sky {
            return false;
        }
    }
    true
}

/// `FluidPredicate.matches` on the fluid of block state `state`: `fluids` (ids or a tag) and
/// the fluid state's properties (`level`, `falling`).
pub fn fluid_matches(j: &Json, state: u16) -> bool {
    use kiln_data::block_logic::FluidKind;
    let f = kiln_data::block_logic::fluid(state);
    let name = match (f.kind, f.source) {
        (FluidKind::Empty, _) => "minecraft:empty",
        (FluidKind::Water, true) => "minecraft:water",
        (FluidKind::Water, false) => "minecraft:flowing_water",
        (FluidKind::Lava, true) => "minecraft:lava",
        (FluidKind::Lava, false) => "minecraft:flowing_lava",
    };
    if let Some(fluids) = j.get("fluids") {
        let one = |v: &Json| match v.as_str() {
            Some(t) if t.starts_with('#') => {
                let tag = t.trim_start_matches('#');
                let tag = if tag.contains(':') { tag.to_string() } else { format!("minecraft:{tag}") };
                kiln_data::live::tags()
                    .iter()
                    .find(|(r, _)| *r == "minecraft:fluid")
                    .and_then(|(_, tags)| tags.iter().find(|(n, _)| *n == tag))
                    .is_some_and(|(_, ids)| kiln_data::builtin_id("minecraft:fluid", name).is_some_and(|id| ids.contains(&(id as _))))
            }
            Some(id) => kiln_item::Identifier::parse(id).is_some_and(|i| i.to_string() == name),
            None => false,
        };
        let ok = match fluids {
            Json::Arr(list) => list.iter().any(one),
            v => one(v),
        };
        if !ok {
            return false;
        }
    }
    if let Some(Json::Obj(props)) = j.get("state") {
        for (k, want) in props {
            let have = match k.as_str() {
                "level" if !f.source => f.amount.to_string(),
                "falling" => f.falling.to_string(),
                _ => return false,
            };
            let want = match want {
                Json::Str(s) => s.clone(),
                Json::Bool(b) => b.to_string(),
                other => other.canonical(),
            };
            if want != have {
                return false;
            }
        }
    }
    true
}

/// The level around an entity, as an effect's requirements ask about it: the blocks and sky
/// ([`WorldProbe`]), the dimension, the weather.
#[derive(Clone, Copy)]
pub struct WorldFacts<'a> {
    pub probe: &'a dyn WorldProbe,
    /// `minecraft:overworld` and the like.
    pub dim: &'a str,
    pub raining: bool,
    pub thundering: bool,
}
