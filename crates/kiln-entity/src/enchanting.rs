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
