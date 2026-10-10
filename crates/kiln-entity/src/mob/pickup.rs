//! `Mob.aiStep`'s pick up of items (`canPickUpLoot`, reach 1 x 0 x 1), `Mob.wantsToPickUp`, `pickUpItem`
//! and `equipItemIfPossible` for the types that have no `pickUpItem` of their own: zombies, skeletons, wolves and whatever
//! else a command, a dispenser or a spawner sets `CanPickUpLoot` on. The types with their own (piglins, foxes, pandas,
//! dolphins, allays, villagers, raiders, sulfur cubes) keep theirs in `kinds`.

use super::dispense::{self, Facts};
use super::{MAINHAND, MobData, MobKind, kinds};
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_item::component::EquipmentSlot;

/// `EquipmentSlot.BODY` as `dispense::equip` numbers it (the six `Mob` slots come first, then the body, then the saddle).
const BODY: usize = 6;

/// The types whose `pickUpItem` (or `aiStep` pickup) is their own.
pub fn has_own_pickup(kind: MobKind) -> bool {
    use MobKind::*;
    matches!(
        kind,
        Fox | Panda | Dolphin | Allay | Piglin | PiglinBrute | Villager | SulfurCube | Pillager | Vindicator | Evoker | Ravager | Witch | Illusioner
    )
}

fn is_zombie(kind: MobKind) -> bool {
    use MobKind::*;
    matches!(kind, Zombie | Husk | Drowned | ZombieVillager | ZombifiedPiglin)
}

fn is_skeleton(kind: MobKind) -> bool {
    use MobKind::*;
    matches!(kind, Skeleton | Stray | Bogged | WitherSkeleton | Parched)
}

fn tag(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && super::item_tag(stack.item(), name)
}

/// `Mob.canHoldItem` with the types' overrides (`Zombie`: no egg for a baby on a mount; `WitherSkeleton`: not the weapons it
/// dislikes).
fn can_hold_item(e: &Entity, m: &MobData, stack: &ItemStack) -> bool {
    if is_zombie(m.kind) && tag(stack, "minecraft:eggs") && m.baby() && e.vehicle.is_some() {
        return false;
    }
    if m.kind == MobKind::WitherSkeleton && tag(stack, "minecraft:wither_skeleton_disliked_weapons") {
        return false;
    }
    true
}

/// `Mob.wantsToPickUp` with the types' overrides.
fn wants_to_pick_up(e: &Entity, m: &MobData, stack: &ItemStack) -> bool {
    use MobKind::*;
    match m.kind {
        // `ZombifiedPiglin.wantsToPickUp`: whatever it can hold.
        ZombifiedPiglin => can_hold_item(e, m, stack),
        // `Drowned.wantsToPickUp` over `Zombie.wantsToPickUp`.
        Drowned if tag(stack, "minecraft:spears") => false,
        k if is_zombie(k) && stack.item_name() == "minecraft:glow_ink_sac" => false,
        k if is_skeleton(k) && tag(stack, "minecraft:spears") => false,
        _ => can_hold_item(e, m, stack),
    }
}

/// `getPreferredWeaponType` of the types that have one.
fn preferred_weapons(m: &MobData) -> Option<&'static str> {
    use MobKind::*;
    match m.kind {
        Drowned => Some("minecraft:drowned_preferred_weapons"),
        WitherSkeleton => None,
        k if is_skeleton(k) => Some("minecraft:skeleton_preferred_weapons"),
        _ => None,
    }
}

/// `EquipmentSlot.isArmor`: the four armor slots and the body slot.
fn is_armor(slot: usize) -> bool {
    (super::FEET..=super::HEAD).contains(&slot) || slot == BODY
}

fn slot_enum(i: usize) -> EquipmentSlot {
    match i {
        0 => EquipmentSlot::MainHand,
        1 => EquipmentSlot::OffHand,
        2 => EquipmentSlot::Feet,
        3 => EquipmentSlot::Legs,
        4 => EquipmentSlot::Chest,
        5 => EquipmentSlot::Head,
        BODY => EquipmentSlot::Body,
        _ => EquipmentSlot::Saddle,
    }
}

fn item_in(m: &MobData, slot: usize) -> ItemStack {
    if slot < 6 {
        return m.equipment[slot].clone();
    }
    m.kind.ext().and_then(|k| k.extra_equipment(m).into_iter().find(|(s, _)| *s as usize == slot).map(|(_, s)| s)).unwrap_or_else(ItemStack::empty)
}

/// `Mob.getDropChances().byEquipment(slot)`.
fn drop_chance(m: &MobData, slot: usize) -> f32 {
    if slot < 6 {
        return m.drop_chances[slot];
    }
    m.kind.ext().map_or(0.085, |k| k.extra_drop_chance(m, slot as u8))
}

/// `LivingEntity.getEquipmentSlotForItem`: the item's slot if the type can use it, else the main hand.
fn slot_for(f: &Facts, stack: &ItemStack) -> usize {
    match stack.get(kiln_item::keys::EQUIPPABLE) {
        Some(q) if dispense::can_use_slot(f, q.slot) => q.slot as usize,
        _ => MAINHAND,
    }
}

/// `LivingEntity.isEquippableInSlot`.
fn is_equippable_in_slot(f: &Facts, entity_type: &str, stack: &ItemStack, slot: usize) -> bool {
    match stack.get(kiln_item::keys::EQUIPPABLE) {
        None => slot == MAINHAND && dispense::can_use_slot(f, EquipmentSlot::MainHand),
        Some(q) => q.slot as usize == slot && dispense::can_use_slot(f, q.slot) && kinds::horse::equippable_in_slot(stack, q.slot, entity_type),
    }
}

/// `Mob.canReplaceEqualItem`.
fn can_replace_equal_item(candidate: &ItemStack, current: &ItemStack) -> bool {
    kinds::piglin::can_replace_equal_item(candidate, current)
}

/// `Mob.canReplaceCurrentItem(candidate, current, slot)`: armor by armor value and toughness, weapons by the type's preferred
/// kind and attack damage; nothing else is replaced.
fn can_replace_current_item(m: &MobData, candidate: &ItemStack, current: &ItemStack, slot: usize) -> bool {
    use super::attributes::Attr::{Armor, ArmorToughness, AttackDamage};
    if current.is_empty() {
        return true;
    }
    let value = |s: &ItemStack, attr| kinds::piglin::approximate_attribute(m, s, attr, slot);
    if is_armor(slot) {
        // `compareArmor`: a curse of binding keeps what is worn.
        let binding = kiln_item::registry::ENCHANTMENT.id("minecraft:binding_curse").is_some_and(|id| current.get(kiln_item::keys::ENCHANTMENTS).is_some_and(|e| e.level(id) > 0));
        if binding {
            return false;
        }
        let (a, b) = (value(candidate, Armor), value(current, Armor));
        if a != b {
            return a > b;
        }
        let (a, b) = (value(candidate, ArmorToughness), value(current, ArmorToughness));
        if a != b {
            return a > b;
        }
        return can_replace_equal_item(candidate, current);
    }
    if slot == MAINHAND {
        // `compareWeapons`.
        if let Some(t) = preferred_weapons(m) {
            let (cur_is, new_is) = (tag(current, t), tag(candidate, t));
            if cur_is && !new_is {
                return false;
            }
            if !cur_is && new_is {
                return true;
            }
        }
        let (a, b) = (value(candidate, AttackDamage), value(current, AttackDamage));
        if a != b {
            return a > b;
        }
        return can_replace_equal_item(candidate, current);
    }
    false
}

/// `Mob.equipItemIfPossible`: the part of `stack` that was put on (empty: nothing).
pub fn equip_item_if_possible(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, stack: ItemStack) -> ItemStack {
    let f = dispense::facts_of(m);
    let mut slot = slot_for(&f, &stack);
    if !is_equippable_in_slot(&f, e.type_name, &stack, slot) {
        return ItemStack::empty();
    }
    let mut current = item_in(m, slot);
    let mut replace = can_replace_current_item(m, &stack, &current, slot);
    // An armor piece that is no better goes to the hand when that is empty.
    if is_armor(slot) && !replace {
        slot = MAINHAND;
        current = item_in(m, slot);
        replace = current.is_empty();
    }
    if !(replace && can_hold_item(e, m, &stack)) {
        return ItemStack::empty();
    }
    let chance = drop_chance(m, slot) as f64;
    if !current.is_empty() && ((e.random.next_float() - 0.1f32).max(0.0) as f64) < chance {
        super::spawn_at_location(e, level, current.clone());
    }
    // `EquipmentSlot.limit`: one piece of armor, a whole stack in a hand.
    let limited = if slot == MAINHAND || slot == 1 { stack } else { stack.with_count(1) };
    // `setItemSlotAndDropWhenKilled`.
    if slot < 6 {
        m.equipment[slot] = limited.clone();
        m.drop_chances[slot] = 2.0;
        super::sync_equipment_modifiers(m);
    } else if let Some(k) = m.kind.ext() {
        k.set_extra_equipment(m, slot as u8, limited.clone());
    }
    m.persistence_required = true;
    // `LivingEntity.onEquipItem`: the equip sound of armor (its seed comes off the mob's random).
    kinds::horse::equip_sound(e, level, slot_enum(slot), &current, &limited);
    limited
}

/// `Mob.aiStep`'s loop over the item entities within reach (1 x 0 x 1): each one the mob wants is picked up.
pub fn ai_step(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if std::env::var_os("KILN_PICKUP_DEBUG").is_some() {
        let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
        eprintln!("PDBG {:?} alive={} dead={} griefing={} items={:?} bb={:?}", m.kind, super::is_alive(e, m), m.dead, level.mob_griefing(), level.entities_in(&area, EntityFilter::Item, e.id), area);
    }
    if !m.can_pick_up_loot || !super::is_alive(e, m) || m.dead || !level.mob_griefing() {
        return;
    }
    let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
    for id in level.entities_in(&area, EntityFilter::Item, e.id) {
        let Some(item) = level.entity(id) else { continue };
        let EntityKind::Item(d) = &item.kind else { continue };
        if item.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 || !wants_to_pick_up(e, m, &d.stack) {
            continue;
        }
        // `Mob.pickUpItem`.
        let (whole, thrower) = (d.stack.clone(), d.thrower);
        let taken = equip_item_if_possible(e, m, level, whole.clone());
        if taken.is_empty() {
            continue;
        }
        super::on_item_pickup(e, m, level, thrower, &whole);
        let Some(item) = level.entity_mut(id) else { continue };
        let EntityKind::Item(d) = &mut item.kind else { continue };
        d.stack.shrink(taken.count());
        if d.stack.is_empty() {
            item.discard();
        }
    }
}
