//! Enchanting what a mob spawns with (`EnchantmentHelper.enchantItemFromProvider`).
//!
//! The enchantment definitions are the datapack's, which this crate does not read: whoever runs
//! the entities installs an [`Enchanter`] for the thread that does ([`install`]), and the mobs'
//! `finalizeSpawn` and raid code ask it. Without one nothing gets enchanted (the random draws
//! of the rolls are still made).

use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use std::cell::RefCell;
use std::rc::Rc;

/// Enchants a stack from an `enchantment_provider` entry.
pub trait Enchanter {
    /// `EnchantmentHelper.enchantItemFromProvider(stack, provider, difficulty, random)` for a
    /// difficulty whose special multiplier is `special_multiplier`.
    fn enchant(&self, stack: &mut ItemStack, provider: &str, special_multiplier: f32, random: &mut dyn RandomSource);
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

/// The `Mob.enchantSpawnedEquipment` of one slot: with `chance * special_multiplier` odds
/// (drawn only for a filled slot) the item is enchanted from `minecraft:mob_spawn_equipment`.
pub fn enchant_spawned_equipment(stack: &mut ItemStack, chance: f32, special_multiplier: f32, random: &mut dyn RandomSource) {
    if !stack.is_empty() && random.next_float() < chance * special_multiplier {
        enchant_from_provider(stack, "minecraft:mob_spawn_equipment", special_multiplier, random);
    }
}
