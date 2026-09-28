//! Mob effects shared by players and mobs: the `minecraft:mob_effect` table (`MobEffects`: what
//! each effect does on its ticks, its category, particle colour and attribute modifier), one
//! active effect (`MobEffectInstance`: duration, amplifier, flags and the hidden weaker or
//! shorter instance underneath), the potions' effects (`Potions`) and the saved form
//! (`active_effects`).
//!
//! The simulation's players (kiln-sim `effects`) and this crate's mobs ([`crate::mob::effects`])
//! keep their effects as a map from network id to [`Effect`] and run the same instance logic;
//! what an effect does to its bearer (healing, damage, hunger) is theirs.
//!
//! Vanilla keeps effects in a hash map keyed by identity-hashed holders, so the order in which
//! different effects tick is arbitrary; Kiln ticks them in registry order.

use kiln_item::component::AttributeOperation as Op;
use kiln_item::registry::MOB_EFFECT;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// What an effect does on its ticks (the `MobEffect` subclass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No tick behaviour (attribute modifiers or client-side effects only).
    Plain,
    Regeneration,
    Poison,
    Wither,
    Hunger,
    Saturation,
    Absorption,
    /// `HealOrHarmMobEffect`: instant health, or instant damage when `harm`.
    HealOrHarm { harm: bool },
    /// `BadOmenMobEffect`: ticks every tick (a player in a village turns it into raid omen).
    BadOmen,
    /// `RaidOmenMobEffect`: its last tick starts a raid.
    RaidOmen,
    /// `InfestedMobEffect`: silverfish out of the bearer when it is hurt.
    Infested,
    /// `OozingMobEffect`: slimes when the bearer dies.
    Oozing,
    /// `WeavingMobEffect`: cobwebs when the bearer dies.
    Weaving,
    /// `WindChargedMobEffect`: a wind burst when the bearer dies.
    WindCharged,
}

impl Kind {
    /// `MobEffect.isInstantaneous`.
    pub fn instantaneous(self) -> bool {
        matches!(self, Kind::Saturation | Kind::HealOrHarm { .. })
    }

    /// `shouldApplyEffectTickThisTick(tick, amplifier)`: `tick` is the remaining duration, or
    /// the entity's age for infinite effects. Java masks shift counts to five bits.
    pub fn applies_this_tick(self, tick: i32, amplifier: i32) -> bool {
        let every = |base: i32| {
            let interval = base.wrapping_shr(amplifier as u32);
            interval <= 0 || tick % interval == 0
        };
        match self {
            Kind::Plain | Kind::Infested | Kind::Oozing | Kind::Weaving | Kind::WindCharged => false,
            Kind::Regeneration => every(50),
            Kind::Poison => every(25),
            Kind::Wither => every(40),
            Kind::Hunger | Kind::Absorption | Kind::BadOmen => true,
            Kind::RaidOmen => tick == 1,
            Kind::Saturation | Kind::HealOrHarm { .. } => tick >= 1,
        }
    }
}

/// `MobEffectCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Beneficial,
    Harmful,
    Neutral,
}

/// An effect's attribute modifier template (`MobEffect.AttributeTemplate`): the modifier is
/// `amount * (amplifier + 1)`.
#[derive(Debug, Clone, Copy)]
pub struct Modifier {
    /// The `minecraft:attribute` name.
    pub attr: &'static str,
    pub id: &'static str,
    pub amount: f64,
    pub op: Op,
}

impl Modifier {
    /// `AttributeTemplate.create(amplifier)`: the modifier's amount at `amplifier`.
    pub fn amount_at(&self, amplifier: i32) -> f64 {
        self.amount * (amplifier + 1) as f64
    }
}

/// A `minecraft:mob_effect` entry.
#[derive(Debug, Clone, Copy)]
pub struct EffectType {
    pub name: &'static str,
    pub kind: Kind,
    pub category: Category,
    /// RGB colour of the effect's particles.
    pub color: i32,
    pub modifier: Option<Modifier>,
    /// The particle type of an effect with its own particle (`ParticleTypes.TRIAL_OMEN`...);
    /// `None`: `entity_effect` in the effect's colour.
    pub particle: Option<&'static str>,
    /// `withSoundOnAdded`: played at the bearer when the effect is first added.
    pub sound_on_added: Option<&'static str>,
}

macro_rules! effects {
    ($($name:literal $kind:expr, $cat:ident $color:literal $( => $attr:literal $id:literal $amount:literal $op:ident)? $(; particle $particle:literal)? $(; sound $sound:literal)?;)*) => {
        &[$(EffectType {
            name: concat!("minecraft:", $name),
            kind: $kind,
            category: Category::$cat,
            color: $color,
            modifier: effects!(@m $($attr $id $amount $op)?),
            particle: effects!(@o $($particle)?),
            sound_on_added: effects!(@o $($sound)?),
        },)*]
    };
    (@m) => { None };
    (@m $attr:literal $id:literal $amount:literal $op:ident) => {
        Some(Modifier { attr: concat!("minecraft:", $attr), id: concat!("minecraft:", $id), amount: $amount, op: Op::$op })
    };
    (@o) => { None };
    (@o $v:literal) => { Some(concat!("minecraft:", $v)) };
}

/// `MobEffects` of 26.3, in registry order (checked against the vanilla vectors' registry dump
/// by kiln-sim's `effect_parity`).
pub const EFFECTS: &[EffectType] = effects! {
    "speed" Kind::Plain, Beneficial 3402751 => "movement_speed" "effect.speed" 0.20000000298023224 AddMultipliedTotal;
    "slowness" Kind::Plain, Harmful 9154528 => "movement_speed" "effect.slowness" -0.15000000596046448 AddMultipliedTotal;
    "haste" Kind::Plain, Beneficial 14270531 => "attack_speed" "effect.haste" 0.10000000149011612 AddMultipliedTotal;
    "mining_fatigue" Kind::Plain, Harmful 4866583 => "attack_speed" "effect.mining_fatigue" -0.10000000149011612 AddMultipliedTotal;
    "strength" Kind::Plain, Beneficial 16762624 => "attack_damage" "effect.strength" 3.0 AddValue;
    "instant_health" Kind::HealOrHarm { harm: false }, Beneficial 16262179;
    "instant_damage" Kind::HealOrHarm { harm: true }, Harmful 11101546;
    "jump_boost" Kind::Plain, Beneficial 16646020 => "safe_fall_distance" "effect.jump_boost" 1.0 AddValue;
    "nausea" Kind::Plain, Harmful 5578058;
    "regeneration" Kind::Regeneration, Beneficial 13458603;
    "resistance" Kind::Plain, Beneficial 9520880;
    "fire_resistance" Kind::Plain, Beneficial 16750848;
    "water_breathing" Kind::Plain, Beneficial 10017472;
    "invisibility" Kind::Plain, Beneficial 16185078 => "waypoint_transmit_range" "effect.waypoint_transmit_range_hide" -1.0 AddMultipliedTotal;
    "blindness" Kind::Plain, Harmful 2039587;
    "night_vision" Kind::Plain, Beneficial 12779366;
    "hunger" Kind::Hunger, Harmful 5797459;
    "weakness" Kind::Plain, Harmful 4738376 => "attack_damage" "effect.weakness" -4.0 AddValue;
    "poison" Kind::Poison, Harmful 8889187;
    "wither" Kind::Wither, Harmful 7561558;
    "health_boost" Kind::Plain, Beneficial 16284963 => "max_health" "effect.health_boost" 4.0 AddValue;
    "absorption" Kind::Absorption, Beneficial 2445989 => "max_absorption" "effect.absorption" 4.0 AddValue;
    "saturation" Kind::Saturation, Beneficial 16262179;
    "glowing" Kind::Plain, Neutral 9740385;
    "levitation" Kind::Plain, Harmful 13565951;
    "luck" Kind::Plain, Beneficial 5882118 => "luck" "effect.luck" 1.0 AddValue;
    "unluck" Kind::Plain, Harmful 12624973 => "luck" "effect.unluck" -1.0 AddValue;
    "slow_falling" Kind::Plain, Beneficial 15978425;
    "conduit_power" Kind::Plain, Beneficial 1950417;
    "dolphins_grace" Kind::Plain, Beneficial 8954814;
    "bad_omen" Kind::BadOmen, Neutral 745784; sound "event.mob_effect.bad_omen";
    "hero_of_the_village" Kind::Plain, Beneficial 4521796;
    "darkness" Kind::Plain, Harmful 2696993;
    "trial_omen" Kind::Plain, Neutral 1484454; particle "trial_omen"; sound "event.mob_effect.trial_omen";
    "raid_omen" Kind::RaidOmen, Neutral 14565464; particle "raid_omen"; sound "event.mob_effect.raid_omen";
    "wind_charged" Kind::WindCharged, Harmful 12438015; particle "small_gust";
    "weaving" Kind::Weaving, Harmful 7891290; particle "item_cobweb";
    "oozing" Kind::Oozing, Harmful 10092451; particle "item_slime";
    "infested" Kind::Infested, Harmful 9214860; particle "infested";
    "breath_of_the_nautilus" Kind::Plain, Beneficial 65518;
};

/// The effect with network id `id`.
pub fn effect_type(id: i32) -> Option<&'static EffectType> {
    static BY_ID: OnceLock<Vec<Option<&'static EffectType>>> = OnceLock::new();
    let table = BY_ID.get_or_init(|| {
        let n = MOB_EFFECT.entries().len();
        (0..n as i32).map(|i| MOB_EFFECT.name(i).and_then(|name| EFFECTS.iter().find(|e| e.name == name))).collect()
    });
    usize::try_from(id).ok().and_then(|i| table.get(i).copied().flatten())
}

/// Network id of a `minecraft:mob_effect` entry.
pub fn effect_id(name: &str) -> Option<i32> {
    MOB_EFFECT.id(name)
}

/// Network ids of the effects other code asks about by name, looked up once.
pub mod ids {
    use std::sync::OnceLock;

    macro_rules! ids {
        ($($f:ident $name:literal;)*) => {
            $(
                pub fn $f() -> i32 {
                    static ID: OnceLock<i32> = OnceLock::new();
                    *ID.get_or_init(|| super::effect_id($name).unwrap_or(-1))
                }
            )*
        };
    }

    ids! {
        speed "minecraft:speed";
        slowness "minecraft:slowness";
        strength "minecraft:strength";
        instant_health "minecraft:instant_health";
        instant_damage "minecraft:instant_damage";
        jump_boost "minecraft:jump_boost";
        regeneration "minecraft:regeneration";
        resistance "minecraft:resistance";
        fire_resistance "minecraft:fire_resistance";
        water_breathing "minecraft:water_breathing";
        invisibility "minecraft:invisibility";
        weakness "minecraft:weakness";
        poison "minecraft:poison";
        wither "minecraft:wither";
        absorption "minecraft:absorption";
        glowing "minecraft:glowing";
        levitation "minecraft:levitation";
        slow_falling "minecraft:slow_falling";
        conduit_power "minecraft:conduit_power";
        dolphins_grace "minecraft:dolphins_grace";
        breath_of_the_nautilus "minecraft:breath_of_the_nautilus";
        infested "minecraft:infested";
        oozing "minecraft:oozing";
        weaving "minecraft:weaving";
        wind_charged "minecraft:wind_charged";
    }
}

/// `MobEffectInstance.INFINITE_DURATION`.
pub const INFINITE: i32 = -1;

/// `MobEffect.AMBIENT_ALPHA`: `Mth.floor(38.25f)`.
pub const AMBIENT_ALPHA: i32 = 38;

/// `MobEffectInstance`: one active effect.
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    /// Network id in `minecraft:mob_effect`.
    pub id: i32,
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
    /// What remains when this instance runs out.
    pub hidden: Option<Box<Effect>>,
}

impl Effect {
    /// The full constructor; the amplifier is clamped to 0..=255.
    pub fn new(id: i32, duration: i32, amplifier: i32, ambient: bool, visible: bool, show_icon: bool) -> Self {
        Effect { id, duration, amplifier: amplifier.clamp(0, 255), ambient, visible, show_icon, hidden: None }
    }

    /// `new MobEffectInstance(effect, duration, amplifier)`: visible, with an icon.
    pub fn simple(id: i32, duration: i32, amplifier: i32) -> Self {
        Effect::new(id, duration, amplifier, false, true, true)
    }

    /// `new MobEffectInstance(effect, duration, amplifier, ambient, visible)`: the icon shows
    /// when the particles do.
    pub fn with_flags(id: i32, duration: i32, amplifier: i32, ambient: bool, visible: bool) -> Self {
        Effect::new(id, duration, amplifier, ambient, visible, visible)
    }

    /// An effect by `minecraft:mob_effect` name.
    pub fn named(name: &str, duration: i32, amplifier: i32) -> Option<Self> {
        effect_id(name).map(|id| Effect::simple(id, duration, amplifier))
    }

    pub fn kind(&self) -> Kind {
        effect_type(self.id).map_or(Kind::Plain, |t| t.kind)
    }

    pub fn effect_type(&self) -> Option<&'static EffectType> {
        effect_type(self.id)
    }

    /// The copy constructor: the details without the hidden effect.
    pub fn copy_details(&self) -> Effect {
        Effect { hidden: None, ..self.clone() }
    }

    fn set_details_from(&mut self, o: &Effect) {
        self.duration = o.duration;
        self.amplifier = o.amplifier;
        self.ambient = o.ambient;
        self.visible = o.visible;
        self.show_icon = o.show_icon;
    }

    pub fn is_infinite(&self) -> bool {
        self.duration == INFINITE
    }

    fn is_shorter_duration_than(&self, o: &Effect) -> bool {
        !self.is_infinite() && (self.duration < o.duration || o.is_infinite())
    }

    /// `endsWithin(ticks)`.
    pub fn ends_within(&self, ticks: i32) -> bool {
        !self.is_infinite() && self.duration <= ticks
    }

    pub fn has_remaining_duration(&self) -> bool {
        self.is_infinite() || self.duration > 0
    }

    /// `mapDuration`: infinite and zero durations stay as they are.
    pub fn map_duration(&self, f: impl Fn(i32) -> i32) -> i32 {
        if self.is_infinite() || self.duration == 0 { self.duration } else { f(self.duration) }
    }

    /// `withScaledDuration(scale)` (potion duration scale): at least one tick.
    pub fn scaled(&self, scale: f32) -> Effect {
        let mut e = self.copy_details();
        e.duration = self.map_duration(|d| kiln_javamath::math::floor_f32(d as f32 * scale).max(1));
        e
    }

    pub fn tick_down(&mut self) {
        if let Some(h) = &mut self.hidden {
            h.tick_down();
        }
        self.duration = self.map_duration(|d| d - 1);
    }

    /// `downgradeToHiddenEffect`: at zero, the hidden instance takes over.
    pub fn downgrade(&mut self) -> bool {
        if self.duration == 0
            && let Some(h) = self.hidden.take()
        {
            self.set_details_from(&h);
            self.hidden = h.hidden;
            return true;
        }
        false
    }

    /// `update(takeOver)`: merges a new instance of the same effect; returns whether this one
    /// changed (a weaker one only goes underneath).
    pub fn update(&mut self, o: &Effect) -> bool {
        let mut changed = false;
        if o.amplifier > self.amplifier {
            if o.is_shorter_duration_than(self) {
                let old = self.hidden.take();
                let mut hidden = self.copy_details();
                hidden.hidden = old;
                self.hidden = Some(Box::new(hidden));
            }
            self.amplifier = o.amplifier;
            self.duration = o.duration;
            changed = true;
        } else if self.is_shorter_duration_than(o) {
            if o.amplifier == self.amplifier {
                self.duration = o.duration;
                changed = true;
            } else {
                match &mut self.hidden {
                    None => self.hidden = Some(Box::new(o.copy_details())),
                    Some(h) => {
                        h.update(o);
                    }
                }
            }
        }
        if (!o.ambient && self.ambient) || changed {
            self.ambient = o.ambient;
            changed = true;
        }
        if o.visible != self.visible {
            self.visible = o.visible;
            changed = true;
        }
        if o.show_icon != self.show_icon {
            self.show_icon = o.show_icon;
            changed = true;
        }
        changed
    }

    /// The effect's `MobEffectInstance.getParticleOptions` for the entity data: the effect's own
    /// particle, or `entity_effect` in its colour (faint when ambient).
    pub fn particle(&self) -> Option<kiln_proto::packets::entity::metadata::Particle> {
        let t = effect_type(self.id)?;
        if let Some(p) = t.particle {
            let kind = kiln_data::builtin_id("minecraft:particle_type", p)?;
            return Some(kiln_proto::packets::entity::metadata::Particle { kind, options: Vec::new() });
        }
        let kind = kiln_data::builtin_id("minecraft:particle_type", "minecraft:entity_effect")?;
        let alpha = if self.ambient { AMBIENT_ALPHA } else { 255 };
        let argb = (alpha << 24) | (t.color & 0xFF_FFFF);
        Some(kiln_proto::packets::entity::metadata::Particle { kind, options: argb.to_be_bytes().to_vec() })
    }

    /// From the item/save form (`MobEffectInstance.CODEC` / stream codec).
    pub fn from_item(e: &kiln_item::component::MobEffectInstance) -> Effect {
        Effect::from_details(e.effect, &e.details)
    }

    fn from_details(id: i32, d: &kiln_item::component::EffectDetails) -> Effect {
        let mut e = Effect::new(id, d.duration, d.amplifier, d.ambient, d.show_particles, d.show_icon);
        e.hidden = d.hidden_effect.as_deref().map(|h| Box::new(Effect::from_details(id, h)));
        e
    }

    pub fn to_item(&self) -> kiln_item::component::MobEffectInstance {
        kiln_item::component::MobEffectInstance { effect: self.id, details: self.details() }
    }

    fn details(&self) -> kiln_item::component::EffectDetails {
        kiln_item::component::EffectDetails {
            amplifier: self.amplifier,
            duration: self.duration,
            ambient: self.ambient,
            show_particles: self.visible,
            show_icon: self.show_icon,
            hidden_effect: self.hidden.as_ref().map(|h| Box::new(h.details())),
        }
    }
}

/// Active effects by network id (registry order).
pub type Effects = BTreeMap<i32, Effect>;

/// `DATA_EFFECT_PARTICLES`: one particle per visible effect.
pub fn particles(effects: &Effects) -> Vec<kiln_proto::packets::entity::metadata::Particle> {
    effects.values().filter(|e| e.visible).filter_map(Effect::particle).collect()
}

/// `areAllEffectsAmbient`: no visible effect that is not ambient.
pub fn all_ambient(effects: &Effects) -> bool {
    effects.values().all(|e| !e.visible || e.ambient)
}

/// The saved `active_effects` list (`MobEffectInstance.CODEC`); `None` when there are none.
pub fn save(effects: &Effects) -> Option<kiln_proto::nbt::Tag> {
    if effects.is_empty() {
        return None;
    }
    let list = effects.values().map(|e| e.to_item().to_value().to_nbt()).collect();
    Some(kiln_proto::nbt::Tag::List(list))
}

/// Loads `active_effects` (unknown or invalid entries are dropped, as vanilla's lenient list
/// codec does).
pub fn load(tag: &kiln_proto::nbt::Tag) -> Effects {
    let mut out = BTreeMap::new();
    let kiln_proto::nbt::Tag::List(items) = tag else { return out };
    for item in items {
        let value = kiln_item::Value::from_nbt(item);
        if let Ok(e) = kiln_item::component::MobEffectInstance::from_value(&value) {
            let e = Effect::from_item(&e);
            out.insert(e.id, e);
        }
    }
    out
}

/// A potion and its effects: (effect, duration, amplifier).
pub type PotionEntry = (&'static str, &'static [(&'static str, i32, i32)]);

/// `minecraft:potion` entries' effects (`Potions`).
pub const POTIONS: &[PotionEntry] = &[
    ("water", &[]),
    ("mundane", &[]),
    ("thick", &[]),
    ("awkward", &[]),
    ("night_vision", &[("night_vision", 3600, 0)]),
    ("long_night_vision", &[("night_vision", 9600, 0)]),
    ("invisibility", &[("invisibility", 3600, 0)]),
    ("long_invisibility", &[("invisibility", 9600, 0)]),
    ("leaping", &[("jump_boost", 3600, 0)]),
    ("long_leaping", &[("jump_boost", 9600, 0)]),
    ("strong_leaping", &[("jump_boost", 1800, 1)]),
    ("fire_resistance", &[("fire_resistance", 3600, 0)]),
    ("long_fire_resistance", &[("fire_resistance", 9600, 0)]),
    ("swiftness", &[("speed", 3600, 0)]),
    ("long_swiftness", &[("speed", 9600, 0)]),
    ("strong_swiftness", &[("speed", 1800, 1)]),
    ("slowness", &[("slowness", 1800, 0)]),
    ("long_slowness", &[("slowness", 4800, 0)]),
    ("strong_slowness", &[("slowness", 400, 3)]),
    ("turtle_master", &[("slowness", 400, 3), ("resistance", 400, 2)]),
    ("long_turtle_master", &[("slowness", 800, 3), ("resistance", 800, 2)]),
    ("strong_turtle_master", &[("slowness", 400, 5), ("resistance", 400, 3)]),
    ("water_breathing", &[("water_breathing", 3600, 0)]),
    ("long_water_breathing", &[("water_breathing", 9600, 0)]),
    ("healing", &[("instant_health", 1, 0)]),
    ("strong_healing", &[("instant_health", 1, 1)]),
    ("harming", &[("instant_damage", 1, 0)]),
    ("strong_harming", &[("instant_damage", 1, 1)]),
    ("poison", &[("poison", 900, 0)]),
    ("long_poison", &[("poison", 1800, 0)]),
    ("strong_poison", &[("poison", 432, 1)]),
    ("regeneration", &[("regeneration", 900, 0)]),
    ("long_regeneration", &[("regeneration", 1800, 0)]),
    ("strong_regeneration", &[("regeneration", 450, 1)]),
    ("strength", &[("strength", 3600, 0)]),
    ("long_strength", &[("strength", 9600, 0)]),
    ("strong_strength", &[("strength", 1800, 1)]),
    ("weakness", &[("weakness", 1800, 0)]),
    ("long_weakness", &[("weakness", 4800, 0)]),
    ("luck", &[("luck", 6000, 0)]),
    ("slow_falling", &[("slow_falling", 1800, 0)]),
    ("long_slow_falling", &[("slow_falling", 4800, 0)]),
    ("wind_charged", &[("wind_charged", 3600, 0)]),
    ("weaving", &[("weaving", 3600, 0)]),
    ("oozing", &[("oozing", 3600, 0)]),
    ("infested", &[("infested", 3600, 0)]),
];

/// `PotionContents.forEachEffect(consumer, durationScale)`: the potion's effects, then the
/// custom ones, each with its duration scaled.
pub fn potion_effects(contents: &kiln_item::component::PotionContents, scale: f32) -> Vec<Effect> {
    let mut out = Vec::new();
    if let Some(name) = contents.potion.and_then(|id| kiln_item::registry::POTION.name(id)) {
        out.extend(named_potion_effects(name, scale));
    }
    out.extend(contents.custom_effects.iter().map(|e| Effect::from_item(e).scaled(scale)));
    out
}

/// The effects of the potion `name` (`minecraft:` prefixed or not), durations scaled.
pub fn named_potion_effects(name: &str, scale: f32) -> Vec<Effect> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let Some((_, list)) = POTIONS.iter().find(|(n, _)| *n == short) else { return Vec::new() };
    list.iter()
        .filter_map(|(effect, duration, amplifier)| effect_id(&format!("minecraft:{effect}")).map(|id| Effect::simple(id, *duration, *amplifier).scaled(scale)))
        .collect()
}

/// `PotionContents.BASE_POTION_COLOR`.
pub const BASE_POTION_COLOR: i32 = -13083194;

/// `PotionContents.getColor`: the custom colour, else the visible effects' colours mixed by
/// level, else the base potion colour (opaque ARGB).
pub fn potion_color(contents: &kiln_item::component::PotionContents) -> i32 {
    if let Some(c) = contents.custom_color {
        return c;
    }
    let (mut r, mut g, mut b, mut w) = (0, 0, 0, 0);
    for fx in potion_effects(contents, 1.0) {
        if !fx.visible {
            continue;
        }
        let c = effect_type(fx.id).map_or(0, |t| t.color);
        let a = fx.amplifier + 1;
        r += a * ((c >> 16) & 0xFF);
        g += a * ((c >> 8) & 0xFF);
        b += a * (c & 0xFF);
        w += a;
    }
    if w == 0 {
        return BASE_POTION_COLOR;
    }
    (0xFF00_0000u32 as i32) | ((r / w) << 16) | ((g / w) << 8) | (b / w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speed(duration: i32, amplifier: i32) -> Effect {
        Effect::simple(effect_id("minecraft:speed").unwrap(), duration, amplifier)
    }

    #[test]
    fn table_follows_the_registry() {
        for (i, t) in EFFECTS.iter().enumerate() {
            assert_eq!(effect_id(t.name), Some(i as i32), "{}", t.name);
        }
        assert_eq!(effect_type(effect_id("minecraft:poison").unwrap()).unwrap().kind, Kind::Poison);
        assert_eq!(ids::wither(), effect_id("minecraft:wither").unwrap());
    }

    #[test]
    fn stronger_shorter_effect_hides_the_old_one() {
        let mut e = speed(100, 0);
        assert!(e.update(&speed(20, 2)));
        assert_eq!((e.amplifier, e.duration), (2, 20));
        let hidden = e.hidden.as_deref().unwrap();
        assert_eq!((hidden.amplifier, hidden.duration), (0, 100));
        for _ in 0..20 {
            e.tick_down();
        }
        assert!(e.downgrade());
        assert_eq!((e.amplifier, e.duration), (0, 80));
        assert!(e.hidden.is_none());
    }

    #[test]
    fn weaker_longer_effect_goes_underneath() {
        let mut e = speed(20, 2);
        assert!(!e.update(&speed(50, 0)));
        assert!(!e.update(&speed(40, 1)));
        // The stronger of the two hidden ones sits on top of the weaker, longer one.
        let h = e.hidden.as_deref().unwrap();
        assert_eq!((h.amplifier, h.duration), (1, 40));
        let hh = h.hidden.as_deref().unwrap();
        assert_eq!((hh.amplifier, hh.duration), (0, 50));
    }

    #[test]
    fn saved_effects_load_back() {
        let mut e = speed(20, 2);
        e.update(&speed(100, 0));
        e.ambient = true;
        let mut map = Effects::new();
        map.insert(e.id, e.clone());
        let tag = save(&map).unwrap();
        let loaded = load(&tag);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[&e.id], e);
        let Some(kiln_proto::nbt::Tag::Compound(fields)) = tag.as_list().and_then(|l| l.first()).cloned() else { panic!() };
        let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"id") && keys.contains(&"hidden_effect") && keys.contains(&"amplifier"), "{keys:?}");
    }

    #[test]
    fn intervals() {
        assert!(Kind::Regeneration.applies_this_tick(50, 0));
        assert!(!Kind::Regeneration.applies_this_tick(49, 0));
        assert!(Kind::Regeneration.applies_this_tick(7, 6));
        assert!(Kind::Poison.applies_this_tick(12, 1));
        assert!(!Kind::HealOrHarm { harm: true }.applies_this_tick(0, 0));
        // Java masks shift counts: 50 >> 32 is 50.
        assert!(!Kind::Regeneration.applies_this_tick(7, 32));
        assert!(Kind::RaidOmen.applies_this_tick(1, 0) && !Kind::RaidOmen.applies_this_tick(2, 0));
    }

    #[test]
    fn own_particles() {
        let omen = Effect::named("minecraft:trial_omen", 100, 0).unwrap();
        let p = omen.particle().unwrap();
        assert!(p.options.is_empty());
        let speed = speed(10, 0).particle().unwrap();
        assert_eq!(speed.options, (0xFF33EBFFu32 as i32).to_be_bytes().to_vec());
    }
}
