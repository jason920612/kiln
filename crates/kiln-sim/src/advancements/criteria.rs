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
    /// The other triggers Kiln fires: their conditions by key (entity conditions, entity
    /// condition lists, item predicates, the rest as written), tested where they fire.
    Conds(Conds),
    /// A trigger Kiln does not fire (yet).
    Staged,
}

/// A trigger's conditions by key.
#[derive(Debug, Clone)]
pub(crate) struct Conds {
    pub caps: Vec<(String, Cap)>,
    pub cap_lists: Vec<(String, Vec<Cap>)>,
    pub items: Vec<(String, ItemPredicate)>,
    pub locations: Vec<(String, LocationPredicate)>,
    pub json: Json,
}

/// Condition keys that hold a `ContextAwarePredicate` about an entity.
const ENTITY_KEYS: &[&str] = &["entity", "parent", "partner", "child", "zombie", "villager", "source", "projectile", "lightning", "cause"];
/// Keys that hold a list of them.
const ENTITY_LIST_KEYS: &[&str] = &["victims", "bystander"];
/// Keys that hold an item predicate.
const ITEM_KEYS: &[&str] = &["item", "fired_from_weapon", "rod"];
/// Keys that hold a location predicate.
const LOCATION_KEYS: &[&str] = &["start_position"];

impl Conds {
    fn parse(p: &Parser, c: &Json) -> PResult<Conds> {
        let mut out = Conds { caps: Vec::new(), cap_lists: Vec::new(), items: Vec::new(), locations: Vec::new(), json: c.clone() };
        let Some(fields) = c.as_object() else { return Ok(out) };
        for (k, v) in fields {
            if ENTITY_KEYS.contains(&k.as_str()) {
                out.caps.push((k.clone(), cap(p, v).map_err(|e| e.at(k))?));
            } else if ENTITY_LIST_KEYS.contains(&k.as_str()) {
                let list = match v {
                    Json::Arr(items) => items.iter().map(|i| cap(p, i)).collect::<PResult<Vec<_>>>(),
                    // `bystander` is one condition.
                    _ => cap(p, v).map(|c| vec![c]),
                };
                out.cap_lists.push((k.clone(), list.map_err(|e| e.at(k))?));
            } else if ITEM_KEYS.contains(&k.as_str()) {
                out.items.push((k.clone(), predicate::item_predicate(v).map_err(|e| e.at(k))?));
            } else if LOCATION_KEYS.contains(&k.as_str()) {
                out.locations.push((k.clone(), LocationPredicate::parse(p, v).map_err(|e| e.at(k))?));
            }
        }
        Ok(out)
    }

    pub fn cap(&self, key: &str) -> Option<&Cap> {
        self.caps.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    pub fn cap_list(&self, key: &str) -> Option<&[Cap]> {
        self.cap_lists.iter().find(|(k, _)| k == key).map(|(_, c)| c.as_slice())
    }

    pub fn location(&self, key: &str) -> Option<&LocationPredicate> {
        self.locations.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    pub fn item(&self, key: &str) -> Option<&ItemPredicate> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.json.get(key)
    }

    /// An `IntBounds` condition (absent: any).
    pub fn ints(&self, key: &str) -> IntBounds {
        self.get(key).and_then(|v| IntBounds::from_value(&v.to_value()).ok()).unwrap_or(IntBounds::ANY)
    }

    /// A `DistancePredicate` (`x`, `y`, `z`, `horizontal`, `absolute`) between two points;
    /// absent: any.
    pub fn distance(&self, key: &str, a: [f64; 3], b: [f64; 3]) -> bool {
        self.get(key).is_none_or(|d| distance_matches(d, a, b))
    }
}

/// `DistancePredicate.matches`: per-axis distances, the horizontal and the absolute one.
pub(crate) fn distance_matches(d: &Json, a: [f64; 3], b: [f64; 3]) -> bool {
    let b_of = |k: &str| d.get(k).and_then(|v| DoubleBounds::from_value(&v.to_value()).ok()).unwrap_or(DoubleBounds::ANY);
    let (dx, dy, dz) = ((a[0] - b[0]) as f32, (a[1] - b[1]) as f32, (a[2] - b[2]) as f32);
    let sq = |bd: &DoubleBounds, v: f64| bd.min.is_none_or(|m| m * m <= v) && bd.max.is_none_or(|m| v <= m * m);
    bounds(&b_of("x"), dx.abs() as f64)
        && bounds(&b_of("y"), dy.abs() as f64)
        && bounds(&b_of("z"), dz.abs() as f64)
        && sq(&b_of("horizontal"), (dx * dx + dz * dz) as f64)
        && sq(&b_of("absolute"), (dx * dx + dy * dy + dz * dz) as f64)
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
    "minecraft:player_hurt_entity",
    "minecraft:bred_animals",
    "minecraft:tame_animal",
    "minecraft:effects_changed",
    "minecraft:levitation",
    "minecraft:used_totem",
    "minecraft:player_interacted_with_entity",
    "minecraft:item_durability_changed",
    "minecraft:fall_from_height",
    "minecraft:nether_travel",
    "minecraft:ride_entity_in_lava",
    "minecraft:lightning_strike",
    "minecraft:cured_zombie_villager",
    "minecraft:player_generates_container_loot",
    "minecraft:brewed_potion",
    "minecraft:fishing_rod_hooked",
    "minecraft:construct_beacon",
    "minecraft:slept_in_bed",
    "minecraft:filled_bucket",
    "minecraft:shot_crossbow",
    "minecraft:using_item",
    "minecraft:killed_by_arrow",
    "minecraft:channeled_lightning",
    "minecraft:summoned_entity",
    "minecraft:slide_down_block",
    "minecraft:avoid_vibration",
    "minecraft:kill_mob_near_sculk_catalyst",
    "minecraft:target_hit",
    "minecraft:player_sheared_equipment",
    "minecraft:fall_after_explosion",
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
            "tick" | "location" | "slept_in_bed" | "hero_of_the_village" | "avoid_vibration" | "started_riding" | "voluntary_exile" => Trigger::Player,
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
            "player_killed_entity" | "entity_killed_player" | "kill_mob_near_sculk_catalyst" => Trigger::Killed {
                entity: opt_cap(p, c, "entity")?,
                killing_blow: c.get("killing_blow").map(|v| DamageSourcePredicate::parse(p, v)).transpose()?,
            },
            "consume_item" => Trigger::ConsumeItem { item: opt_item(c, "item")? },
            "placed_block" | "item_used_on_block" | "default_block_use" | "any_block_use" | "allay_drop_item_on_block" => {
                Trigger::Location { location: opt_cap(p, c, "location")? }
            }
            "enter_block" | "slide_down_block" => Trigger::EnterBlock {
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
            _ if FIRED.contains(&trigger_id.as_str()) => Trigger::Conds(Conds::parse(p, c)?),
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
    /// Entity data components (a mob's variant) for `components`.
    pub components: std::borrow::Cow<'a, [kiln_item::Component]>,
    /// Active effects: (`minecraft:mob_effect` network id, amplifier, duration, ambient,
    /// visible).
    pub effects: Vec<(i32, i32, i32, bool, bool)>,
    /// The type of the entity it rides.
    pub vehicle: Option<&'static str>,
    /// A lightning bolt's `blocksSetOnFire` (`type_specific/lightning`).
    pub lightning_fires: Option<i32>,
}

/// World queries for location predicates.
pub(crate) trait WorldProbe {
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

impl Subject<'_> {
    /// `EntityPredicate.matches` with `origin` the context's origin (the player, for
    /// criteria): parts Kiln cannot evaluate fail.
    pub fn matches(&self, tags: &kiln_loot::tags::Tags, p: &EntityPredicate, origin: [f64; 3]) -> bool {
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
            EntitySubPredicate::Components(list) => list.iter().all(|c| self.components.contains(c)),
            EntitySubPredicate::Vehicle(v) => {
                // Kiln knows the vehicle's type: predicates on anything more about it fail.
                let Some(t) = self.vehicle else { return false };
                let type_id = kiln_item::registry::ENTITY_TYPE.id(t).unwrap_or(-1);
                v.parts.iter().all(|part| matches!(part, EntitySubPredicate::EntityType(set) if set.contains(type_id)))
            }
            EntitySubPredicate::Other(id, j) => match id.as_str() {
                "minecraft:distance" => distance_matches(j, origin, self.pos),
                "minecraft:effects" => effects_match(j, &self.effects),
                // `LightningBoltPredicate`: blocks set on fire (struck entities are not known).
                "minecraft:type_specific/lightning" => {
                    self.lightning_fires.is_some_and(|n| {
                        let b = j.get("blocks_set_on_fire").and_then(|v| IntBounds::from_value(&v.to_value()).ok()).unwrap_or(IntBounds::ANY);
                        predicate::item::int_bounds(&b, n)
                    }) && j.get("entity_struck").is_none()
                }
                _ => false,
            },
            _ => false,
        })
    }
}

/// `MobEffectsPredicate.matches`: every listed effect is active with its amplifier and
/// duration in bounds and the ambient and visible flags as given.
pub(crate) fn effects_match(j: &Json, effects: &[(i32, i32, i32, bool, bool)]) -> bool {
    let Some(fields) = j.as_object() else { return false };
    fields.iter().all(|(name, want)| {
        let Some(id) = kiln_data::synced_id("minecraft:mob_effect", name).or_else(|| kiln_data::builtin_id("minecraft:mob_effect", name)) else {
            return false;
        };
        let Some(&(_, amplifier, duration, ambient, visible)) = effects.iter().find(|e| e.0 == id) else { return false };
        let ints = |k: &str| want.get(k).and_then(|v| IntBounds::from_value(&v.to_value()).ok()).unwrap_or(IntBounds::ANY);
        predicate::item::int_bounds(&ints("amplifier"), amplifier)
            && predicate::item::int_bounds(&ints("duration"), duration)
            && want.get("ambient").and_then(Json::as_bool).is_none_or(|a| a == ambient)
            && want.get("visible").and_then(Json::as_bool).is_none_or(|v| v == visible)
    })
}

fn bounds(b: &DoubleBounds, v: f64) -> bool {
    b.min.is_none_or(|m| m <= v) && b.max.is_none_or(|m| v <= m)
}

/// `LocationPredicate.matches`: position, dimension, then (where the chunks are at hand)
/// biome, structure, light, block, fluid and sky access. Smoke from campfires is not
/// evaluated (a predicate asking fails).
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
fn fluid_matches(j: &Json, state: u16) -> bool {
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
                kiln_data::registries::TAGS
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
        target == EntityTarget::This && self.this.matches(self.tags, predicate, self.origin.unwrap_or(self.this.pos))
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
