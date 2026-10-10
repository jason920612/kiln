//! Mob types that bring their behaviour in a module of their own ([`super::kinds`]): a static
//! [`Kind`] with hooks the shared mob tick calls at vanilla's override points, type state in
//! [`Species::Ext`](super::Species::Ext) and type goals as [`CustomGoal`]s. The first eight
//! types (pig ... spider) predate this and live in the shared code.

use super::attributes::Attr;
use super::goals::{Goal, Living};
use super::interact::{Interactor, Outcome};
use super::{Category, DamageSource, GroupData, MobData, MobKind, SpawnContext};
use crate::entity::Entity;
use crate::level::{DamageKind, EntityLevel};
use crate::math::{BlockPos, Vec3};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::packets::entity::EntityData;
use std::any::Any;
use std::fmt::Debug;

/// Static facts about a type.
#[derive(Debug)]
pub struct Info {
    pub name: &'static str,
    pub category: Category,
    /// `createAttributes` on top of the mob defaults (`Mob.createMobAttributes`: follow range 16,
    /// plus attack damage 2 for monsters, tempt range 10 for animals).
    pub attrs: &'static [(Attr, f64)],
    /// Extends `Animal` (love mode, breeding food, animal walk target and experience).
    pub animal: bool,
    /// Extends `AgeableMob` (an age; babies grow up).
    pub ageable: bool,
    /// `EntityTypeTags.BURN_IN_DAYLIGHT`.
    pub burns_in_daylight: bool,
    /// `EntityTypeTags.CAN_BREATHE_UNDER_WATER`.
    pub breathes_under_water: bool,
    /// `EntityType.fireImmune`.
    pub fire_immune: bool,
    /// `Monster.createMonsterAttributes` (attack damage 2) under the type's attributes; false for
    /// types built on `Mob.createMobAttributes`.
    pub monster_base: bool,
    /// `getMaxHeadYRot`, `getMaxHeadXRot`, `getHeadRotSpeed`.
    pub head: (i32, i32, i32),
    /// `getAmbientSoundInterval`.
    pub ambient_interval: i32,
    /// The `SoundSource` name.
    pub sound_source: &'static str,
    /// `entity.<sounds>.ambient` / `.hurt` / `.death` / `.step` (None: no ambient sound).
    pub sounds: Option<&'static str>,
    /// The class extends `Monster` (`updateNoActionTime`: bright light ages the idle time).
    pub extends_monster: bool,
}

impl Info {
    /// Monster defaults for `name` (`minecraft:` prefixed).
    pub const fn monster(name: &'static str, attrs: &'static [(Attr, f64)]) -> Info {
        Info {
            name,
            category: Category::Monster,
            attrs,
            animal: false,
            ageable: false,
            burns_in_daylight: false,
            breathes_under_water: false,
            fire_immune: false,
            monster_base: true,
            head: (75, 40, 10),
            ambient_interval: 80,
            sound_source: "hostile",
            sounds: None,
            extends_monster: true,
        }
    }

    /// `MobCategory.MISC` defaults (villagers, golems: `Mob.createMobAttributes`).
    pub const fn misc(name: &'static str, attrs: &'static [(Attr, f64)]) -> Info {
        Info {
            name,
            category: Category::Misc,
            attrs,
            animal: false,
            ageable: false,
            burns_in_daylight: false,
            breathes_under_water: false,
            fire_immune: false,
            monster_base: false,
            head: (75, 40, 10),
            ambient_interval: 80,
            sound_source: "neutral",
            sounds: None,
            extends_monster: false,
        }
    }

    /// `Animal` defaults.
    pub const fn animal(name: &'static str, attrs: &'static [(Attr, f64)]) -> Info {
        Info {
            name,
            category: Category::Creature,
            attrs,
            animal: true,
            ageable: true,
            burns_in_daylight: false,
            breathes_under_water: false,
            fire_immune: false,
            monster_base: false,
            head: (75, 40, 10),
            ambient_interval: 120,
            sound_source: "neutral",
            sounds: None,
            extends_monster: false,
        }
    }
}

/// Type state kept in [`Species::Ext`](super::Species::Ext).
pub trait MobExt: Any + Debug + Send + Sync {
    fn box_clone(&self) -> Box<dyn MobExt>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any + Debug + Clone + Send + Sync> MobExt for T {
    fn box_clone(&self) -> Box<dyn MobExt> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Clone for Box<dyn MobExt> {
    fn clone(&self) -> Self {
        (**self).box_clone()
    }
}

impl PartialEq for Box<dyn MobExt> {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

/// The type state of `m` as `T` (the type's own state struct).
pub fn state<T: 'static>(m: &MobData) -> Option<&T> {
    match &m.species {
        super::Species::Ext(s) => (**s).as_any().downcast_ref::<T>(),
        _ => None,
    }
}

pub fn state_mut<T: 'static>(m: &mut MobData) -> Option<&mut T> {
    match &mut m.species {
        super::Species::Ext(s) => (**s).as_any_mut().downcast_mut::<T>(),
        _ => None,
    }
}

/// A goal of one type (vanilla's goal classes the shared [`Goal`] enum does not have). `name`
/// is vanilla's simple class name, which the parity harness compares directly.
pub trait CustomGoal: Debug + Send + Sync {
    fn box_clone(&self) -> Box<dyn CustomGoal>;
    fn name(&self) -> &'static str;
    /// `Goal.Flag`s ([`super::goals::MOVE`] ...).
    fn flags(&self) -> u8;
    /// `requiresUpdateEveryTick`.
    fn every_tick(&self) -> bool {
        false
    }
    /// `Bee$BaseBeeGoal`: `canUse` and `canContinueToUse` are the goal's own check (with its side
    /// effects) and then "not angry".
    fn bee_base(&self) -> bool {
        false
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool;
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.can_use(e, m, level)
    }
    /// `isInterruptable`.
    fn interruptable(&self) -> bool {
        true
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl Clone for Box<dyn CustomGoal> {
    fn clone(&self) -> Self {
        CustomGoal::box_clone(&**self)
    }
}

/// Implements the boilerplate of [`CustomGoal`] (`box_clone`, `as_any`) for a `Clone` goal.
#[macro_export]
macro_rules! custom_goal_boilerplate {
    () => {
        fn box_clone(&self) -> Box<dyn $crate::mob::ext::CustomGoal> {
            Box::new(self.clone())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

/// What natural spawning lets a type's spawn rules see.
pub trait SpawnView {
    fn block(&self, pos: BlockPos) -> u16;
    /// `getRawBrightness(pos, skyDarken)`.
    fn raw_brightness(&self, pos: BlockPos, sky_darken: i32) -> i32;
    fn sky_darken(&self) -> i32;
    fn sky_light(&self, pos: BlockPos) -> i32;
    fn block_light(&self, pos: BlockPos) -> i32;
    /// The biome's `minecraft:worldgen/biome` id.
    fn biome(&self, pos: BlockPos) -> i32;
    fn difficulty(&self) -> u8;
    fn world_seed(&self) -> i64;
    /// `DimensionType.moonBrightness` now (1 at full moon).
    fn moon_brightness(&self) -> f32;
    fn min_y(&self) -> i32;
    fn sea_level(&self) -> i32;
    /// One above the highest non-air block of the column at `x`, `z` (`WORLD_SURFACE`), if the
    /// view knows it without scanning; `None` makes callers scan the column.
    fn world_surface(&self, x: i32, z: i32) -> Option<i32> {
        let _ = (x, z);
        None
    }
    /// `EntitySpawnReason.isSpawner`: the spawn comes from a spawner block, which needs no valid
    /// block below (`Mob.checkMobSpawnRules`) and no open sky (`checkSurfaceMonstersSpawnRules`).
    fn spawner(&self) -> bool {
        false
    }
    /// The dimension's `monster_spawn_block_light_limit` (overworld 0, nether 15).
    fn monster_block_light_limit(&self) -> i32 {
        0
    }
    /// The dimension's `monster_spawn_light_level` as an inclusive range (the overworld's
    /// uniform 0..=7; a constant has equal ends and draws nothing).
    fn monster_light_test(&self) -> (i32, i32) {
        (0, 7)
    }
    /// `Level.isThundering` (monsters spawn in the thunder's light).
    fn thundering(&self) -> bool {
        false
    }
}

/// `SpawnPlacementTypes` of a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    OnGround,
    InWater,
    InLava,
    NoRestrictions,
}

/// A mob type's behaviour. Every hook has vanilla's base behaviour as its default.
pub trait Kind: Sync + Send {
    fn info(&self) -> &'static Info;

    /// The type state (with the constructor's random draws from the mob's own `random`).
    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        let _ = (m, random);
        None
    }

    /// `makeBrain`: the brain of a brain-driven type (built after the constructor's yaw draw;
    /// its sensors' first scans are delayed by draws from `random`). `None` for goal-driven types.
    fn make_brain(&self, m: &MobData, random: &mut dyn RandomSource) -> Option<super::brain::Brain> {
        let _ = (m, random);
        None
    }

    /// `registerGoals` (and goals the constructor adds).
    fn register_goals(&self, m: &mut MobData) {
        default_goals(m, self.info());
    }

    /// Before `LivingEntity.tick` (`Creeper.tick`-style pre-tick work).
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// After `LivingEntity.tick` and `Mob.tick`'s control flags.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// The type's `aiStep` additions after `Mob.aiStep` (and `AgeableMob.aiStep` for ageable types).
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// Before `LivingEntity.aiStep`'s body runs (types that override `aiStep` and do work before `super.aiStep()`).
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `customServerAiStep` (after the navigation tick, before the controls).
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `travel` in place of `LivingEntity.travel` (flying types); true when handled.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        let _ = (e, m, level, input);
        false
    }
    /// `travelInWater` in place of the shared one; true when handled (turtles swim their way).
    fn travel_in_water(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        let _ = (e, m, level, input);
        false
    }
    /// `getWaterSlowDown` (0.8; polar bears 0.98).
    fn water_slow_down(&self, m: &MobData) -> f32 {
        let _ = m;
        0.8
    }
    /// The type's `MoveControl.tick`; true when handled.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// The type's `LookControl.tick`; true when handled.
    fn tick_look(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// The type's `JumpControl.tick`; true when handled (rabbits start a hop instead).
    fn tick_jump(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// The type's `BodyRotationControl` (`tickHeadTurn`); true when handled.
    fn tick_body(&self, e: &mut Entity, m: &mut MobData) -> bool {
        let _ = (e, m);
        false
    }
    /// `Creaking.HomeNodeEvaluator`: the point beyond 32 blocks of which the navigation finds no
    /// way (unless it leads back toward it).
    fn path_home(&self, m: &MobData) -> Option<BlockPos> {
        let _ = m;
        None
    }
    /// `PathNavigation.tick` (creakings that cannot move skip it); false skips the tick.
    fn ticks_navigation(&self, m: &MobData) -> bool {
        let _ = m;
        true
    }
    /// `isPushable` where it depends on the mob's state (a frozen creaking): false, and
    /// `Entity.push` moves nothing.
    fn can_be_pushed(&self, m: &MobData) -> bool {
        let _ = m;
        true
    }
    /// `hurtServer` overrides that decide before the shared code: `Some(result)` ends it.
    fn hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> Option<bool> {
        let _ = (e, m, level, source, amount);
        None
    }
    /// The type's `actuallyHurt` additions, right after the shared health and absorption change
    /// (and before the hurt time, the attacker bookkeeping and the knockback).
    fn actually_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) {
        let _ = (e, m, level, source, amount);
    }
    /// `onOffspringSpawnedFromEgg(player, baby)`: what the type does when a spawn egg brought a baby of it (a fox trusts the player).
    fn offspring_from_egg(&self, m: &mut MobData, baby: &mut Entity, level: &mut dyn EntityLevel, player: i32) {
        let _ = (m, baby, level, player);
    }
    /// An `actuallyHurt` that does not call the shared one (a wolf's armor takes the blow): whether it dealt with `amount`.
    fn override_actually_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32) -> bool {
        let _ = (e, m, level, source, amount);
        false
    }
    /// After the shared `hurtServer` (reinforcements, anger, ...), with its result.
    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, amount: f32, hurt: bool) {
        let _ = (e, m, level, source, amount, hurt);
    }
    /// `isInvulnerableTo` / `fireImmune` for a damage type.
    fn is_invulnerable_to(&self, m: &MobData, kind: DamageKind) -> bool {
        let _ = m;
        self.info().fire_immune && kind.is_tag("minecraft:is_fire")
    }
    /// `dropCustomDeathLoot` past the equipment (an enderman's carried block), when the mob drops
    /// loot at all.
    fn drop_custom_death_loot(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        let _ = (e, m, level, source);
    }
    /// Extra work in `die` (after loot and experience).
    fn die(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource) {
        let _ = (e, m, level, source);
    }
    /// `remove(KILLED)` work (slimes split).
    fn on_killed_removal(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `aiStep` overridden without `super.aiStep()` (the ender dragon): runs in place of
    /// `LivingEntity.aiStep` when it returns true.
    fn replaces_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// `tickDeath` in place of `LivingEntity.tickDeath` (20 ticks then removal); true when
    /// handled.
    fn tick_death(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// `handleKillingBlow` overrides: true when the mob is not marked `dead` (the ender dragon
    /// starts its dying phase instead).
    fn handle_killing_blow(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// `knockback` overridden to do nothing (a sitting ender dragon).
    fn knockback_immune(&self, m: &MobData) -> bool {
        let _ = m;
        false
    }
    /// false: `checkDespawn` overridden to do nothing (never despawns, not even on peaceful).
    fn despawns(&self) -> bool {
        true
    }
    /// `doHurtTarget` in place of the shared one: `Some(hit)` when handled.
    fn do_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) -> Option<bool> {
        let _ = (e, m, level, t);
        None
    }
    /// `setTarget` overrides, before the target changes to `target` (zombified piglins draw their
    /// anger timers when they first get one).
    fn on_set_target(&self, e: &mut Entity, m: &mut MobData, target: Option<i32>) {
        let _ = (e, m, target);
    }
    /// `TargetingConditions` selector of the type's player `NearestAttackableTargetGoal`
    /// (drowned `okTarget`, zombified piglin `isAngryAt`).
    fn player_target_ok(&self, e: &Entity, m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        let _ = (e, m, level, t);
        true
    }
    /// `AbstractSkeleton.getArrow` / `performRangedAttack` extras on the arrow just made
    /// (stray slowness, wither skeleton fire).
    fn ranged_arrow(&self, e: &mut Entity, m: &mut MobData, arrow: &mut Entity) {
        let _ = (e, m, arrow);
    }
    /// After a successful shared `doHurtTarget` (husk hunger, wither skeleton wither).
    fn after_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, t: &Living) {
        let _ = (e, m, level, t);
    }
    /// The whole `finalizeSpawn` (call [`mob_finalize`] where vanilla calls `Mob.finalizeSpawn`).
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        if self.info().ageable {
            ageable_finalize(e, m, r, group, 0.05);
        }
        mob_finalize(m, r);
        let _ = ctx;
    }
    /// `readAdditionalSaveData` of the type (after the shared mob fields).
    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let _ = (e, m, r);
    }
    /// `addAdditionalSaveData` of the type.
    fn save(&self, e: &Entity, m: &MobData, o: &mut Output) {
        let _ = (e, m, o);
    }
    /// The type's entity data for viewers (after the shared living/mob/ageable fields).
    fn entity_data(&self, e: &Entity, m: &MobData, d: &mut EntityData) {
        let _ = (e, m, d);
    }
    /// `mobInteract` (and item interactions on this type): `Some` when the type handles the click.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let _ = (e, m, level, who, stack);
        None
    }
    /// `getDefaultDimensions`: (width, height, eye height) from the type's `base`.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (base.0 * 0.5, base.1 * 0.5, base.2 * 0.5) } else { base }
    }
    /// `getWalkTargetValue`: `None` for the category's default.
    fn walk_target_value(&self, m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        let _ = (m, level, p);
        None
    }
    /// `SpawnPlacements.checkSpawnRules`: `None` for the category's default rules.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        let _ = (view, pos, r);
        None
    }
    fn placement(&self) -> Placement {
        Placement::OnGround
    }
    /// `checkSpawnObstruction` lets the type spawn with liquid in its box (drowned).
    fn spawn_in_liquids(&self) -> bool {
        false
    }
    /// `getBaseExperienceReward`: `None` for the shared rule.
    fn experience(&self, e: &mut Entity, m: &MobData) -> Option<i32> {
        let _ = (e, m);
        None
    }
    /// The death loot table (`None`: `minecraft:entities/<type>`).
    fn loot_table(&self, m: &MobData) -> Option<String> {
        let _ = m;
        None
    }
    /// `Animal.isFood` (breeding and tempting food).
    fn is_food(&self, item: i32) -> bool {
        let _ = item;
        false
    }
    /// `sunProtectionSlot` is `BODY` (zombie horses and zombie nautiluses: their armor keeps the
    /// sun off, not a helmet).
    fn sun_protection_on_body(&self) -> bool {
        false
    }
    /// The body slot's stack, for [`Kind::sun_protection_on_body`] types.
    fn body_slot_mut<'a>(&self, m: &'a mut MobData) -> Option<&'a mut ItemStack> {
        let _ = m;
        None
    }
    /// `AgeableMob.canBeABaby`: false for the types that are never babies (zombie horses, camel
    /// husks, zombie nautiluses): no baby size or sounds, no `Age` in their saved form.
    fn can_be_baby(&self) -> bool {
        true
    }
    /// `EntityType.isAllowedInPeaceful` of a type the category would send away in peaceful: the
    /// monsters that are not `notInPeaceful` (zombie horses, camel husks) stay.
    fn allowed_in_peaceful(&self) -> Option<bool> {
        None
    }
    /// The type `getBreedOffspring` makes a baby of with `partner` (a horse and a donkey have a
    /// mule).
    fn offspring_kind(&self, m: &MobData, partner: &MobData) -> MobKind {
        let _ = partner;
        m.kind
    }
    /// `getBreedOffspring` extras: set up `child` from the parents (variants, colors).
    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, partner, child, level);
    }
    /// `Monster.updateNoActionTime` at the start of `aiStep` (`Raider`s: always two more).
    fn update_no_action_time(&self, e: &Entity, m: &mut MobData, level: &dyn EntityLevel) {
        if self.info().extends_monster && super::light_magic_value(e, level) > 0.5 {
            m.no_action_time += 2;
        }
    }
    /// `canBeAffected` of the type: `base` is `LivingEntity`'s answer (the type tags).
    fn can_be_affected(&self, m: &MobData, effect: &crate::effect::Effect, base: bool) -> bool {
        let _ = (m, effect);
        base
    }
    /// After `LivingEntity.tickEffects` (end of `baseTick`): a type's own per-tick work there.
    fn tick_effects(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `getDamageAfterMagicAbsorb` additions (after armor): mob `id` takes `amount`.
    fn damage_after_magic_absorb(&self, id: i32, m: &MobData, source: &DamageSource, amount: f32) -> f32 {
        let _ = (id, m, source);
        amount
    }
    /// `removeWhenFarAway` for a type that despawns differently from its category.
    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        let _ = m;
        None
    }
    /// `removeWhenFarAway(distSqr)` for a type whose answer depends on the distance to the
    /// nearest player (patrolling raiders).
    fn remove_when_far_away_at(&self, m: &MobData, dist_sqr: f64) -> Option<bool> {
        let _ = dist_sqr;
        self.remove_when_far_away(m)
    }
    /// `LivingEntity.updatingUsingItem` before the use counter advances: the used item's
    /// `onUseTick` (a crossbow charging).
    fn update_using_item(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `hasLineOfSight` overrides: false when the mob cannot see at all now (a stunned ravager).
    fn can_see(&self, m: &MobData) -> bool {
        let _ = m;
        true
    }
    /// `jumpFromGround` in place of `LivingEntity.jumpFromGround`; true when handled.
    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        let _ = (e, m, level);
        false
    }
    /// `jumpInLiquid(water or lava)` in place of the shared rise of 0.04; true when handled.
    fn jump_in_liquid(&self, e: &mut Entity, m: &mut MobData, lava: bool) -> bool {
        let _ = (e, m, lava);
        false
    }
    /// `isSensitiveToWater`: hurt (drowning, 1) in water or rain at the end of `LivingEntity.aiStep`.
    fn sensitive_to_water(&self) -> bool {
        false
    }
    /// `playerTouch`: a player's box inflated by (1, 0.5, 1) touches the mob (vanilla runs it in
    /// the player's `aiStep`; the simulation calls [`super::player_touch`] after the entity tick).
    fn player_touch(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: &Living) {
        let _ = (e, m, level, player);
    }
    /// Whether [`Kind::player_touch`] is the type's own (the simulation looks for touching
    /// players only around such mobs).
    fn touches_players(&self) -> bool {
        false
    }
    /// `getMaxSpawnClusterSize`.
    fn max_spawn_cluster(&self) -> i32 {
        4
    }
    /// Natural spawning's walk target test (`isValidPositionForMob` → `checkSpawnRules`) always
    /// passes: `PathfinderMob`s that are neither monsters nor animals (value 0), plain `Mob`s.
    fn spawn_ignores_light(&self) -> bool {
        false
    }
    /// false: `checkFallDamage` overridden to do nothing (ghasts, phantoms: no fall distance,
    /// no landing, no fluid refresh after the move).
    fn checks_fall_damage(&self) -> bool {
        true
    }
    /// `LivingEntity.canAttack` extras (a tamed animal never attacks its owner): false vetoes `t`.
    fn can_attack(&self, m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        let _ = (m, level, t);
        true
    }
    /// `Animal.canMate` beyond both being in love (tamed wolves only, not sitting ...).
    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        let _ = (m, partner);
        true
    }
    /// `canBreatheUnderwater` when it depends on the state (`None`: the type's `EntityTypeTags.CAN_BREATHE_UNDER_WATER`).
    fn breathes_under_water_now(&self, m: &MobData) -> Option<bool> {
        let _ = m;
        None
    }
    /// `increaseAirSupply(current)`(4 more a tick, up to the maximum; a dolphin takes a full breath at once).
    fn increase_air_supply(&self, current: i32, max: i32) -> i32 {
        (current + 4).min(max)
    }
    /// `getAmbientSound` when it draws randomness or depends on state: `Some(sound)` replaces the
    /// type's `ambient` sound (`Some(None)`: silent this time, no pitch draws).
    fn ambient_sound(&self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let _ = (e, m, level);
        None
    }
    /// `getMaxHeadXRot` (wolves look less far up while sitting).
    fn max_head_x_rot(&self, m: &MobData) -> i32 {
        let _ = m;
        self.info().head.1
    }
    /// `isEffectiveAi` (and so `isControlledByLocalInstance`): false for a mob that does not move at all, gravity and
    /// push included (an immovable mannequin).
    fn effective_ai(&self, m: &MobData) -> bool {
        let _ = m;
        true
    }
    /// `isImmobile` beyond dying (a grazing or rearing horse): no AI and no input this tick.
    fn is_immobile(&self, m: &MobData) -> bool {
        let _ = m;
        false
    }
    /// Whether a player riding first steers the mob (`getControllingPassenger` returns the player:
    /// a saddled horse, a saddled strider when the rider holds a warped fungus on a stick).
    fn steerable_by(&self, m: &MobData, rider: &crate::level::PlayerView) -> bool {
        let _ = (m, rider);
        false
    }
    /// `getControllingPassenger` when it is a player: the player's client moves the mob, the
    /// server runs no AI for it.
    fn controlling_player(&self, e: &Entity, m: &MobData, level: &dyn EntityLevel) -> Option<i32> {
        let first = level.player(*e.passengers.first()?)?;
        self.steerable_by(m, &first).then_some(first.id)
    }
    /// `FoodOnAStickItem`: the stick that steers the type (its item and the durability one boost
    /// costs), for the types that are `ItemSteerable`.
    fn stick(&self) -> Option<(&'static str, i32)> {
        None
    }
    /// `ItemSteerable.boost`: whether a boost began (the stick is used up then).
    fn boost(&self, e: &mut Entity, m: &mut MobData) -> bool {
        let _ = (e, m);
        false
    }
    /// `tickRidden` with the controlling player (rotations follow the rider).
    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, rider: &crate::level::PlayerView) {
        let _ = (e, m, level, rider);
    }
    /// Equipment beyond the six hand and armor slots, as (`EquipmentSlot` ordinal, stack): a
    /// horse's saddle (7).
    /// `Shearable.readyForShearing` of an alive mob of the type (a dispenser's shears look at it).
    fn ready_for_shearing(&self, m: &MobData) -> bool {
        let _ = m;
        false
    }
    /// `setItemSlot` and `setGuaranteedDrop` for the slots past the six (`BODY` is 6, `SADDLE` 7) when a
    /// dispenser puts a piece on: whether the type has the slot.
    fn set_extra_equipment(&self, m: &mut MobData, slot: u8, stack: ItemStack) -> bool {
        let _ = (m, slot, stack);
        false
    }
    /// `setItemSlot(slot, EMPTY)` for a slot past the six (shears take the piece off): the piece, if the type has one there.
    fn remove_extra_equipment(&self, m: &mut MobData, slot: u8) -> Option<ItemStack> {
        let _ = (m, slot);
        None
    }
    /// `Mob.canShearEquipment` where it is not "no passenger" (a wolf: only its owner `player`).
    fn can_shear_equipment(&self, m: &MobData, level: &dyn EntityLevel, player: i32) -> Option<bool> {
        let _ = (m, level, player);
        None
    }
    /// A dispenser puts a chest on a pack animal (`AbstractChestedHorse`'s slot 499): whether it has one now.
    fn put_chest(&self, m: &mut MobData) -> bool {
        let _ = m;
        false
    }
    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let _ = m;
        Vec::new()
    }
    /// The equipment beyond the six slots that `dropCustomDeathLoot` looks at (`BODY`, then `SADDLE`),
    /// taken off the mob as (stack, drop chance).
    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let _ = m;
        Vec::new()
    }
    /// `dropEquipment`: what the type drops besides equipment and loot (a horse's inventory and
    /// chest).
    fn drop_equipment(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `doPush(other)` before the push itself (iron golems pick fights with monsters they bump).
    fn do_push(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, other: i32) {
        let _ = (e, m, level, other);
    }
    /// The type's path finder measures the cost of a step horizontally (`Node.distanceToXZ`: the
    /// warden's `Warden$1$1`).
    fn path_distance_xz(&self) -> bool {
        false
    }
    /// `getFluidJumpThreshold` when the type overrides it (the breeze: its eye height).
    fn fluid_jump_threshold(&self, e: &Entity) -> Option<f64> {
        let _ = e;
        None
    }
    /// [`Kind::do_push`] with the level at hand for changes (wp28: the warden gets angry at what
    /// bumps it, which plays a sound and changes its brain).
    fn do_push_mut(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, other: i32) {
        let _ = (e, m, level, other);
    }
    /// `isStableDestination` of the type's navigation (striders stand on lava): `None` for
    /// the ground navigation's.
    fn stable_destination(&self, level: &dyn EntityLevel, p: BlockPos) -> Option<bool> {
        let _ = (level, p);
        None
    }
    /// [`Kind::stable_destination`] for types whose answer depends on their state (a turtle
    /// travelling out wants water).
    fn stable_destination_for(&self, m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<bool> {
        let _ = m;
        self.stable_destination(level, p)
    }
    /// `shouldPassengersInheritMalus`: a mob rider takes the mount's path maluses (a zombified
    /// piglin on a strider walks on lava).
    fn passengers_inherit_malus(&self) -> bool {
        false
    }
    /// `updateControlFlags` of the type when it is not `Mob`'s: a ravager keeps its goals'
    /// flags for riders that are raiders.
    fn keeps_flags_for_raiders(&self) -> bool {
        false
    }
    /// The items a `TemptGoal` of the type follows.
    fn tempted_by(&self, item: i32) -> bool {
        let _ = item;
        false
    }
    /// `positionRider` / `getPassengerAttachmentPoint`: where a passenger sits, relative to the
    /// vehicle's position (`None`: vanilla's default, on top of the box).
    fn passenger_offset(&self, e: &Entity, m: &MobData) -> Option<Vec3> {
        let _ = (e, m);
        None
    }
    /// `checkDespawn` in place of `Mob.checkDespawn` (the wither stays and never idles): true
    /// when handled.
    fn check_despawn(&self, e: &mut Entity, level: &dyn EntityLevel) -> bool {
        let _ = (e, level);
        false
    }
    /// `isPushedByFluid` (water animals are not).
    fn pushed_by_fluid(&self) -> bool {
        true
    }
    /// `getSwimSound` when `getMovementEmission` emits sounds; `None` for `MovementEmission.EVENTS`
    /// (no step or swim sounds, no pitch draws).
    fn swim_sound(&self) -> Option<&'static str> {
        Some("minecraft:entity.generic.swim")
    }
    /// A swim sound that depends on the mob's state (a calf's), when it has one.
    /// `getSwimSplashSound` and `getSwimSound` when they are not the generic ones (`doWaterSplashEffect`).
    fn splash_sounds(&self) -> Option<(&'static str, &'static str)> {
        None
    }
    fn swim_sound_for(&self, _m: &MobData) -> Option<&'static str> {
        None
    }
    /// After `Mob.baseTick` (the ambient sound roll): `WaterAnimal.handleAirSupply` with the air
    /// supply from before the base tick.
    fn after_base_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
        let _ = (e, m, level, air_before);
    }
    /// `getAmbientSound` for types that draw from the level's random (parrots): [`Kind::ambient_sound`]
    /// unless the type says otherwise.
    fn ambient_sound_mut(&self, e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel) -> Option<Option<&'static str>> {
        self.ambient_sound(e, m, &*level)
    }
    /// `omnidirectionalAirMover` (parrots, bees): the vertical air drag is the horizontal one.
    fn omnidirectional_air_mover(&self) -> bool {
        false
    }
    /// [`Kind::omnidirectional_air_mover`] for types that decide by their state (a sulfur cube with an
    /// item in it).
    fn omnidirectional_air_mover_now(&self, m: &MobData) -> bool {
        let _ = m;
        self.omnidirectional_air_mover()
    }
    /// `detectEquipmentUpdates` past the attribute modifiers of the six slots: the type's own slots
    /// (a sulfur cube's body) noticed a tick after they changed.
    fn detect_equipment_updates(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `knockback(strength, dx, dz, source, amount)` of a full hit, when the type has its own
    /// (a sulfur cube with an item in it): true when it moved the mob.
    fn hit_knockback(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, strength: f64, dx: f64, dz: f64, source: &DamageSource, amount: f32) -> bool {
        let _ = (e, m, level, strength, dx, dz, source, amount);
        false
    }
    /// What `travelInFluid` of the type adds after the shared movement (a floating sulfur cube bobs up).
    fn after_travel_in_fluid(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `doPush(player)` overridden to nothing (parrots): neither side moves.
    fn do_push_skips_players(&self) -> bool {
        false
    }
    /// `getSoundVolume` (squids 0.4, bats 0.1).
    fn sound_volume(&self, m: &MobData) -> f32 {
        let _ = m;
        1.0
    }
    /// `getVoicePitch` when it is a constant that draws nothing (happy ghasts: 1).
    fn fixed_voice_pitch(&self) -> Option<f32> {
        None
    }
    /// `getVoicePitch` from the shared one (bats: 0.95 of it).
    fn voice_pitch(&self, m: &MobData, pitch: f32) -> f32 {
        let _ = m;
        pitch
    }
    /// `thunderHit` by bolt `bolt` in place of `Entity.thunderHit`; true when handled (mooshrooms
    /// change color instead of burning).
    fn thunder_hit(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, bolt: i32) -> bool {
        let _ = (e, m, level, bolt);
        false
    }
    /// What the type does after the plain `Entity.thunderHit` (which the caller runs when `thunder_hit` is false).
    fn after_thunder_hit(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, bolt: i32) {
        let _ = (e, m, level, bolt);
    }
    /// `isPushable` (false: bats neither push nor get pushed).
    fn pushable(&self) -> bool {
        true
    }
    /// `AbstractSkeleton.getAttackInterval` / `getHardAttackInterval` (`None`: 40 and 20).
    fn bow_interval(&self, hard: bool) -> Option<i32> {
        let _ = hard;
        None
    }
    /// `spawnChildFromBreeding` dropping an item instead of a baby (sniffers lay an egg).
    fn breed_as_item(&self) -> Option<&'static str> {
        None
    }
    /// `calculateFallDamage` overridden to take points off (frogs: 5, goats: 10).
    fn fall_damage_reduction(&self) -> i32 {
        0
    }
    /// `spawnChildFromBreeding` without a child: the mother is pregnant (frogs lay frogspawn).
    fn breed_as_pregnancy(&self) -> bool {
        false
    }
    /// The parity replay's pin (`brain::pin`): seeds the randoms vanilla cannot seed
    /// (`Collections.shuffle`) from `base`, the mob's random state.
    fn pin_replay(&self, m: &mut MobData, base: i64) {
        let _ = (m, base);
    }
    /// [`Kind::passenger_offset`] for the passenger at `index` (camels seat two).
    fn passenger_offset_at(&self, e: &Entity, m: &MobData, index: usize) -> Option<Vec3> {
        let _ = index;
        self.passenger_offset(e, m)
    }
    /// `AgeableMob.ageBoundaryReached` (a baby grows up, an adult is made a baby): villagers
    /// rebuild their brain for the other age.
    fn age_boundary_reached(&self, e: &mut Entity, m: &mut MobData) {
        let _ = (e, m);
    }
    /// The part of `ageBoundaryReached` that needs the level (a happy ghast stops its brain).
    fn age_boundary_reached_in(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let _ = (e, m, level);
    }
    /// `shouldDiscardFriction`: in the air the motion is kept, without drag (a long-jumping goat).
    fn discard_friction(&self, m: &MobData) -> bool {
        let _ = m;
        false
    }
    /// `getHurtSound` / `getDeathSound` that depend on the mob's state (screaming goats): `None`
    /// for the type's own `hurt` / `death` sound.
    fn hurt_sound_for(&self, m: &MobData) -> Option<&'static str> {
        let _ = m;
        None
    }
    /// `getHurtSound(source)` of a type whose sound depends on what hurt it (a wolf in armor).
    fn hurt_sound_from(&self, m: &MobData, source: &DamageSource) -> Option<&'static str> {
        let _ = source;
        self.hurt_sound_for(m)
    }
    fn death_sound_for(&self, m: &MobData) -> Option<&'static str> {
        let _ = m;
        None
    }
    /// `setYHeadRot` overrides: the head rotation `head` asked for while the body is at `body`
    /// (goats keep their head within 15 degrees of the body).
    fn set_head_rot(&self, body: f32, head: f32) -> f32 {
        let _ = body;
        head
    }
}

/// `WaterAnimal.handleAirSupply` / `AgeableWaterCreature.handleAirSupply`: out of the water the
/// air runs out a point a tick, then drowning hurts for 2; in the water it is full.
pub fn water_animal_air(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
    if super::is_alive(e, m) && !e.is_in_water() {
        e.air_supply = air_before - 1;
        if e.air_supply <= -20 {
            e.air_supply = 0;
            super::hurt(e, m, level, DamageSource::of(DamageKind::Drown), 2.0);
        }
    } else {
        e.air_supply = 300;
    }
}

/// `Mob.finalizeSpawn`: the follow range bonus and left-handedness, from `r` (the level's random).
pub fn mob_finalize(m: &mut MobData, r: &mut dyn RandomSource) {
    let bonus = super::mth::triangle(r, 0.0, 0.11485000000000001);
    m.attrs.set_modifier(Attr::FollowRange, "minecraft:random_spawn_bonus", bonus, super::attributes::Op::AddMultipliedBase);
    m.left_handed = r.next_float() < 0.05;
}

/// `AgeableMob.finalizeSpawn` with an `AgeableMobGroupData` of `chance`: after the first mob of a
/// group, babies at `chance`.
pub fn ageable_finalize(e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, group: &mut GroupData, chance: f32) {
    if group.ageable_group_size > 0 && r.next_float() <= chance {
        super::set_age(e, m, super::breed::BABY_START_AGE);
    }
    group.ageable_group_size += 1;
}

/// The goals of a type without its own `registerGoals` yet: float, stroll, look.
pub fn default_goals(m: &mut MobData, info: &Info) {
    let g = &mut m.goals;
    g.add(0, Goal::Float);
    if info.category == Category::Creature {
        g.add(1, Goal::Panic { speed: 1.25, pos: Vec3::ZERO });
    }
    g.add(6, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
    g.add(7, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
    g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
}
