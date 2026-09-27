//! What a loot evaluation reads from the world: the [`LootContext`] trait.
//!
//! Vanilla's `LootContext` wraps a `LootParams` (a map of `LootContextParams` to values) and the
//! server level. Kiln keeps loot independent of the simulation, so every parameter and every
//! world query goes through this trait; each method's default is what vanilla does when the
//! parameter is absent, so a caller implements only what its table kind provides.

use crate::json::Json;
use crate::predicate::{BlockEntityPredicate, DamageSourcePredicate, EntityPredicate, LocationPredicate};
use kiln_command::nbt_path::NbtPath;
use kiln_item::component::EquipmentSlotGroup;
use kiln_item::{Component, Identifier, ItemStack, Text};
use kiln_proto::nbt::Tag;

/// `LootContext.EntityTarget`: which entity parameter a condition or function reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityTarget {
    /// `this_entity`.
    This,
    /// `attacking_entity`.
    Attacker,
    /// `direct_attacking_entity`.
    DirectAttacker,
    /// `last_damage_player`.
    AttackingPlayer,
    /// `target_entity`.
    TargetEntity,
    /// `interacting_entity`.
    InteractingEntity,
}

impl EntityTarget {
    pub const ALL: [EntityTarget; 6] = [
        EntityTarget::This,
        EntityTarget::Attacker,
        EntityTarget::DirectAttacker,
        EntityTarget::AttackingPlayer,
        EntityTarget::TargetEntity,
        EntityTarget::InteractingEntity,
    ];

    /// The serialized name (`this`, `attacker`, ...).
    pub fn name(self) -> &'static str {
        match self {
            EntityTarget::This => "this",
            EntityTarget::Attacker => "attacker",
            EntityTarget::DirectAttacker => "direct_attacker",
            EntityTarget::AttackingPlayer => "attacking_player",
            EntityTarget::TargetEntity => "target_entity",
            EntityTarget::InteractingEntity => "interacting_entity",
        }
    }

    pub fn by_name(name: &str) -> Option<EntityTarget> {
        EntityTarget::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// A context value that `copy_name` and `copy_components` read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Entity(EntityTarget),
    BlockEntity,
    /// The `tool` parameter.
    Tool,
}

impl Source {
    pub fn by_name(name: &str) -> Option<Source> {
        match name {
            "block_entity" => Some(Source::BlockEntity),
            "tool" => Some(Source::Tool),
            _ => EntityTarget::by_name(name).map(Source::Entity),
        }
    }
}

/// Who a score belongs to (`ScoreHolder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreHolder<'a> {
    /// `ScoreHolder.forNameOnly`.
    Name(&'a str),
    Entity(EntityTarget),
}

/// A numeric NBT tag (`NumericTag.box()`), converted the way `Number` does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
}

impl Number {
    pub fn from_tag(tag: &Tag) -> Option<Number> {
        Some(match *tag {
            Tag::Byte(v) => Number::Byte(v),
            Tag::Short(v) => Number::Short(v),
            Tag::Int(v) => Number::Int(v),
            Tag::Long(v) => Number::Long(v),
            Tag::Float(v) => Number::Float(v),
            Tag::Double(v) => Number::Double(v),
            _ => return None,
        })
    }

    /// `Number.intValue()`.
    pub fn int_value(self) -> i32 {
        match self {
            Number::Byte(v) => v as i32,
            Number::Short(v) => v as i32,
            Number::Int(v) => v,
            Number::Long(v) => v as i32,
            Number::Float(v) => v as i32,
            Number::Double(v) => v as i32,
        }
    }

    /// `Number.floatValue()`.
    pub fn float_value(self) -> f32 {
        match self {
            Number::Byte(v) => v as f32,
            Number::Short(v) => v as f32,
            Number::Int(v) => v as f32,
            Number::Long(v) => v as f32,
            Number::Float(v) => v,
            Number::Double(v) => v as f32,
        }
    }
}

/// The request of an `exploration_map` function.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplorationMap<'a> {
    /// `minecraft:worldgen/structure` entries to search for.
    pub destination: &'a crate::parse::NameSet,
    /// `minecraft:map_decoration_type` network id.
    pub decoration: i32,
    pub zoom: i8,
    pub search_radius: i32,
    pub skip_existing_chunks: bool,
}

/// Where a `slot_range` slot source reads slots from (`SlotProvider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOwner {
    Entity(EntityTarget),
    BlockEntity,
    /// The `container` parameter.
    Container,
}

/// World access for loot evaluation. Every method has vanilla's behaviour for a missing
/// parameter as its default.
pub trait LootContext {
    // ---- parameters -----------------------------------------------------------------------

    /// Whether the entity parameter of `target` is present.
    fn has_entity(&self, _target: EntityTarget) -> bool {
        false
    }

    /// `origin`.
    fn origin(&self) -> Option<[f64; 3]> {
        None
    }

    /// `block_state` (a block state id).
    fn block_state(&self) -> Option<u16> {
        None
    }

    /// Whether `block_entity` is present.
    fn has_block_entity(&self) -> bool {
        false
    }

    /// `tool`.
    fn tool(&self) -> Option<&ItemStack> {
        None
    }

    /// `explosion_radius`.
    fn explosion_radius(&self) -> Option<f32> {
        None
    }

    /// Whether `damage_source` is present.
    fn has_damage_source(&self) -> bool {
        false
    }

    /// `enchantment_level`.
    fn enchantment_level(&self) -> Option<i32> {
        None
    }

    /// `enchantment_active`.
    fn enchantment_active(&self) -> Option<bool> {
        None
    }

    /// Whether `additional_cost_component_allowed` is present.
    fn additional_cost_component_allowed(&self) -> bool {
        false
    }

    /// `LootParams.getLuck()`.
    fn luck(&self) -> f32 {
        0.0
    }

    // ---- entities -------------------------------------------------------------------------

    /// `EnchantmentHelper.getEnchantmentLevel(enchantment, entity)`: the highest level of the
    /// enchantment (a `minecraft:enchantment` id) among the entity's equipment in `slots`;
    /// 0 when the entity is absent or not a `LivingEntity`.
    fn entity_enchantment_level(&self, _target: EntityTarget, _enchantment: i32, _slots: &[EquipmentSlotGroup]) -> i32 {
        0
    }

    /// `EntityPredicate.matches(level, origin, entity)` for the entity of `target` (present);
    /// the hook the simulation implements.
    fn entity_matches(&self, _target: EntityTarget, _predicate: &EntityPredicate) -> bool {
        false
    }

    /// The value of `objective` for `holder`, if the objective exists and the holder has a
    /// score in it.
    fn score(&self, _holder: &ScoreHolder<'_>, _objective: &str) -> Option<i32> {
        None
    }

    /// The resolved game profile of the entity of `target` when it is a player
    /// (`ResolvableProfile.createResolved(player.getGameProfile())`).
    fn player_profile(&self, _target: EntityTarget) -> Option<kiln_item::component::ResolvableProfile> {
        None
    }

    /// For a `Nameable` source: `Some(custom name)`; `None` when the source is absent or not
    /// nameable.
    fn custom_name(&self, _source: Source) -> Option<Option<Text>> {
        None
    }

    /// The data components of a source (`DataComponentGetter`): the block entity's
    /// `collectComponents()`, or the effective components of an entity or the tool. `None`
    /// when absent.
    fn components(&self, _source: Source) -> Option<Vec<Component>> {
        None
    }

    /// Whether the components of `source` came from a `DataComponentMap` (block entities), which
    /// `copy_components` applies as a whole instead of type by type.
    fn components_are_map(&self, source: Source) -> bool {
        source == Source::BlockEntity
    }

    /// NBT of an entity (`NbtPredicate.getEntityTagToCompare`) or the block entity
    /// (`saveWithFullMetadata`), for `copy_custom_data`.
    fn nbt(&self, _source: Source) -> Option<Tag> {
        None
    }

    /// `ComponentUtils.resolve` with the entity of `target` as the command source; the
    /// default leaves the text as is (which is what resolution does to text without
    /// score, selector or NBT parts).
    fn resolve_text(&self, _target: EntityTarget, text: &Text) -> Text {
        text.clone()
    }

    /// Items of a named slot range (`SlotRanges`, such as `container.*` or `armor.head`) of the
    /// owner's `SlotProvider`; empty when the owner is not one.
    fn slot_items(&self, _owner: SlotOwner, _range: &str) -> Vec<ItemStack> {
        Vec::new()
    }

    // ---- damage source and blocks ---------------------------------------------------------

    /// `DamageSourcePredicate.matches(level, origin, source)` (the damage source and origin
    /// are present).
    fn damage_source_matches(&self, _predicate: &DamageSourcePredicate) -> bool {
        false
    }

    /// `BlockPredicate.matchesBlockEntity`: the NBT and component parts of a `match_block`
    /// predicate against the block entity (`None` when absent).
    fn block_entity_matches(&self, _predicate: &BlockEntityPredicate) -> bool {
        false
    }

    // ---- world ----------------------------------------------------------------------------

    fn is_raining(&self) -> bool {
        false
    }

    fn is_thundering(&self) -> bool {
        false
    }

    /// `ServerClockManager.getInstance(clock).totalTicks()`.
    fn clock_total_ticks(&self, _clock: &Identifier) -> i64 {
        0
    }

    /// `LocationPredicate.matches(level, x, y, z)`.
    fn location_matches(&self, _predicate: &LocationPredicate, _pos: [f64; 3]) -> bool {
        false
    }

    /// Whether `EnvironmentAttributeSystem.getValue(context, attribute)` equals `value` (the
    /// value as written in the condition).
    fn environment_attribute_equals(&self, _attribute: &Identifier, _value: &Json) -> bool {
        false
    }

    /// `EnvironmentAttributeValue.getAsInt`; `None` fails the provider (arithmetic error).
    fn environment_attribute_int(&self, _attribute: &Identifier) -> Option<i32> {
        None
    }

    /// `EnvironmentAttributeValue.getAsFloat`.
    fn environment_attribute_float(&self, _attribute: &Identifier) -> Option<f32> {
        None
    }

    /// Command storage (`CommandStorage.get`): `None` for storage that was never written.
    fn storage(&self, _id: &Identifier) -> Option<Tag> {
        None
    }

    /// `StoredNumberAccess.getNumericTag`: the single numeric tag at `path`.
    fn storage_number(&self, id: &Identifier, path: &NbtPath) -> Option<Number> {
        let root = self.storage(id).unwrap_or(Tag::Compound(Vec::new()));
        match path.get(&root).as_slice() {
            [one] => Number::from_tag(one),
            _ => None,
        }
    }

    /// The result of the first smelting recipe matching `input`
    /// (`RecipeManager.getRecipeFor(SMELTING, ...)` + `assemble`).
    fn smelt(&self, _input: &ItemStack) -> Option<ItemStack> {
        None
    }

    /// Carries out an `exploration_map` function: `None` when no structure was found (the
    /// stack is kept as is).
    fn exploration_map(&self, _stack: &ItemStack, _request: &ExplorationMap<'_>) -> Option<ItemStack> {
        None
    }

    /// Items of a `dynamic` entry (`LootParams.addDynamicDrops`), such as a shulker box's
    /// contents.
    fn dynamic_drops(&self, _name: &Identifier, _sink: &mut dyn FnMut(ItemStack)) {}

    /// `ItemStack.isItemEnabled(level.enabledFeatures())`.
    fn item_enabled(&self, _item: i32) -> bool {
        true
    }
}

/// A context with no parameters.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyContext;

impl LootContext for EmptyContext {}
