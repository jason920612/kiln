//! Enchanting what a mob spawns with (`EnchantmentHelper.enchantItemFromProvider`).
//!
//! The enchantment definitions are the datapack's, which this crate does not read: whoever runs
//! the entities installs an [`Enchanter`] for the thread that does ([`install`]), and the mobs'
//! `finalizeSpawn` and raid code ask it. Without one nothing gets enchanted (the random draws
//! of the rolls are still made).

use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
pub use kiln_loot::view::{EntityView, MobPostAttack};
use std::cell::RefCell;
use std::rc::Rc;

/// A blow a mob's weapon deals, as the weapon's enchantments are evaluated for it
/// (`Enchantment.damageContext`).
pub struct Hit<'a> {
    /// The weapon (`getWeaponItem`: the main hand).
    pub weapon: &'a ItemStack,
    pub attacker: &'a EntityView,
    pub victim: &'a EntityView,
    /// `minecraft:damage_type` network id of the damage source.
    pub damage_type: i32,
}

/// A hit by a projectile fired from `weapon` (a bow's arrow, a trident), as the weapon's
/// enchantments are evaluated for it: the projectile is the `direct_attacker`, its owner the
/// `attacker`.
pub struct ProjectileHit<'a> {
    pub weapon: &'a ItemStack,
    pub attacker: Option<&'a EntityView>,
    pub direct: &'a EntityView,
    pub victim: &'a EntityView,
    pub damage_type: i32,
}

/// Enchants a stack from an `enchantment_provider` entry, and says what a weapon's enchantments
/// do to a blow.
pub trait Enchanter {
    /// `EnchantmentHelper.enchantItemFromProvider(stack, provider, difficulty, random)` for a
    /// difficulty whose special multiplier is `special_multiplier`.
    fn enchant(&self, stack: &mut ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn RandomSource);

    /// `EnchantmentHelper.modifyDamage`.
    fn modify_damage(&self, hit: &Hit, damage: f32, random: &mut dyn RandomSource) -> f32 {
        let _ = (hit, random);
        damage
    }

    /// `EnchantmentHelper.modifyKnockback`.
    fn modify_knockback(&self, hit: &Hit, value: f32, random: &mut dyn RandomSource) -> f32 {
        let _ = (hit, random);
        value
    }

    /// `EnchantmentHelper.modifyDamage` for a projectile fired from the weapon (power, impaling).
    fn modify_projectile_damage(&self, hit: &ProjectileHit, damage: f32, random: &mut dyn RandomSource) -> f32 {
        let _ = (hit, random);
        damage
    }

    /// `EnchantmentHelper.modifyKnockback` for a projectile fired from the weapon (punch).
    fn modify_projectile_knockback(&self, hit: &ProjectileHit, value: f32, random: &mut dyn RandomSource) -> f32 {
        let _ = (hit, random);
        value
    }

    /// `EnchantmentHelper.modifyArmorEffectiveness` of the weapon of a blow to a mob.
    fn armor_effectiveness(&self, weapon: &ItemStack, attacker: &EntityView, victim: &EntityView, damage_type: i32, value: f32, random: &mut dyn RandomSource) -> f32 {
        let _ = (weapon, attacker, victim, damage_type, random);
        value
    }

    /// `EnchantmentHelper.getDamageProtection` of a mob's equipment.
    fn damage_protection(&self, equipment: &[(kiln_item::component::EquipmentSlot, &ItemStack)], victim: &EntityView, attacker: &EntityView, damage_type: i32, random: &mut dyn RandomSource) -> f32 {
        let _ = (equipment, victim, attacker, damage_type, random);
        0.0
    }

    /// What the weapon's `post_attack` enchantments do to the victim.
    fn post_attack(&self, hit: &Hit, random: &mut dyn RandomSource) -> Vec<MobPostAttack> {
        let _ = (hit, random);
        Vec::new()
    }

    /// The enchantment definitions, for the code that evaluates effects itself (a projectile's `hit_block` and
    /// `post_attack`).
    fn loot(&self) -> Option<&kiln_loot::LootData> {
        None
    }
}

thread_local! {
    static ENCHANTER: RefCell<Option<Rc<dyn Enchanter>>> = const { RefCell::new(None) };
}

/// What [`install`] replaced, put back when dropped.
pub struct Installed(Option<Rc<dyn Enchanter>>);

impl Drop for Installed {
    fn drop(&mut self) {
        ENCHANTER.with(|e| *e.borrow_mut() = self.0.take());
    }
}

/// Makes `enchanter` the one this thread's mobs use until the result is dropped.
pub fn install(enchanter: Option<Rc<dyn Enchanter>>) -> Installed {
    Installed(ENCHANTER.with(|e| std::mem::replace(&mut *e.borrow_mut(), enchanter)))
}

/// `enchantItemFromProvider`: through the installed [`Enchanter`], if any.
pub fn enchant_from_provider(stack: &mut ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn RandomSource) {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    if let Some(e) = enchanter {
        e.enchant(stack, provider, special_multiplier, random);
    }
}

/// `EnchantmentHelper.modifyDamage` of a mob's weapon, through the installed [`Enchanter`] (the
/// damage unchanged without one).
pub fn modify_damage(hit: &Hit, damage: f32, random: &mut dyn RandomSource) -> f32 {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    match enchanter {
        Some(e) if !hit.weapon.is_empty() => e.modify_damage(hit, damage, random),
        _ => damage,
    }
}

/// `EnchantmentHelper.modifyKnockback` of a mob's weapon.
pub fn modify_knockback(hit: &Hit, value: f32, random: &mut dyn RandomSource) -> f32 {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    match enchanter {
        Some(e) if !hit.weapon.is_empty() => e.modify_knockback(hit, value, random),
        _ => value,
    }
}

/// `EnchantmentHelper.modifyDamage` of a projectile's weapon, through the installed [`Enchanter`].
pub fn modify_projectile_damage(hit: &ProjectileHit, damage: f32, random: &mut dyn RandomSource) -> f32 {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    match enchanter {
        Some(e) if !hit.weapon.is_empty() => e.modify_projectile_damage(hit, damage, random),
        _ => damage,
    }
}

/// `EnchantmentHelper.modifyKnockback` of a projectile's weapon.
pub fn modify_projectile_knockback(hit: &ProjectileHit, value: f32, random: &mut dyn RandomSource) -> f32 {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    match enchanter {
        Some(e) if !hit.weapon.is_empty() => e.modify_projectile_knockback(hit, value, random),
        _ => value,
    }
}

/// What a mob's weapon enchantments do to the victim of a blow.
pub fn post_attack(hit: &Hit, random: &mut dyn RandomSource) -> Vec<MobPostAttack> {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    match enchanter {
        Some(e) if !hit.weapon.is_empty() => e.post_attack(hit, random),
        _ => Vec::new(),
    }
}

/// The `Mob.enchantSpawnedEquipment` of one slot: with `chance * special_multiplier` odds
/// (drawn only for a filled slot) the item is enchanted from `minecraft:mob_spawn_equipment`.
pub fn enchant_spawned_equipment(stack: &mut ItemStack, chance: f32, special_multiplier: f32, random: &mut dyn RandomSource) {
    if !stack.is_empty() && random.next_float() < chance * special_multiplier {
        enchant_from_provider(stack, "minecraft:mob_spawn_equipment", special_multiplier, random);
    }
}

/// The weapon of the blow being dealt to a mob (`DamageSource.getWeaponItem`) and the attacker
/// as enchantment predicates see it, for the armor effectiveness the weapon's enchantments change
/// (breach).
struct AttackScope {
    weapon: ItemStack,
    attacker: EntityView,
}

thread_local! {
    static ATTACK: RefCell<Option<AttackScope>> = const { RefCell::new(None) };
}

/// What [`attack_scope`] replaced, put back when dropped.
pub struct ScopeGuard(Option<AttackScope>);

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        ATTACK.with(|a| *a.borrow_mut() = self.0.take());
    }
}

/// The blow dealt while the result lives is by `attacker` with `weapon`.
pub fn attack_scope(weapon: ItemStack, attacker: EntityView) -> ScopeGuard {
    ScopeGuard(ATTACK.with(|a| a.borrow_mut().replace(AttackScope { weapon, attacker })))
}

/// `EnchantmentHelper.modifyArmorEffectiveness` for the blow in scope (`value` unchanged without
/// one, or without a datapack).
pub fn armor_effectiveness(victim: &EntityView, damage_type: i32, value: f32, random: &mut dyn RandomSource) -> f32 {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    let Some(e) = enchanter else { return value };
    ATTACK.with(|a| match &*a.borrow() {
        Some(s) if !s.weapon.is_empty() => e.armor_effectiveness(&s.weapon, &s.attacker, victim, damage_type, value, random),
        _ => value,
    })
}

/// `EnchantmentHelper.getDamageProtection(level, victim, source)` of a mob's equipment.
pub fn damage_protection(equipment: &[(kiln_item::component::EquipmentSlot, &ItemStack)], victim: &EntityView, damage_type: i32, random: &mut dyn RandomSource) -> f32 {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone());
    let Some(e) = enchanter else { return 0.0 };
    let attacker = ATTACK.with(|a| a.borrow().as_ref().map(|s| s.attacker.clone())).unwrap_or_default();
    e.damage_protection(equipment, victim, &attacker, damage_type, random)
}

/// Runs `f` with the enchantment definitions of the installed [`Enchanter`], if it has them.
pub fn with_loot<R>(f: impl FnOnce(&kiln_loot::LootData) -> R) -> Option<R> {
    let enchanter = ENCHANTER.with(|e| e.borrow().clone())?;
    let loot = enchanter.loot()?;
    Some(f(loot))
}

/// The blocks and sky of an [`crate::level::EntityLevel`], for the location predicates of enchantment requirements.
struct LevelProbe<'a>(&'a dyn crate::level::EntityLevel);

impl kiln_loot::probe::WorldProbe for LevelProbe<'_> {
    fn block(&self, pos: [i32; 3]) -> Option<u16> {
        Some(self.0.block(crate::math::BlockPos::new(pos[0], pos[1], pos[2])))
    }

    fn biome(&self, pos: [i32; 3]) -> Option<&'static str> {
        let id = self.0.biome(crate::math::BlockPos::new(pos[0], pos[1], pos[2]))?;
        kiln_data::live::synced_name("minecraft:worldgen/biome", id)
    }

    fn light(&self, pos: [i32; 3]) -> Option<i32> {
        Some(self.0.block_light(crate::math::BlockPos::new(pos[0], pos[1], pos[2])))
    }

    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        Some(self.0.can_see_sky(crate::math::BlockPos::new(pos[0], pos[1], pos[2])))
    }
}

/// Which of a projectile weapon's effects act on the projectile.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectileTrigger {
    /// `EnchantmentHelper.onHitBlock`: the projectile hit a block.
    HitBlock,
    /// `EnchantmentHelper.onProjectileSpawned`: the projectile was shot.
    Spawned,
}

/// Carries out the `hit_block` or `projectile_spawned` effects of `weapon` for the projectile `e` (shot by `owner`): the
/// effects whose requirements hold, resolved against the projectile (its random) and done to the projectile and the level.
/// `at` is where the effect happens (the hit block's bottom centre for a hit, the projectile for a shot).
pub fn run_projectile_effects(
    trigger: ProjectileTrigger,
    e: &mut crate::entity::Entity,
    level: &mut dyn crate::level::EntityLevel,
    weapon: &ItemStack,
    owner: Option<i32>,
    at: [f64; 3],
    state: Option<u16>,
) {
    use kiln_loot::fx::{Action, Subject};
    use kiln_loot::view::FxContext;
    if weapon.get(kiln_item::keys::ENCHANTMENTS).is_none_or(|l| l.is_empty()) {
        return;
    }
    let this = crate::mob::weapon::attacker_view(e);
    let p = e.position();
    let subject = Subject {
        pos: [p.x, p.y, p.z],
        rot: [e.y_rot, e.x_rot],
        width: e.width,
        height: e.height,
        movement: [e.delta.x, e.delta.y, e.delta.z],
        silent: e.silent,
        living: false,
    };
    // (The requirements' random: derived, so the level's own stream is not drawn from for effects that mostly do not exist.)
    let mut level_rng = kiln_javamath::random::LegacyRandom::new(((e.id as i64) << 24) ^ level.game_time());
    let mut actions: Vec<Action> = Vec::new();
    let ran = with_loot(|loot| {
        let probe = LevelProbe(&*level);
        let facts = kiln_loot::probe::WorldFacts { probe: &probe, dim: "", raining: level.is_raining(), thundering: level.is_thundering() };
        let ctx = |l: i32| FxContext { block_state: state, ..FxContext::entity(l, &this, at, Some(facts)) };
        let mut found: Vec<(&kiln_loot::effects::EntityEffect, i32)> = Vec::new();
        match trigger {
            ProjectileTrigger::HitBlock => loot.hit_block_effects(weapon, &mut level_rng, ctx, |effect, l| found.push((effect, l))),
            ProjectileTrigger::Spawned => loot.projectile_spawned_effects(weapon, &mut level_rng, ctx, |effect, l| found.push((effect, l))),
        }
        for (effect, l) in found {
            kiln_loot::fx::resolve(effect, l, at, &subject, &mut e.random, &mut level_rng, &mut actions);
        }
    });
    if ran.is_none() {
        return;
    }
    for action in actions {
        match action {
            Action::Ignite { seconds } => e.ignite_for_seconds(seconds),
            Action::Impulse { delta } => e.delta = e.delta.add(delta[0], delta[1], delta[2]),
            other => level_action(level, owner, other),
        }
    }
}

/// What a resolved action does to the level (a sound, a lightning bolt); the rest is the caller's.
fn level_action(level: &mut dyn crate::level::EntityLevel, owner: Option<i32>, action: kiln_loot::fx::Action) {
    use kiln_loot::fx::Action;
    match action {
        Action::Sound { sound, volume, pitch, at } => {
            let sound = kiln_data::live::leak_str(sound.as_str());
            level.emit(crate::level::Event::Sound { pos: crate::math::Vec3::new(at[0], at[1], at[2]), sound, source: "neutral", volume, pitch });
        }
        // `SummonEntityEffect`: a lightning bolt credits the player who shot the projectile.
        Action::Summon { entity, at, .. } if entity.as_str() == "minecraft:lightning_bolt" => {
            let (id, seed) = (level.next_entity_id(), level.fresh_seed());
            let pos = crate::math::Vec3::new(at[0], at[1], at[2]);
            let bolt = match owner.filter(|&o| level.player(o).is_some()) {
                Some(player) => crate::ext_entity::lightning::channeled(id, 0, pos, player, seed),
                None => crate::ext_entity::lightning::new(id, 0, pos, false, seed),
            };
            level.add_entity(bolt);
        }
        // (The other effects need the server's level, or are the shooter's.)
        _ => {}
    }
}

/// `EnchantmentHelper.doPostAttackEffectsWithItemSource` for a hit by the projectile `e` of `weapon` on `victim`: the weapon's
/// `post_attack` effects enchanted `attacker` (the owner) whose requirements hold, done to the victim, the owner or the projectile.
pub fn run_projectile_post_attack(
    e: &mut crate::entity::Entity,
    level: &mut dyn crate::level::EntityLevel,
    weapon: &ItemStack,
    owner: Option<i32>,
    victim: i32,
    damage_type: i32,
) {
    use kiln_loot::effects::Target;
    use kiln_loot::fx::{Action, Subject};
    use kiln_loot::view::FxContext;
    if weapon.get(kiln_item::keys::ENCHANTMENTS).is_none_or(|l| l.is_empty()) {
        return;
    }
    let (att, direct, target) = crate::arrow::projectile_views(&*level, e, owner, victim);
    let mut level_rng = kiln_javamath::random::LegacyRandom::new(((e.id as i64) << 24) ^ level.game_time() ^ victim as i64);
    let mut done: Vec<(Target, Vec<Action>)> = Vec::new();
    let ran = with_loot(|loot| {
        let probe = LevelProbe(&*level);
        let facts = kiln_loot::probe::WorldFacts { probe: &probe, dim: "", raining: level.is_raining(), thundering: level.is_thundering() };
        let ctx = |l: i32| FxContext { attacker: att.as_ref(), direct: Some(&direct), damage_type: Some(damage_type), ..FxContext::entity(l, &target, target.pos, Some(facts)) };
        let mut found: Vec<(Target, &kiln_loot::effects::EntityEffect, i32)> = Vec::new();
        // The owner is a living entity: its weapon in the main hand.
        loot.post_attack_effects(weapon, kiln_item::component::EquipmentSlot::MainHand, Target::Attacker, &mut level_rng, ctx, |t, l| found.push((t.affected, &t.effect, l)));
        for (affected, effect, l) in found {
            let view = match affected {
                Target::Victim => &target,
                Target::Attacker => att.as_ref().unwrap_or(&direct),
                Target::DamagingEntity => &direct,
            };
            let subject = Subject { pos: view.pos, rot: [0.0, 0.0], width: 0.6, height: 1.8, movement: view.movement, silent: false, living: true };
            let mut actions = Vec::new();
            kiln_loot::fx::resolve(effect, l, view.pos, &subject, &mut e.random, &mut level_rng, &mut actions);
            done.push((affected, actions));
        }
    });
    if ran.is_none() {
        return;
    }
    for (affected, actions) in done {
        for action in actions {
            match action {
                Action::Ignite { seconds } => match affected {
                    Target::Victim => level.ignite(victim, seconds),
                    Target::DamagingEntity => e.ignite_for_seconds(seconds),
                    Target::Attacker => {}
                },
                other => level_action(level, owner, other),
            }
        }
    }
}
