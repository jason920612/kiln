//! `BlockTransformer` (`minecraft:block_transformer`: `hoe`, `axe`, `shovel`) as a data pack defines or overrides it.
//!
//! In 26.3 what a hoe, an axe and a shovel do to the block they are used on is data: a list of transforms, each with a block state provider
//! (`rule_based` rules over block predicates, `copy_properties`, a plain state), the faces it does not work on, the item damage, the sound, the
//! particle event, an optional loot table and how its items pop out (`BlockTransformer.transformBlock`). The vanilla entries are vanilla's own
//! `data/minecraft/block_transformer/{hoe,axe,shovel}.json` when the vanilla data is there (Kiln's built-in rules in `tools.rs` stand in
//! without it, and for a file whose transforms Kiln cannot read); when a pack defines or overrides `hoe`, `axe` or `shovel` its transforms
//! run instead, as the game runs them.
//!
//! Supported: providers `rule_based` (rules, `fallback`), `copy_properties`, a state (`{"id":…,"properties":{…}}`, also as
//! `simple_state_provider`); predicates `matching_blocks`, `matching_block_tag`, `all_of`, `any_of`, `not`, `true`, with `offset`. A transform
//! with anything else is left out (logged), as a pack whose data does not decode would be.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::Spawn;
use kiln_blocks::{BlockPos, Direction, Effect, Level, flags, state};
use kiln_data::live::Cached;
use kiln_loot::Json;
use std::collections::HashMap;
use tracing::warn;

/// `BlockPredicate`.
#[derive(Debug, Clone)]
enum Pred {
    Blocks { ids: Vec<String>, offset: [i32; 3] },
    Tag { tag: String, offset: [i32; 3] },
    All(Vec<Pred>),
    Any(Vec<Pred>),
    Not(Box<Pred>),
    True,
}

/// `BlockStateProvider`.
#[derive(Debug, Clone)]
enum Provider {
    State(u16),
    /// `CopyPropertiesProvider`: the source's state with the properties of the block that is there.
    Copy(Box<Provider>),
    /// `RuleBasedStateProvider`.
    Rules { rules: Vec<(Pred, Provider)>, fallback: Option<Box<Provider>> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropStrategy {
    /// `Block.popResource`.
    Default,
    /// `Block.popResourceFromFace`.
    ClickedFace,
}

#[derive(Debug, Clone)]
struct Transform {
    provider: Provider,
    disallowed_faces: Vec<Direction>,
    damage: i32,
    consume_on_use: bool,
    sound: Option<String>,
    /// The level event of the particles (`wax_on` 3003, `wax_off` 3004, `scrape` 3005).
    particle: Option<i32>,
    loot: Option<String>,
    drop: DropStrategy,
    update_from_neighbors: bool,
}

fn offset(j: &Json) -> [i32; 3] {
    match j.get("offset").and_then(Json::as_array) {
        Some([x, y, z]) => [x.as_i32().unwrap_or(0), y.as_i32().unwrap_or(0), z.as_i32().unwrap_or(0)],
        _ => [0; 3],
    }
}

fn name(s: &str) -> String {
    if s.contains(':') || s.starts_with('#') { s.to_owned() } else { format!("minecraft:{s}") }
}

fn parse_pred(j: &Json) -> Option<Pred> {
    let list = |key: &str| j.get(key).and_then(Json::as_array).map(|a| a.iter().map(parse_pred).collect::<Option<Vec<_>>>()).unwrap_or(None);
    Some(match j.get("type")?.as_str()?.trim_start_matches("minecraft:") {
        "matching_blocks" => {
            let ids = match j.get("blocks")? {
                Json::Str(s) => vec![name(s)],
                Json::Arr(items) => items.iter().map(|i| i.as_str().map(name)).collect::<Option<Vec<_>>>()?,
                _ => return None,
            };
            Pred::Blocks { ids, offset: offset(j) }
        }
        "matching_block_tag" => Pred::Tag { tag: name(j.get("tag")?.as_str()?), offset: offset(j) },
        "all_of" => Pred::All(list("predicates")?),
        "any_of" => Pred::Any(list("predicates")?),
        "not" => Pred::Not(Box::new(parse_pred(j.get("predicate")?)?)),
        "true" => Pred::True,
        _ => return None,
    })
}

/// `BlockState.FULL_CODEC`: `{"id": …, "properties": {…}}`.
fn parse_state(j: &Json) -> Option<u16> {
    let block = kiln_data::blocks_types::block_by_name(&name(j.get("id")?.as_str()?))?;
    let mut s = block.default;
    for (key, value) in j.get("properties").and_then(Json::as_object).unwrap_or(&[]) {
        s = block.with_property(s, key, value.as_str()?)?;
    }
    Some(s)
}

fn parse_provider(j: &Json) -> Option<Provider> {
    match j.get("type").and_then(Json::as_str).map(|t| t.trim_start_matches("minecraft:")) {
        None => parse_state(j).map(Provider::State),
        Some("simple_state_provider") => parse_state(j.get("state")?).map(Provider::State),
        Some("copy_properties") => Some(Provider::Copy(Box::new(parse_provider(j.get("source")?)?))),
        Some("rule_based") => {
            let rules = j
                .get("rules")?
                .as_array()?
                .iter()
                .map(|r| Some((parse_pred(r.get("if_true")?)?, parse_provider(r.get("then")?)?)))
                .collect::<Option<Vec<_>>>()?;
            let fallback = match j.get("fallback") {
                Some(f) => Some(Box::new(parse_provider(f)?)),
                None => None,
            };
            Some(Provider::Rules { rules, fallback })
        }
        Some(_) => None,
    }
}

fn parse_transform(j: &Json) -> Option<Transform> {
    let faces = j
        .get("disallowed_faces")
        .and_then(Json::as_array)
        .map(|a| a.iter().map(|f| f.as_str().and_then(Direction::from_name)).collect::<Option<Vec<_>>>())
        .unwrap_or(Some(Vec::new()))?;
    let particle = match j.get("particle").and_then(Json::as_str).map(|p| p.trim_start_matches("minecraft:")) {
        None | Some("none") => None,
        Some("wax_on") => Some(3003),
        Some("wax_off") => Some(3004),
        Some("scrape") => Some(3005),
        Some(_) => return None,
    };
    let drop = match j.get("drop_strategy").and_then(Json::as_str) {
        None | Some("default") => DropStrategy::Default,
        Some("clicked_face") => DropStrategy::ClickedFace,
        Some(_) => return None,
    };
    Some(Transform {
        provider: parse_provider(j.get("block_state_provider")?)?,
        disallowed_faces: faces,
        damage: j.get("item_damage_per_use").and_then(Json::as_i32).unwrap_or(1),
        consume_on_use: j.get("consume_on_use").and_then(Json::as_bool).unwrap_or(false),
        sound: j.get("sound").and_then(Json::as_str).map(name),
        particle,
        loot: j.get("loot").and_then(Json::as_str).map(name),
        drop,
        update_from_neighbors: j.get("update_from_neighbors").and_then(Json::as_bool).unwrap_or(false),
    })
}

/// A transformer: its transforms in order, and whether it is vanilla's own file (not a pack's).
struct Transformer {
    transforms: Vec<Transform>,
    vanilla: bool,
}

/// Tests: the built-in rules run instead of vanilla's files (to compare the two).
static BUILTIN_RULES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes the vanilla transformers `hoe`, `axe` and `shovel` come from Kiln's built-in rules (`tools.rs`) instead of vanilla's files (for tests).
pub(crate) fn use_builtin_rules(on: bool) {
    BUILTIN_RULES.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the hoe, axe and shovel come from vanilla's own files (found and readable), as opposed to the built-in rules.
pub(crate) fn vanilla_files_loaded() -> bool {
    ["minecraft:hoe", "minecraft:axe", "minecraft:shovel"].iter().all(|e| transformers().get(e).is_some_and(|t| t.vanilla))
}

fn parse_list(entry: &str, json: &Json, strict: bool) -> Option<Vec<Transform>> {
    let list = json.as_array()?;
    let mut transforms = Vec::new();
    for (i, t) in list.iter().enumerate() {
        match parse_transform(t) {
            Some(t) => transforms.push(t),
            None if strict => {
                warn!("block_transformer {entry}: transform {i} uses something Kiln does not know; Kiln's built-in rules do it");
                return None;
            }
            None => warn!("block_transformer {entry}: transform {i} uses something Kiln does not know; left out"),
        }
    }
    Some(transforms)
}

/// The transformers: vanilla's `hoe`, `axe` and `shovel` files when the vanilla data has them, and what the data packs define or override.
fn transformers() -> &'static HashMap<&'static str, Transformer> {
    static ALL: Cached<HashMap<&'static str, Transformer>, true> = Cached::new();
    ALL.get(|| {
        let mut out = HashMap::new();
        let vanilla = crate::world_state::worldgen_roots().into_iter().next();
        for (entry, file) in [("minecraft:hoe", "hoe"), ("minecraft:axe", "axe"), ("minecraft:shovel", "shovel")] {
            let Some(root) = &vanilla else { break };
            let Ok(text) = std::fs::read_to_string(root.join(format!("data/minecraft/block_transformer/{file}.json"))) else { continue };
            if let Some(transforms) = Json::parse(&text).ok().and_then(|json| parse_list(entry, &json, true)) {
                out.insert(entry, Transformer { transforms, vanilla: true });
            }
        }
        for entry in kiln_data::live::custom_entries("minecraft:block_transformer") {
            let Some(json) = kiln_data::live::entry_data("minecraft:block_transformer", entry).and_then(|t| Json::parse(t).ok()) else { continue };
            let Some(transforms) = parse_list(entry, &json, false) else { continue };
            out.insert(entry, Transformer { transforms, vanilla: false });
        }
        out
    })
}

impl Pred {
    fn test(&self, level: &RegionLevel, pos: BlockPos) -> bool {
        match self {
            Pred::Blocks { ids, offset } => {
                let at = level.block(pos.offset(offset[0], offset[1], offset[2]));
                let block = kiln_blocks::BlockId::of(at).name();
                ids.iter().any(|id| match id.strip_prefix('#') {
                    Some(tag) => kiln_blocks::tags::is(at, tag),
                    None => id == block,
                })
            }
            Pred::Tag { tag, offset } => kiln_blocks::tags::is(level.block(pos.offset(offset[0], offset[1], offset[2])), tag),
            Pred::All(list) => list.iter().all(|p| p.test(level, pos)),
            Pred::Any(list) => list.iter().any(|p| p.test(level, pos)),
            Pred::Not(p) => !p.test(level, pos),
            Pred::True => true,
        }
    }
}

impl Provider {
    /// `BlockStateProvider.getOptionalState`.
    fn provide(&self, level: &RegionLevel, pos: BlockPos) -> Option<u16> {
        match self {
            Provider::State(s) => Some(*s),
            Provider::Copy(source) => source.provide(level, pos).map(|s| state::with_properties_of(s, level.block(pos))),
            Provider::Rules { rules, fallback } => rules
                .iter()
                .find_map(|(pred, then)| pred.test(level, pos).then(|| then.provide(level, pos)).flatten())
                .or_else(|| fallback.as_ref().and_then(|f| f.provide(level, pos))),
        }
    }
}

/// `BlockTransformer.transformBlock` for the item in `off_hand`'s hand used on `pos`, when the transformer `entry` (`minecraft:hoe`, `axe` or
/// `shovel`) is known (vanilla's file or a pack's): `None` when it is not (the built-in rules apply), else whether the item did something.
/// The item takes its damage here.
pub(crate) fn transform(
    entry: &str,
    p: &mut Player,
    level: &mut RegionLevel,
    pos: BlockPos,
    face: Direction,
    off_hand: bool,
    spawns: &mut Vec<Spawn>,
) -> Option<bool> {
    let transformer = transformers().get(entry)?;
    if transformer.vanilla && BUILTIN_RULES.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let transforms = &transformer.transforms;
    // `playerHasBlockingItemUseIntent`: a shield in the off hand about to be raised.
    if !off_hand && p.inv.equipped(kiln_item::component::EquipmentSlot::OffHand).get(kiln_item::keys::BLOCKS_ATTACKS).is_some() && !p.sneaking {
        return Some(false);
    }
    let tool = p.in_hand(off_hand).clone();
    for t in transforms {
        if t.disallowed_faces.contains(&face) {
            continue;
        }
        let Some(mut to) = t.provider.provide(level, pos) else { continue };
        if t.update_from_neighbors {
            to = kiln_blocks::update::update_from_neighbour_shapes(level, to, pos);
        }
        let before = level.block(pos);
        // The loot is rolled for the block as it was, before it changes.
        let drops = t.loot.as_deref().map(|table| {
            crate::interact_loot::block_interact(level, table, pos, before, Some(&tool), true, 0x7472_616e).unwrap_or_default()
        });
        // The item is used (`ItemStack.consume` of a stackable one, else `hurtAndBreak`).
        if tool.is_stackable() {
            if t.consume_on_use {
                let i = p.hand_index(off_hand);
                kiln_inventory::Container::item_mut(&mut p.inv, i).shrink(1);
                p.inv.times_changed += 1;
            }
        } else if t.damage > 0 {
            p.hurt_and_break(crate::tools::hand_slot(off_hand), t.damage, None);
        }
        kiln_blocks::set_block(level, pos, to, flags::ALL_IMMEDIATE);
        if let Some(sound) = &t.sound {
            level.effect(Effect::ActorSound { pos, sound: kiln_data::live::leak_str(sound), volume: 1.0, pitch: 1.0 });
        }
        if let Some(id) = t.particle {
            level.effect(Effect::ActorLevelEvent { id, pos, data: 0 });
        }
        for item in drops.into_iter().flatten() {
            match t.drop {
                DropStrategy::Default => crate::interact_loot::pop_resource(level, pos, item, spawns),
                DropStrategy::ClickedFace => crate::interact_loot::pop_resource_from_face(level, pos, face, item, spawns),
            }
        }
        return Some(true);
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_shaped_transforms_parse() {
        let hoe = r##"[{"block_state_provider":{"type":"minecraft:rule_based","rules":[{"if_true":{"type":"minecraft:all_of","predicates":[
            {"type":"minecraft:matching_block_tag","tag":"minecraft:turns_into_farmland"},
            {"type":"minecraft:matching_block_tag","offset":[0,1,0],"tag":"minecraft:air"}]},
            "then":{"id":"minecraft:farmland","properties":{"moisture":"0"}}}]},
            "disallowed_faces":["down"],"item_damage_per_use":1,"sound":"minecraft:item.hoe.till"},
            {"block_state_provider":{"type":"minecraft:rule_based","rules":[{"if_true":{"type":"minecraft:matching_blocks","blocks":"minecraft:rooted_dirt"},
            "then":{"id":"minecraft:dirt"}}]},"drop_strategy":"clicked_face","item_damage_per_use":1,"loot":"minecraft:till/rooted_dirt","sound":"minecraft:item.hoe.till"}]"##;
        let json = Json::parse(hoe).unwrap();
        let ts: Vec<Transform> = json.as_array().unwrap().iter().map(|t| parse_transform(t).expect("a transform")).collect();
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].disallowed_faces, [Direction::Down]);
        assert_eq!(ts[1].drop, DropStrategy::ClickedFace);
        assert_eq!(ts[1].loot.as_deref(), Some("minecraft:till/rooted_dirt"));
        // Something unknown leaves the transform out.
        let odd = Json::parse(r#"{"block_state_provider":{"type":"minecraft:weighted_state_provider"},"sound":"minecraft:x"}"#).unwrap();
        assert!(parse_transform(&odd).is_none());
    }
}
