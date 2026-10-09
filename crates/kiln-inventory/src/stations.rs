//! The loom (`LoomMenu`), the cartography table (`CartographyTableMenu`) and the enchanting
//! table (`EnchantmentMenu`).

use crate::menu::{Env, Menu};
use crate::slot::Source;
use crate::stack::{StackExt, matches};
use kiln_item::component::{BannerLayer, BannerPatternLayers, ids};
use kiln_item::{HolderSet, ItemStack, keys};
use kiln_javamath::random::RandomSource;

fn is(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && stack.item_name() == name
}

// ---- loom ---------------------------------------------------------------------------------

/// `BannerItem`: the sixteen banners.
pub fn is_banner(stack: &ItemStack) -> bool {
    !stack.is_empty() && stack.item_name().ends_with("_banner")
}

/// `LoomMenu.isDyeItem`: `#loom_dyes` with a dye color.
pub fn is_loom_dye(stack: &ItemStack) -> bool {
    !stack.is_empty() && crate::tags::contains("minecraft:item", "minecraft:loom_dyes", stack.item()) && stack.has(ids::DYE)
}

/// `LoomMenu.isPatternItem`: `#loom_patterns` with the patterns it provides.
pub fn is_loom_pattern(stack: &ItemStack) -> bool {
    !stack.is_empty() && crate::tags::contains("minecraft:item", "minecraft:loom_patterns", stack.item()) && stack.has(ids::PROVIDES_BANNER_PATTERNS)
}

/// `LoomMenu.getSelectablePatterns`: without a pattern item, `#no_item_required`; else what the
/// item provides (`minecraft:banner_pattern` ids, in the set's order).
fn selectable_patterns(pattern_item: &ItemStack) -> Vec<i32> {
    if pattern_item.is_empty() {
        return crate::tags::entries("minecraft:banner_pattern", "minecraft:no_item_required").map_or(Vec::new(), <[i32]>::to_vec);
    }
    match pattern_item.get(keys::PROVIDES_BANNER_PATTERNS).map(|p| &p.0) {
        Some(HolderSet::Direct(ids)) => ids.clone(),
        Some(HolderSet::Tag(tag)) => crate::tags::entries("minecraft:banner_pattern", tag.as_str()).map_or(Vec::new(), <[i32]>::to_vec),
        None => Vec::new(),
    }
}

/// `LoomMenu.setupResultSlot`: the banner with one more layer of the pattern in the dye's
/// color.
fn loom_setup_result(menu: &mut Menu, env: &mut Env, pattern: i32) {
    let banner = menu.input.items[0].clone();
    let dye = menu.input.items[1].clone();
    let mut out = ItemStack::empty();
    if !banner.is_empty()
        && !dye.is_empty()
        && let Some(color) = dye.get(keys::DYE).copied()
    {
        out = banner.copy_with_count(1);
        let mut layers = out.get(keys::BANNER_PATTERNS).cloned().unwrap_or_default();
        layers.0.push(BannerLayer { pattern: kiln_item::Holder::Reference(pattern), color });
        out.insert(keys::BANNER_PATTERNS, layers);
    }
    if !matches(&out, &menu.result.item) {
        menu.set_slot(env, 3, out);
    }
}

/// `LoomMenu.slotsChanged`.
pub(crate) fn loom_slots_changed(menu: &mut Menu, env: &mut Env) {
    if std::env::var_os("LOOM_DBG").is_some() { eprintln!("slots_changed: in {:?} idx {}", menu.input.items.iter().map(|s| s.item_name().to_string()).collect::<Vec<_>>(), menu.local_data[0]); }
    let banner = menu.input.items[0].clone();
    let dye = menu.input.items[1].clone();
    let pattern_item = menu.input.items[2].clone();
    if banner.is_empty() || dye.is_empty() {
        menu.set_slot(env, 3, ItemStack::empty());
        menu.visible_patterns.clear();
        menu.local_data[0] = -1;
        return;
    }
    let index = menu.local_data[0];
    let valid = usize::try_from(index).is_ok_and(|i| i < menu.visible_patterns.len());
    let old = std::mem::take(&mut menu.visible_patterns);
    menu.visible_patterns = selectable_patterns(&pattern_item);
    let chosen = if menu.visible_patterns.len() == 1 {
        menu.local_data[0] = 0;
        Some(menu.visible_patterns[0])
    } else if !valid {
        menu.local_data[0] = -1;
        None
    } else {
        let previous = old[index as usize];
        match menu.visible_patterns.iter().position(|&p| p == previous) {
            Some(i) => {
                menu.local_data[0] = i as i32;
                Some(previous)
            }
            None => {
                menu.local_data[0] = -1;
                None
            }
        }
    };
    match chosen {
        Some(pattern) => {
            let full = banner.get(keys::BANNER_PATTERNS).map_or(0, |l: &BannerPatternLayers| l.0.len()) >= 6;
            if full {
                menu.local_data[0] = -1;
                menu.set_slot(env, 3, ItemStack::empty());
            } else {
                loom_setup_result(menu, env, pattern);
            }
        }
        None => menu.set_slot(env, 3, ItemStack::empty()),
    }
    menu.broadcast_changes(env);
}

/// `LoomMenu.clickMenuButton`: selects a pattern.
pub(crate) fn loom_click(menu: &mut Menu, env: &mut Env, button: i32) -> bool {
    let Some(&pattern) = usize::try_from(button).ok().and_then(|i| menu.visible_patterns.get(i)) else { return false };
    menu.local_data[0] = button;
    loom_setup_result(menu, env, pattern);
    true
}

/// The loom result slot's `onTake`: one banner and one dye are used (the pattern item stays).
pub(crate) fn loom_take(menu: &mut Menu, env: &mut Env) {
    if std::env::var_os("LOOM_DBG").is_some() { eprintln!("loom_take"); }
    for slot in [0, 1] {
        let removed = crate::container::remove_item(&mut menu.input.items, slot, 1);
        if !removed.is_empty() {
            menu.slots_changed(env, Source::Input);
        }
    }
    if menu.input.items[0].is_empty() || menu.input.items[1].is_empty() {
        menu.local_data[0] = -1;
    }
    env.out.push(crate::Effect::LoomUsed);
}

// ---- cartography table --------------------------------------------------------------------

/// `CartographyTableMenu.slotsChanged`: a result without both inputs goes; with both, the result
/// is worked out from the map's saved data (`setupResultSlot`).
pub(crate) fn cartography_slots_changed(menu: &mut Menu, env: &mut Env) {
    let (map, additional) = (menu.input.items[0].clone(), menu.input.items[1].clone());
    let current = menu.result.item.clone();
    if !current.is_empty() && (map.is_empty() || additional.is_empty()) {
        menu.result.item = ItemStack::empty();
    } else if !map.is_empty() && !additional.is_empty() {
        cartography_setup_result(menu, env, &map, &additional, &current);
    }
}

/// `CartographyTableMenu.setupResultSlot`: paper zooms an unlocked map out, a glass pane locks it,
/// an empty map copies it.
fn cartography_setup_result(menu: &mut Menu, env: &mut Env, map: &ItemStack, additional: &ItemStack, current: &ItemStack) {
    let Some(id) = map.get(keys::MAP_ID).map(|m| m.0) else { return };
    let (Some(scale), Some(locked)) = (env.world.map_scale(id), env.world.map_locked(id)) else { return };
    let result;
    if is(additional, "minecraft:paper") && crate::tags::contains("minecraft:item", "minecraft:extendable_maps", map.item()) && !locked && scale < 4 {
        let mut r = map.copy_with_count(1);
        r.insert(keys::MAP_POST_PROCESSING, kiln_item::component::MapPostProcessing::Scale);
        result = r;
        menu.broadcast_changes(env);
    } else if is(additional, "minecraft:glass_pane") && !locked {
        let mut r = map.copy_with_count(1);
        r.insert(keys::MAP_POST_PROCESSING, kiln_item::component::MapPostProcessing::Lock);
        result = r;
        menu.broadcast_changes(env);
    } else if is(additional, "minecraft:map") {
        result = map.copy_with_count(2);
        menu.broadcast_changes(env);
    } else {
        menu.result.item = ItemStack::empty();
        menu.broadcast_changes(env);
        return;
    }
    if !matches(&result, current) {
        menu.result.item = result;
        menu.broadcast_changes(env);
    }
}

/// `CartographyTableMenu$5.onTake`: one map and one additional item are used, and the table sounds.
pub(crate) fn cartography_take(menu: &mut Menu, env: &mut Env) {
    for slot in [0, 1] {
        let removed = crate::container::remove_item(&mut menu.input.items, slot, 1);
        if !removed.is_empty() {
            menu.slots_changed(env, Source::Input);
        }
    }
    env.out.push(crate::Effect::CartographyUsed);
}

/// The cartography table's additional slot: paper, an empty map or a glass pane.
pub fn is_cartography_additional(stack: &ItemStack) -> bool {
    is(stack, "minecraft:paper") || is(stack, "minecraft:map") || is(stack, "minecraft:glass_pane")
}

// ---- enchanting table ---------------------------------------------------------------------

/// `EnchantmentHelper.getEnchantmentCost`.
fn enchantment_cost(rng: &mut dyn RandomSource, slot: i32, bookshelves: i32, stack: &ItemStack) -> i32 {
    if stack.get(keys::ENCHANTABLE).is_none() {
        return 0;
    }
    let b = bookshelves.min(15);
    let i = rng.next_int_bounded(8) + 1 + (b >> 1) + rng.next_int_bounded(b + 1);
    match slot {
        0 => (i / 3).max(1),
        1 => i * 2 / 3 + 1,
        _ => i.max(b * 2),
    }
}

/// `ItemStack.isEnchantable`: enchantable and not enchanted yet.
fn is_enchantable(stack: &ItemStack) -> bool {
    stack.has(ids::ENCHANTABLE) && stack.get(keys::ENCHANTMENTS).is_some_and(|e| e.is_empty())
}

/// `EnchantmentMenu.getEnchantmentList`: the enchantments option `slot` gives (from
/// `#in_enchanting_table`), drawn from the seed; a book loses one at random.
fn enchantment_list(menu: &mut Menu, env: &Env, stack: &ItemStack, slot: i32, cost: i32) -> Vec<(i32, i32)> {
    menu.enchant.rng = kiln_javamath::random::LegacyRandom::new((menu.enchant.seed as i64).wrapping_add(slot as i64));
    let mut list = env.world.select_enchantments(&mut menu.enchant.rng, stack, cost);
    if is(stack, "minecraft:book") && list.len() > 1 {
        let i = menu.enchant.rng.next_int_bounded(list.len() as i32) as usize;
        list.remove(i);
    }
    list
}

/// `EnchantmentMenu.slotsChanged`: costs and clues of the three options for the item.
pub(crate) fn enchantment_slots_changed(menu: &mut Menu, env: &mut Env) {
    let stack = menu.input.items[0].clone();
    if stack.is_empty() || !is_enchantable(&stack) {
        for i in 0..3 {
            menu.local_data[i] = 0;
            menu.local_data[4 + i] = -1;
            menu.local_data[7 + i] = -1;
        }
        return;
    }
    let bookshelves = env.world.enchanting_bookshelves();
    menu.enchant.rng = kiln_javamath::random::LegacyRandom::new(menu.enchant.seed as i64);
    for j in 0..3 {
        let mut cost = enchantment_cost(&mut menu.enchant.rng, j as i32, bookshelves, &stack);
        menu.local_data[4 + j] = -1;
        menu.local_data[7 + j] = -1;
        if cost < j as i32 + 1 {
            cost = 0;
        }
        menu.local_data[j] = cost;
    }
    for j in 0..3 {
        let cost = menu.local_data[j];
        if cost > 0 {
            let list = enchantment_list(menu, env, &stack, j as i32, cost);
            if !list.is_empty() {
                let (e, level) = list[menu.enchant.rng.next_int_bounded(list.len() as i32) as usize];
                menu.local_data[4 + j] = e;
                menu.local_data[7 + j] = level;
            }
        }
    }
    menu.broadcast_changes(env);
}

/// `EnchantmentMenu.clickMenuButton`: enchants the item with option `id` when the player can
/// pay its lapis and levels (infinite materials pay nothing).
pub(crate) fn enchantment_click(menu: &mut Menu, env: &mut Env, id: i32) -> bool {
    if !(0..3).contains(&id) {
        return false;
    }
    let item = menu.input.items[0].clone();
    let lapis = menu.input.items[1].clone();
    let need = id + 1;
    let infinite = env.player.infinite_materials;
    let cost = menu.local_data[id as usize];
    if (lapis.is_empty() || lapis.count() < need) && !infinite {
        return false;
    }
    if cost <= 0 || item.is_empty() || (env.player.xp_level < need || env.player.xp_level < cost) && !infinite {
        return false;
    }
    let list = enchantment_list(menu, env, &item, id, cost);
    if list.is_empty() {
        return true;
    }
    // `Player.onEnchantmentPerformed`: the levels go and the seed changes.
    let seed = env.world.next_player_int();
    env.out.push(crate::Effect::Enchanted { levels: need, seed });
    let mut result = item;
    if is(&result, "minecraft:book") {
        let enchanted = kiln_item::registry::ITEM.id("minecraft:enchanted_book").unwrap_or(0);
        result = ItemStack::from_parts(enchanted, result.count(), result.patch().clone());
        crate::Container::set_item(&mut menu.input, 0, result.clone());
    }
    for (e, level) in list {
        // `ItemStack.enchant`: `EnchantmentHelper.updateEnchantments` with `upgrade`.
        let key = if is(&result, "minecraft:enchanted_book") { keys::STORED_ENCHANTMENTS } else { keys::ENCHANTMENTS };
        let mut ench = result.get(key).cloned().unwrap_or_default();
        if level > 0 && ench.level(e) < level.min(255) {
            ench.set(e, level.min(255));
        }
        result.insert(key, ench);
    }
    menu.input.items[0] = result;
    // `ItemStack.consume`: nothing for infinite materials.
    let mut lapis = lapis;
    if !infinite {
        lapis.shrink_count(need);
    }
    menu.input.items[1] = lapis.clone();
    if lapis.is_empty() {
        crate::Container::set_item(&mut menu.input, 1, ItemStack::empty());
    }
    menu.enchant.seed = seed;
    enchantment_slots_changed(menu, env);
    true
}
