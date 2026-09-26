//! Data components: one typed value per component type, each with vanilla's network codec
//! (`DataComponentType.streamCodec()`) and persistent codec (`codec()`, as a [`Value`]).

pub mod basic;
pub mod book;
pub mod common;
pub mod combat;
pub mod consume;
pub mod container;
pub mod enchant;
pub mod misc;
pub mod predicate;
pub mod variant;

pub use basic::*;
pub use book::*;
pub use common::*;
pub use combat::*;
pub use consume::*;
pub use container::*;
pub use enchant::*;
pub use misc::*;
pub use predicate::*;
pub use variant::*;

/// Generated component type ids (`ids::DAMAGE`, ...) and names.
pub use crate::generated::components as ids;
use crate::ident::Identifier;
use crate::text::Text;
use crate::value::{DataError, DataResult, Value};
use crate::wire::{self, WireResult};
use bytes::BytesMut;
use kiln_proto::{DecodeError, Reader, WriteExt};
use std::marker::PhantomData;

/// Network id of a data component type (its index in `minecraft:data_component_type`).
pub type ComponentId = u16;

/// A component payload: its network codec and its persistent codec.
pub trait ComponentValue: Sized {
    fn read(r: &mut Reader<'_>) -> WireResult<Self>;
    fn write(&self, out: &mut BytesMut);
    fn to_value(&self) -> Value;
    fn from_value(v: &Value) -> DataResult<Self>;
}

/// `read` for components whose network codec is their persistent codec as NBT
/// (`ByteBufCodecs.fromCodecWithRegistries`, the default when no stream codec is given).
pub fn read_via_nbt<T: ComponentValue>(r: &mut Reader<'_>) -> WireResult<T> {
    T::from_value(&wire::read_value(r)?).map_err(|_| DecodeError::Invalid("component NBT"))
}

pub fn write_via_nbt<T: ComponentValue>(value: &T, out: &mut BytesMut) {
    wire::write_value(out, &value.to_value());
}

/// Placeholder payload for a component without a typed model: it cannot be decoded (the
/// network codec is not length-delimited, so the value cannot be skipped either).
#[derive(Debug, Clone, PartialEq)]
pub enum Unsupported {}

impl ComponentValue for Unsupported {
    fn read(_: &mut Reader<'_>) -> WireResult<Self> {
        Err(DecodeError::Invalid("unsupported data component"))
    }
    fn write(&self, _: &mut BytesMut) {
        match *self {}
    }
    fn to_value(&self) -> Value {
        match *self {}
    }
    fn from_value(_: &Value) -> DataResult<Self> {
        Err(DataError("unsupported data component".into()))
    }
}

/// `VAR_INT` on the network, an int in NBT.
impl ComponentValue for i32 {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.varint()
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_varint(*self);
    }
    fn to_value(&self) -> Value {
        Value::Int(*self)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_i32()
    }
}

/// `FLOAT` on the network, a float in NBT.
impl ComponentValue for f32 {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.f32()
    }
    fn write(&self, out: &mut BytesMut) {
        bytes::BufMut::put_f32(out, *self);
    }
    fn to_value(&self) -> Value {
        Value::Float(*self)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_f32()
    }
}

impl ComponentValue for bool {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        r.bool()
    }
    fn write(&self, out: &mut BytesMut) {
        out.put_bool(*self);
    }
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_bool()
    }
}

/// `Unit`: nothing on the network, `{}` in NBT.
impl ComponentValue for () {
    fn read(_: &mut Reader<'_>) -> WireResult<Self> {
        Ok(())
    }
    fn write(&self, _: &mut BytesMut) {}
    fn to_value(&self) -> Value {
        Value::empty_map()
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        v.as_map().map(|_| ())
    }
}

impl ComponentValue for Identifier {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Identifier::read(r)
    }
    fn write(&self, out: &mut BytesMut) {
        Identifier::write(self, out);
    }
    fn to_value(&self) -> Value {
        Identifier::to_value(self)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Identifier::from_value(v)
    }
}

impl ComponentValue for Text {
    fn read(r: &mut Reader<'_>) -> WireResult<Self> {
        Text::read(r)
    }
    fn write(&self, out: &mut BytesMut) {
        Text::write(self, out);
    }
    fn to_value(&self) -> Value {
        Text::to_value(self)
    }
    fn from_value(v: &Value) -> DataResult<Self> {
        Text::from_value(v)
    }
}

/// A typed accessor for one component type (see [`keys`]).
pub struct Key<T> {
    pub id: ComponentId,
    get: fn(&Component) -> Option<&T>,
    wrap: fn(T) -> Component,
    _t: PhantomData<fn() -> T>,
}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Key<T> {}

impl<T> Key<T> {
    pub fn get<'a>(&self, c: &'a Component) -> Option<&'a T> {
        (self.get)(c)
    }

    pub fn wrap(&self, value: T) -> Component {
        (self.wrap)(value)
    }
}

macro_rules! components {
    ($($variant:ident($id:ident, $ty:ty),)*) => {
        /// A data component value, tagged with its type.
        #[derive(Debug, Clone, PartialEq)]
        pub enum Component {
            $($variant($ty),)*
        }

        impl Component {
            pub fn id(&self) -> ComponentId {
                match self {
                    $(Component::$variant(_) => ids::$id,)*
                }
            }

            /// Decodes a value of type `id` with its network codec.
            pub fn read(id: ComponentId, r: &mut Reader<'_>) -> WireResult<Component> {
                match id {
                    $(ids::$id => <$ty as ComponentValue>::read(r).map(Component::$variant),)*
                    _ => Err(DecodeError::Invalid("unknown data component type")),
                }
            }

            pub fn write(&self, out: &mut BytesMut) {
                match self {
                    $(Component::$variant(v) => v.write(out),)*
                }
            }

            /// The persistent form; `None` for transient components (not saved, not hashed).
            pub fn to_value(&self) -> Option<Value> {
                if !is_persistent(self.id()) {
                    return None;
                }
                Some(match self {
                    $(Component::$variant(v) => v.to_value(),)*
                })
            }

            pub fn from_value(id: ComponentId, v: &Value) -> DataResult<Component> {
                match id {
                    _ if !is_persistent(id) => Err(DataError(format!("{} is transient", name(id)))),
                    $(ids::$id => <$ty as ComponentValue>::from_value(v).map(Component::$variant),)*
                    _ => Err(DataError(format!("unknown data component type {id}"))),
                }
            }
        }

        /// Whether the component type has a typed model.
        pub fn is_supported(id: ComponentId) -> bool {
            match id {
                $(ids::$id => !std::any::type_name::<$ty>().ends_with("Unsupported"),)*
                _ => false,
            }
        }

        /// Typed accessors: `stack.get(keys::DAMAGE)`.
        #[allow(non_upper_case_globals)]
        pub mod keys {
            use super::*;
            $(
                pub const $id: Key<$ty> = Key {
                    id: ids::$id,
                    get: |c| match c { Component::$variant(v) => Some(v), #[allow(unreachable_patterns)] _ => None },
                    wrap: Component::$variant,
                    _t: PhantomData,
                };
            )*
        }

        #[cfg(test)]
        pub(crate) const MODELLED: &[ComponentId] = &[$(ids::$id,)*];
    };
}

components! {
    CustomData(CUSTOM_DATA, CustomData),
    MaxStackSize(MAX_STACK_SIZE, i32),
    MaxDamage(MAX_DAMAGE, i32),
    Damage(DAMAGE, i32),
    Unbreakable(UNBREAKABLE, ()),
    UseEffects(USE_EFFECTS, UseEffects),
    CustomName(CUSTOM_NAME, Text),
    MinimumAttackCharge(MINIMUM_ATTACK_CHARGE, f32),
    DamageType(DAMAGE_TYPE, DamageTypeRef),
    ItemName(ITEM_NAME, Text),
    ItemModel(ITEM_MODEL, Identifier),
    Lore(LORE, Lore),
    Rarity(RARITY, Rarity),
    Enchantments(ENCHANTMENTS, Enchantments),
    CanPlaceOn(CAN_PLACE_ON, AdventureModePredicate),
    CanBreak(CAN_BREAK, AdventureModePredicate),
    AttributeModifiers(ATTRIBUTE_MODIFIERS, AttributeModifiers),
    CustomModelData(CUSTOM_MODEL_DATA, CustomModelData),
    TooltipDisplay(TOOLTIP_DISPLAY, TooltipDisplay),
    RepairCost(REPAIR_COST, i32),
    CreativeSlotLock(CREATIVE_SLOT_LOCK, ()),
    EnchantmentGlintOverride(ENCHANTMENT_GLINT_OVERRIDE, bool),
    IntangibleProjectile(INTANGIBLE_PROJECTILE, IntangibleProjectile),
    Food(FOOD, Food),
    Consumable(CONSUMABLE, Consumable),
    UseRemainder(USE_REMAINDER, UseRemainder),
    UseCooldown(USE_COOLDOWN, UseCooldown),
    DamageResistant(DAMAGE_RESISTANT, DamageResistant),
    Tool(TOOL, Tool),
    Weapon(WEAPON, Weapon),
    AttackRange(ATTACK_RANGE, AttackRange),
    Enchantable(ENCHANTABLE, Enchantable),
    Equippable(EQUIPPABLE, Equippable),
    Repairable(REPAIRABLE, Repairable),
    Glider(GLIDER, ()),
    TooltipStyle(TOOLTIP_STYLE, Identifier),
    DeathProtection(DEATH_PROTECTION, DeathProtection),
    BlocksAttacks(BLOCKS_ATTACKS, BlocksAttacks),
    PiercingWeapon(PIERCING_WEAPON, PiercingWeapon),
    KineticWeapon(KINETIC_WEAPON, KineticWeapon),
    AttackAnimation(ATTACK_ANIMATION, SwingAnimation),
    InteractAnimation(INTERACT_ANIMATION, SwingAnimation),
    AdditionalTradeCost(ADDITIONAL_TRADE_COST, i32),
    BlockTransformer(BLOCK_TRANSFORMER, BlockTransformerRef),
    VillagerFood(VILLAGER_FOOD, VillagerFood),
    StoredEnchantments(STORED_ENCHANTMENTS, Enchantments),
    Dye(DYE, DyeColor),
    DyedColor(DYED_COLOR, DyedColor),
    MapId(MAP_ID, MapId),
    MapDecorations(MAP_DECORATIONS, MapDecorations),
    MapPostProcessing(MAP_POST_PROCESSING, MapPostProcessing),
    ChargedProjectiles(CHARGED_PROJECTILES, ChargedProjectiles),
    BundleContents(BUNDLE_CONTENTS, BundleContents),
    PotionContents(POTION_CONTENTS, PotionContents),
    PotionDurationScale(POTION_DURATION_SCALE, f32),
    SuspiciousStewEffects(SUSPICIOUS_STEW_EFFECTS, SuspiciousStewEffects),
    WritableBookContent(WRITABLE_BOOK_CONTENT, WritableBookContent),
    WrittenBookContent(WRITTEN_BOOK_CONTENT, WrittenBookContent),
    Trim(TRIM, ArmorTrim),
    DebugStickState(DEBUG_STICK_STATE, DebugStickState),
    EntityData(ENTITY_DATA, EntityData),
    BucketEntityData(BUCKET_ENTITY_DATA, CustomData),
    BlockEntityData(BLOCK_ENTITY_DATA, BlockEntityData),
    Instrument(INSTRUMENT, InstrumentComponent),
    ProvidesTrimMaterial(PROVIDES_TRIM_MATERIAL, ProvidesTrimMaterial),
    OminousBottleAmplifier(OMINOUS_BOTTLE_AMPLIFIER, OminousBottleAmplifier),
    JukeboxPlayable(JUKEBOX_PLAYABLE, JukeboxPlayable),
    ProvidesBannerPatterns(PROVIDES_BANNER_PATTERNS, ProvidesBannerPatterns),
    Recipes(RECIPES, Recipes),
    LodestoneTracker(LODESTONE_TRACKER, LodestoneTracker),
    FireworkExplosion(FIREWORK_EXPLOSION, FireworkExplosion),
    Fireworks(FIREWORKS, Fireworks),
    Profile(PROFILE, ResolvableProfile),
    NoteBlockSound(NOTE_BLOCK_SOUND, Identifier),
    BannerPatterns(BANNER_PATTERNS, BannerPatternLayers),
    BaseColor(BASE_COLOR, DyeColor),
    PotDecorations(POT_DECORATIONS, PotDecorations),
    Container(CONTAINER, ItemContainerContents),
    BlockState(BLOCK_STATE, BlockItemStateProperties),
    Bees(BEES, Bees),
    SulfurCubeContent(SULFUR_CUBE_CONTENT, SulfurCubeContent),
    Lock(LOCK, LockCode),
    ContainerLoot(CONTAINER_LOOT, SeededContainerLoot),
    BreakSound(BREAK_SOUND, SoundEventRef),
    Compostable(COMPOSTABLE, Compostable),
    CookingFuel(COOKING_FUEL, CookingFuel),
    BrewingFuel(BREWING_FUEL, BrewingFuel),
    MobVisibility(MOB_VISIBILITY, MobVisibility),
    VillagerVariant(VILLAGER_VARIANT, VillagerVariant),
    WolfVariant(WOLF_VARIANT, WolfVariant),
    WolfSoundVariant(WOLF_SOUND_VARIANT, WolfSoundVariant),
    WolfCollar(WOLF_COLLAR, DyeColor),
    FoxVariant(FOX_VARIANT, FoxVariant),
    SalmonSize(SALMON_SIZE, SalmonSize),
    ParrotVariant(PARROT_VARIANT, ParrotVariant),
    TropicalFishPattern(TROPICAL_FISH_PATTERN, TropicalFishPattern),
    TropicalFishBaseColor(TROPICAL_FISH_BASE_COLOR, DyeColor),
    TropicalFishPatternColor(TROPICAL_FISH_PATTERN_COLOR, DyeColor),
    MooshroomVariant(MOOSHROOM_VARIANT, MooshroomVariant),
    RabbitVariant(RABBIT_VARIANT, RabbitVariant),
    PigVariant(PIG_VARIANT, PigVariant),
    PigSoundVariant(PIG_SOUND_VARIANT, PigSoundVariant),
    CowVariant(COW_VARIANT, CowVariant),
    CowSoundVariant(COW_SOUND_VARIANT, CowSoundVariant),
    ChickenVariant(CHICKEN_VARIANT, ChickenVariant),
    ChickenSoundVariant(CHICKEN_SOUND_VARIANT, ChickenSoundVariant),
    ZombieNautilusVariant(ZOMBIE_NAUTILUS_VARIANT, ZombieNautilusVariant),
    FrogVariant(FROG_VARIANT, FrogVariant),
    HorseVariant(HORSE_VARIANT, HorseVariant),
    PaintingVariant(PAINTING_VARIANT, PaintingVariant),
    LlamaVariant(LLAMA_VARIANT, LlamaVariant),
    AxolotlVariant(AXOLOTL_VARIANT, AxolotlVariant),
    CatVariant(CAT_VARIANT, CatVariant),
    CatSoundVariant(CAT_SOUND_VARIANT, CatSoundVariant),
    CatCollar(CAT_COLLAR, DyeColor),
    SheepColor(SHEEP_COLOR, DyeColor),
    ShulkerColor(SHULKER_COLOR, DyeColor),
    ProvidesPotteryPattern(PROVIDES_POTTERY_PATTERN, PotteryPattern),
    SignTextFront(SIGN_TEXT_FRONT, SignText),
    SignTextBack(SIGN_TEXT_BACK, SignText),
    Waxed(WAXED, ()),
    CushionColor(CUSHION_COLOR, DyeColor),
}

/// `minecraft:...` name of a component type.
pub fn name(id: ComponentId) -> &'static str {
    ids::NAMES.get(id as usize).copied().unwrap_or("minecraft:unknown")
}

/// Component type by name (a missing namespace means `minecraft`).
pub fn by_name(name: &str) -> Option<ComponentId> {
    crate::registry::DATA_COMPONENT_TYPE.id(name).map(|i| i as ComponentId)
}

/// Whether values of this type are saved and hashed (vanilla: the type has a `codec()`).
pub fn is_persistent(id: ComponentId) -> bool {
    ids::PERSISTENT.get(id as usize).copied().unwrap_or(false)
}

/// Number of component types.
pub fn count() -> usize {
    ids::NAMES.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_component_type_has_a_variant() {
        let mut modelled = MODELLED.to_vec();
        modelled.sort_unstable();
        modelled.dedup();
        assert_eq!(modelled.len(), MODELLED.len(), "a component type is listed twice");
        let missing: Vec<&str> = (0..count() as ComponentId).filter(|id| !modelled.contains(id)).map(name).collect();
        assert!(missing.is_empty(), "component types without a variant: {missing:?}");
    }
}
