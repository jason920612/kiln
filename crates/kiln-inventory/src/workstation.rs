//! Workstations without block entities that combine or strip items: the grindstone
//! (`GrindstoneMenu`) and the anvil (`AnvilMenu`, an `ItemCombinerMenu`). Enchantment
//! definitions and translations come through the menu's [`crate::World`].

use crate::menu::{Env, Menu};
use crate::slot::Source;
use crate::stack::{StackExt, matches};
use kiln_item::component::{Enchantments, ids};
use kiln_item::{HolderSet, ItemStack, Text, keys};

/// `AnvilMenu.MAX_NAME_LENGTH`.
pub const MAX_NAME_LENGTH: usize = 50;

fn is(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && stack.item_name() == name
}

/// `EnchantmentHelper.getComponentType`: stored enchantments on an enchanted book.
fn enchantments_key(stack: &ItemStack) -> kiln_item::Key<Enchantments> {
    if is(stack, "minecraft:enchanted_book") { keys::STORED_ENCHANTMENTS } else { keys::ENCHANTMENTS }
}

/// `EnchantmentHelper.getEnchantmentsForCrafting`.
fn crafting_enchantments(stack: &ItemStack) -> Enchantments {
    stack.get(enchantments_key(stack)).cloned().unwrap_or_default()
}

/// `EnchantmentHelper.hasAnyEnchantments`.
pub fn has_any_enchantments(stack: &ItemStack) -> bool {
    stack.get(keys::ENCHANTMENTS).is_some_and(|e| !e.is_empty()) || stack.get(keys::STORED_ENCHANTMENTS).is_some_and(|e| !e.is_empty())
}

/// `EnchantmentHelper.updateEnchantments`: edits the enchantments component the stack has.
fn update_enchantments(stack: &mut ItemStack, edit: impl FnOnce(&mut Enchantments)) -> Enchantments {
    let key = enchantments_key(stack);
    let Some(current) = stack.get(key) else { return Enchantments::default() };
    let mut e = current.clone();
    edit(&mut e);
    stack.insert(key, e.clone());
    e
}

/// `Enchantment` curses (`#minecraft:curse`).
fn is_curse(enchantment: i32) -> bool {
    crate::tags::contains("minecraft:enchantment", "minecraft:curse", enchantment)
}

/// `AnvilMenu.calculateIncreasedRepairCost`.
pub fn increased_repair_cost(cost: i32) -> i32 {
    (cost as i64 * 2 + 1).min(i32::MAX as i64) as i32
}

fn repair_cost(stack: &ItemStack) -> i32 {
    stack.get(keys::REPAIR_COST).copied().unwrap_or(0)
}

/// `ItemStack.setDamageValue`: clamped to the maximum.
fn set_damage(stack: &mut ItemStack, damage: i32) {
    let max = stack.max_damage();
    stack.insert(keys::DAMAGE, damage.clamp(0, max.max(0)));
}

fn damage_value(stack: &ItemStack) -> i32 {
    stack.get(keys::DAMAGE).copied().unwrap_or(0)
}

// ---- grindstone ---------------------------------------------------------------------------

/// `GrindstoneMenu.removeNonCursesFrom`: only curses stay; an enchanted book left with none
/// becomes a book; the repair cost follows the curses left.
fn remove_non_curses(mut stack: ItemStack) -> ItemStack {
    let left = update_enchantments(&mut stack, |e| e.0.retain(|(id, _)| is_curse(*id)));
    if is(&stack, "minecraft:enchanted_book") && left.is_empty() {
        let book = kiln_item::registry::ITEM.id("minecraft:book").unwrap_or(0);
        stack = ItemStack::from_parts(book, stack.count(), stack.patch().clone());
    }
    let mut cost = 0;
    for _ in 0..left.0.len() {
        cost = increased_repair_cost(cost);
    }
    stack.insert(keys::REPAIR_COST, cost);
    stack
}

/// `GrindstoneMenu.mergeEnchantsFrom`: the curses of `from` join (keeping higher levels).
fn merge_enchants_from(into: &mut ItemStack, from: &ItemStack) {
    let theirs = crafting_enchantments(from);
    update_enchantments(into, |e| {
        for &(id, level) in &theirs.0 {
            if is_curse(id) && e.level(id) == 0 {
                e.set(id, level);
            }
        }
    });
}

/// `GrindstoneMenu.mergeItems`: two of the same item repair each other (with a 5% bonus) and
/// keep the first one's enchantments plus the second one's curses.
fn grindstone_merge(a: &ItemStack, b: &ItemStack) -> ItemStack {
    if a.effective_item() != b.effective_item() {
        return ItemStack::empty();
    }
    let max = a.max_damage().max(b.max_damage());
    let durability_a = a.max_damage() - damage_value(a);
    let durability_b = b.max_damage() - damage_value(b);
    let sum = durability_a + durability_b + max * 5 / 100;
    let mut count = 1;
    if !a.is_damageable_item() {
        if a.max_stack_size() < 2 || !matches(a, b) {
            return ItemStack::empty();
        }
        count = 2;
    }
    let mut out = a.copy_with_count(count);
    if out.is_damageable_item() {
        out.insert(keys::MAX_DAMAGE, max);
        set_damage(&mut out, (max - sum).max(0));
    }
    merge_enchants_from(&mut out, b);
    remove_non_curses(out)
}

/// `GrindstoneMenu.computeResult`.
pub(crate) fn grindstone_result(a: &ItemStack, b: &ItemStack) -> ItemStack {
    if a.is_empty() && b.is_empty() {
        return ItemStack::empty();
    }
    if a.count() > 1 || b.count() > 1 {
        return ItemStack::empty();
    }
    if a.is_empty() || b.is_empty() {
        let one = if a.is_empty() { b } else { a };
        if !has_any_enchantments(one) {
            return ItemStack::empty();
        }
        return remove_non_curses(one.copy());
    }
    grindstone_merge(a, b)
}

/// `GrindstoneMenu.createResult` (its `slotsChanged` on the input slots).
pub(crate) fn grindstone_slots_changed(menu: &mut Menu, env: &mut Env, source: Source) {
    menu.broadcast_changes(env);
    if source != Source::Input {
        return;
    }
    let out = grindstone_result(&menu.input.items[0], &menu.input.items[1]);
    menu.result.item = out;
    menu.broadcast_changes(env);
}

/// The grindstone result slot's `getExperienceFromItem`: the minimum costs of the enchantments
/// that are not curses.
fn experience_from_item(env: &Env, stack: &ItemStack) -> i32 {
    crafting_enchantments(stack).0.iter().filter(|(id, _)| !is_curse(*id)).map(|&(id, level)| env.world.enchantment_min_cost(id, level)).sum()
}

/// The grindstone result slot's `onTake`: experience for the removed enchantments (half, plus a
/// random part), the grindstone sound, and both inputs used up.
pub(crate) fn grindstone_take(menu: &mut Menu, env: &mut Env) {
    let xp = experience_from_item(env, &menu.input.items[0]) + experience_from_item(env, &menu.input.items[1]);
    let amount = if xp > 0 {
        let half = (xp as f64 / 2.0).ceil() as i32;
        half + env.world.random_int(half)
    } else {
        0
    };
    env.out.push(crate::Effect::GrindstoneUsed { experience: amount });
    for k in 0..2 {
        crate::Container::set_item(&mut menu.input, k, ItemStack::empty());
        menu.slots_changed(env, Source::Input);
    }
}

// ---- anvil --------------------------------------------------------------------------------

/// `Repairable.isValidRepairItem`.
fn valid_repair_item(stack: &ItemStack, repair: &ItemStack) -> bool {
    let Some(r) = stack.get(keys::REPAIRABLE) else { return false };
    let item = repair.effective_item();
    match &r.items {
        HolderSet::Direct(ids) => ids.contains(&item),
        HolderSet::Tag(tag) => crate::tags::contains("minecraft:item", tag.as_str(), item),
    }
}

/// The plain text of a text component (`Component.getString`), translations resolved with
/// `translate`.
fn text_string(t: &kiln_proto::nbt::Tag, translate: &dyn Fn(&str) -> Option<String>) -> String {
    use kiln_proto::nbt::Tag;
    match t {
        Tag::String(s) => s.clone(),
        Tag::List(items) => items.iter().map(|i| text_string(i, translate)).collect(),
        Tag::Compound(_) => {
            let mut out = String::new();
            if let Some(s) = t.get("text").and_then(Tag::as_str) {
                out.push_str(s);
            } else if let Some(key) = t.get("translate").and_then(Tag::as_str) {
                let args: Vec<String> = t.get("with").and_then(Tag::as_list).unwrap_or(&[]).iter().map(|a| text_string(a, translate)).collect();
                let pattern = translate(key).or_else(|| t.get("fallback").and_then(Tag::as_str).map(str::to_owned)).unwrap_or_else(|| key.to_owned());
                out.push_str(&format_translation(&pattern, &args));
            }
            if let Some(extra) = t.get("extra").and_then(Tag::as_list) {
                for e in extra {
                    out.push_str(&text_string(e, translate));
                }
            }
            out
        }
        other => format!("{other:?}"),
    }
}

/// `TranslatableContents` formatting: `%s`, `%1$s` and `%%`.
fn format_translation(pattern: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some('s') => {
                chars.next();
                out.push_str(args.get(next).map_or("", String::as_str));
                next += 1;
            }
            Some(d) if d.is_ascii_digit() => {
                let mut n = 0usize;
                while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                    n = n * 10 + d.to_digit(10).unwrap() as usize;
                    chars.next();
                }
                if chars.peek() == Some(&'$') {
                    chars.next();
                }
                if chars.peek() == Some(&'s') {
                    chars.next();
                }
                out.push_str(args.get(n.saturating_sub(1)).map_or("", String::as_str));
            }
            _ => out.push('%'),
        }
    }
    out
}

/// `ItemStack.getHoverName().getString()`: the custom name, else the item name.
pub fn hover_name(stack: &ItemStack, translate: &dyn Fn(&str) -> Option<String>) -> String {
    let name: Option<&Text> = stack.get(keys::CUSTOM_NAME).or_else(|| stack.get(keys::ITEM_NAME));
    name.map_or_else(String::new, |t| text_string(t.nbt(), translate))
}

/// `StringUtil.isBlank`.
fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `AnvilMenu.createResult`.
pub(crate) fn anvil_create_result(menu: &mut Menu, env: &mut Env) {
    let input = menu.input.items[0].clone();
    menu.anvil.only_renaming = false;
    menu.local_data[0] = 1;
    let mut cost = 0i32;
    let mut base: i64 = 0;
    let mut rename = 0i32;
    let key_ok = !input.is_empty() && input.get(enchantments_key(&input)).is_some();
    if !key_ok {
        menu.result.item = ItemStack::empty();
        menu.local_data[0] = 0;
        return;
    }
    let mut result = input.copy();
    let addition = menu.input.items[1].clone();
    let mut enchants = crafting_enchantments(&result);
    base += repair_cost(&input) as i64 + repair_cost(&addition) as i64;
    menu.anvil.repair_item_count_cost = 0;
    let fail = |menu: &mut Menu| {
        menu.result.item = ItemStack::empty();
        menu.local_data[0] = 0;
    };
    if !addition.is_empty() {
        let is_book = addition.has(ids::STORED_ENCHANTMENTS);
        if result.is_damageable_item() && valid_repair_item(&input, &addition) {
            let mut step = damage_value(&result).min(result.max_damage() / 4);
            if step <= 0 {
                fail(menu);
                return;
            }
            let mut used = 0;
            while step > 0 && used < addition.count() {
                let d = damage_value(&result) - step;
                set_damage(&mut result, d);
                cost += 1;
                step = damage_value(&result).min(result.max_damage() / 4);
                used += 1;
            }
            menu.anvil.repair_item_count_cost = used;
        } else {
            if !is_book && (result.effective_item() != addition.effective_item() || !result.is_damageable_item()) {
                fail(menu);
                return;
            }
            if result.is_damageable_item() && !is_book {
                let left = input.max_damage() - damage_value(&input);
                let right = addition.max_damage() - damage_value(&addition);
                let bonus = right + result.max_damage() * 12 / 100;
                let total = left + bonus;
                let damage = (result.max_damage() - total).max(0);
                if damage < damage_value(&result) {
                    set_damage(&mut result, damage);
                    cost += 2;
                }
            }
            let theirs = crafting_enchantments(&addition);
            let (mut any_ok, mut any_bad) = (false, false);
            for &(id, level) in &theirs.0 {
                let mine = enchants.level(id);
                let mut level = if mine == level { level + 1 } else { level.max(mine) };
                let mut ok = env.world.enchantment_can_enchant(id, &input);
                if env.player.infinite_materials || is(&input, "minecraft:enchanted_book") {
                    ok = true;
                }
                for &(other, _) in &enchants.0 {
                    if other != id && !env.world.enchantments_compatible(id, other) {
                        ok = false;
                        cost += 1;
                    }
                }
                if !ok {
                    any_bad = true;
                    continue;
                }
                any_ok = true;
                let max = env.world.enchantment_max_level(id);
                if level > max {
                    level = max;
                }
                enchants.set(id, level);
                let mut anvil_cost = env.world.enchantment_anvil_cost(id);
                if is_book {
                    anvil_cost = (anvil_cost / 2).max(1);
                }
                cost += anvil_cost * level;
                if input.count() > 1 {
                    cost = 40;
                }
            }
            if any_bad && !any_ok {
                fail(menu);
                return;
            }
        }
    }
    let translate = |k: &str| env.world.translate(k);
    match menu.anvil.item_name.as_deref() {
        Some(name) if !is_blank(name) => {
            if name != hover_name(&input, &translate) {
                rename = 1;
                cost += rename;
                result.insert(keys::CUSTOM_NAME, Text::literal(name));
            }
        }
        _ => {
            if input.has(ids::CUSTOM_NAME) {
                rename = 1;
                cost += rename;
                result.remove(ids::CUSTOM_NAME);
            }
        }
    }
    let total = if cost <= 0 { 0 } else { (base + cost as i64).clamp(0, i32::MAX as i64) as i32 };
    menu.local_data[0] = total;
    if cost <= 0 {
        result = ItemStack::empty();
    }
    if rename == cost && rename > 0 {
        if menu.local_data[0] >= 40 {
            menu.local_data[0] = 39;
        }
        menu.anvil.only_renaming = true;
    }
    if menu.local_data[0] >= 40 && !env.player.infinite_materials {
        result = ItemStack::empty();
    }
    if !result.is_empty() {
        let mut rc = repair_cost(&result);
        if rc < repair_cost(&addition) {
            rc = repair_cost(&addition);
        }
        if rename != cost || rename == 0 {
            rc = increased_repair_cost(rc);
        }
        result.insert(keys::REPAIR_COST, rc);
        result.insert(enchantments_key(&result), enchants);
    }
    menu.result.item = result;
    menu.broadcast_changes(env);
}

/// `ItemCombinerMenu.slotsChanged` for the anvil.
pub(crate) fn anvil_slots_changed(menu: &mut Menu, env: &mut Env, source: Source) {
    menu.broadcast_changes(env);
    if source == Source::Input {
        anvil_create_result(menu, env);
    }
}

/// `AnvilMenu.mayPickup`: enough levels (or infinite materials) for a positive cost.
pub(crate) fn anvil_may_pickup(menu: &Menu, env: &Env) -> bool {
    let cost = menu.local_data[0];
    (env.player.infinite_materials || env.player.xp_level >= cost) && cost > 0
}

/// `AnvilMenu.onTake`: the levels, the used inputs, and the anvil's wear.
pub(crate) fn anvil_take(menu: &mut Menu, env: &mut Env) {
    let levels = if env.player.infinite_materials { 0 } else { menu.local_data[0] };
    if menu.anvil.repair_item_count_cost > 0 {
        let mut addition = menu.input.items[1].clone();
        if !addition.is_empty() && addition.count() > menu.anvil.repair_item_count_cost {
            addition.shrink_count(menu.anvil.repair_item_count_cost);
            crate::Container::set_item(&mut menu.input, 1, addition);
        } else {
            crate::Container::set_item(&mut menu.input, 1, ItemStack::empty());
        }
        menu.slots_changed(env, Source::Input);
    } else if !menu.anvil.only_renaming {
        crate::Container::set_item(&mut menu.input, 1, ItemStack::empty());
        menu.slots_changed(env, Source::Input);
    }
    menu.local_data[0] = 0;
    crate::Container::set_item(&mut menu.input, 0, ItemStack::empty());
    menu.slots_changed(env, Source::Input);
    env.out.push(crate::Effect::AnvilUsed { levels });
}

/// `AnvilMenu.setItemName` (`ServerboundRenameItemPacket`): a valid new name renames the result
/// and recomputes it.
pub fn anvil_set_item_name(menu: &mut Menu, env: &mut Env, name: &str) -> bool {
    // `StringUtil.filterText`: control characters and the section sign go.
    let filtered: String = name.chars().filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}').collect();
    if filtered.chars().count() > MAX_NAME_LENGTH || menu.anvil.item_name.as_deref() == Some(filtered.as_str()) {
        return false;
    }
    menu.anvil.item_name = Some(filtered.clone());
    if !menu.result.item.is_empty() {
        if is_blank(&filtered) {
            menu.result.item.remove(ids::CUSTOM_NAME);
        } else {
            menu.result.item.insert(keys::CUSTOM_NAME, Text::literal(filtered));
        }
    }
    anvil_create_result(menu, env);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translations_format_their_arguments() {
        assert_eq!(format_translation("%s of %s", &["A".into(), "B".into()]), "A of B");
        assert_eq!(format_translation("%2$s %1$s 100%%", &["a".into(), "b".into()]), "b a 100%");
        assert_eq!(increased_repair_cost(0), 1);
        assert_eq!(increased_repair_cost(3), 7);
    }

    #[test]
    fn grindstones_strip_enchantments_but_keep_curses() {
        let mut sword = ItemStack::of("minecraft:iron_sword", 1).unwrap();
        let sharpness = kiln_item::registry::ENCHANTMENT.id("minecraft:sharpness").unwrap();
        let vanishing = kiln_item::registry::ENCHANTMENT.id("minecraft:vanishing_curse").unwrap();
        sword.insert(keys::ENCHANTMENTS, Enchantments(vec![(sharpness, 3), (vanishing, 1)]));
        sword.insert(keys::REPAIR_COST, 7);
        let out = grindstone_result(&sword, &ItemStack::empty());
        assert_eq!(out.get(keys::ENCHANTMENTS).unwrap().0, vec![(vanishing, 1)]);
        assert_eq!(out.get(keys::REPAIR_COST), Some(&1));
        // Plain items do not go in alone.
        assert!(grindstone_result(&ItemStack::of("minecraft:iron_sword", 1).unwrap(), &ItemStack::empty()).is_empty());
        // Two damaged swords repair each other with a 5% bonus.
        let mut a = ItemStack::of("minecraft:iron_sword", 1).unwrap();
        let mut b = a.clone();
        set_damage(&mut a, 200);
        set_damage(&mut b, 200);
        let merged = grindstone_result(&a, &b);
        let max = a.max_damage();
        assert_eq!(damage_value(&merged), (max - ((max - 200) * 2 + max * 5 / 100)).max(0));
    }
}
