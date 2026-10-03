//! `enchantment_provider/` of the vanilla datapack: the `single` providers of raids and pillagers
//! and the cost based one of mob spawns (the latter is compared with vanilla by kiln-entity's
//! `finalize_parity`).

use kiln_item::{ItemStack, keys, registry};
use kiln_javamath::random::LegacyRandom;
use kiln_loot::LootData;
use std::path::PathBuf;

fn datapack() -> Option<LootData> {
    let dir = std::env::var_os("KILN_DATAPACK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated"));
    if !dir.join("data/minecraft/enchantment_provider").is_dir() {
        eprintln!("no vanilla datapack (KILN_DATAPACK or work/generated): skipped");
        return None;
    }
    Some(LootData::load(&dir).expect("datapack loads"))
}

fn level_of(stack: &ItemStack, enchantment: &str) -> i32 {
    let id = registry::ENCHANTMENT.id(enchantment).unwrap();
    stack.get(keys::ENCHANTMENTS).map_or(0, |e| e.level(id))
}

#[test]
fn single_providers_apply_their_enchantment_without_drawing() {
    let Some(loot) = datapack() else { return };
    let mut rng = LegacyRandom::new(5);
    let before = rng.clone();
    for (provider, item, enchantment, level) in [
        ("minecraft:raid/pillager_post_wave_3", "minecraft:crossbow", "minecraft:quick_charge", 1),
        ("minecraft:raid/pillager_post_wave_5", "minecraft:crossbow", "minecraft:quick_charge", 2),
        ("minecraft:raid/vindicator", "minecraft:iron_axe", "minecraft:sharpness", 1),
        ("minecraft:raid/vindicator_post_wave_5", "minecraft:iron_axe", "minecraft:sharpness", 2),
        ("minecraft:pillager_spawn_crossbow", "minecraft:crossbow", "minecraft:piercing", 1),
        ("minecraft:enderman_loot_drop", "minecraft:diamond_axe", "minecraft:silk_touch", 1),
    ] {
        let mut stack = ItemStack::of(item, 1).unwrap();
        loot.enchant_from_provider(provider, &mut stack, 1.0, &mut rng);
        assert_eq!(level_of(&stack, enchantment), level, "{provider} on {item}");
    }
    assert_eq!(rng.state(), before.state(), "a single enchantment draws nothing");
    // An unknown provider changes nothing.
    let mut stack = ItemStack::of("minecraft:crossbow", 1).unwrap();
    loot.enchant_from_provider("minecraft:nonexistent", &mut stack, 1.0, &mut rng);
    assert!(stack.get(keys::ENCHANTMENTS).is_none_or(|e| e.is_empty()));
}

#[test]
fn mob_spawn_equipment_enchants_by_cost() {
    let Some(loot) = datapack() else { return };
    let mut enchanted = 0;
    for seed in 0..200 {
        let mut stack = ItemStack::of("minecraft:iron_sword", 1).unwrap();
        let mut rng = LegacyRandom::new(seed);
        loot.enchant_from_provider("minecraft:mob_spawn_equipment", &mut stack, 1.0, &mut rng);
        if stack.get(keys::ENCHANTMENTS).is_some_and(|e| !e.is_empty()) {
            enchanted += 1;
        }
    }
    assert!(enchanted > 100, "{enchanted} of 200 iron swords were enchanted at cost 5 to 22");
}
