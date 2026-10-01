//! Mobs: vanilla's `LivingEntity` / `Mob` / `PathfinderMob` behaviour for the types Kiln
//! simulates (pig, cow, sheep, chicken, zombie, skeleton, creeper, spider).
//!
//! A mob is an [`Entity`] whose kind is [`EntityKind::Mob`] holding a [`MobData`]: health and
//! attributes, rotations and movement inputs, the goal and target selectors, the look, move,
//! jump and body rotation controls, the path navigation and the type's own state. While a mob
//! ticks, its data is taken out of the entity (the kind becomes [`EntityKind::MobTicking`]) so
//! the tick can borrow both.
//!
//! Randomness follows vanilla: the mob's own random (`Entity.random`) for AI and sounds, the
//! level's random for spawning decisions.

pub mod attributes;
pub mod brain;
pub mod breed;
pub mod ext;
pub mod fly;
pub mod control;
pub mod convert;
pub mod effects;
pub mod goals;
pub mod gossip;
pub mod interact;
pub mod kinds;
pub mod mth;
pub mod path;
pub mod persist;
pub mod random_pos;
pub mod species;

use crate::entity::{Entity, EntityKind, MoverType};
use crate::level::{DamageKind, EntityFilter, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use attributes::{Attr, Attributes, Op};
use goals::{Goal, GoalSelector, Living, MeleeKind, Wanted};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

/// Equipment slot indices in [`MobData::equipment`] (`EquipmentSlot` order).
pub const MAINHAND: usize = 0;
pub const OFFHAND: usize = 1;
pub const FEET: usize = 2;
pub const LEGS: usize = 3;
pub const CHEST: usize = 4;
pub const HEAD: usize = 5;
pub const SLOT_NAMES: [&str; 6] = ["mainhand", "offhand", "feet", "legs", "chest", "head"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MobKind {
    Pig,
    Cow,
    Sheep,
    Chicken,
    Zombie,
    Skeleton,
    Creeper,
    Spider,
    // Extension types (behaviour in `kinds`).
    Husk,
    Stray,
    Drowned,
    ZombieVillager,
    ZombifiedPiglin,
    WitherSkeleton,
    Enderman,
    Endermite,
    Shulker,
    Witch,
    Slime,
    MagmaCube,
    Phantom,
    Ghast,
    Blaze,
    Wolf,
    Cat,
    Horse,
    Donkey,
    Mule,
    Strider,
    IronGolem,
    Villager,
    Piglin,
    Hoglin,
    Silverfish,
    // Slice 3 work packages add their types below their own marker (keep the blank lines
    // between markers so parallel additions merge cleanly).
    // -- slice 3: raids
    Pillager,
    Vindicator,
    Evoker,
    Vex,
    Ravager,
    Illusioner,

    // -- slice 3: the end
    EnderDragon,

    // -- slice 3: wither and guardians
    Wither,
    Guardian,
    ElderGuardian,

    // -- slice 3: warden
    Warden,

    // -- slice 3: common mobs A
    Rabbit,
    PolarBear,
    Turtle,
    Fox,
    Panda,

    // -- slice 3: common mobs B
    Squid,
    GlowSquid,
    Cod,
    Salmon,
    TropicalFish,
    Pufferfish,
    Mooshroom,
    Ocelot,
    Bat,
    SnowGolem,
    Bogged,
    Armadillo,
    Camel,
    Allay,
    Breeze,
    Creaking,
    Sniffer,

    // -- wp28: brain mobs (new types; their modules are stubs until their owners fill them in)
    PiglinBrute,
    Zoglin,
    Axolotl,
    Goat,
    Frog,
    Tadpole,

}

/// `MobCategory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Monster,
    Creature,
    /// Villagers and golems: never spawned by the natural spawner, never despawn.
    Misc,
    /// Bats.
    Ambient,
    Axolotls,
    /// Glow squids.
    UndergroundWaterCreature,
    /// Squids, dolphins.
    WaterCreature,
    /// Fish.
    WaterAmbient,
}

impl Category {
    /// `NaturalSpawner.SPAWNING_CATEGORIES` (every category but `MISC`, in `MobCategory` order).
    pub const SPAWNING: [Category; 7] = [
        Category::Monster,
        Category::Creature,
        Category::Ambient,
        Category::Axolotls,
        Category::UndergroundWaterCreature,
        Category::WaterCreature,
        Category::WaterAmbient,
    ];

    pub fn max_instances(self) -> i32 {
        match self {
            Category::Monster => 70,
            Category::Creature => 10,
            Category::Misc => -1,
            Category::Ambient => 15,
            Category::Axolotls | Category::UndergroundWaterCreature | Category::WaterCreature => 5,
            Category::WaterAmbient => 20,
        }
    }
    pub fn friendly(self) -> bool {
        self != Category::Monster
    }
    pub fn persistent(self) -> bool {
        matches!(self, Category::Creature | Category::Misc)
    }
    pub fn despawn_distance(self) -> i32 {
        if self == Category::WaterAmbient { 64 } else { 128 }
    }
    pub fn no_despawn_distance(self) -> i32 {
        32
    }
    pub fn name(self) -> &'static str {
        match self {
            Category::Monster => "monster",
            Category::Creature => "creature",
            Category::Misc => "misc",
            Category::Ambient => "ambient",
            Category::Axolotls => "axolotls",
            Category::UndergroundWaterCreature => "underground_water_creature",
            Category::WaterCreature => "water_creature",
            Category::WaterAmbient => "water_ambient",
        }
    }
}

pub const ALL_KINDS: &[MobKind] = &[
    MobKind::Pig,
    MobKind::Cow,
    MobKind::Sheep,
    MobKind::Chicken,
    MobKind::Zombie,
    MobKind::Skeleton,
    MobKind::Creeper,
    MobKind::Spider,
    MobKind::Husk,
    MobKind::Stray,
    MobKind::Drowned,
    MobKind::ZombieVillager,
    MobKind::ZombifiedPiglin,
    MobKind::WitherSkeleton,
    MobKind::Enderman,
    MobKind::Endermite,
    MobKind::Shulker,
    MobKind::Witch,
    MobKind::Slime,
    MobKind::MagmaCube,
    MobKind::Phantom,
    MobKind::Ghast,
    MobKind::Blaze,
    MobKind::Wolf,
    MobKind::Cat,
    MobKind::Horse,
    MobKind::Donkey,
    MobKind::Mule,
    MobKind::Strider,
    MobKind::IronGolem,
    MobKind::Villager,
    MobKind::Piglin,
    MobKind::Hoglin,
    MobKind::Silverfish,
    // -- slice 3: raids
    MobKind::Pillager,
    MobKind::Vindicator,
    MobKind::Evoker,
    MobKind::Vex,
    MobKind::Ravager,
    MobKind::Illusioner,

    // -- slice 3: the end
    MobKind::EnderDragon,

    // -- slice 3: wither and guardians
    MobKind::Wither,
    MobKind::Guardian,
    MobKind::ElderGuardian,

    // -- slice 3: warden
    MobKind::Warden,

    // -- slice 3: common mobs A
    MobKind::Rabbit,
    MobKind::PolarBear,
    MobKind::Turtle,
    MobKind::Fox,
    MobKind::Panda,

    // -- slice 3: common mobs B
    MobKind::Squid,
    MobKind::GlowSquid,
    MobKind::Cod,
    MobKind::Salmon,
    MobKind::TropicalFish,
    MobKind::Pufferfish,
    MobKind::Mooshroom,
    MobKind::Ocelot,
    MobKind::Bat,
    MobKind::SnowGolem,
    MobKind::Bogged,
    MobKind::Armadillo,
    MobKind::Camel,
    MobKind::Allay,
    MobKind::Breeze,
    MobKind::Creaking,
    MobKind::Sniffer,

    // -- wp28: brain mobs
    MobKind::PiglinBrute,
    MobKind::Zoglin,
    MobKind::Axolotl,
    MobKind::Goat,
    MobKind::Frog,
    MobKind::Tadpole,

];

impl MobKind {
    pub fn by_name(name: &str) -> Option<MobKind> {
        // Called per move (fall damage, fluids): a table rather than a scan of the types.
        static BY_NAME: std::sync::OnceLock<std::collections::HashMap<&'static str, MobKind>> = std::sync::OnceLock::new();
        BY_NAME.get_or_init(|| ALL_KINDS.iter().map(|&k| (k.type_name(), k)).collect()).get(name).copied()
    }

    /// The extension type's behaviour (`None` for the shared-code types).
    pub fn ext(self) -> Option<&'static dyn ext::Kind> {
        kinds::of(self)
    }

    pub fn type_name(self) -> &'static str {
        if let Some(k) = self.ext() {
            return k.info().name;
        }
        match self {
            MobKind::Pig => "minecraft:pig",
            MobKind::Cow => "minecraft:cow",
            MobKind::Sheep => "minecraft:sheep",
            MobKind::Chicken => "minecraft:chicken",
            MobKind::Zombie => "minecraft:zombie",
            MobKind::Skeleton => "minecraft:skeleton",
            MobKind::Creeper => "minecraft:creeper",
            MobKind::Spider => "minecraft:spider",
            _ => unreachable!("extension type"),
        }
    }

    pub fn short_name(self) -> &'static str {
        &self.type_name()[10..]
    }

    pub fn category(self) -> Category {
        if let Some(k) = self.ext() {
            return k.info().category;
        }
        match self {
            MobKind::Pig | MobKind::Cow | MobKind::Sheep | MobKind::Chicken => Category::Creature,
            _ => Category::Monster,
        }
    }

    /// Extends `Animal`.
    pub fn is_animal(self) -> bool {
        match self.ext() {
            Some(k) => k.info().animal,
            None => self.category() == Category::Creature,
        }
    }

    /// `DefaultAttributes` for the type.
    pub fn attributes(self) -> Attributes {
        use Attr::*;
        let animal = [(FollowRange, 16.0), (TemptRange, 10.0)];
        let monster = [(FollowRange, 16.0), (AttackDamage, 2.0)];
        if let Some(k) = self.ext() {
            let info = k.info();
            let mut v = vec![(FollowRange, 16.0)];
            if info.animal {
                v.push((TemptRange, 10.0));
            } else if info.monster_base {
                v.push((AttackDamage, 2.0));
            }
            v.extend_from_slice(info.attrs);
            return Attributes::new(&v);
        }
        let mut v: Vec<(Attr, f64)> = if self.is_animal() { animal.to_vec() } else { monster.to_vec() };
        match self {
            MobKind::Pig => v.extend([(MaxHealth, 10.0), (MovementSpeed, 0.25)]),
            MobKind::Cow => v.extend([(MaxHealth, 10.0), (MovementSpeed, 0.20000000298023224)]),
            MobKind::Sheep => v.extend([(MaxHealth, 8.0), (MovementSpeed, 0.23000000417232513)]),
            MobKind::Chicken => v.extend([(MaxHealth, 4.0), (MovementSpeed, 0.25)]),
            MobKind::Zombie => v.extend([
                (FollowRange, 35.0),
                (MovementSpeed, 0.23000000417232513),
                (AttackDamage, 3.0),
                (Armor, 2.0),
                (SpawnReinforcements, 0.0),
            ]),
            MobKind::Skeleton | MobKind::Creeper => v.push((MovementSpeed, 0.25)),
            MobKind::Spider => v.extend([(MaxHealth, 16.0), (MovementSpeed, 0.30000001192092896)]),
            _ => {}
        }
        Attributes::new(&v)
    }

    /// `getAmbientSoundInterval`.
    pub fn ambient_sound_interval(self) -> i32 {
        if let Some(k) = self.ext() {
            return k.info().ambient_interval;
        }
        if self.is_animal() { 120 } else { 80 }
    }

    pub fn max_head_y_rot(self) -> i32 {
        self.ext().map_or(75, |k| k.info().head.0)
    }
    pub fn max_head_x_rot(self) -> i32 {
        self.ext().map_or(40, |k| k.info().head.1)
    }
    pub fn head_rot_speed(self) -> i32 {
        self.ext().map_or(10, |k| k.info().head.2)
    }

    /// `EntityTypeTags.BURN_IN_DAYLIGHT`.
    pub fn burns_in_daylight(self) -> bool {
        match self.ext() {
            Some(k) => k.info().burns_in_daylight,
            None => matches!(self, MobKind::Zombie | MobKind::Skeleton),
        }
    }

    /// `EntityTypeTags.CAN_BREATHE_UNDER_WATER` (undead).
    pub fn breathes_under_water(self) -> bool {
        match self.ext() {
            Some(k) => k.info().breathes_under_water,
            None => matches!(self, MobKind::Zombie | MobKind::Skeleton),
        }
    }

    /// `EntityType.fireImmune`.
    pub fn fire_immune(self) -> bool {
        self.ext().is_some_and(|k| k.info().fire_immune)
    }

    /// `instanceof Zombie` (husks, drowned, zombie villagers and zombified piglins too).
    pub fn is_zombie(self) -> bool {
        matches!(self, MobKind::Zombie | MobKind::Husk | MobKind::Drowned | MobKind::ZombieVillager | MobKind::ZombifiedPiglin)
    }

    /// `instanceof AbstractSkeleton`.
    pub fn is_skeleton(self) -> bool {
        matches!(self, MobKind::Skeleton | MobKind::Stray | MobKind::WitherSkeleton | MobKind::Bogged)
    }

    pub fn loot_table(self) -> String {
        format!("minecraft:entities/{}", self.short_name())
    }

    /// The type's `entity.<name>.<what>` sound, if it has one.
    fn sound_opt(self, what: &str) -> Option<&'static str> {
        let base = self.ext().and_then(|k| k.info().sounds).unwrap_or(self.short_name());
        let s = format!("minecraft:entity.{base}.{what}");
        kiln_data::builtin_entries("minecraft:sound_event").and_then(|e| e.iter().find(|x| **x == s).copied())
    }

    fn sound(self, what: &str) -> &'static str {
        self.sound_opt(what).unwrap_or("minecraft:entity.generic.hurt")
    }

    pub fn ambient_sound(self) -> Option<&'static str> {
        if self.ext().is_some() {
            return self.sound_opt("ambient");
        }
        (self != MobKind::Creeper).then(|| self.sound("ambient"))
    }
    pub fn hurt_sound(self) -> &'static str {
        self.sound("hurt")
    }
    pub fn death_sound(self) -> &'static str {
        self.sound("death")
    }
    pub fn step_sound(self) -> &'static str {
        self.sound("step")
    }
    pub fn sound_source(self) -> &'static str {
        if let Some(k) = self.ext() {
            return k.info().sound_source;
        }
        if self.is_animal() { "neutral" } else { "hostile" }
    }
}

/// Why a mob took damage (`DamageSource`): the damage type, who caused it and what dealt it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageSource {
    pub kind: DamageKind,
    /// `getEntity`: the responsible entity (a player, a skeleton for its arrow).
    pub attacker: Option<i32>,
    /// `getDirectEntity` when different (the arrow).
    pub direct: Option<i32>,
    /// `getSourcePosition`.
    pub pos: Option<Vec3>,
    pub attacker_is_player: bool,
}

impl DamageSource {
    pub fn of(kind: DamageKind) -> DamageSource {
        DamageSource { kind, attacker: None, direct: None, pos: None, attacker_is_player: false }
    }
}

/// Type-specific state.
#[derive(Clone, Debug, PartialEq)]
pub enum Species {
    Pig,
    Cow,
    Sheep { color: u8, sheared: bool },
    Chicken { egg_time: i32 },
    Zombie { can_break_doors: bool, drowning: kinds::zombie::Tracker },
    /// `Skeleton.freezingTracker` (powder snow turns it into a stray).
    Skeleton { freezing: kinds::zombie::Tracker },
    Creeper { swell: i32, old_swell: i32, swell_dir: i32, max_swell: i32, radius: i32, powered: bool, ignited: bool },
    Spider { climbing: bool },
    /// An extension type without state of its own.
    Plain,
    /// An extension type's state (see [`ext::state`]).
    Ext(Box<dyn ext::MobExt>),
}

#[derive(Clone, Debug)]
pub struct MobData {
    pub kind: MobKind,
    pub attrs: Attributes,
    pub health: f32,
    pub absorption: f32,
    pub y_head_rot: f32,
    pub y_head_rot_o: f32,
    pub y_body_rot: f32,
    pub y_body_rot_o: f32,
    pub xxa: f32,
    pub yya: f32,
    pub zza: f32,
    pub speed: f32,
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub hurt_time: i32,
    pub hurt_duration: i32,
    pub damage_cooldown: i32,
    pub last_hurt: f32,
    pub death_time: i32,
    pub dead: bool,
    pub last_hurt_by_mob: Option<i32>,
    pub last_hurt_by_mob_timestamp: i32,
    pub last_hurt_by_player: Option<i32>,
    pub last_hurt_by_player_memory: i32,
    pub last_hurt_mob: Option<i32>,
    pub last_damage_source: Option<DamageSource>,
    pub last_damage_stamp: i64,
    pub no_action_time: i32,
    pub ambient_sound_time: i32,
    pub persistence_required: bool,
    pub no_ai: bool,
    pub left_handed: bool,
    pub aggressive: bool,
    pub can_pick_up_loot: bool,
    /// `AgeableMob.age` (negative for babies); zombies keep a baby flag instead.
    pub age: i32,
    pub zombie_baby: bool,
    /// `AgeableMob.forcedAge`, `forcedAgeTimer`, the age lock (golden dandelion) and its
    /// particle timer.
    pub forced_age: i32,
    pub forced_age_timer: i32,
    pub age_locked: bool,
    pub age_lock_timer: i32,
    /// `Animal.inLove` ticks and the player who fed it (`loveCause`).
    pub in_love: i32,
    pub love_cause: Option<i32>,
    pub target: Option<i32>,
    pub look: control::LookControl,
    pub mov: control::MoveControl,
    pub jump: control::JumpControl,
    pub body: control::BodyRotationControl,
    pub nav: path::Navigation,
    pub maluses: Vec<(path::PathType, f32)>,
    /// `Sensing`: line of sight results of this tick.
    pub seen: Vec<i32>,
    pub unseen: Vec<i32>,
    pub goals: GoalSelector,
    pub targets: GoalSelector,
    pub equipment: [ItemStack; 6],
    pub drop_chances: [f32; 6],
    /// Ticks the current item (a bow) has been used.
    pub using_item: Option<i32>,
    pub species: Species,
    /// `*_variant` and `*_sound_variant` registry ids (pigs, cows, chickens).
    pub variant: i32,
    pub sound_variant: i32,
    pub is_vehicle: bool,
    /// Arm swung this tick (for the animation packet).
    pub swing: bool,
    /// Entity events for viewers (hurt, death, eating...) are emitted as [`Event::EntityEvent`].
    pub air_supply_max: i32,
    /// `ServerEntity` needs the last hurt direction for the damage event.
    pub hurt_by: Option<(DamageKind, Option<i32>, Option<i32>)>,
    /// The attribute modifiers the equipment added (`collectEquipmentChanges`).
    pub equip_mods: Vec<(Attr, String)>,
    /// `activeEffects` (see [`effects`]).
    pub effects: crate::effect::Effects,
    /// The `Brain` of brain-driven types (taken out of the mob while it ticks).
    pub brain: Option<Box<brain::Brain>>,
    /// The mob's own stream for what vanilla draws from `level.getRandom()` in its AI (per entity,
    /// so the outcome does not depend on which entities share a region).
    pub brain_random: kiln_javamath::random::LegacyRandom,
    /// `LivingEntity.discardFriction` (long-jumping frogs and goats keep their momentum).
    pub discard_friction: bool,
}

impl MobData {
    /// A mob as its constructor leaves it (`EntityType.create`): full health, default
    /// attributes, the type's goals. The constructor's random draws (the initial yaw, a
    /// chicken's egg timer) come from `random`.
    pub fn new(kind: MobKind, random: &mut dyn RandomSource) -> MobData {
        let attrs = kind.attributes();
        let health = attrs.value(Attr::MaxHealth) as f32;
        let species = match kind {
            MobKind::Pig => Species::Pig,
            MobKind::Cow => Species::Cow,
            MobKind::Sheep => Species::Sheep { color: 0, sheared: false },
            MobKind::Chicken => Species::Chicken { egg_time: 0 },
            MobKind::Zombie => Species::Zombie { can_break_doors: false, drowning: kinds::zombie::Tracker::default() },
            MobKind::Skeleton => Species::Skeleton { freezing: kinds::zombie::Tracker::default() },
            MobKind::Creeper => {
                Species::Creeper { swell: 0, old_swell: 0, swell_dir: -1, max_swell: 30, radius: 3, powered: false, ignited: false }
            }
            MobKind::Spider => Species::Spider { climbing: false },
            _ => Species::Plain,
        };
        let mut m = MobData {
            kind,
            attrs,
            health,
            absorption: 0.0,
            y_head_rot: 0.0,
            y_head_rot_o: 0.0,
            y_body_rot: 0.0,
            y_body_rot_o: 0.0,
            xxa: 0.0,
            yya: 0.0,
            zza: 0.0,
            speed: 0.0,
            jumping: false,
            no_jump_delay: 0,
            hurt_time: 0,
            hurt_duration: 0,
            damage_cooldown: 0,
            last_hurt: 0.0,
            death_time: 0,
            dead: false,
            last_hurt_by_mob: None,
            last_hurt_by_mob_timestamp: 0,
            last_hurt_by_player: None,
            last_hurt_by_player_memory: 0,
            last_hurt_mob: None,
            last_damage_source: None,
            last_damage_stamp: 0,
            no_action_time: 0,
            ambient_sound_time: 0,
            persistence_required: false,
            no_ai: false,
            left_handed: false,
            aggressive: false,
            can_pick_up_loot: false,
            age: 0,
            zombie_baby: false,
            forced_age: 0,
            forced_age_timer: 0,
            age_locked: false,
            age_lock_timer: 0,
            in_love: 0,
            love_cause: None,
            target: None,
            look: control::LookControl::default(),
            mov: control::MoveControl::default(),
            jump: control::JumpControl::default(),
            body: control::BodyRotationControl::default(),
            nav: path::Navigation::new(kind == MobKind::Spider),
            maluses: Vec::new(),
            seen: Vec::new(),
            unseen: Vec::new(),
            goals: GoalSelector::default(),
            targets: GoalSelector::default(),
            equipment: std::array::from_fn(|_| ItemStack::empty()),
            drop_chances: [0.085; 6],
            using_item: None,
            species,
            variant: kiln_data::synced_id(&format!("{}_variant", kind.type_name()), "minecraft:temperate").unwrap_or(0),
            sound_variant: kiln_data::synced_id(&format!("{}_sound_variant", kind.type_name()), "minecraft:classic").unwrap_or(0),
            is_vehicle: false,
            swing: false,
            air_supply_max: 300,
            hurt_by: None,
            equip_mods: Vec::new(),
            effects: crate::effect::Effects::new(),
            brain: None,
            brain_random: kiln_javamath::random::LegacyRandom::new(0),
            discard_friction: false,
        };
        if kind.is_animal() {
            m.maluses.push((path::PathType::FireInNeighbor, 16.0));
            m.maluses.push((path::PathType::Fire, -1.0));
        }
        if let Species::Chicken { egg_time } = &mut m.species {
            *egg_time = random.next_int_bounded(6000) + 6000;
            m.maluses.push((path::PathType::Water, 0.0));
        }
        if let Some(k) = kind.ext() {
            if let Some(s) = k.new_state(&mut m, random) {
                m.species = Species::Ext(s);
            }
            k.register_goals(&mut m);
            if m.goals.goals.iter().any(|w| matches!(w.goal, Goal::Float)) {
                m.nav.can_float = true;
            }
            return m;
        }
        register_goals(&mut m);
        m
    }

    pub fn baby(&self) -> bool {
        // (`canBeABaby`: frogs never are.)
        self.zombie_baby || (breed::is_ageable(self.kind) && self.age < 0 && self.kind != MobKind::Frog)
    }

    pub fn holding_bow(&self) -> bool {
        [MAINHAND, OFFHAND].iter().any(|&i| !self.equipment[i].is_empty() && item_name(&self.equipment[i]) == "minecraft:bow")
    }

    pub fn set_aggressive(&mut self, on: bool) {
        self.aggressive = on;
    }

    pub fn swell_dir(&self) -> i32 {
        match self.species {
            Species::Creeper { swell_dir, .. } => swell_dir,
            _ => -1,
        }
    }

    pub fn set_swell_dir(&mut self, dir: i32) {
        if let Species::Creeper { swell_dir, .. } = &mut self.species {
            *swell_dir = dir;
        }
    }

    pub fn start_using_item(&mut self) {
        if self.using_item.is_none() {
            self.using_item = Some(0);
        }
    }

    pub fn stop_using_item(&mut self) {
        self.using_item = None;
    }

    pub fn ticks_using_item(&self) -> i32 {
        self.using_item.unwrap_or(0)
    }

    /// `getMaxHeadXRot` of this mob (its type's, or what its state makes it).
    pub fn max_head_x_rot(&self) -> i32 {
        match self.kind.ext() {
            Some(k) => k.max_head_x_rot(self),
            None => self.kind.max_head_x_rot(),
        }
    }

    pub fn max_health(&self) -> f32 {
        self.attrs.value(Attr::MaxHealth) as f32
    }

    /// `setHealth`: clamped to [0, max health].
    pub fn set_health(&mut self, h: f32) {
        self.health = mth::clamp(h, 0.0, self.max_health());
    }

    pub fn is_dead_or_dying(&self) -> bool {
        self.health <= 0.0 || self.dead
    }

    /// `getLastDamageSource`: the last hit if it was within 40 ticks.
    pub fn last_damage_source(&self, now: i64) -> Option<DamageSource> {
        if now - self.last_damage_stamp > 40 { None } else { self.last_damage_source }
    }

    /// A brain's active activities and running behaviours (`act:idle`, `run:MoveToTargetSink`),
    /// for parity traces (anonymous behaviours have no name and are left out).
    pub fn brain_trace(&self) -> Vec<String> {
        let Some(b) = &self.brain else { return Vec::new() };
        let mut acts: Vec<String> = b.st.active_activities().iter().map(|a| format!("act:{}", a.name())).collect();
        acts.sort();
        let mut run: Vec<String> = b.running_names().into_iter().filter(|n| !n.is_empty()).map(|n| format!("run:{n}")).collect();
        run.sort();
        acts.extend(run);
        // The memories that hold a value, with the ticks left of expiring ones.
        let mut mems: Vec<String> = b
            .st
            .mem
            .iter()
            .map(|(m, _, ttl)| if ttl == i64::MAX { format!("m:{}", m.name()) } else { format!("m:{}@{ttl}", m.name()) })
            .collect();
        mems.sort();
        acts.extend(mems);
        // The stream vanilla's level random plays (`MobVectors` prints it as `lr:`).
        acts.push(format!("lr:{}", self.brain_random.state()));
        acts
    }

    /// Running goal names, for tests and parity traces.
    pub fn running_goals(&self) -> Vec<&'static str> {
        let mut v = self.goals.running_names();
        v.extend(self.targets.running_names());
        v
    }
}

/// `getArmorCoverPercentage`: the share of the four armor slots that hold something.
pub fn armor_cover(m: &MobData) -> f32 {
    let worn = [FEET, LEGS, CHEST, HEAD].iter().filter(|&&i| !m.equipment[i].is_empty()).count();
    worn as f32 / 4.0
}

/// A `minecraft:sound_event` id as a static name (the generic hurt sound if unknown).
pub fn sound_event(name: &str) -> &'static str {
    kiln_data::builtin_entries("minecraft:sound_event").and_then(|e| e.iter().find(|x| **x == name).copied()).unwrap_or("minecraft:entity.generic.hurt")
}

/// Whether item id `item` is in the `minecraft:item` tag `tag`.
pub fn item_tag(item: i32, tag: &str) -> bool {
    item > 0
        && kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:item")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
            .is_some_and(|(_, ids)| ids.contains(&item))
}

/// Whether entity type `type_name` is in the `minecraft:entity_type` tag `tag` (e.g.
/// `minecraft:undead`, `minecraft:raiders`).
pub fn entity_type_tag(type_name: &str, tag: &str) -> bool {
    let Some(id) = kiln_data::builtin_id("minecraft:entity_type", type_name) else { return false };
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:entity_type")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

/// The variant components of a mob (`Entity.get(DataComponents.*_VARIANT)`) that entity
/// predicates' `components` can match.
pub fn variant_components(m: &MobData) -> Vec<kiln_item::Component> {
    use kiln_item::Component as C;
    use kiln_item::component::variant as v;
    match m.kind {
        MobKind::Cat => vec![C::CatVariant(v::CatVariant(m.variant)), C::CatSoundVariant(v::CatSoundVariant(m.sound_variant))],
        MobKind::Wolf => vec![C::WolfVariant(v::WolfVariant(m.variant)), C::WolfSoundVariant(v::WolfSoundVariant(m.sound_variant))],
        MobKind::Pig => vec![C::PigVariant(v::PigVariant(m.variant)), C::PigSoundVariant(v::PigSoundVariant(m.sound_variant))],
        MobKind::Cow => vec![C::CowVariant(v::CowVariant(m.variant)), C::CowSoundVariant(v::CowSoundVariant(m.sound_variant))],
        MobKind::Chicken => {
            vec![C::ChickenVariant(v::ChickenVariant(m.variant)), C::ChickenSoundVariant(v::ChickenSoundVariant(m.sound_variant))]
        }
        MobKind::Salmon | MobKind::TropicalFish | MobKind::Mooshroom => kinds::fish::variant_components(m).unwrap_or_default(),
        MobKind::Axolotl => kinds::axolotl::variant_components(m).unwrap_or_default(),
        MobKind::Frog => vec![C::FrogVariant(v::FrogVariant(m.variant))],
        _ => Vec::new(),
    }
}

pub fn item_name(s: &ItemStack) -> &'static str {
    kiln_data::builtin_entries("minecraft:item").and_then(|e| e.get(s.item() as usize).copied()).unwrap_or("minecraft:air")
}

/// `registerGoals` of each type.
fn register_goals(m: &mut MobData) {
    let g = &mut m.goals;
    let t = &mut m.targets;
    let stroll = |speed: f64| Goal::RandomStroll { speed, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false };
    let look = |dist: f32| Goal::LookAtPlayer { dist, probability: 0.02, look_at: None, look_time: 0 };
    let around = || Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 };
    let tempt = |speed: f64| Goal::Tempt { speed, calm_down: 0, player: None };
    let panic = |speed: f64| Goal::Panic { speed, pos: Vec3::ZERO };
    let breed = |speed: f64| Goal::Breed { speed, partner: None, love_time: 0 };
    let follow_parent = |speed: f64| Goal::FollowParent { speed, parent: None, recalc: 0 };
    let melee = |kind: MeleeKind, speed: f64, follow: bool| Goal::Melee {
        kind,
        speed,
        follow_unseen: follow,
        path: None,
        recalc: 0,
        next_attack: 0,
        last_can_use: 0,
        pathed: Vec3::ZERO,
        raise_arm: 0,
    };
    let nearest = |wanted: Wanted, must_see: bool| Goal::NearestAttackable { wanted, interval: mth::reduced_tick_delay(10), must_see, target: None, unseen: 0, spider: false };
    let hurt_by = |alert: bool| Goal::HurtByTarget { timestamp: 0, alert_others: alert, target_mob: None, unseen: 0, unseen_memory: 60 };
    match m.kind {
        _ if m.kind.ext().is_some() => {}
        MobKind::Pig => {
            g.add(0, Goal::Float);
            g.add(1, panic(1.25));
            g.add(3, breed(1.0));
            g.add(4, tempt(1.2));
            g.add(4, tempt(1.2));
            g.add(5, follow_parent(1.1));
            g.add(6, stroll(1.0));
            g.add(7, look(6.0));
            g.add(8, around());
        }
        MobKind::Cow => {
            g.add(0, Goal::Float);
            g.add(1, panic(2.0));
            g.add(2, breed(1.0));
            g.add(3, tempt(1.25));
            g.add(4, follow_parent(1.25));
            g.add(5, stroll(1.0));
            g.add(6, look(6.0));
            g.add(7, around());
        }
        MobKind::Sheep => {
            g.add(0, Goal::Float);
            g.add(1, panic(1.25));
            g.add(2, breed(1.0));
            g.add(3, tempt(1.1));
            g.add(4, follow_parent(1.1));
            g.add(5, Goal::EatBlock { tick: 0 });
            g.add(6, stroll(1.0));
            g.add(7, look(6.0));
            g.add(8, around());
        }
        MobKind::Chicken => {
            g.add(0, Goal::Float);
            g.add(1, panic(1.4));
            g.add(2, breed(1.0));
            g.add(3, tempt(1.0));
            g.add(4, follow_parent(1.1));
            g.add(5, stroll(1.0));
            g.add(6, look(6.0));
            g.add(7, around());
        }
        MobKind::Zombie => kinds::zombie::register_goals(m),
        MobKind::Skeleton => kinds::skeleton::register_goals(m),
        MobKind::Creeper => {
            g.add(1, Goal::Float);
            g.add(2, Goal::Swell { target: None });
            g.add(3, Goal::AvoidEntity);
            g.add(3, Goal::AvoidEntity);
            g.add(4, melee(MeleeKind::Plain, 1.0, false));
            g.add(5, stroll(0.8));
            g.add(6, look(8.0));
            g.add(6, around());
            t.add(1, nearest(Wanted::Player, true));
            t.add(2, hurt_by(false));
        }
        MobKind::Spider => {
            g.add(1, Goal::Float);
            g.add(2, Goal::AvoidEntity);
            g.add(3, Goal::LeapAtTarget { yd: 0.4, target: None });
            g.add(4, melee(MeleeKind::Spider, 1.0, true));
            g.add(5, stroll(0.8));
            g.add(6, look(8.0));
            g.add(6, around());
            t.add(1, hurt_by(false));
            let mut sp = nearest(Wanted::Player, true);
            if let Goal::NearestAttackable { spider, .. } = &mut sp {
                *spider = true;
            }
            t.add(2, sp.clone());
            if let Goal::NearestAttackable { wanted, .. } = &mut sp {
                *wanted = Wanted::Unsimulated;
            }
            t.add(3, sp);
        }
        _ => {}
    }
    // `FloatGoal` makes the navigation float.
    if m.goals.goals.iter().any(|w| matches!(w.goal, Goal::Float)) {
        m.nav.can_float = true;
    }
}

/// `AbstractSkeleton.reassessWeaponGoal`: the bow goal (priority 4) with a bow, else melee.
pub fn reassess_weapon_goal(m: &mut MobData, hard: bool) {
    if !m.kind.is_skeleton() {
        return;
    }
    m.goals.goals.retain(|w| !(matches!(w.goal, Goal::Melee { .. } | Goal::RangedBow { .. }) && w.priority == 4 && !w.running));
    let goal = if m.holding_bow() {
        Goal::RangedBow {
            speed: 1.0,
            interval_min: m.kind.ext().and_then(|k| k.bow_interval(hard)).unwrap_or(if hard { 20 } else { 40 }),
            radius_sqr: 15.0 * 15.0,
            attack_time: -1,
            see_time: 0,
            strafing_clockwise: false,
            strafing_backwards: false,
            strafing_time: -1,
        }
    } else {
        Goal::Melee {
            kind: MeleeKind::Plain,
            speed: 1.2,
            follow_unseen: false,
            path: None,
            recalc: 0,
            next_attack: 0,
            last_can_use: 0,
            pathed: Vec3::ZERO,
            raise_arm: 0,
        }
    };
    m.goals.add(4, goal);
}

/// A new mob entity at the origin (before `finalizeSpawn`).
pub fn new(kind: MobKind, id: i32, uuid: u128, seed: i64) -> Entity {
    let mut e = Entity::new(kind.type_name(), id, uuid, EntityKind::MobTicking { gravity: 0.08 }, seed);
    let mut m = MobData::new(kind, &mut e.random);
    // `Entity`'s constructor: `airSupply = getMaxAirSupply()` (axolotls: 6000).
    e.air_supply = m.air_supply_max;
    // `LivingEntity`'s constructor: a random yaw (in radians-sized degrees, as vanilla).
    e.y_rot = e.random.next_float() * 6.2831855;
    m.y_head_rot = e.y_rot;
    // ... then the brain (its sensors' first scans are delayed by draws from the mob's random).
    m.brain_random = kiln_javamath::random::LegacyRandom::new(seed ^ 0x2545_F491_4F6C_DD1D);
    if let Some(k) = kind.ext() {
        m.brain = k.make_brain(&m, &mut e.random).map(Box::new);
    }
    e.max_up_step = m.attrs.value(Attr::StepHeight) as f32;
    // `EnderDragon`'s constructor: `noPhysics`.
    e.no_physics = kind == MobKind::EnderDragon;
    // Constructors that size the mob by its state (salmon, pufferfish: `refreshDimensions`).
    refresh_dimensions(&mut e, &m);
    e.kind = EntityKind::Mob(Box::new(m));
    e
}

/// `AgeableMob.setAge`: crossing zero toggles the baby flag and the size.
pub fn set_age(e: &mut Entity, m: &mut MobData, age: i32) {
    let old = m.age;
    m.age = age;
    if (old < 0) != (age < 0) {
        refresh_dimensions(e, m);
        if let Some(k) = m.kind.ext() {
            k.age_boundary_reached(e, m);
        }
    }
}

/// `AgeableMob.ageUp(seconds, forced)`.
pub fn age_up(e: &mut Entity, m: &mut MobData, seconds: i32, forced: bool) {
    let old = m.age;
    let age = (old + seconds * 20).min(0);
    let delta = age - old;
    set_age(e, m, age);
    if forced {
        m.forced_age += delta;
        if m.forced_age_timer == 0 {
            m.forced_age_timer = 40;
        }
    }
    if m.age == 0 {
        let f = m.forced_age;
        set_age(e, m, f);
    }
}

/// `Entity.getRandomX(scale)`, `getRandomY()`, `getRandomZ(scale)`: a random point of the box
/// (particle positions: only the draws matter on the server).
pub fn random_point(e: &mut Entity, scale: f64) -> Vec3 {
    let x = e.x() + e.width as f64 * (2.0 * e.random.next_double() - 1.0) * scale;
    let y = e.y() + e.height as f64 * e.random.next_double();
    let z = e.z() + e.width as f64 * (2.0 * e.random.next_double() - 1.0) * scale;
    Vec3::new(x, y, z)
}

/// `Entity.refreshDimensions` for a mob: the type's size (or its baby size: half size, or the
/// type's own `BABY_DIMENSIONS`) scaled by the scale attribute. Approximation: a mob that grows
/// next to a wall is not nudged out of it (`fudgePositionAfterSizeChange`).
pub fn refresh_dimensions(e: &mut Entity, m: &MobData) {
    let Some(t) = kiln_data::entities::by_name(e.type_name) else { return };
    let (mut w, mut h, mut eye) = species::dimensions(m, (t.width, t.height, t.eye_height));
    let scale = m.attrs.value(Attr::Scale) as f32;
    if scale != 1.0 {
        (w, h, eye) = (w * scale, h * scale, eye * scale);
    }
    if (w, h, eye) == (e.width, e.height, e.eye_height) {
        return;
    }
    e.width = w;
    e.height = h;
    e.eye_height = eye;
    let p = e.position();
    e.set_pos(p);
}

/// `Entity.refreshDimensions` with the level at hand: a mob that grew (outside its first tick)
/// is moved to the free spot nearest its old center (`fudgePositionAfterSizeChange`).
pub fn refresh_dimensions_in(e: &mut Entity, m: &MobData, level: &dyn EntityLevel) {
    let (old_w, old_h) = (e.width, e.height);
    refresh_dimensions(e, m);
    let (w, h) = (e.width, e.height);
    if e.first_tick || e.no_physics || w > 4.0 || h > 4.0 || !(w > old_w || h > old_h) {
        return;
    }
    let old_center = e.position().add(0.0, old_h as f64 / 2.0, 0.0);
    let wd = (w - old_w).max(0.0) as f64 + 1.0e-6;
    let hd = (h - old_h).max(0.0) as f64 + 1.0e-6;
    if let Some(p) = find_free_position(e, level, old_center, wd, hd, w as f64, h as f64) {
        e.set_pos(p.add(0.0, -(h as f64) / 2.0, 0.0));
        return;
    }
    if w > old_w && h > old_h
        && let Some(p) = find_free_position(e, level, old_center, wd, 1.0e-6, w as f64, old_h as f64)
    {
        e.set_pos(p.add(0.0, -(old_h as f64) / 2.0 + 1.0e-6, 0.0));
    }
}

/// `CollisionGetter.findFreePosition` over an allowed box of centers (`wd` by `hd` around
/// `center`): the point of it nearest `center` where a `w` by `h` box touches no block.
fn find_free_position(e: &Entity, level: &dyn EntityLevel, center: Vec3, wd: f64, hd: f64, w: f64, h: f64) -> Option<Vec3> {
    let allowed = Aabb::new(center.x - wd / 2.0, center.y - hd / 2.0, center.z - wd / 2.0, center.x + wd / 2.0, center.y + hd / 2.0, center.z + wd / 2.0);
    let search = allowed.inflate(w, h, w);
    let mut blocked: Vec<Aabb> = Vec::new();
    let ctx = e.collision_context();
    crate::collision::for_each_block_collision(level, &ctx, &search, |pos, shape, _| {
        for b in shape.boxes() {
            blocked.push(b.offset(pos.x as f64, pos.y as f64, pos.z as f64).inflate(w / 2.0, h / 2.0, w / 2.0));
        }
        true
    });
    // The free part of the allowed box, cut along every blocked face, nearest cell first.
    let cuts = |lo: f64, hi: f64, f: &dyn Fn(&Aabb) -> [f64; 2]| {
        let mut v = vec![lo, hi];
        for b in &blocked {
            for c in f(b) {
                if c > lo && c < hi {
                    v.push(c);
                }
            }
        }
        v.sort_by(f64::total_cmp);
        v.dedup();
        v
    };
    let xs = cuts(allowed.min_x, allowed.max_x, &|b| [b.min_x, b.max_x]);
    let ys = cuts(allowed.min_y, allowed.max_y, &|b| [b.min_y, b.max_y]);
    let zs = cuts(allowed.min_z, allowed.max_z, &|b| [b.min_z, b.max_z]);
    let mut best: Option<(f64, Vec3)> = None;
    for i in 0..xs.len() - 1 {
        for j in 0..ys.len() - 1 {
            for k in 0..zs.len() - 1 {
                let mid = Vec3::new((xs[i] + xs[i + 1]) / 2.0, (ys[j] + ys[j + 1]) / 2.0, (zs[k] + zs[k + 1]) / 2.0);
                if blocked.iter().any(|b| b.min_x < mid.x && mid.x < b.max_x && b.min_y < mid.y && mid.y < b.max_y && b.min_z < mid.z && mid.z < b.max_z) {
                    continue;
                }
                let p = Vec3::new(center.x.clamp(xs[i], xs[i + 1]), center.y.clamp(ys[j], ys[j + 1]), center.z.clamp(zs[k], zs[k + 1]));
                let d = p.distance_to_sqr(center);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

pub fn data(e: &Entity) -> Option<&MobData> {
    match &e.kind {
        EntityKind::Mob(m) => Some(m),
        _ => None,
    }
}

/// `/kill` outside a level tick: health drops to zero and the mob plays its death (the
/// death tick removes it). Approximation: no loot, no death event.
pub fn kill(e: &mut Entity) {
    if let Some(m) = data_mut(e)
        && !m.dead
    {
        m.health = 0.0;
        m.dead = true;
    }
}

/// `Mob.setTarget`, with the type's override first (zombified piglins draw their anger timers).
pub fn set_target(e: &mut Entity, m: &mut MobData, target: Option<i32>) {
    if let Some(k) = m.kind.ext() {
        k.on_set_target(e, m, target);
    }
    m.target = target;
}

/// `setTarget` on mob `id` (not the one ticking).
pub fn set_target_of(level: &mut dyn EntityLevel, id: i32, target: Option<i32>) {
    let Some(o) = level.entity_mut(id) else { return };
    if !matches!(o.kind, EntityKind::Mob(_)) {
        return;
    }
    let mut m = take(o);
    set_target(o, &mut m, target);
    put(o, m);
}

pub fn data_mut(e: &mut Entity) -> Option<&mut MobData> {
    match &mut e.kind {
        EntityKind::Mob(m) => Some(m),
        _ => None,
    }
}

fn take(e: &mut Entity) -> Box<MobData> {
    let gravity = match &e.kind {
        EntityKind::Mob(m) => m.attrs.value(Attr::Gravity),
        _ => 0.08,
    };
    match std::mem::replace(&mut e.kind, EntityKind::MobTicking { gravity: if e.no_gravity { 0.0 } else { gravity } }) {
        EntityKind::Mob(m) => m,
        _ => unreachable!("not a mob"),
    }
}

fn put(e: &mut Entity, m: Box<MobData>) {
    e.kind = EntityKind::Mob(m);
}

// ---------------------------------------------------------------------- the tick

/// `Mob.tick` (the level ran `commonTick`): the type's pre-tick, `LivingEntity.tick`, then
/// `Mob.tick`'s control flags.
pub fn tick(e: &mut Entity, level: &mut dyn EntityLevel) {
    let mut m = take(e);
    m.swing = false;
    species::pre_tick(e, &mut m, level);
    living_tick(e, &mut m, level);
    if e.tick_count % 5 == 0 {
        // `updateControlFlags`: no mob passengers, no boats.
        m.goals.set_control_flag(goals::MOVE, true);
        m.goals.set_control_flag(goals::JUMP, true);
        m.goals.set_control_flag(goals::LOOK, true);
    }
    species::post_tick(e, &mut m, level);
    // `LivingEntity.remove`: a mob that was removed (killed, converted, discarded) forgets what
    // its brain remembered.
    if e.is_removed()
        && let Some(b) = m.brain.as_mut()
    {
        b.st.mem.clear_all();
        m.target = None;
    }
    put(e, m);
}

fn living_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    base_tick(e, m, level);
    if m.using_item.is_some()
        && let Some(k) = m.kind.ext()
    {
        k.update_using_item(e, m, level);
    }
    if let Some(t) = m.using_item.as_mut() {
        *t += 1;
    }
    sync_equipment_modifiers(m);
    if !e.is_removed() {
        ai_step(e, m, level);
    }
    // Body and head rotation (`Mob.tickHeadTurn`), then angle unwinding of the old values.
    if !m.kind.ext().is_some_and(|k| k.tick_body(e, m)) {
        control::tick_body(e, m);
    }
    while e.y_rot - e.y_rot_o < -180.0 {
        e.y_rot_o -= 360.0;
    }
    while e.y_rot - e.y_rot_o >= 180.0 {
        e.y_rot_o += 360.0;
    }
    while m.y_body_rot - m.y_body_rot_o < -180.0 {
        m.y_body_rot_o -= 360.0;
    }
    while m.y_body_rot - m.y_body_rot_o >= 180.0 {
        m.y_body_rot_o += 360.0;
    }
    while e.x_rot - e.x_rot_o < -180.0 {
        e.x_rot_o -= 360.0;
    }
    while e.x_rot - e.x_rot_o >= 180.0 {
        e.x_rot_o += 360.0;
    }
    while m.y_head_rot - m.y_head_rot_o < -180.0 {
        m.y_head_rot_o -= 360.0;
    }
    while m.y_head_rot - m.y_head_rot_o >= 180.0 {
        m.y_head_rot_o += 360.0;
    }
}

/// `LivingEntity.detectEquipmentUpdates`: the attribute modifiers of the items worn and held
/// (a sword's attack damage, armor points) replace those of the previous equipment. Broken
/// items give none; enchantment attribute effects are not applied to mobs.
pub fn sync_equipment_modifiers(m: &mut MobData) {
    use kiln_item::component::{AttributeOperation, EquipmentSlotGroup as G};
    let mut want: Vec<(Attr, String, f64, Op)> = Vec::new();
    for slot in 0..6 {
        let stack = &m.equipment[slot];
        if stack.is_empty() || (stack.is_damageable_item() && stack.damage() >= stack.max_damage()) {
            continue;
        }
        let Some(mods) = stack.get(kiln_item::keys::ATTRIBUTE_MODIFIERS) else { continue };
        for md in &mods.0 {
            let fits = match md.slot {
                G::Any => true,
                G::MainHand => slot == MAINHAND,
                G::OffHand => slot == OFFHAND,
                G::Hand => slot <= OFFHAND,
                G::Feet => slot == FEET,
                G::Legs => slot == LEGS,
                G::Chest => slot == CHEST,
                G::Head => slot == HEAD,
                G::Armor => slot >= FEET,
                G::Body | G::Saddle => false,
            };
            let name = kiln_data::builtin_entries("minecraft:attribute").and_then(|e| e.get(md.attribute as usize).copied());
            let Some(attr) = name.and_then(Attr::by_name) else { continue };
            if !fits || m.attrs.get(attr).is_none() {
                continue;
            }
            let op = match md.operation {
                AttributeOperation::AddValue => Op::AddValue,
                AttributeOperation::AddMultipliedBase => Op::AddMultipliedBase,
                AttributeOperation::AddMultipliedTotal => Op::AddMultipliedTotal,
            };
            let id = md.id.as_str().to_owned();
            want.retain(|(a, i, _, _)| !(*a == attr && *i == id));
            want.push((attr, id, md.amount, op));
        }
    }
    let same = want.len() == m.equip_mods.len() && want.iter().zip(&m.equip_mods).all(|(w, (a, i))| w.0 == *a && w.1 == *i);
    if same {
        return;
    }
    for (a, id) in std::mem::take(&mut m.equip_mods) {
        m.attrs.remove_modifier(a, &id);
    }
    for (a, id, amount, op) in want {
        m.attrs.set_modifier(a, &id, amount, op);
        m.equip_mods.push((a, id));
    }
}

/// `Mob.baseTick` → `LivingEntity.baseTick` → `Entity.baseTick`.
fn base_tick(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let air_before = e.air_supply;
    e.compute_speed();
    e.was_in_powder_snow = e.is_in_powder_snow;
    e.is_in_powder_snow = false;
    e.was_eye_in_water = e.fluid.is_eye_in_water();
    e.update_fluid_interaction(level);
    if e.remaining_fire_ticks > 0 {
        if m.kind.fire_immune() {
            e.clear_fire();
        } else {
            if e.remaining_fire_ticks % 20 == 0 && !e.is_in_lava() {
                hurt(e, m, level, DamageSource::of(DamageKind::OnFire), 1.0);
            }
            e.remaining_fire_ticks -= 1;
        }
    }
    if e.is_in_lava() {
        e.fall_distance *= 0.5;
    }
    if e.y() < (level.min_y() - 64) as f64 {
        hurt(e, m, level, DamageSource::of(DamageKind::OutOfWorld), 4.0);
    }
    e.first_tick = false;
    // `LivingEntity.baseTick`.
    if is_alive(e, m) {
        if in_wall(e, level) {
            hurt(e, m, level, DamageSource::of(DamageKind::InWall), 1.0);
        }
        let eye = BlockPos::containing(e.x(), e.eye_y(), e.z());
        let bubble = crate::blocks::block_name(level.block(eye)) == "minecraft:bubble_column";
        if e.fluid.is_eye_in_water() && !bubble {
            if !m.kind.breathes_under_water() && !effects::has_water_breathing(m) {
                e.air_supply -= 1;
                if e.air_supply <= -20 {
                    e.air_supply = 0;
                    level.emit(Event::EntityEvent { entity: e.id, event: 67 });
                    hurt(e, m, level, DamageSource::of(DamageKind::Drown), 2.0);
                }
            } else if e.air_supply < m.air_supply_max && effects::effects_refill_air(m) {
                e.air_supply = (e.air_supply + 4).min(m.air_supply_max);
            }
        } else if e.air_supply < m.air_supply_max {
            e.air_supply = (e.air_supply + 4).min(m.air_supply_max);
        }
    }
    if m.hurt_time > 0 {
        m.hurt_time -= 1;
    }
    if m.damage_cooldown > 0 {
        m.damage_cooldown -= 1;
    }
    if m.is_dead_or_dying() && !m.kind.ext().is_some_and(|k| k.tick_death(e, m, level)) {
        m.death_time += 1;
        if m.death_time >= 20 && !e.is_removed() {
            level.emit(Event::EntityEvent { entity: e.id, event: 60 });
            e.removed = Some(crate::entity::RemovalReason::Killed);
            // `LivingEntity.remove`: the brain forgets everything.
            if let Some(b) = m.brain.as_mut() {
                b.st.mem.clear_all();
            }
            effects::on_killed_removal(e, m, level);
            if let Some(k) = m.kind.ext() {
                k.on_killed_removal(e, m, level);
            }
        }
    }
    if m.last_hurt_by_player_memory > 0 {
        m.last_hurt_by_player_memory -= 1;
    } else {
        m.last_hurt_by_player = None;
    }
    if let Some(id) = m.last_hurt_mob
        && goals::living(level, id).is_none_or(|l| !l.alive)
    {
        m.last_hurt_mob = None;
    }
    if let Some(id) = m.last_hurt_by_mob {
        if goals::living(level, id).is_none_or(|l| !l.alive) || e.tick_count - m.last_hurt_by_mob_timestamp > 100 {
            m.last_hurt_by_mob = None;
        }
    }
    effects::tick(e, m, level);
    if let Some(k) = m.kind.ext() {
        k.tick_effects(e, m, level);
    }
    m.y_head_rot_o = m.y_head_rot;
    m.y_body_rot_o = m.y_body_rot;
    // `Mob.baseTick`: ambient sounds.
    if is_alive(e, m) {
        let t = m.ambient_sound_time;
        m.ambient_sound_time += 1;
        if e.random.next_int_bounded(1000) < t {
            m.ambient_sound_time = -m.kind.ambient_sound_interval();
            let sound = match m.kind.ext().and_then(|k| k.ambient_sound(e, m, &*level)) {
                Some(s) => s,
                None => m.kind.ambient_sound(),
            };
            if let Some(s) = sound {
                make_sound(e, m, level, s);
            }
        }
    }
    if let Some(k) = m.kind.ext() {
        k.after_base_tick(e, m, level, air_before);
    }
}

/// `Entity.isInWall`.
fn in_wall(e: &Entity, level: &dyn EntityLevel) -> bool {
    if e.no_physics {
        return false;
    }
    let f = (e.width * 0.8) as f64;
    let (x, y, z) = (e.x(), e.eye_y(), e.z());
    let b = Aabb::new(x - f / 2.0, y - 5.0e-7, z - f / 2.0, x + f / 2.0, y + 5.0e-7, z + f / 2.0);
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::floor(b.max_x), crate::math::floor(b.max_y), crate::math::floor(b.max_z));
    for bx in x0..=x1 {
        for by in y0..=y1 {
            for bz in z0..=z1 {
                let p = BlockPos::new(bx, by, bz);
                let s = level.block(p);
                if kiln_data::blocks_types::is_air(s) || !crate::physics::is_suffocating(s) {
                    continue;
                }
                let (shape, _) = crate::collision::collision_shape(s, p, &crate::collision::CollisionContext::EMPTY);
                if shape.boxes().iter().any(|sb| sb.offset(p.x as f64, p.y as f64, p.z as f64).intersects(&b)) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn is_alive(e: &Entity, m: &MobData) -> bool {
    !e.is_removed() && m.health > 0.0
}

/// `LivingEntity.aiStep` then the mob types' additions.
fn ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if let Some(k) = m.kind.ext() {
        if k.replaces_ai_step(e, m, level) {
            return;
        }
        k.ai_step_before(e, m, level);
        k.update_no_action_time(e, m, &*level);
    } else if m.kind.category() == Category::Monster && light_magic_value(e, level) > 0.5 {
        // `Monster.updateNoActionTime` (zombies, skeletons, creepers, spiders).
        m.no_action_time += 2;
    }
    if m.no_jump_delay > 0 {
        m.no_jump_delay -= 1;
    }
    // A mount steered by a player: its client moves it (`canSimulateMovement` is false).
    let rider = m.kind.ext().and_then(|k| k.controlling_player(e, m, &*level)).and_then(|id| level.player(id));
    if rider.is_some() {
        e.delta = e.delta.scale(0.98);
    }
    let v = e.delta;
    let (mut x, mut y, mut z) = (v.x, v.y, v.z);
    if x.abs() < 0.003 {
        x = 0.0;
    }
    if z.abs() < 0.003 {
        z = 0.0;
    }
    if y.abs() < 0.003 {
        y = 0.0;
    }
    e.delta = Vec3::new(x, y, z);
    // `applyInput`.
    m.xxa *= 0.98;
    m.zza *= 0.98;
    if m.is_dead_or_dying() || m.kind.ext().is_some_and(|k| k.is_immobile(m)) {
        m.jumping = false;
        m.xxa = 0.0;
        m.zza = 0.0;
    } else if !m.no_ai && rider.is_none() {
        server_ai_step(e, m, level);
    }
    if m.jumping {
        let h = if e.is_in_lava() { e.fluid_height_lava() } else { e.fluid_height_water() };
        let in_water = e.is_in_water() && h > 0.0;
        let threshold = if (e.eye_height as f64) < 0.4 { 0.0 } else { 0.4 };
        // `Mob.jumpInLiquid`: a mob whose navigation cannot float gets a strong push instead.
        let lift = if m.nav.can_float { 0.03999999910593033 } else { 0.3 };
        if in_water && (!e.on_ground || h > threshold) {
            if !m.kind.ext().is_some_and(|k| k.jump_in_liquid(e, m, false)) {
                e.delta = e.delta.add(0.0, lift, 0.0);
            }
        } else if e.is_in_lava() && (!e.on_ground || e.fluid_height_lava() > threshold) {
            if !m.kind.ext().is_some_and(|k| k.jump_in_liquid(e, m, true)) {
                e.delta = e.delta.add(0.0, lift, 0.0);
            }
        } else if (e.on_ground || (in_water && h <= threshold)) && m.no_jump_delay == 0 {
            if !m.kind.ext().is_some_and(|k| k.jump_from_ground(e, m, level)) {
                jump_from_ground(e, m, level);
            }
            m.no_jump_delay = 10;
        }
    } else {
        m.no_jump_delay = 0;
    }
    let input = Vec3::new(m.xxa as f64, m.yya as f64, m.zza as f64);
    if effects::has(m, crate::effect::ids::slow_falling()) || effects::has(m, crate::effect::ids::levitation()) {
        e.fall_distance = 0.0;
    }
    if let Some(r) = rider {
        // `travelRidden`: the rider turns the mount; the move comes from the rider's client.
        if let Some(k) = m.kind.ext() {
            k.tick_ridden(e, m, level, &r);
        }
        e.delta = Vec3::ZERO;
    } else if !m.no_ai && !m.kind.ext().is_some_and(|k| k.travel(e, m, level, input)) {
        travel(e, m, level, input);
    }
    if let Some((distance, multiplier)) = e.pending_fall.take() {
        cause_fall_damage(e, m, level, distance, multiplier);
    }
    e.apply_effects_from_blocks(level);
    // Freezing.
    if !e.is_in_powder_snow {
        e.ticks_frozen = (e.ticks_frozen - 2).max(0);
    }
    push_entities(e, m, level);
    if m.kind.ext().is_some_and(|k| k.sensitive_to_water()) && (e.is_in_water() || level.is_raining_at(e.block_position())) {
        hurt(e, m, level, DamageSource::of(DamageKind::Drown), 1.0);
    }
    // `Mob.aiStep`: burning in daylight.
    if m.kind.burns_in_daylight() && is_alive(e, m) && is_sun_burn_tick(e, level) {
        if m.equipment[HEAD].is_empty() {
            e.ignite_for_seconds(8.0);
        } else {
            // `hurtAndBreak(random.nextInt(2))` on the helmet that keeps the sun off.
            let n = e.random.next_int_bounded(2);
            let helmet = &mut m.equipment[HEAD];
            if n > 0 && helmet.is_damageable_item() {
                let d = helmet.damage() + n;
                if d >= helmet.max_damage() {
                    *helmet = ItemStack::empty();
                    level.emit(Event::EntityEvent { entity: e.id, event: 49 });
                } else {
                    helmet.insert(kiln_item::keys::DAMAGE, d);
                }
            }
        }
    }
    species::ai_step(e, m, level);
}

/// `Mob.isSunBurnTick`.
fn is_sun_burn_tick(e: &mut Entity, level: &dyn EntityLevel) -> bool {
    if !level.monsters_burn() {
        return false;
    }
    let f = light_magic_value(e, level);
    let p = BlockPos::containing(e.x(), e.eye_y(), e.z());
    let wet = e.is_in_water() || level.is_raining_at(e.block_position()) || e.is_in_powder_snow || e.was_in_powder_snow;
    f > 0.5 && e.random.next_float() * 30.0 < (f - 0.4) * 2.0 && !wet && level.can_see_sky(p)
}

/// `Entity.getLightLevelDependentMagicValue` at the eyes.
pub fn light_magic_value(e: &Entity, level: &dyn EntityLevel) -> f32 {
    let p = BlockPos::containing(e.x(), e.eye_y(), e.z());
    if !level.is_loaded(p) {
        return 0.0;
    }
    light_magic_value_at(level, p)
}

/// `LevelReader.getLightLevelDependentMagicValue` (ambient light 0: the overworld).
pub fn light_magic_value_at(level: &dyn EntityLevel, p: BlockPos) -> f32 {
    let f = level.raw_brightness(p, level.sky_darken()) as f32 / 15.0;
    let g = f / (4.0 - 3.0 * f);
    mth::lerp_f(level.ambient_light(), g, 1.0)
}

/// `PathfinderMob.getWalkTargetValue` for the type.
pub fn walk_target_value(m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> f32 {
    if let Some(v) = m.kind.ext().and_then(|k| k.walk_target_value(m, level, p)) {
        return v;
    }
    if m.kind.is_animal() {
        if crate::blocks::block_name(level.block(p.below())) == "minecraft:grass_block" {
            return 10.0;
        }
        return light_magic_value_at(level, p) - 0.5;
    }
    -(light_magic_value_at(level, p) - 0.5)
}

/// `Mob.serverAiStep`.
fn server_ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    m.no_action_time += 1;
    m.seen.clear();
    m.unseen.clear();
    let full = (e.tick_count + e.id) % 2 == 0 || e.tick_count <= 1;
    let mut targets = std::mem::take(&mut m.targets);
    if full {
        goals::tick(&mut targets, e, m, level);
    } else {
        goals::tick_running(&mut targets, e, m, level, false);
    }
    m.targets = targets;
    let mut sel = std::mem::take(&mut m.goals);
    if full {
        goals::tick(&mut sel, e, m, level);
    } else {
        goals::tick_running(&mut sel, e, m, level, false);
    }
    m.goals = sel;
    path::tick(e, m, level);
    breed::custom_server_ai_step(m);
    let k = m.kind.ext();
    if let Some(k) = k {
        k.custom_server_ai_step(e, m, level);
    }
    if !k.is_some_and(|k| k.tick_move(e, m, level)) {
        control::tick_move(e, m, level);
    }
    if !k.is_some_and(|k| k.tick_look(e, m, level)) {
        control::tick_look(e, m);
    }
    if !k.is_some_and(|k| k.tick_jump(e, m, level)) {
        control::tick_jump(m);
    }
}

/// `LivingEntity.causeFallDamage` (the landing happened in the move just done): the fall
/// power above the safe fall distance, scaled by the multiplier attribute, as fall damage
/// with the small or big fall sound. Approximation: the landing block's fall sound is not
/// played.
fn cause_fall_damage(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, distance: f64, multiplier: f32) {
    if entity_type_tag(e.type_name, "minecraft:fall_damage_immune") {
        return;
    }
    let power = distance + 1.0e-6 - m.attrs.value(Attr::SafeFallDistance);
    let dmg = crate::math::floor(power * multiplier as f64 * m.attrs.value(Attr::FallDamageMultiplier)) - m.kind.ext().map_or(0, |k| k.fall_damage_reduction());
    if dmg <= 0 {
        return;
    }
    let (small, big) = if m.kind.category() == Category::Monster {
        ("minecraft:entity.hostile.small_fall", "minecraft:entity.hostile.big_fall")
    } else {
        ("minecraft:entity.generic.small_fall", "minecraft:entity.generic.big_fall")
    };
    play_sound(e, m, level, if dmg > 4 { big } else { small }, 1.0, 1.0);
    hurt(e, m, level, DamageSource::of(DamageKind::Fall), dmg as f32);
}

/// `LivingEntity.jumpFromGround`.
fn jump_from_ground(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let power = (m.attrs.value(Attr::JumpStrength) as f32) * e.block_jump_factor(level) + effects::jump_boost_power(m);
    if power <= 1.0e-5 {
        return;
    }
    e.delta = Vec3::new(e.delta.x, (power as f64).max(e.delta.y), e.delta.z);
    e.needs_sync = true;
}

/// `LivingEntity.travel`.
pub fn travel(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) {
    if e.is_in_water() || e.is_in_lava() {
        travel_in_fluid(e, m, level, input);
    } else {
        travel_in_air(e, m, level, input);
    }
}

fn gravity(e: &Entity, m: &MobData) -> f64 {
    if e.no_gravity { 0.0 } else { m.attrs.value(Attr::Gravity) }
}

/// `getEffectiveGravity`: slow falling caps it at 0.01 while falling.
pub fn effective_gravity(e: &Entity, m: &MobData) -> f64 {
    let g = gravity(e, m);
    if e.delta.y <= 0.0 && effects::has(m, crate::effect::ids::slow_falling()) { g.min(0.01) } else { g }
}

/// `computeModifiedFriction`.
fn modified_friction(f: f32, modifier: f32) -> f32 {
    mth::clamp(1.0 - (1.0 - f) * modifier, 0.0, 1.0)
}

pub fn travel_in_air(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) {
    let below = e.block_pos_below_that_affects_movement(level);
    let friction = if e.on_ground {
        modified_friction(crate::physics::block_factors(level.block(below)).friction, m.attrs.value(Attr::FrictionModifier) as f32)
    } else {
        1.0
    };
    let v = relative_friction_movement(e, m, level, input, friction);
    let mut y = v.y;
    if let Some(a) = effects::amplifier(m, crate::effect::ids::levitation()) {
        y += (0.05 * (a + 1) as f64 - v.y) * 0.2;
    } else if level.is_loaded(below) {
        y -= effective_gravity(e, m);
    } else if e.y() > level.min_y() as f64 {
        y = -0.1;
    } else {
        y = 0.0;
    }
    // `shouldDiscardFriction`: no drag.
    if m.discard_friction || m.kind.ext().is_some_and(|k| k.discard_friction(m)) {
        e.delta = Vec3::new(v.x, y, v.z);
        return;
    }
    let drag = m.attrs.value(Attr::AirDragModifier) as f32;
    let h = friction * modified_friction(0.91, drag);
    let vy = modified_friction(0.98, drag);
    e.delta = Vec3::new(v.x * h as f64, y * vy as f64, v.z * h as f64);
}

/// `handleRelativeFrictionAndCalculateMovement`.
fn relative_friction_movement(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3, friction: f32) -> Vec3 {
    let speed = if e.on_ground {
        if friction as f64 <= 0.6 { m.speed } else { m.speed * (0.21600002 / (friction * friction * friction)) }
    } else {
        0.02
    };
    move_relative(e, speed, input);
    e.delta = handle_on_climbable(e, m, level, e.delta);
    let d = e.delta;
    e.do_move(level, MoverType::SelfMove, d);
    let mut v = e.delta;
    if (e.horizontal_collision || m.jumping) && on_climbable(e, m, level) {
        v = Vec3::new(v.x, 0.2, v.z);
    }
    v
}

/// `Entity.moveRelative`.
pub fn move_relative(e: &mut Entity, speed: f32, input: Vec3) {
    let l = input.length_sqr();
    if l < 1.0e-7 {
        return;
    }
    let v = if l > 1.0 { input.normalize() } else { input }.scale(speed as f64);
    let s = mth::sin((e.y_rot * 0.017453292) as f64);
    let c = mth::cos((e.y_rot * 0.017453292) as f64);
    e.delta = e.delta + Vec3::new(v.x * c as f64 - v.z * s as f64, v.y, v.z * c as f64 + v.x * s as f64);
}

fn on_climbable(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    if let Species::Spider { climbing } = m.species {
        return climbing;
    }
    crate::blocks::has_tag(level.block(e.block_position()), crate::blocks::Tag::Climbable)
}

fn handle_on_climbable(e: &mut Entity, m: &MobData, level: &dyn EntityLevel, v: Vec3) -> Vec3 {
    if !on_climbable(e, m, level) {
        return v;
    }
    e.fall_distance = 0.0;
    let x = mth::clamp_d(v.x, -0.15000000596046448, 0.15000000596046448);
    let z = mth::clamp_d(v.z, -0.15000000596046448, 0.15000000596046448);
    let y = v.y.max(-0.15000000596046448);
    Vec3::new(x, y, z)
}

fn travel_in_fluid(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) {
    let falling = e.delta.y <= 0.0;
    let y0 = e.y();
    let g = effective_gravity(e, m);
    if e.is_in_water() {
        if m.kind.ext().is_some_and(|k| k.travel_in_water(e, m, level, input)) {
            return;
        }
        let mut slow = m.kind.ext().map_or(0.8f32, |k| k.water_slow_down(m));
        let mut speed = 0.02f32;
        let mut eff = m.attrs.value(Attr::WaterMovementEfficiency) as f32;
        if !e.on_ground {
            eff *= 0.5;
        }
        if eff > 0.0 {
            slow += (0.54600006 - slow) * eff;
            speed += (m.speed - speed) * eff;
        }
        if effects::has(m, crate::effect::ids::dolphins_grace()) {
            slow = 0.96;
        }
        move_relative(e, speed, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        let mut v = e.delta;
        if e.horizontal_collision && on_climbable(e, m, level) {
            v = Vec3::new(v.x, 0.2, v.z);
        }
        v = v.multiply(slow as f64, 0.800000011920929, slow as f64);
        e.delta = fluid_falling_adjusted(g, falling, v);
    } else {
        move_relative(e, 0.02, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        let threshold = if (e.eye_height as f64) < 0.4 { 0.0 } else { 0.4 };
        if e.fluid_height_lava() <= threshold {
            e.delta = e.delta.multiply(0.5, 0.800000011920929, 0.5);
            e.delta = fluid_falling_adjusted(g, falling, e.delta);
        } else {
            e.delta = e.delta.scale(0.5);
        }
        if g != 0.0 {
            e.delta = e.delta.add(0.0, -g / 4.0, 0.0);
        }
    }
    // `jumpOutOfFluid`.
    let v = e.delta;
    if e.horizontal_collision {
        let b = e.bounding_box().offset(v.x, v.y + 0.6000000238418579 - e.y() + y0, v.z);
        let ctx = e.collision_context();
        if crate::collision::no_collision(level, &ctx, e.id, &b) && !contains_any_liquid(level, &b) {
            e.delta = Vec3::new(v.x, 0.30000001192092896, v.z);
        }
    }
}

fn contains_any_liquid(level: &dyn EntityLevel, b: &Aabb) -> bool {
    let (x0, y0, z0) = (crate::math::floor(b.min_x), crate::math::floor(b.min_y), crate::math::floor(b.min_z));
    let (x1, y1, z1) = (crate::math::ceil(b.max_x), crate::math::ceil(b.max_y), crate::math::ceil(b.max_z));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                if !crate::physics::fluid_state(level.block(BlockPos::new(x, y, z))).is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

/// `getFluidFallingAdjustedMovement`.
fn fluid_falling_adjusted(g: f64, falling: bool, v: Vec3) -> Vec3 {
    if g == 0.0 {
        return v;
    }
    let y = if falling && (v.y - 0.005).abs() >= 0.003 && (v.y - g / 16.0).abs() < 0.003 { -0.003 } else { v.y - g / 16.0 };
    Vec3::new(v.x, y, v.z)
}

/// `LivingEntity.pushEntities`: pushable living entities touching this one push each other
/// apart (`Entity.push`). Players push the mob; their own half is their client's.
fn push_entities(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    // `pushEntities` overridden with nothing (bats).
    if m.kind.ext().is_some_and(|k| !k.pushable()) {
        return;
    }
    let bb = e.bounding_box();
    let mut others: Vec<(i32, f64, f64, bool)> = Vec::new();
    // Players first: they joined the entity sections before the mobs around them (the order
    // the pushes add up in shows in the last bits of the motion). Riding together: no pushes
    // between a vehicle and its passengers or passengers of the same vehicle.
    let near = level.entities_in(&bb, EntityFilter::Living, e.id);
    let riding = |id: i32, vehicle: Option<i32>| e.vehicle == Some(id) || e.passengers.contains(&id) || (e.vehicle.is_some() && vehicle == e.vehicle);
    for &id in &near {
        if let Some(p) = level.player(id)
            && !p.spectator
            && p.alive
            && !riding(id, p.vehicle)
        {
            others.push((id, p.pos.x, p.pos.z, true));
        }
    }
    // Players without a stand-in among the entities.
    for p in level.players() {
        // The box test first: the lookups below scan the level's players.
        let h = if p.sneaking { 1.5 } else { 1.8 };
        let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
        if !pb.intersects(&bb) {
            continue;
        }
        if p.spectator || !p.alive || others.iter().any(|o| o.0 == p.id) || level.entity(p.id).is_some() || riding(p.id, p.vehicle) {
            continue;
        }
        others.push((p.id, p.pos.x, p.pos.z, true));
    }
    for id in near {
        if level.player(id).is_some() {
            continue;
        }
        let Some(o) = level.entity(id) else { continue };
        if let EntityKind::Mob(om) = &o.kind
            && om.health > 0.0
            && om.kind.ext().is_none_or(|k| k.pushable())
            && !riding(id, o.vehicle)
        {
            others.push((id, o.x(), o.z(), false));
        }
    }
    for (id, ox, oz, player) in others {
        if let Some(k) = m.kind.ext() {
            k.do_push(e, m, &*level, id);
        }
        // `Entity.push(Entity)`: nothing moves when either side has no physics (a vex).
        if e.no_physics || (!player && level.entity(id).is_some_and(|o| o.no_physics)) {
            continue;
        }
        let (dx, dz) = (ox - e.x(), oz - e.z());
        let mut d = dx.abs().max(dz.abs());
        if d < 0.009999999776482582 {
            continue;
        }
        d = d.sqrt();
        let (mut dx, mut dz) = (dx / d, dz / d);
        let f = (1.0 / d).min(1.0);
        dx *= f;
        dz *= f;
        dx *= 0.05000000074505806;
        dz *= 0.05000000074505806;
        // `Entity.push`: vehicles and dead (not `isPushable`) mobs are not pushed.
        if e.passengers.is_empty() && m.health > 0.0 {
            e.delta = e.delta.add(-dx, 0.0, -dz);
            e.needs_sync = true;
        }
        if !player
            && let Some(o) = level.entity_mut(id)
            && o.passengers.is_empty()
        {
            o.delta = o.delta.add(dx, 0.0, dz);
            o.needs_sync = true;
        }
    }
}

// ---------------------------------------------------------------------- damage

/// `Entity.playSound` with the mob's sound source.
fn play_sound(e: &Entity, m: &MobData, level: &mut dyn EntityLevel, sound: &'static str, volume: f32, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: m.kind.sound_source(), volume, pitch });
    }
}

/// `LivingEntity.makeSound`: volume 1, the voice pitch.
pub fn make_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel, sound: &'static str) {
    let mut pitch = if m.baby() {
        (e.random.next_float() - e.random.next_float()) * 0.2 + 1.5
    } else {
        (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0
    };
    let mut volume = 1.0;
    if let Some(k) = m.kind.ext() {
        volume = k.sound_volume(m);
        pitch = k.voice_pitch(m, pitch);
    }
    play_sound(e, m, level, sound, volume, pitch);
}

/// Damage to a mob from outside its own tick (explosions, arrows, players).
/// The mob overrides of `Entity.thunderHit` (the bolt is entity `_bolt`): creepers get charged
/// after the usual fire and damage; pigs turn into zombified piglins with a golden sword and
/// villagers into witches, both persistent, unless the difficulty is peaceful. Returns false
/// when the plain `Entity.thunderHit` applies.
pub fn thunder_hit(e: &mut Entity, level: &mut dyn EntityLevel, _bolt: i32) -> bool {
    let Some(kind) = data(e).map(|m| m.kind) else { return false };
    if let Some(k) = kind.ext() {
        let mut m = take(e);
        let handled = k.thunder_hit(e, &mut m, level, _bolt);
        put(e, m);
        if handled {
            return true;
        }
    }
    match kind {
        MobKind::Creeper => {
            crate::ext_entity::lightning::base_thunder_hit(e, level);
            if let Some(m) = data_mut(e)
                && let Species::Creeper { powered, .. } = &mut m.species
            {
                *powered = true;
            }
            true
        }
        // `Turtle.thunderHit`: struck dead.
        MobKind::Turtle => {
            hurt_entity(e, level, DamageSource::of(DamageKind::LightningBolt), f32::MAX);
            true
        }
        MobKind::Pig | MobKind::Villager if level.difficulty() != 0 => {
            let mut m = take(e);
            let to = if kind == MobKind::Pig { MobKind::ZombifiedPiglin } else { MobKind::Witch };
            let converted = convert::convert_to(e, &mut m, level, to, false, kind == MobKind::Pig, |_, nm, _| {
                if to == MobKind::ZombifiedPiglin {
                    nm.equipment[MAINHAND] = ItemStack::of("minecraft:golden_sword", 1).unwrap_or_else(ItemStack::empty);
                }
                nm.persistence_required = true;
            });
            put(e, m);
            if converted.is_none() {
                crate::ext_entity::lightning::base_thunder_hit(e, level);
            }
            true
        }
        _ => false,
    }
}

pub fn hurt_entity(e: &mut Entity, level: &mut dyn EntityLevel, source: DamageSource, amount: f32) -> bool {
    let mut m = take(e);
    let r = hurt(e, &mut m, level, source, amount);
    put(e, m);
    r
}

/// `isPushedByFluid` of mob type `type_name`.
pub fn pushed_by_fluid(type_name: &str) -> bool {
    MobKind::by_name(type_name).and_then(MobKind::ext).is_none_or(|k| k.pushed_by_fluid())
}

/// The swim sound of mob type `type_name` (`None`: it makes no movement sounds).
pub fn swim_sound(type_name: &str) -> Option<&'static str> {
    match MobKind::by_name(type_name).and_then(MobKind::ext) {
        Some(k) => k.swim_sound(),
        None => Some("minecraft:entity.generic.swim"),
    }
}

/// Whether mob type `type_name` runs `checkFallDamage` (flying types override it with nothing).
pub fn checks_fall_damage(type_name: &str) -> bool {
    MobKind::by_name(type_name).and_then(MobKind::ext).is_none_or(|k| k.checks_fall_damage())
}

/// `Entity.playerTouch`: player `player` touches mob `e` (slimes and magma cubes hurt it).
pub fn player_touch(e: &mut Entity, level: &mut dyn EntityLevel, player: i32) {
    let Some(k) = data(e).and_then(|m| m.kind.ext()) else { return };
    let Some(p) = goals::living(level, player) else { return };
    let mut m = take(e);
    k.player_touch(e, &mut m, level, &p);
    put(e, m);
}

/// `LivingEntity.hurtServer` for a mob, with the type's overrides around it.
pub fn hurt(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: DamageSource, amount: f32) -> bool {
    let Some(k) = m.kind.ext() else { return hurt_base(e, m, level, source, amount) };
    if k.is_invulnerable_to(m, source.kind) && !source.kind.is_tag("minecraft:bypasses_invulnerability") {
        return false;
    }
    if let Some(r) = k.hurt(e, m, level, &source, amount) {
        return r;
    }
    let r = hurt_base(e, m, level, source, amount);
    k.after_hurt(e, m, level, &source, amount, r);
    r
}

/// The shared `LivingEntity.hurtServer`.
pub fn hurt_base(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: DamageSource, amount: f32) -> bool {
    let kind = source.kind;
    if e.is_removed() || (e.invulnerable && !kind.is_tag("minecraft:bypasses_invulnerability")) || m.is_dead_or_dying() {
        return false;
    }
    if kind.is_tag("minecraft:is_fire") && effects::has(m, crate::effect::ids::fire_resistance()) {
        return false;
    }
    m.no_action_time = 0;
    let mut amount = amount.max(0.0);
    if kind.is_tag("minecraft:damages_helmet") && !m.equipment[HEAD].is_empty() {
        amount *= 0.75;
    }
    if !amount.is_finite() {
        amount = f32::MAX;
    }
    let full = if m.damage_cooldown as f32 > 10.0 && !kind.is_tag("minecraft:bypasses_cooldown") {
        if amount <= m.last_hurt {
            return false;
        }
        let dealt = amount - m.last_hurt;
        actually_hurt(e.id, m, source, dealt);
        if let Some(k) = m.kind.ext() {
            k.actually_hurt(e, m, level, &source, dealt);
        }
        m.last_hurt = amount;
        false
    } else {
        m.last_hurt = amount;
        m.damage_cooldown = 20;
        actually_hurt(e.id, m, source, amount);
        if let Some(k) = m.kind.ext() {
            k.actually_hurt(e, m, level, &source, amount);
        }
        m.hurt_duration = 10;
        m.hurt_time = 10;
        true
    };
    if let Some(a) = source.attacker
        && !kind.is_tag("minecraft:no_anger")
        && goals::living(level, a).is_some()
    {
        m.last_hurt_by_mob = Some(a);
        m.last_hurt_by_mob_timestamp = e.tick_count;
    }
    if source.attacker_is_player {
        m.last_hurt_by_player = source.attacker;
        m.last_hurt_by_player_memory = 100;
    }
    if full {
        m.hurt_by = Some((kind, source.attacker, source.direct.or(source.attacker)));
        level.emit(Event::MobHurt { entity: e.id, kind, attacker: source.attacker, direct: source.direct.or(source.attacker) });
        if !kind.is_tag("minecraft:no_impact") {
            e.needs_sync = true;
        }
        if !kind.is_tag("minecraft:no_knockback") {
            let (mut dx, mut dz) = (0.0, 0.0);
            if let Some(p) = source.pos {
                dx = p.x - e.x();
                dz = p.z - e.z();
            }
            knockback(e, m, 0.4000000059604645, dx, dz);
        }
    }
    if m.is_dead_or_dying() {
        if full {
            let sound = m.kind.ext().and_then(|k| k.death_sound_for(m)).unwrap_or_else(|| m.kind.death_sound());
            make_sound(e, m, level, sound);
        }
        die(e, m, level, source);
    } else if full {
        m.ambient_sound_time = -m.kind.ambient_sound_interval();
        let sound = m.kind.ext().and_then(|k| k.hurt_sound_for(m)).unwrap_or_else(|| m.kind.hurt_sound());
        make_sound(e, m, level, sound);
    }
    m.last_damage_source = Some(source);
    m.last_damage_stamp = level.game_time();
    effects::on_hurt(e, m, level, &source, amount);
    if m.kind == MobKind::Zombie {
        kinds::zombie::reinforcements(e, m, level, &source);
    }
    true
}

/// `LivingEntity.actuallyHurt`: armor, absorption, health.
fn actually_hurt(id: i32, m: &mut MobData, source: DamageSource, amount: f32) {
    let mut amount = amount;
    if !source.kind.is_tag("minecraft:bypasses_armor") {
        let armor = crate::math::floor(m.attrs.value(Attr::Armor)) as f32;
        let toughness = m.attrs.value(Attr::ArmorToughness) as f32;
        let f = 2.0 + toughness / 4.0;
        let g = mth::clamp(armor - amount / f, armor * 0.2, 20.0);
        amount *= 1.0 - g / 25.0;
    }
    // `getDamageAfterMagicAbsorb`: resistance, then the type's additions.
    amount = effects::resist(m, &source, amount);
    if let Some(k) = m.kind.ext() {
        amount = k.damage_after_magic_absorb(id, m, &source, amount);
    }
    let before = amount;
    amount = (amount - m.absorption).max(0.0);
    m.absorption = (m.absorption - (before - amount)).max(0.0);
    if amount == 0.0 {
        return;
    }
    let h = m.health - amount;
    m.set_health(h);
}

/// `LivingEntity.knockback` on a mob entity (from outside its tick).
pub fn knockback_entity(e: &mut Entity, strength: f64, dx: f64, dz: f64) {
    if !matches!(e.kind, EntityKind::Mob(_)) {
        return;
    }
    let m = take(e);
    knockback(e, &m, strength, dx, dz);
    put(e, m);
}

/// `LivingEntity.knockback`.
pub fn knockback(e: &mut Entity, m: &MobData, strength: f64, mut dx: f64, mut dz: f64) {
    if m.kind.ext().is_some_and(|k| k.knockback_immune(m)) {
        return;
    }
    let strength = strength * (1.0 - m.attrs.value(Attr::KnockbackResistance));
    if strength <= 0.0 {
        return;
    }
    e.needs_sync = true;
    let v = e.delta;
    while dx * dx + dz * dz < 9.999999747378752e-6 {
        dx = (e.random.next_double() - e.random.next_double()) * 0.01;
        dz = (e.random.next_double() - e.random.next_double()) * 0.01;
    }
    let k = Vec3::new(dx, 0.0, dz).normalize().scale(strength);
    let y = if e.on_ground { 0.4f64.min(v.y / 2.0 + strength) } else { v.y };
    e.delta = Vec3::new(v.x / 2.0 - k.x, y, v.z / 2.0 - k.z);
}

/// `LivingEntity.die`: loot, experience, the death event.
fn die(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: DamageSource) {
    if e.is_removed() || m.dead {
        return;
    }
    if !m.kind.ext().is_some_and(|k| k.handle_killing_blow(e, m, level)) {
        m.dead = true;
    }
    let killed_by_player = m.last_hurt_by_player_memory > 0;
    level.emit(Event::Killed {
        entity: e.id,
        entity_type: e.type_name,
        credit: m.last_hurt_by_player.filter(|_| killed_by_player),
        kind: source.kind,
        attacker: source.attacker,
        direct: source.direct.or(source.attacker),
        equipment: m.equipment.iter().zip(SLOT_NAMES).filter(|(s, _)| !s.is_empty()).map(|(s, n)| (n, s.clone())).collect(),
    });
    // `gameEvent(ENTITY_DIE)`: sculk sensors hear it; the nearest sculk catalyst takes the
    // experience as charge (`CatalystListener`: `getExperienceReward` if the mob would drop
    // any, then `skipDropExperience`).
    level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: Some(e.id) });
    let consumed = level.sculk_catalyst_near(e.position());
    if consumed {
        let xp = experience_reward(e, m);
        let charge = if m.baby() { 0 } else { xp };
        level.feed_sculk_catalyst(e.position(), charge);
        // `tryAwardItSpreadsAdvancement`: the player that last hurt it.
        if charge > 0
            && let Some(player) = m.last_hurt_by_player
        {
            let direct = source.direct.is_none() || source.direct == source.attacker;
            let criterion = crate::level::Criterion::KillMobNearSculkCatalyst { victim: crate::level::Seen::of_mob(e, m), kind: source.kind, direct };
            level.emit(Event::Criterion { player, criterion });
        }
    }
    if !m.baby() && level.mob_drops() {
        level.emit(Event::DeathLoot {
            entity: e.id,
            table: m.kind.ext().and_then(|k| k.loot_table(m)).unwrap_or_else(|| m.kind.loot_table()),
            pos: e.position(),
            killer: m.last_hurt_by_player.filter(|_| killed_by_player),
            attacker: source.attacker,
            direct: source.direct.or(source.attacker),
            kind: source.kind,
            on_fire: e.is_on_fire(),
        });
    }
    // `Mob.dropCustomDeathLoot`: equipment with its drop chance.
    for i in 0..6 {
        let chance = m.drop_chances[i];
        if chance == 0.0 || m.equipment[i].is_empty() {
            continue;
        }
        let preserved = chance > 1.0;
        if (killed_by_player || preserved) && e.random.next_float() < chance {
            let mut stack = std::mem::replace(&mut m.equipment[i], ItemStack::empty());
            if !preserved && stack.is_damageable_item() {
                let max = stack.max_damage();
                let inner = e.random.next_int_bounded((max - 3).max(1));
                let d = max - e.random.next_int_bounded(1 + inner);
                stack.insert(kiln_item::keys::DAMAGE, d);
            }
            spawn_at_location(e, level, stack);
        }
    }
    // `dropExperience`.
    if killed_by_player && level.mob_drops() && !consumed {
        let xp = experience_reward(e, m);
        award_experience(level, e.position(), xp);
    }
    level.emit(Event::EntityEvent { entity: e.id, event: 3 });
    if let Some(k) = m.kind.ext() {
        k.die(e, m, level, &source);
    }
}

/// `getBaseExperienceReward`.
fn experience_reward(e: &mut Entity, m: &MobData) -> i32 {
    if let Some(xp) = m.kind.ext().and_then(|k| k.experience(e, m)) {
        return xp;
    }
    if m.kind.is_animal() {
        return 1 + e.random.next_int_bounded(3);
    }
    let mut xp = 5;
    for i in 0..6 {
        if !m.equipment[i].is_empty() && m.drop_chances[i] <= 1.0 {
            xp += 1 + e.random.next_int_bounded(3);
        }
    }
    xp
}

/// `ExperienceOrb.award`.
pub fn award_experience(level: &mut dyn EntityLevel, pos: Vec3, mut amount: i32) {
    const VALUES: [i32; 11] = [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3, 1];
    while amount > 0 {
        let v = *VALUES.iter().find(|&&t| amount >= t).unwrap_or(&1);
        amount -= v;
        let b = Aabb::new(pos.x - 0.5, pos.y - 0.5, pos.z - 0.5, pos.x + 0.5, pos.y + 0.5, pos.z + 0.5);
        let r = level.random().next_int_bounded(40);
        let merged = level.entities_in(&b, EntityFilter::ExperienceOrb, i32::MIN).into_iter().find(|&id| {
            level.entity(id).is_some_and(|o| {
                matches!(&o.kind, EntityKind::ExperienceOrb(d) if d.value == v) && (o.id - r).rem_euclid(40) == 0 && !o.is_removed()
            })
        });
        if let Some(id) = merged {
            if let Some(o) = level.entity_mut(id)
                && let EntityKind::ExperienceOrb(d) = &mut o.kind
            {
                d.count += 1;
                d.age = 0;
            }
            continue;
        }
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut o = Entity::new("minecraft:experience_orb", id, 0, EntityKind::ExperienceOrb(crate::xp_orb::OrbData::new(v)), seed);
        o.set_pos(pos);
        o.y_rot = o.random.next_float() * 360.0;
        let dx = (o.random.next_double() * 0.2 - 0.1) * 2.0;
        let dy = o.random.next_double() * 0.2 * 2.0;
        let dz = (o.random.next_double() * 0.2 - 0.1) * 2.0;
        o.delta = Vec3::new(dx, dy, dz);
        o.set_old_pos_and_rot();
        level.add_entity(o);
    }
}

/// `Entity.spawnAtLocation(stack)`: an item at the entity's position, offset 0 up, with the
/// default random throw.
pub fn spawn_at_location(e: &Entity, level: &mut dyn EntityLevel, stack: ItemStack) {
    spawn_at_location_offset(e, level, stack, 0.0);
}

pub fn spawn_at_location_offset(e: &Entity, level: &mut dyn EntityLevel, stack: ItemStack, y_off: f32) {
    if stack.is_empty() {
        return;
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let pos = Vec3::new(e.x(), e.y() + y_off as f64, e.z());
    let mut item = crate::item::new(id, 0, stack, seed);
    item.set_pos(pos);
    // `new ItemEntity(level, x, y, z, stack)`: a random throw from the item's own random.
    let dx = item.random.next_double() * 0.2 - 0.1;
    let dz = item.random.next_double() * 0.2 - 0.1;
    item.delta = Vec3::new(dx, 0.2, dz);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
    }
    item.set_old_pos_and_rot();
    level.add_entity(item);
}

// ---------------------------------------------------------------------- combat helpers

/// `LivingEntity.hasLineOfSight` through the mob's per-tick `Sensing` cache.
pub fn has_line_of_sight_cached(e: &Entity, m: &mut MobData, level: &dyn EntityLevel, t: &Living) -> bool {
    if m.seen.contains(&t.id) {
        return true;
    }
    if m.unseen.contains(&t.id) {
        return false;
    }
    if m.kind.ext().is_some_and(|k| !k.can_see(m)) {
        m.unseen.push(t.id);
        return false;
    }
    let from = Vec3::new(e.x(), e.eye_y(), e.z());
    let to = Vec3::new(t.pos.x, t.eye_y, t.pos.z);
    let v = to.distance_to_sqr(from).sqrt() <= 128.0 && !clip_blocks(level, from, to);
    if v { m.seen.push(t.id) } else { m.unseen.push(t.id) }
    v
}

/// `Level.clip` with `COLLIDER` shapes and no fluids: whether a block is in the way.
pub fn clip_blocks(level: &dyn EntityLevel, from: Vec3, to: Vec3) -> bool {
    crate::clip::traverse_blocks(from, to, |p| {
        let s = level.block(p);
        let (shape, _) = crate::collision::collision_shape(s, p, &crate::collision::CollisionContext::EMPTY);
        crate::clip::shape_clips(&shape, from, to, p).then_some(())
    })
    .is_some()
}

/// `Mob.isWithinMeleeAttackRange` (`DEFAULT_ATTACK_REACH`: sqrt(2.04) - 0.6).
pub fn within_melee_range(e: &Entity, t: &Living) -> bool {
    let reach = 2.04f64.sqrt() - 0.6000000238418579;
    let mut b = e.bounding_box().inflate(reach, 0.0, reach);
    // `Ravager.getAttackBoundingBox`: a little narrower.
    if e.type_name == "minecraft:ravager" {
        b = b.deflate(0.05, 0.0, 0.05);
    }
    b.intersects(&t.bb)
}

/// `Mob.lookAt(entity, maxY, maxX)`: turns the body directly.
pub fn mob_look_at(e: &mut Entity, t: &Living, max_y: f32, max_x: f32) {
    let dx = t.pos.x - e.x();
    let dz = t.pos.z - e.z();
    let dy = t.eye_y - e.eye_y();
    let d = (dx * dx + dz * dz).sqrt();
    let yaw = (mth::atan2(dz, dx) * 57.2957763671875) as f32 - 90.0;
    let pitch = (-(mth::atan2(dy, d) * 57.2957763671875)) as f32;
    let rot = |from: f32, to: f32, max: f32| {
        let w = mth::wrap_degrees(to - from).clamp(-max, max);
        from + w
    };
    e.x_rot = rot(e.x_rot, pitch, max_x);
    e.y_rot = rot(e.y_rot, yaw, max_y);
}

/// `Mob.doHurtTarget`: hits the target with the attack damage attribute.
pub fn do_hurt_target(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> bool {
    let k = m.kind.ext();
    if let Some(r) = k.and_then(|k| k.do_hurt_target(e, m, level, t)) {
        return r;
    }
    let r = do_hurt_target_base(e, m, level, t);
    if r && let Some(k) = k {
        k.after_hurt_target(e, m, level, t);
    }
    r
}

/// `target.hurtServer(source, damage)` for a player or a mob of the level.
pub fn hurt_living(level: &mut dyn EntityLevel, t: &Living, source: DamageSource, damage: f32) -> bool {
    if t.player {
        return level.hurt_player(t.id, source, damage);
    }
    match level.entity_mut(t.id) {
        Some(o) => {
            let mut o2 = std::mem::replace(o, Entity::new("minecraft:marker", 0, 0, EntityKind::Other { type_name: "minecraft:marker" }, 0));
            let r = hurt_entity(&mut o2, level, source, damage);
            if let Some(slot) = level.entity_mut(t.id) {
                *slot = o2;
            }
            r
        }
        None => false,
    }
}

/// The shared `Mob.doHurtTarget`.
pub fn do_hurt_target_base(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> bool {
    let damage = m.attrs.value(Attr::AttackDamage) as f32;
    let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
    let hurt = hurt_living(level, t, source, damage);
    if hurt {
        m.last_hurt_mob = Some(t.id);
        // `Zombie.doHurtTarget`: a burning, empty-handed zombie sets its target on fire.
        if m.kind.is_zombie() && m.equipment[MAINHAND].is_empty() && e.is_on_fire() {
            let difficulty = level.effective_difficulty(e.block_position());
            if e.random.next_float() < difficulty * 0.3 {
                level.ignite(t.id, (2 * difficulty as i32) as f32);
            }
        }
    }
    hurt
}

// ---------------------------------------------------------------------- spawning

/// What `finalizeSpawn` needs to know about where the mob appears.
#[derive(Clone, Copy, Debug)]
pub struct SpawnContext {
    /// The `minecraft:worldgen/biome` id where the mob appears (variants follow it).
    pub biome: Option<i32>,
    /// `DimensionType.moonBrightness` (1 at full moon).
    pub moon_brightness: f32,
    /// `DifficultyInstance.getSpecialMultiplier`.
    pub special_multiplier: f32,
    /// `getEffectiveDifficulty`.
    pub effective_difficulty: f32,
    pub hard: bool,
    pub halloween: bool,
}

/// Group data shared by the mobs of one spawn group.
#[derive(Clone, Debug, Default)]
pub struct GroupData {
    pub ageable_group_size: i32,
    pub zombie_baby: Option<bool>,
    pub sheep_colors: bool,
    pub spider_effect: Option<Option<&'static str>>,
    /// `EntitySpawnReason.NATURAL` (set by [`finalize_spawn`]).
    pub natural: bool,
    /// The variant the first mob of a group picked (`WolfPackData`, horses' `HorseGroupData`).
    pub variant: Option<i32>,
    /// `EntitySpawnReason.PATROL`, `EVENT` (raids) and `STRUCTURE`.
    pub patrol: bool,
    pub event: bool,
    pub structure: bool,
}

/// `Mob.finalizeSpawn` and the types' overrides; random draws from `r` (the level's).
pub fn finalize_spawn(e: &mut Entity, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData, natural: bool) {
    let mut m = take(e);
    let kind = m.kind;
    group.natural = natural;
    if kind == MobKind::Zombie {
        kinds::zombie::finalize(e, &mut m, r, ctx, group, false);
        put(e, m);
        return;
    }
    if kind == MobKind::Skeleton {
        kinds::skeleton::finalize(e, &mut m, r, ctx);
        put(e, m);
        return;
    }
    if let Some(k) = kind.ext() {
        k.finalize_spawn(e, &mut m, r, ctx, group);
        put(e, m);
        return;
    }
    // Types that pick a variant first (from the biome, no randomness) draw their sound
    // variant before `Mob.finalizeSpawn`.
    if matches!(kind, MobKind::Pig | MobKind::Cow | MobKind::Chicken) {
        // `VariantUtils.selectVariantToSpawn`: the climate's variant wins over the temperate
        // fallback; one draw among the (single) best candidates.
        let name = match species::climate(ctx.biome) {
            species::Climate::Temperate => "minecraft:temperate",
            species::Climate::Warm => "minecraft:warm",
            species::Climate::Cold => "minecraft:cold",
        };
        let _ = r.next_int_bounded(1);
        if let Some(v) = kiln_data::synced_id(&format!("{}_variant", kind.type_name()), name) {
            m.variant = v;
        }
        m.sound_variant = r.next_int_bounded(sound_variant_count(kind).max(1));
    }
    if breed::is_ageable(kind) {
        // `AgeableMob.finalizeSpawn`: after the first mob of a group, 5% are babies.
        if group.ageable_group_size > 0 && r.next_float() <= 0.05 {
            set_age(e, &mut m, breed::BABY_START_AGE);
        }
        group.ageable_group_size += 1;
    }
    if kind == MobKind::Sheep {
        // `Sheep.finalizeSpawn`: the wool color from the biome's color spawn rules.
        let color = species::sheep_color(r, species::climate(ctx.biome));
        if let Species::Sheep { color: c, .. } = &mut m.species {
            *c = color;
        }
    }
    // `Mob.finalizeSpawn`.
    let bonus = mth::triangle(r, 0.0, 0.11485000000000001);
    m.attrs.set_modifier(Attr::FollowRange, "minecraft:random_spawn_bonus", bonus, Op::AddMultipliedBase);
    m.left_handed = r.next_float() < 0.05;
    match kind {
        MobKind::Spider => {
            // Spider jockeys (1 in 100) are not simulated; the draw still happens.
            let _ = r.next_int_bounded(100);
            if group.spider_effect.is_none() {
                let mut effect = None;
                if ctx.hard && r.next_float() < 0.1 * ctx.special_multiplier {
                    effect = Some(species::spider_effect(r));
                }
                group.spider_effect = Some(effect);
            }
        }
        _ => {}
    }
    let _ = natural;
    put(e, m);
}

fn sound_variant_count(kind: MobKind) -> i32 {
    let reg = match kind {
        MobKind::Pig => "minecraft:pig_sound_variant",
        MobKind::Cow => "minecraft:cow_sound_variant",
        _ => "minecraft:chicken_sound_variant",
    };
    kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == reg).map_or(1, |(_, e)| e.len() as i32)
}
/// `Mob.checkDespawn`: `nearest` is the squared distance to the nearest player (`None`: no
/// player in the dimension).
pub fn check_despawn(e: &mut Entity, level: &dyn EntityLevel, nearest: Option<f64>) {
    let Some(m) = data(e) else { return };
    if m.kind.ext().is_some_and(|k| !k.despawns()) {
        return;
    }
    if let Some(k) = m.kind.ext()
        && k.check_despawn(e, level)
    {
        return;
    }
    let Some(m) = data(e) else { return };
    if level.difficulty() == 0 && !m.kind.is_animal() {
        e.discard();
        return;
    }
    let persistent = m.persistence_required;
    let category = m.kind.category();
    let Some(d) = nearest else { return };
    let removable = m.kind.ext().and_then(|k| k.remove_when_far_away_at(m, d)).unwrap_or(!category.persistent());
    let far = category.despawn_distance() as f64;
    if !persistent && removable && d > far * far {
        e.discard();
        return;
    }
    let near = category.no_despawn_distance() as f64;
    let no_action = m.no_action_time;
    if !persistent && removable && no_action > 600 && e.random.next_int_bounded(800) == 0 && d > near * near {
        e.discard();
    } else if d < near * near
        && let Some(m) = data_mut(e)
    {
        m.no_action_time = 0;
    }
}

impl DamageKind {
    /// The `minecraft:damage_type` entry.
    pub fn type_name(self) -> &'static str {
        match self {
            DamageKind::OnFire => "minecraft:on_fire",
            DamageKind::InFire => "minecraft:in_fire",
            DamageKind::Lava => "minecraft:lava",
            DamageKind::FallingBlock => "minecraft:falling_block",
            DamageKind::FallingAnvil => "minecraft:falling_anvil",
            DamageKind::FallingStalactite => "minecraft:falling_stalactite",
            DamageKind::Explosion => "minecraft:explosion",
            DamageKind::Cactus => "minecraft:cactus",
            DamageKind::SweetBerryBush => "minecraft:sweet_berry_bush",
            DamageKind::HotFloor => "minecraft:hot_floor",
            DamageKind::Freeze => "minecraft:freeze",
            DamageKind::Arrow => "minecraft:arrow",
            DamageKind::Thrown => "minecraft:thrown",
            DamageKind::Generic => "minecraft:generic",
            DamageKind::MobAttack => "minecraft:mob_attack",
            DamageKind::PlayerAttack => "minecraft:player_attack",
            DamageKind::Drown => "minecraft:drown",
            DamageKind::InWall => "minecraft:in_wall",
            DamageKind::OutOfWorld => "minecraft:out_of_world",
            DamageKind::Fall => "minecraft:fall",
            DamageKind::Kill => "minecraft:generic_kill",
            DamageKind::Cramming => "minecraft:cramming",
            DamageKind::PlayerExplosion => "minecraft:player_explosion",
            DamageKind::Fireball => "minecraft:fireball",
            DamageKind::Trident => "minecraft:trident",
            DamageKind::Fireworks => "minecraft:fireworks",
            DamageKind::MobProjectile => "minecraft:mob_projectile",
            DamageKind::Magic => "minecraft:magic",
            DamageKind::IndirectMagic => "minecraft:indirect_magic",
            DamageKind::LightningBolt => "minecraft:lightning_bolt",
            // -- slice 3: mob effects
            DamageKind::Wither => "minecraft:wither",

            // -- slice 3: raids
            DamageKind::Starve => "minecraft:starve",

            // -- slice 3: the end

            // -- slice 3: wither and guardians
            DamageKind::WitherSkull => "minecraft:wither_skull",
            DamageKind::Thorns => "minecraft:thorns",

            // -- slice 3: warden
            DamageKind::SonicBoom => "minecraft:sonic_boom",

            // -- slice 3: common mobs A

            // -- slice 3: common mobs B
            DamageKind::WindCharge => "minecraft:wind_charge",

            // -- wp28: axolotl and goat
            DamageKind::DryOut => "minecraft:dry_out",
            DamageKind::NoAggroMobAttack => "minecraft:mob_attack_no_aggro",

        }
    }

    /// Whether the damage type is in a `minecraft:damage_type` tag.
    pub fn is_tag(self, tag: &str) -> bool {
        let Some(id) = kiln_data::synced_id("minecraft:damage_type", self.type_name()) else { return false };
        kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:damage_type")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
            .is_some_and(|(_, ids)| ids.contains(&id))
    }
}
