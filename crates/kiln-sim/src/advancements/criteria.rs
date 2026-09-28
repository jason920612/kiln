//! Advancement criteria (`Criterion`: a trigger and its conditions) and how they are tested.
//!
//! Every criterion has an optional `player` condition (`ContextAwarePredicate`: loot
//! conditions on the player as `this`, or the legacy entity predicate form). The triggers Kiln
//! fires parse their own conditions below; the others are loaded (so requirements and
//! `/advancement` work) but never fire.

use kiln_item::ItemStack;
use kiln_item::component::{DoubleBounds, IntBounds, ItemPredicate};
use kiln_loot::condition::Condition;
use kiln_loot::parse::{PResult, ParseError, Parser, Ref};
use kiln_loot::predicate::world::{EntityPredicate, EntitySubPredicate, LocationPredicate};
use kiln_loot::predicate::{self, DamageSourcePredicate};
use kiln_loot::{EntityTarget, Json, LootContext, LootData};

/// `ContextAwarePredicate`: loot conditions that must all hold.
pub(crate) type Cap = Vec<Ref<Condition>>;

/// A trigger's conditions, for the triggers Kiln fires.
#[derive(Debug, Clone)]
pub(crate) enum Trigger {
    /// `minecraft:impossible`: only `/advancement` grants it.
    Impossible,
    /// `PlayerTrigger` (`tick`, `location`, `slept_in_bed`, `hero_of_the_village`,
    /// `avoid_vibration`) and `started_riding`: the player condition only.
    Player,
    InventoryChanged { items: Vec<ItemPredicate>, occupied: IntBounds, full: IntBounds, empty: IntBounds },
    RecipeUnlocked { recipes: Vec<String> },
    RecipeCrafted { recipes: Vec<String>, ingredients: Vec<ItemPredicate> },
    /// `player_killed_entity` and `entity_killed_player`.
    Killed { entity: Option<Cap>, killing_blow: Option<DamageSourcePredicate> },
    ConsumeItem { item: Option<ItemPredicate> },
    /// `placed_block`, `item_used_on_block`, `default_block_use`, `any_block_use`: conditions
    /// on the location, block and tool.
    Location { location: Option<Cap> },
    EnterBlock { blocks: Option<kiln_loot::parse::IdSet>, state: Option<Json> },
    ChangedDimension { from: Option<String>, to: Option<String> },
    VillagerTrade { villager: Option<Cap>, item: Option<ItemPredicate> },
    EnchantedItem { item: Option<ItemPredicate>, levels: IntBounds },
    /// `entity_hurt_player` and `player_hurt_entity` (the damage predicate's taken/dealt
    /// bounds and source; `entity` for the hurt entity).
    Hurt { dealt: DoubleBounds, taken: DoubleBounds, blocked: Option<bool>, source: Option<DamageSourcePredicate>, entity: Option<Cap> },
    /// A trigger Kiln does not fire (yet).
    Staged,
}

#[derive(Debug, Clone)]
pub(crate) struct Criterion {
    /// `minecraft:inventory_changed`, ...
    pub trigger_id: String,
    pub trigger: Trigger,
    pub player: Option<Cap>,
}

/// The triggers Kiln fires; everything else in `minecraft:trigger_type` loads as staged.
pub(crate) const FIRED: &[&str] = &[
    "minecraft:impossible",
    "minecraft:tick",
    "minecraft:location",
    "minecraft:inventory_changed",
    "minecraft:recipe_unlocked",
    "minecraft:recipe_crafted",
    "minecraft:player_killed_entity",
    "minecraft:entity_killed_player",
    "minecraft:consume_item",
    "minecraft:placed_block",
    "minecraft:item_used_on_block",
    "minecraft:enter_block",
    "minecraft:changed_dimension",
    "minecraft:villager_trade",
    "minecraft:enchanted_item",
    "minecraft:entity_hurt_player",
    "minecraft:started_riding",
];

fn err<T>(m: impl Into<String>) -> PResult<T> {
    Err(ParseError::new(m))
}

/// `ContextAwarePredicate.CODEC`: a list of loot conditions, one condition (an object with
/// `type`), or an entity predicate (the legacy form, an `entity_properties` condition on
/// `this`).
fn cap(p: &Parser, j: &Json) -> PResult<Cap> {
    match j {
        Json::Arr(_) => Condition::parse_list(p, j),
        _ if j.get("type").is_some() => Ok(vec![Ref::direct(Condition::parse(p, j)?)]),
        _ => {
            let e = EntityPredicate::parse(p, j)?;
            Ok(vec![Ref::direct(Condition::EntityProperties { target: EntityTarget::This, predicate: Some(Box::new(e)) })])
        }
    }
}

fn opt_cap(p: &Parser, c: &Json, key: &str) -> PResult<Option<Cap>> {
    c.get(key).map(|v| cap(p, v).map_err(|e| e.at(key))).transpose()
}

fn opt_item(c: &Json, key: &str) -> PResult<Option<ItemPredicate>> {
    c.get(key).map(|v| predicate::item_predicate(v).map_err(|e| e.at(key))).transpose()
}

fn int_bounds(c: &Json, key: &str) -> PResult<IntBounds> {
    match c.get(key) {
        None => Ok(IntBounds::ANY),
        Some(v) => IntBounds::from_value(&v.to_value()).map_err(|e| ParseError::new(e.0).at(key)),
    }
}

fn double_bounds(c: &Json, key: &str) -> PResult<DoubleBounds> {
    match c.get(key) {
        None => Ok(DoubleBounds::ANY),
        Some(v) => DoubleBounds::from_value(&v.to_value()).map_err(|e| ParseError::new(e.0).at(key)),
    }
}

/// A holder set of recipe ids (one id or a list; recipes have no tags).
fn ids(c: &Json, key: &str) -> PResult<Vec<String>> {
    let one = |v: &Json| v.as_str().and_then(kiln_item::Identifier::parse).map(|i| i.to_string());
    match c.get(key) {
        None => err(format!("No key {key}")),
        Some(Json::Arr(list)) => list.iter().map(|v| one(v).ok_or_else(|| ParseError::new(format!("{key}: not an id")))).collect(),
        Some(v) => Ok(vec![one(v).ok_or_else(|| ParseError::new(format!("{key}: not an id")))?]),
    }
}

fn opt_id(c: &Json, key: &str) -> PResult<Option<String>> {
    match c.get(key) {
        None => Ok(None),
        Some(v) => match v.as_str().and_then(kiln_item::Identifier::parse) {
            Some(id) => Ok(Some(id.to_string())),
            None => err(format!("{key}: not a valid identifier")),
        },
    }
}

impl Criterion {
    /// `Criterion.CODEC`: `{trigger, conditions}`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<Criterion> {
        let trigger_id = match j.get("trigger").and_then(Json::as_str).and_then(kiln_item::Identifier::parse) {
            Some(t) => t.to_string(),
            None => return err("missing trigger"),
        };
        if kiln_data::builtin_id("minecraft:trigger_type", &trigger_id).is_none() {
            return err(format!("Unknown registry key in ResourceKey[minecraft:root / minecraft:trigger_type]: {trigger_id}"));
        }
        let empty = Json::Obj(Vec::new());
        let c = j.get("conditions").unwrap_or(&empty);
        let player = opt_cap(p, c, "player")?;
        let trigger = match trigger_id.trim_start_matches("minecraft:") {
            "impossible" => Trigger::Impossible,
            "tick" | "location" | "slept_in_bed" | "hero_of_the_village" | "avoid_vibration" | "started_riding" => Trigger::Player,
            "inventory_changed" => {
                let items = match c.get("items") {
                    Some(Json::Arr(list)) => list.iter().map(predicate::item_predicate).collect::<PResult<Vec<_>>>()?,
                    Some(_) => return err("items: not a list"),
                    None => Vec::new(),
                };
                let empty = Json::Obj(Vec::new());
                let slots = c.get("slots").unwrap_or(&empty);
                Trigger::InventoryChanged {
                    items,
                    occupied: int_bounds(slots, "occupied")?,
                    full: int_bounds(slots, "full")?,
                    empty: int_bounds(slots, "empty")?,
                }
            }
            "recipe_unlocked" => Trigger::RecipeUnlocked { recipes: ids(c, "recipes")? },
            "recipe_crafted" | "crafter_recipe_crafted" => Trigger::RecipeCrafted {
                recipes: ids(c, "recipes")?,
                ingredients: match c.get("ingredients") {
                    Some(Json::Arr(list)) => list.iter().map(predicate::item_predicate).collect::<PResult<Vec<_>>>()?,
                    _ => Vec::new(),
                },
            },
            "player_killed_entity" | "entity_killed_player" => Trigger::Killed {
                entity: opt_cap(p, c, "entity")?,
                killing_blow: c.get("killing_blow").map(|v| DamageSourcePredicate::parse(p, v)).transpose()?,
            },
            "consume_item" => Trigger::ConsumeItem { item: opt_item(c, "item")? },
            "placed_block" | "item_used_on_block" | "default_block_use" | "any_block_use" | "allay_drop_item_on_block" => {
                Trigger::Location { location: opt_cap(p, c, "location")? }
            }
            "enter_block" => Trigger::EnterBlock {
                blocks: c.get("blocks").map(|v| p.id_set(v, kiln_item::registry::BLOCK)).transpose()?,
                state: c.get("state").cloned(),
            },
            "changed_dimension" => Trigger::ChangedDimension { from: opt_id(c, "from")?, to: opt_id(c, "to")? },
            "villager_trade" => Trigger::VillagerTrade { villager: opt_cap(p, c, "villager")?, item: opt_item(c, "item")? },
            "enchanted_item" => Trigger::EnchantedItem { item: opt_item(c, "item")?, levels: int_bounds(c, "levels")? },
            "entity_hurt_player" | "player_hurt_entity" => {
                let empty = Json::Obj(Vec::new());
                let d = c.get("damage").unwrap_or(&empty);
                Trigger::Hurt {
                    dealt: double_bounds(d, "dealt")?,
                    taken: double_bounds(d, "taken")?,
                    blocked: d.get("blocked").and_then(Json::as_bool),
                    source: d.get("type").map(|v| DamageSourcePredicate::parse(p, v)).transpose()?,
                    entity: opt_cap(p, c, "entity")?,
                }
            }
            _ => Trigger::Staged,
        };
        Ok(Criterion { trigger_id, trigger, player })
    }
}

/// What an entity predicate can see of an entity (the player, or the entity a trigger is
/// about).
#[derive(Clone)]
pub(crate) struct Subject<'a> {
    pub type_id: i32,
    pub pos: [f64; 3],
    pub dim: &'static str,
    pub on_ground: bool,
    pub on_fire: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    pub flying: bool,
    pub baby: bool,
    /// Equipment by slot name (`mainhand`, `head`, ...), for players.
    pub equipment: Vec<(&'static str, &'a ItemStack)>,
    /// The world around it, when the trigger runs where the region's chunks are at hand.
    pub world: Option<&'a dyn WorldProbe>,
}

/// World queries for location predicates.
pub(crate) trait WorldProbe {
    fn block(&self, pos: [i32; 3]) -> Option<u16>;
    /// `minecraft:worldgen/biome` name at `pos`.
    fn biome(&self, pos: [i32; 3]) -> Option<&'static str>;
}

impl Subject<'_> {
    /// `EntityPredicate.matches`: parts Kiln cannot evaluate fail.
    pub fn matches(&self, tags: &kiln_loot::tags::Tags, p: &EntityPredicate) -> bool {
        p.parts.iter().all(|part| match part {
            EntitySubPredicate::EntityType(set) => set.contains(self.type_id),
            EntitySubPredicate::Flags(f) => {
                let ok = |want: Option<bool>, have: bool| want.is_none_or(|w| w == have);
                ok(f.is_on_ground, self.on_ground)
                    && ok(f.is_on_fire, self.on_fire)
                    && ok(f.is_sneaking, self.sneaking)
                    && ok(f.is_sprinting, self.sprinting)
                    && ok(f.is_flying, self.flying)
                    && ok(f.is_baby, self.baby)
                    && ok(f.is_swimming, false)
                    && ok(f.is_fall_flying, false)
                    && f.is_in_water.is_none()
            }
            EntitySubPredicate::Location(l) => location_matches(l, self.pos, self.dim, self.world),
            EntitySubPredicate::SteppingOn(l) => location_matches(l, [self.pos[0], self.pos[1] - 1e-5, self.pos[2]], self.dim, self.world),
            EntitySubPredicate::Equipment(slots) => slots.iter().all(|(slot, ip)| {
                self.equipment.iter().find(|(s, _)| s == slot).is_some_and(|(_, st)| predicate::item_matches(tags, ip, st))
            }),
            _ => false,
        })
    }
}

fn bounds(b: &DoubleBounds, v: f64) -> bool {
    b.min.is_none_or(|m| m <= v) && b.max.is_none_or(|m| v <= m)
}

/// `LocationPredicate.matches`: structures, light, fluids, smoke and sky access are not
/// evaluated (a predicate using them fails).
pub(crate) fn location_matches(l: &LocationPredicate, pos: [f64; 3], dim: &str, world: Option<&dyn WorldProbe>) -> bool {
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
        let Some(id) = kiln_item::Identifier::parse(b) else { return false };
        if !biomes.contains(&id) {
            return false;
        }
    }
    if let Some(block) = &l.block {
        let Some(state) = world.and_then(|w| w.block(block_pos)) else { return false };
        if !block.matches_state(state) {
            return false;
        }
    }
    l.structures.is_none() && l.light.is_none() && l.fluid.is_none() && l.smokey.is_none() && l.can_see_sky.is_none()
}

/// The loot context a criterion's conditions see: `this` is the subject, at its position;
/// optionally a block and tool (block triggers).
pub(crate) struct TriggerCtx<'a> {
    pub this: &'a Subject<'a>,
    pub tags: &'a kiln_loot::tags::Tags,
    /// For location conditions of block triggers: the block position's center.
    pub origin: Option<[f64; 3]>,
    pub block: Option<u16>,
    pub tool: Option<&'a ItemStack>,
}

impl LootContext for TriggerCtx<'_> {
    fn has_entity(&self, target: EntityTarget) -> bool {
        target == EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin.unwrap_or(self.this.pos))
    }
    fn block_state(&self) -> Option<u16> {
        self.block
    }
    fn tool(&self) -> Option<&ItemStack> {
        self.tool
    }
    fn entity_matches(&self, target: EntityTarget, predicate: &EntityPredicate) -> bool {
        target == EntityTarget::This && self.this.matches(self.tags, predicate)
    }
    fn location_matches(&self, predicate: &LocationPredicate, pos: [f64; 3]) -> bool {
        location_matches(predicate, pos, self.this.dim, self.this.world)
    }
}

/// `ContextAwarePredicate.matches`: every condition holds. Random conditions draw from a
/// fixed seed (no vanilla criterion uses them).
pub(crate) fn test_cap(loot: &LootData, cap: &Cap, ctx: &TriggerCtx) -> bool {
    let mut rng = kiln_javamath::random::LegacyRandom::new(0);
    let mut eval = kiln_loot::Eval::new(loot, ctx, &mut rng);
    cap.iter().all(|c| eval.test(c))
}

/// `InventoryChangeTrigger.TriggerInstance.matches`.
pub(crate) fn inventory_matches(
    tags: &kiln_loot::tags::Tags,
    t: &Trigger,
    items: &[ItemStack],
    changed: &ItemStack,
    counts: (i32, i32, i32),
) -> bool {
    let Trigger::InventoryChanged { items: preds, occupied, full, empty } = t else { return false };
    let (f, e, o) = counts;
    if !(predicate::item::int_bounds(full, f) && predicate::item::int_bounds(empty, e) && predicate::item::int_bounds(occupied, o)) {
        return false;
    }
    match preds.as_slice() {
        [] => true,
        [one] => !changed.is_empty() && predicate::item_matches(tags, one, changed),
        many => {
            let mut left: Vec<&ItemPredicate> = many.iter().collect();
            if !changed.is_empty() {
                left.retain(|p| !predicate::item_matches(tags, p, changed));
            }
            for s in items {
                if left.is_empty() {
                    return true;
                }
                if !s.is_empty() {
                    left.retain(|p| !predicate::item_matches(tags, p, s));
                }
            }
            left.is_empty()
        }
    }
}
