//! Piglin: admires gold, barters, hunts hoglins, avoids zombified piglins, celebrates kills,
//! rides baby hoglins, attacks players without gold armor, zombifies outside the nether.
//!
//! Driven by the brain of `PiglinAi` (core: looking, moving, doors, avoiding nemeses and
//! zombified piglins, admiring and celebrating triggers; idle: looking at players with gold,
//! attacking, hunting hoglins, strolling and looking about; admire item; fight: melee or
//! crossbow; celebrate; avoid; ride), on [`crate::mob::brain`]. `AbstractPiglin`'s zombification
//! and `Piglin`'s inventory, equipment and bartering are here too. Not ported: the golden
//! spear's `SpearApproach`/`SpearAttack`/`SpearRetreat` behaviours (Kiln has no kinetic weapons:
//! a piglin with a spear fights it as a melee weapon).

use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::attributes::Op;
use crate::mob::brain::behaviors::*;
use crate::mob::brain::combat::{
    back_up_if_too_close, melee_attack, set_walk_target_from_attack_target_if_out_of_reach, start_attacking, stop_attacking_if_target_invalid,
};
use crate::mob::brain::memory::{Memories, Val};
use crate::mob::brain::nether::*;
use crate::mob::brain::sensors;
use crate::mob::brain::util;
use crate::mob::brain::{self, Activity, ActivityData, Brain, Control, Cx, Gate, Mem, Shot, ShotBehavior, Status, TriggerGate, shot};
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::Living;
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, DamageSource, GroupData, MAINHAND, MobData, MobKind, OFFHAND, SpawnContext};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

use Status::{Registered, ValueAbsent, ValuePresent};

pub struct Piglin;

pub static KIND: Piglin = Piglin;

static INFO: Info = Info { sounds: Some("piglin"), ..Info::monster("minecraft:piglin", &[(MaxHealth, 16.0), (MovementSpeed, 0.3499999940395355), (AttackDamage, 5.0)]) };

/// `PiglinAi.ADMIRE_DURATION`.
pub const ADMIRE_DURATION: i32 = 119;
/// `AbstractPiglin`: ticks outside the nether before zombifying.
pub const CONVERSION_TIME: i32 = 300;
/// `PiglinAi.BARTERING_ITEM`.
pub const BARTERING_ITEM: &str = "minecraft:gold_ingot";
pub const BARTERING_TABLE: &str = "minecraft:gameplay/piglin_bartering";

/// `PiglinAi.TIME_BETWEEN_HUNTS`, `RETREAT_DURATION`, ... (ticks).
const TIME_BETWEEN_HUNTS: (i32, i32) = seconds(30, 120);
const RIDE_START_INTERVAL: (i32, i32) = seconds(10, 40);
const RIDE_DURATION: (i32, i32) = seconds(10, 30);
const RETREAT_DURATION: (i32, i32) = seconds(5, 20);
const AVOID_ZOMBIFIED_DURATION: (i32, i32) = seconds(5, 7);
const BABY_AVOID_NEMESIS_DURATION: (i32, i32) = seconds(5, 7);

/// The state `Piglin` and `PiglinBrute` (`AbstractPiglin`) keep besides the brain.
#[derive(Clone, Debug, Default)]
pub struct PiglinState {
    pub immune_to_zombification: bool,
    /// `timeInOverworld`.
    pub time_in_overworld: i32,
    pub cannot_hunt: bool,
    /// The 8-slot inventory (`InventoryCarrier`); the brute has none.
    pub inventory: Vec<ItemStack>,
    /// `DATA_IS_CHARGING_CROSSBOW`.
    pub charging_crossbow: bool,
    /// `DATA_IS_DANCING`.
    pub dancing: bool,
    /// Attackers whose hits the piglin reacts to at the start of its next brain tick (the hit came
    /// from a mob in the middle of its own tick, which the level cannot show).
    pub pending_hurt: Vec<i32>,
}

pub fn state(m: &MobData) -> Option<&PiglinState> {
    ext::state::<PiglinState>(m)
}

pub fn state_mut(m: &mut MobData) -> Option<&mut PiglinState> {
    ext::state_mut::<PiglinState>(m)
}

pub fn set_charging_crossbow(m: &mut MobData, on: bool) {
    if let Some(s) = state_mut(m) {
        s.charging_crossbow = on;
    }
}

fn is_item(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && mob::item_name(stack) == name
}

/// `PiglinAi.isLovedItem`.
pub fn is_loved(stack: &ItemStack) -> bool {
    !stack.is_empty() && loved_item(stack.item())
}

/// `Piglin.setBaby`: the flag and the speed bonus.
pub fn set_baby(e: &mut Entity, m: &mut MobData, baby: bool) {
    m.zombie_baby = baby;
    m.attrs.remove_modifier(MovementSpeed, "minecraft:baby");
    if baby {
        m.attrs.set_modifier(MovementSpeed, "minecraft:baby", 0.20000000298023224, Op::AddMultipliedBase);
    }
    mob::refresh_dimensions(e, m);
}

/// `Piglin.canHunt` (`PiglinBrute.canHunt` is false).
pub fn can_hunt(m: &MobData) -> bool {
    m.kind == MobKind::Piglin && !state(m).is_some_and(|s| s.cannot_hunt)
}

// ---------------------------------------------------------------------------- inventory

/// `InventoryCarrier` / `SimpleContainer.addItem`: what does not fit comes back.
fn add_to_inventory(st: &mut PiglinState, mut stack: ItemStack) -> ItemStack {
    for slot in st.inventory.iter_mut() {
        if stack.is_empty() {
            break;
        }
        if !slot.is_empty() && slot.is_same_item_same_components(&stack) {
            let n = (slot.max_stack_size() - slot.count()).min(stack.count());
            slot.grow(n);
            stack.shrink(n);
        }
    }
    for slot in st.inventory.iter_mut() {
        if stack.is_empty() {
            break;
        }
        if slot.is_empty() {
            *slot = std::mem::replace(&mut stack, ItemStack::empty());
        }
    }
    stack
}

fn can_add_to_inventory(st: &PiglinState, stack: &ItemStack) -> bool {
    st.inventory.iter().any(|s| s.is_empty() || (s.is_same_item_same_components(stack) && s.count() < s.max_stack_size()))
}

// ---------------------------------------------------------------------------- equipment

fn equippable_slot(stack: &ItemStack) -> Option<usize> {
    use kiln_item::component::EquipmentSlot as S;
    stack.get(kiln_item::keys::EQUIPPABLE).and_then(|q| match q.slot {
        S::MainHand => Some(MAINHAND),
        S::OffHand => Some(OFFHAND),
        S::Feet => Some(mob::FEET),
        S::Legs => Some(mob::LEGS),
        S::Chest => Some(mob::CHEST),
        S::Head => Some(mob::HEAD),
        _ => None,
    })
}

/// `getApproximateAttributeWith`: the base of a piglin's attribute with the item's modifiers of
/// that slot (`ItemAttributeModifiers.compute`).
pub(crate) fn approximate_attribute(m: &MobData, stack: &ItemStack, attr: Attr, slot: usize) -> f64 {
    use kiln_item::component::{AttributeOperation, EquipmentSlotGroup as G};
    let base = m.attrs.get(attr).map_or(0.0, |a| a.base);
    let Some(mods) = stack.get(kiln_item::keys::ATTRIBUTE_MODIFIERS) else { return base };
    let (mut add, mut mul_base, mut mul_total) = (0.0, 0.0, 1.0);
    for md in &mods.0 {
        let name = kiln_data::builtin_entries("minecraft:attribute").and_then(|e| e.get(md.attribute as usize).copied());
        if name.and_then(Attr::by_name) != Some(attr) {
            continue;
        }
        let fits = match md.slot {
            G::Any => true,
            G::MainHand => slot == MAINHAND,
            G::OffHand => slot == OFFHAND,
            G::Hand => slot <= OFFHAND,
            G::Feet => slot == mob::FEET,
            G::Legs => slot == mob::LEGS,
            G::Chest => slot == mob::CHEST,
            G::Head => slot == mob::HEAD,
            G::Armor => (mob::FEET..=mob::HEAD).contains(&slot) || slot == 6,
            G::Body => slot == 6,
            G::Saddle => slot == 7,
            _ => false,
        };
        if !fits {
            continue;
        }
        match md.operation {
            AttributeOperation::AddValue => add += md.amount,
            AttributeOperation::AddMultipliedBase => mul_base += md.amount,
            AttributeOperation::AddMultipliedTotal => mul_total *= 1.0 + md.amount,
        }
    }
    let v = base + add;
    (v + v * mul_base) * mul_total
}

/// `Mob.canReplaceEqualItem`.
pub(crate) fn can_replace_equal_item(candidate: &ItemStack, current: &ItemStack) -> bool {
    let enchants = |s: &ItemStack| s.get(kiln_item::keys::ENCHANTMENTS).map_or(0, |e| e.0.len());
    let (a, b) = (enchants(candidate), enchants(current));
    if a != b {
        return a > b;
    }
    let (dc, dcur) = (candidate.damage(), current.damage());
    if dc != dcur {
        return dc < dcur;
    }
    candidate.has(kiln_item::component::ids::CUSTOM_NAME) && !current.has(kiln_item::component::ids::CUSTOM_NAME)
}

/// `Piglin.canReplaceCurrentItem(candidate, current, slot)` over `Mob.canReplaceCurrentItem`.
fn can_replace_current_item_in(m: &MobData, candidate: &ItemStack, current: &ItemStack, slot: usize, preferred: Option<&str>) -> bool {
    let pref = |s: &ItemStack| is_loved(s) || (preferred.is_some_and(|t| !s.is_empty() && mob::item_tag(s.item(), t)));
    if m.kind == MobKind::Piglin {
        let (new_pref, cur_pref) = (pref(candidate), pref(current));
        if new_pref && !cur_pref {
            return true;
        }
        if !new_pref && cur_pref {
            return false;
        }
    }
    if current.is_empty() {
        return true;
    }
    if slot >= mob::FEET {
        let armor = |s: &ItemStack| approximate_attribute(m, s, Armor, slot);
        let tough = |s: &ItemStack| approximate_attribute(m, s, ArmorToughness, slot);
        let (a, b) = (armor(candidate), armor(current));
        if a != b {
            return a > b;
        }
        let (a, b) = (tough(candidate), tough(current));
        if a != b {
            return a > b;
        }
        return can_replace_equal_item(candidate, current);
    }
    if slot == MAINHAND {
        if let Some(t) = preferred {
            let cur_is = mob::item_tag(current.item(), t);
            let new_is = mob::item_tag(candidate.item(), t);
            if cur_is && !new_is {
                return false;
            }
            if !cur_is && new_is {
                return true;
            }
        }
        let (a, b) = (approximate_attribute(m, candidate, AttackDamage, slot), approximate_attribute(m, current, AttackDamage, slot));
        if a != b {
            return a > b;
        }
        return can_replace_equal_item(candidate, current);
    }
    false
}

/// `Piglin.getPreferredWeaponType`.
fn preferred_weapons(m: &MobData) -> Option<&'static str> {
    if m.kind == MobKind::Piglin && !m.baby() { Some("minecraft:piglin_preferred_weapons") } else { None }
}

/// `Piglin.canReplaceCurrentItem(stack)`.
fn can_replace_current_item(m: &MobData, stack: &ItemStack) -> bool {
    let slot = equippable_slot(stack).unwrap_or(MAINHAND);
    can_replace_current_item_in(m, stack, &m.equipment[slot], slot, preferred_weapons(m))
}

/// `Mob.equipItemIfPossible`: what was put on (empty: nothing).
pub fn equip_item_if_possible(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, stack: ItemStack) -> ItemStack {
    let mut slot = equippable_slot(&stack).unwrap_or(MAINHAND);
    if stack.get(kiln_item::keys::EQUIPPABLE).is_some() && equippable_slot(&stack).is_none() {
        return ItemStack::empty();
    }
    let preferred = preferred_weapons(m);
    let mut replace = can_replace_current_item_in(m, &stack, &m.equipment[slot], slot, preferred);
    if slot >= mob::FEET && !replace {
        slot = MAINHAND;
        replace = m.equipment[MAINHAND].is_empty();
    }
    if !replace {
        return ItemStack::empty();
    }
    let chance = m.drop_chances[slot] as f64;
    if !m.equipment[slot].is_empty() && ((e.random.next_float() - 0.1f32).max(0.0) as f64) < chance {
        let old = m.equipment[slot].clone();
        mob::spawn_at_location(e, level, old);
    }
    let limited = if slot >= mob::FEET { stack.with_count(1) } else { stack };
    m.equipment[slot] = limited.clone();
    m.drop_chances[slot] = 2.0;
    m.persistence_required = true;
    limited
}

// ---------------------------------------------------------------------------- PiglinAi: memory helpers

fn admiring_item(mem: &Memories) -> bool {
    mem.has(Mem::AdmiringItem)
}

fn has_eaten_recently(mem: &Memories) -> bool {
    mem.has(Mem::AteRecently)
}

fn is_not_holding_loved_item_in_offhand(m: &MobData) -> bool {
    m.equipment[OFFHAND].is_empty() || !is_loved(&m.equipment[OFFHAND])
}

/// `PiglinAi.wantsToPickup`.
pub fn wants_to_pickup(m: &MobData, mem: &Memories, stack: &ItemStack) -> bool {
    if m.baby() && mob::item_tag(stack.item(), "minecraft:ignored_by_piglin_babies") {
        return false;
    }
    if mob::item_tag(stack.item(), "minecraft:piglin_repellents") {
        return false;
    }
    if mem.has(Mem::AdmiringDisabled) && mem.has(Mem::AttackTarget) {
        return false;
    }
    if is_item(stack, BARTERING_ITEM) {
        return is_not_holding_loved_item_in_offhand(m);
    }
    let can_add = state(m).is_some_and(|s| can_add_to_inventory(s, stack));
    if is_item(stack, "minecraft:gold_nugget") {
        return can_add;
    }
    if mob::item_tag(stack.item(), "minecraft:piglin_food") {
        return !has_eaten_recently(mem) && can_add;
    }
    if is_loved(stack) {
        return is_not_holding_loved_item_in_offhand(m) && can_add;
    }
    can_replace_current_item(m, stack)
}

/// `Piglin.wantsToPickUp(level, stack)` for the item sensor.
fn sensor_wants(cx: &Cx, stack: &ItemStack) -> bool {
    cx.level.mob_griefing() && cx.m.can_pick_up_loot && wants_to_pickup(cx.m, &cx.b.mem, stack)
}

/// `PiglinAi.stopWalking`.
pub fn stop_walking(m: &mut MobData, mem: &mut Memories) {
    mem.erase(Mem::WalkTarget);
    m.nav.stop();
}

/// `PiglinAi.holdInOffhand`: the previous off hand item drops; gold being bartered does not
/// make the piglin persistent.
fn hold_in_offhand(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, stack: ItemStack) {
    if !m.equipment[OFFHAND].is_empty() {
        let old = std::mem::replace(&mut m.equipment[OFFHAND], ItemStack::empty());
        mob::spawn_at_location(e, level, old);
    }
    let barter = is_item(&stack, BARTERING_ITEM);
    m.equipment[OFFHAND] = stack;
    m.drop_chances[OFFHAND] = 2.0;
    if !barter {
        m.persistence_required = true;
    }
}

/// `PiglinAi.admireGoldItem`.
fn admire_gold_item(mem: &mut Memories) {
    mem.set_expiring(Mem::AdmiringItem, Val::Bool(true), ADMIRE_DURATION as i64);
}

/// `PiglinAi.getRandomNearbyPos`: `LandRandomPos.getPos(mob, 4, 2)` or where the piglin stands.
fn random_nearby_pos(e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Vec3 {
    crate::mob::random_pos::land_pos(e, m, level, 4, 2).unwrap_or_else(|| e.position())
}

/// `BehaviorUtils.throwItem(mob, stack, target)`: an item leaves from the chest toward `target`.
fn throw_item(e: &Entity, level: &mut dyn EntityLevel, stack: ItemStack, target: Vec3) {
    if stack.is_empty() {
        return;
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let mut item = crate::item::new_at(id, 0, stack, Vec3::new(e.x(), e.eye_y() - 0.30000001192092896, e.z()), seed);
    let dir = (target - e.position()).normalize();
    item.delta = Vec3::new(dir.x * 0.30000001192092896, dir.y * 0.30000001192092896, dir.z * 0.30000001192092896);
    if let EntityKind::Item(d) = &mut item.kind {
        d.pickup_delay = 10;
        d.thrower = Some(e.uuid);
    }
    item.set_old_pos_and_rot();
    level.add_entity(item);
}

/// `PiglinAi.throwItemsTowardPos`.
fn throw_items_toward_pos(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, items: Vec<ItemStack>, pos: Vec3) {
    if items.is_empty() {
        return;
    }
    m.swing = true;
    for s in items {
        throw_item(e, level, s, pos.add(0.0, 1.0, 0.0));
    }
}

/// `PiglinAi.throwItemsTowardRandomPos`.
fn throw_items_toward_random_pos(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, items: Vec<ItemStack>) {
    let pos = random_nearby_pos(e, m, &*level);
    throw_items_toward_pos(e, m, level, items, pos);
}

/// `PiglinAi.putInInventory`.
fn put_in_inventory(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, stack: ItemStack) {
    let left = match state_mut(m) {
        Some(st) => add_to_inventory(st, stack),
        None => stack,
    };
    throw_items_toward_random_pos(e, m, level, vec![left]);
}

/// `PiglinAi.throwItems` (toward the nearest visible player, or a random spot). The bartering
/// loot table drops at the spot two blocks that way (Kiln's loot events are spawned, not thrown).
fn barter(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, nearest_player: Option<i32>) {
    m.swing = true;
    let p = e.position();
    let toward = nearest_player.and_then(|id| level.player(id).map(|v| v.pos).or_else(|| level.entity(id).map(|o| o.position())));
    let toward = match toward {
        Some(t) => Some(t),
        None => {
            let t = random_nearby_pos(e, m, &*level);
            Some(t)
        }
    };
    let dir = match toward {
        Some(t) => {
            let d = Vec3::new(t.x - p.x, 0.0, t.z - p.z);
            if d.length() < 1.0e-5 { Vec3::new(0.0, 0.0, 0.0) } else { d.normalize() }
        }
        None => Vec3::ZERO,
    };
    level.emit(Event::GiftLoot { entity: e.id, table: BARTERING_TABLE, pos: Vec3::new(p.x + dir.x * 2.0, p.y + 1.0, p.z + dir.z * 2.0) });
}

/// `PiglinAi.stopHoldingOffHandItem`.
fn stop_holding_offhand(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, mem: &mut Memories, do_barter: bool) {
    let off = std::mem::replace(&mut m.equipment[OFFHAND], ItemStack::empty());
    let nearest_player = mem.entity(Mem::NearestVisiblePlayer);
    if !m.baby() {
        let currency = is_item(&off, BARTERING_ITEM);
        if do_barter && currency {
            barter(e, m, level, nearest_player);
        } else if !currency {
            let equipped = !equip_item_if_possible(e, m, level, off.clone()).is_empty();
            if !equipped {
                put_in_inventory(e, m, level, off);
            }
        }
    } else {
        let equipped = !equip_item_if_possible(e, m, level, off.clone()).is_empty();
        if !equipped {
            let main = m.equipment[MAINHAND].clone();
            if is_loved(&main) {
                put_in_inventory(e, m, level, main);
            } else {
                // `throwItems(piglin, [mainHand])`.
                match nearest_player.and_then(|id| level.player(id).map(|v| v.pos).or_else(|| level.entity(id).map(|o| o.position()))) {
                    Some(t) => throw_items_toward_pos(e, m, level, vec![main], t),
                    None => throw_items_toward_random_pos(e, m, level, vec![main]),
                }
            }
            m.equipment[MAINHAND] = off;
            m.drop_chances[MAINHAND] = 2.0;
            m.persistence_required = true;
        }
    }
}

/// `PiglinAi.cancelAdmiring`.
fn cancel_admiring(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, mem: &Memories) {
    if admiring_item(mem) && !m.equipment[OFFHAND].is_empty() {
        let s = std::mem::replace(&mut m.equipment[OFFHAND], ItemStack::empty());
        mob::spawn_at_location(e, level, s);
    }
}

/// `PiglinAi.pickUpItem` for an item entity `id` the piglin reaches.
fn pick_up_item(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, mem: &mut Memories, id: i32) {
    stop_walking(m, mem);
    let Some(item) = level.entity_mut(id) else { return };
    let EntityKind::Item(d) = &mut item.kind else { return };
    let (thrower, whole) = (d.thrower, d.stack.clone());
    let taken = if is_item(&d.stack, "minecraft:gold_nugget") {
        let s = std::mem::replace(&mut d.stack, ItemStack::empty());
        item.discard();
        s
    } else {
        let s = d.stack.split(1);
        if d.stack.is_empty() {
            item.discard();
        }
        s
    };
    mob::on_item_pickup(e, m, level, thrower, &whole);
    if is_loved(&taken) {
        mem.erase(Mem::TimeTryingToReachAdmireItem);
        hold_in_offhand(e, m, level, taken);
        admire_gold_item(mem);
        return;
    }
    if mob::item_tag(taken.item(), "minecraft:piglin_food") && !has_eaten_recently(mem) {
        // `eat`.
        mem.set_expiring(Mem::AteRecently, Val::Bool(true), 200);
        return;
    }
    if !equip_item_if_possible(e, m, level, taken.clone()).is_empty() {
        return;
    }
    put_in_inventory(e, m, level, taken);
}

/// `Mob.aiStep`'s item pickup (reach 1 horizontally) with `Piglin.wantsToPickUp`.
pub fn pick_up_loot(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if !m.can_pick_up_loot || !mob::is_alive(e, m) || !level.mob_griefing() {
        return;
    }
    let Some(mut brain) = m.brain.take() else { return };
    let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
    for id in level.entities_in(&area, EntityFilter::Item, e.id) {
        let Some(o) = level.entity(id) else { continue };
        let EntityKind::Item(d) = &o.kind else { continue };
        if o.is_removed() || d.stack.is_empty() || d.pickup_delay > 0 {
            continue;
        }
        let stack = d.stack.clone();
        let wants = if m.kind == MobKind::PiglinBrute { is_item(&stack, "minecraft:golden_axe") } else { wants_to_pickup(m, &brain.st.mem, &stack) };
        if !wants {
            continue;
        }
        if m.kind == MobKind::PiglinBrute {
            // `Mob.pickUpItem`.
            let equipped = equip_item_if_possible(e, m, level, stack.clone());
            let mut picked = None;
            if !equipped.is_empty()
                && let Some(item) = level.entity_mut(id)
                && let EntityKind::Item(d) = &mut item.kind
            {
                picked = Some((d.thrower, d.stack.clone()));
                d.stack.shrink(equipped.count());
                if d.stack.is_empty() {
                    item.discard();
                }
            }
            if let Some((thrower, whole)) = picked {
                mob::on_item_pickup(e, m, level, thrower, &whole);
            }
        } else {
            pick_up_item(e, m, level, &mut brain.st.mem, id);
        }
    }
    m.brain = Some(brain);
}

// ---------------------------------------------------------------------------- PiglinAi: anger and hunting

/// `Sensor.isEntityAttackableIgnoringLineOfSight` as the mob `who` sees it (its own position,
/// follow range, attack target and vetoes).
fn attackable_ignoring_los(cx: &mut Cx, t: &Living) -> bool {
    util::is_entity_attackable_ignoring_los(cx, t)
}

/// `PiglinAi.setAngerTarget(level, piglin, target)`.
pub fn set_anger_target(cx: &mut Cx, target: &Living) {
    if !attackable_ignoring_los(cx, target) {
        return;
    }
    cx.b.mem.erase(Mem::CantReachWalkTargetSince);
    let u = uuid_of(&*cx.level, target.id);
    cx.b.mem.set_expiring(Mem::AngryAt, Val::Uuid(u), 600);
    if target.type_name == HOGLIN && can_hunt(cx.m) {
        dont_kill_any_more_hoglins_for_a_while(cx);
    }
    if target.player && cx.level.universal_anger() {
        cx.b.mem.set_expiring(Mem::UniversalAnger, Val::Bool(true), 600);
    }
}

/// `PiglinAi.dontKillAnyMoreHoglinsForAWhile`.
pub fn dont_kill_any_more_hoglins_for_a_while(cx: &mut Cx) {
    let n = sample(cx.rng(), TIME_BETWEEN_HUNTS.0, TIME_BETWEEN_HUNTS.1) as i64;
    cx.b.mem.set_expiring(Mem::HuntedRecently, Val::Bool(true), n);
}

/// `BehaviorUtils.getNearestTarget(mob, current, candidate)`.
fn nearest_target(cx: &Cx, current: Option<&Living>, candidate: &Living) -> i32 {
    match current {
        None => candidate.id,
        Some(c) => util::nearest_of(cx, c, candidate),
    }
}

/// `PiglinAi.setAngerTargetIfCloserThanCurrent`.
fn set_anger_target_if_closer_than_current(cx: &mut Cx, target: &Living) {
    let current = living_from_uuid_memory(cx, Mem::AngryAt);
    let nearest = nearest_target(cx, current.as_ref(), target);
    if current.as_ref().is_some_and(|c| c.id == nearest) {
        return;
    }
    let t = if nearest == target.id { target.clone() } else { current.expect("current is the nearest") };
    set_anger_target(cx, &t);
}

/// `PiglinAi.broadcastAngerTarget`.
pub fn broadcast_anger_target(cx: &mut Cx, target: &Living) {
    let others = cx.b.mem.entities(Mem::NearbyAdultPiglins).to_vec();
    for id in others {
        let t = target.clone();
        as_mob(cx, id, |c| {
            if t.type_name == HOGLIN && !(can_hunt(c.m) && hoglin_can_be_hunted(c, t.id)) {
                return;
            }
            set_anger_target_if_closer_than_current(c, &t);
        });
    }
}

fn hoglin_can_be_hunted(cx: &Cx, id: i32) -> bool {
    mob_data(cx, id).is_some_and(|m| !m.baby() && !crate::mob::kinds::hoglin::state(m).is_some_and(|s| s.cannot_be_hunted))
}

/// `PiglinAi.getNearestVisibleTargetablePlayer`.
fn nearest_visible_targetable_player(cx: &Cx) -> Option<i32> {
    cx.b.mem.entity(Mem::NearestVisibleAttackablePlayer)
}

/// `PiglinAi.setAngerTargetToNearestTargetablePlayerIfFound`.
fn set_anger_target_to_nearest_targetable_player_if_found(cx: &mut Cx, fallback: &Living) {
    match nearest_visible_targetable_player(cx).and_then(|id| living_now(cx, id)) {
        Some(p) => set_anger_target(cx, &p),
        None => set_anger_target(cx, fallback),
    }
}

/// `PiglinAi.broadcastUniversalAnger`.
pub fn broadcast_universal_anger(cx: &mut Cx) {
    let others = cx.b.mem.entities(Mem::NearbyAdultPiglins).to_vec();
    for id in others {
        as_mob(cx, id, |c| {
            if let Some(p) = nearest_visible_targetable_player(c).and_then(|pid| living_now(c, pid)) {
                set_anger_target(c, &p);
            }
        });
    }
}

/// `PiglinAi.maybeRetaliate` (also the brute's).
pub fn maybe_retaliate(cx: &mut Cx, attacker: &Living) {
    if cx.b.is_active(Activity::Avoid) {
        return;
    }
    if !attackable_ignoring_los(cx, attacker) {
        return;
    }
    if util::other_target_much_further(cx, attacker, 4.0) {
        return;
    }
    if attacker.player && cx.level.universal_anger() {
        set_anger_target_to_nearest_targetable_player_if_found(cx, attacker);
        broadcast_universal_anger(cx);
    } else {
        set_anger_target(cx, attacker);
        broadcast_anger_target(cx, attacker);
    }
}

/// `PiglinAi.hoglinsOutnumberPiglins`.
fn hoglins_outnumber_piglins(cx: &Cx) -> bool {
    let piglins = cx.b.mem.int(Mem::VisibleAdultPiglinCount).unwrap_or(0) + 1;
    let hoglins = cx.b.mem.int(Mem::VisibleAdultHoglinCount).unwrap_or(0);
    hoglins > piglins
}

/// `PiglinAi.setAvoidTargetAndDontHuntForAWhile`.
fn set_avoid_target_and_dont_hunt_for_a_while(cx: &mut Cx, target: &Living) {
    cx.b.mem.erase(Mem::AngryAt);
    cx.b.mem.erase(Mem::AttackTarget);
    cx.b.mem.erase(Mem::WalkTarget);
    let n = sample(cx.rng(), RETREAT_DURATION.0, RETREAT_DURATION.1) as i64;
    cx.b.mem.set_expiring(Mem::AvoidTarget, Val::Entity(target.id), n);
    dont_kill_any_more_hoglins_for_a_while(cx);
}

/// `PiglinAi.retreatFromNearestTarget`.
fn retreat_from_nearest_target(cx: &mut Cx, target: &Living) {
    let avoid = cx.b.mem.entity(Mem::AvoidTarget).and_then(|id| living_now(cx, id));
    let mut t = target.clone();
    let n = nearest_target(cx, avoid.as_ref(), &t);
    if n != t.id {
        t = avoid.expect("avoid is the nearest");
    }
    let attack = cx.b.mem.entity(Mem::AttackTarget).and_then(|id| living_now(cx, id));
    let n = nearest_target(cx, attack.as_ref(), &t);
    if n != t.id {
        t = attack.expect("attack is the nearest");
    }
    set_avoid_target_and_dont_hunt_for_a_while(cx, &t);
}

/// `PiglinAi.broadcastRetreat`.
fn broadcast_retreat(cx: &mut Cx, target: &Living) {
    let others = cx.b.mem.entities(Mem::NearestVisibleAdultPiglins).to_vec();
    for id in others {
        if cx.level.entity(id).is_some_and(|o| o.type_name == PIGLIN) {
            let t = target.clone();
            as_mob(cx, id, |c| retreat_from_nearest_target(c, &t));
        }
    }
}

/// `PiglinAi.wasHurtBy`.
pub fn was_hurt_by(cx: &mut Cx, attacker: &Living) {
    if attacker.type_name == PIGLIN {
        return;
    }
    if !cx.m.equipment[OFFHAND].is_empty() {
        let (e, m, level) = (&mut *cx.e, &mut *cx.m, &mut *cx.level);
        stop_holding_offhand(e, m, level, &mut cx.b.mem, false);
    }
    cx.b.mem.erase(Mem::CelebrateLocation);
    cx.b.mem.erase(Mem::Dancing);
    cx.b.mem.erase(Mem::AdmiringItem);
    if attacker.player {
        cx.b.mem.set_expiring(Mem::AdmiringDisabled, Val::Bool(true), 400);
    }
    if let Some(avoid) = cx.b.mem.entity(Mem::AvoidTarget)
        && living_now(cx, avoid).is_none_or(|a| a.type_name != attacker.type_name)
    {
        cx.b.mem.erase(Mem::AvoidTarget);
    }
    if cx.m.baby() {
        cx.b.mem.set_expiring(Mem::AvoidTarget, Val::Entity(attacker.id), 100);
        if attackable_ignoring_los(cx, attacker) {
            broadcast_anger_target(cx, attacker);
        }
        return;
    }
    if attacker.type_name == HOGLIN && hoglins_outnumber_piglins(cx) {
        set_avoid_target_and_dont_hunt_for_a_while(cx, attacker);
        broadcast_retreat(cx, attacker);
        return;
    }
    maybe_retaliate(cx, attacker);
}

/// `PiglinAi.angerNearbyPiglins(level, player, needsLineOfSight)`: a player opened a chest or
/// broke gold; idle piglins within 16 blocks (that can see the player) turn on the player.
pub fn anger_nearby_piglins(level: &mut dyn EntityLevel, player: i32, needs_line_of_sight: bool) {
    let Some(p) = level.player(player) else { return };
    let area = crate::math::Aabb::new(p.pos.x - 16.0, p.pos.y - 16.0, p.pos.z - 16.0, p.pos.x + 16.0, p.pos.y + 1.8 + 16.0, p.pos.z + 16.0);
    for id in level.entities_in(&area, EntityFilter::Living, i32::MIN) {
        if !level.entity(id).is_some_and(|e| e.type_name == PIGLIN) {
            continue;
        }
        let Some(slot) = level.entity_mut(id) else { continue };
        let mut e = std::mem::replace(slot, marker());
        let mut m = mob::take(&mut e);
        let mut brain = m.brain.take();
        if let Some(b) = brain.as_mut() {
            let time = level.game_time();
            let mut cx = Cx { e: &mut e, m: &mut m, level: &mut *level, b: &mut b.st, time };
            let idle = cx.b.is_active(Activity::Idle);
            let sees = !needs_line_of_sight || util::can_see(&mut cx, player);
            if idle && sees
                && let Some(t) = living_now(&cx, player)
            {
                if cx.level.universal_anger() {
                    set_anger_target_to_nearest_targetable_player_if_found(&mut cx, &t);
                } else {
                    set_anger_target(&mut cx, &t);
                }
            }
        }
        m.brain = brain;
        mob::put(&mut e, m);
        if let Some(slot) = level.entity_mut(id) {
            *slot = e;
        }
    }
}

// ---------------------------------------------------------------------------- the brain's parts

/// `PiglinAi.isNearZombified`.
fn is_near_zombified(cx: &mut Cx) -> bool {
    match cx.b.mem.entity(Mem::NearestVisibleZombified) {
        Some(id) => cx.level.entity(id).is_some_and(|o| cx.e.position().distance_to_sqr(o.position()) < 36.0),
        None => false,
    }
}

/// `PiglinAi.findNearestValidAttackTarget`.
fn find_nearest_valid_attack_target(cx: &mut Cx) -> Option<i32> {
    if is_near_zombified(cx) {
        return None;
    }
    if let Some(a) = living_from_uuid_memory(cx, Mem::AngryAt)
        && util::is_entity_attackable_ignoring_los(cx, &a)
    {
        return Some(a.id);
    }
    if cx.b.mem.has(Mem::UniversalAnger)
        && let Some(p) = cx.b.mem.entity(Mem::NearestVisibleAttackablePlayer)
    {
        return Some(p);
    }
    if let Some(n) = cx.b.mem.entity(Mem::NearestVisibleNemesis) {
        return Some(n);
    }
    if let Some(p) = cx.b.mem.entity(Mem::NearestTargetablePlayerNotWearingGold)
        && let Some(l) = living_now(cx, p)
        && util::is_entity_attackable(cx, &l)
    {
        return Some(p);
    }
    None
}

fn is_adult(cx: &mut Cx) -> bool {
    !cx.m.baby()
}

fn is_baby_cx(cx: &mut Cx) -> bool {
    cx.m.baby()
}

fn wants_to_dance(cx: &mut Cx, t: &Living) -> bool {
    t.type_name == HOGLIN && thread_local_random(cx.level.game_time()).next_float() < 0.1
}

fn stop_holding_item_if_no_longer_admiring() -> Box<dyn Control> {
    shot("StopHoldingItemIfNoLongerAdmiring", &[(Mem::AdmiringItem, ValueAbsent)], |cx| {
        if cx.m.equipment[OFFHAND].is_empty() || cx.m.equipment[OFFHAND].has(kiln_item::component::ids::BLOCKS_ATTACKS) {
            return false;
        }
        let (e, m, level) = (&mut *cx.e, &mut *cx.m, &mut *cx.level);
        stop_holding_offhand(e, m, level, &mut cx.b.mem, true);
        true
    })
}

fn start_admiring_item_if_seen(duration: i32) -> Box<dyn Control> {
    shot(
        "StartAdmiringItemIfSeen",
        &[(Mem::NearestVisibleWantedItem, ValuePresent), (Mem::AdmiringItem, ValueAbsent), (Mem::AdmiringDisabled, ValueAbsent), (Mem::DisableWalkToAdmireItem, ValueAbsent)],
        move |cx| {
            let Some(item) = cx.b.mem.entity(Mem::NearestVisibleWantedItem) else { return false };
            let loved = item_stack_of(&*cx.level, item).is_some_and(|s| is_loved(&s));
            if !loved {
                return false;
            }
            cx.b.mem.set_expiring(Mem::AdmiringItem, Val::Bool(true), duration as i64);
            true
        },
    )
}

fn stop_admiring_if_item_too_far_away(max: i32) -> Box<dyn Control> {
    shot("StopAdmiringIfItemTooFarAway", &[(Mem::AdmiringItem, ValuePresent), (Mem::NearestVisibleWantedItem, Registered)], move |cx| {
        if !cx.m.equipment[OFFHAND].is_empty() {
            return false;
        }
        if let Some(item) = cx.b.mem.entity(Mem::NearestVisibleWantedItem)
            && let Some(pos) = cx.level.entity(item).map(|o| o.position())
            && closer_than(cx, pos, max as f64)
        {
            return false;
        }
        cx.b.mem.erase(Mem::AdmiringItem);
        true
    })
}

fn stop_admiring_if_tired_of_trying_to_reach_item(max_time: i32, disable_time: i32) -> Box<dyn Control> {
    shot(
        "StopAdmiringIfTiredOfTryingToReachItem",
        &[(Mem::AdmiringItem, ValuePresent), (Mem::NearestVisibleWantedItem, ValuePresent), (Mem::TimeTryingToReachAdmireItem, Registered), (Mem::DisableWalkToAdmireItem, Registered)],
        move |cx| {
            if !cx.m.equipment[OFFHAND].is_empty() {
                return false;
            }
            match cx.b.mem.int(Mem::TimeTryingToReachAdmireItem) {
                None => cx.b.mem.set(Mem::TimeTryingToReachAdmireItem, Val::Int(0)),
                Some(t) if t > max_time => {
                    cx.b.mem.erase(Mem::AdmiringItem);
                    cx.b.mem.erase(Mem::TimeTryingToReachAdmireItem);
                    cx.b.mem.set_expiring(Mem::DisableWalkToAdmireItem, Val::Bool(true), disable_time as i64);
                }
                Some(t) => cx.b.mem.set(Mem::TimeTryingToReachAdmireItem, Val::Int(t + 1)),
            }
            true
        },
    )
}

/// `StartHuntingHoglin.create()`.
fn start_hunting_hoglin() -> Box<dyn Control> {
    shot(
        "StartHuntingHoglin",
        &[(Mem::NearestVisibleHuntableHoglin, ValuePresent), (Mem::AngryAt, ValueAbsent), (Mem::HuntedRecently, ValueAbsent), (Mem::NearestVisibleAdultPiglins, Registered)],
        |cx| {
            let adults = cx.b.mem.entities(Mem::NearestVisibleAdultPiglins).to_vec();
            let recent = |cx: &Cx, id: i32| {
                mob_data(cx, id).and_then(|m| m.brain.as_ref()).is_some_and(|b| b.st.mem.has(Mem::HuntedRecently))
            };
            if cx.m.baby() || adults.iter().any(|&id| recent(cx, id)) {
                return false;
            }
            let Some(h) = cx.b.mem.entity(Mem::NearestVisibleHuntableHoglin) else { return false };
            let Some(l) = living_now(cx, h) else { return false };
            set_anger_target(cx, &l);
            dont_kill_any_more_hoglins_for_a_while(cx);
            broadcast_anger_target(cx, &l);
            for id in adults {
                as_mob(cx, id, |c| dont_kill_any_more_hoglins_for_a_while(c));
            }
            true
        },
    )
}

/// `RememberIfHoglinWasKilled.create()`.
fn remember_if_hoglin_was_killed() -> Box<dyn Control> {
    shot("RememberIfHoglinWasKilled", &[(Mem::AttackTarget, ValuePresent), (Mem::HuntedRecently, Registered)], |cx| {
        let Some(id) = cx.b.mem.entity(Mem::AttackTarget) else { return false };
        if cx.level.entity(id).is_some_and(|o| o.type_name == HOGLIN) && is_dead_or_dying(cx, id) {
            let n = sample(cx.rng(), TIME_BETWEEN_HUNTS.0, TIME_BETWEEN_HUNTS.1) as i64;
            cx.b.mem.set_expiring(Mem::HuntedRecently, Val::Bool(true), n);
        }
        true
    })
}

fn is_dancing(cx: &mut Cx) -> bool {
    state(cx.m).is_some_and(|s| s.dancing)
}

fn is_not_dancing(cx: &mut Cx) -> bool {
    !is_dancing(cx)
}

fn has_crossbow(cx: &mut Cx) -> bool {
    [MAINHAND, OFFHAND].iter().any(|&i| is_item(&cx.m.equipment[i], "minecraft:crossbow"))
}

fn is_passenger(cx: &mut Cx) -> bool {
    cx.e.vehicle.is_some()
}

fn always(_cx: &mut Cx) -> bool {
    true
}

fn can_hunt_cx(cx: &mut Cx) -> bool {
    can_hunt(cx.m)
}

fn does_not_see_any_player_holding_loved_item(cx: &mut Cx) -> bool {
    !cx.b.mem.has(Mem::NearestPlayerHoldingWantedItem)
}

fn is_not_holding_loved_item_in_offhand_cx(cx: &Cx) -> bool {
    is_not_holding_loved_item_in_offhand(cx.m)
}

fn wants_to_stop_fleeing(cx: &mut Cx) -> bool {
    let Some(a) = cx.b.mem.entity(Mem::AvoidTarget) else { return true };
    let Some(t) = cx.level.entity(a).map(|o| o.type_name) else { return true };
    if t == HOGLIN {
        return !hoglins_outnumber_piglins(cx);
    }
    if is_zombified_type(t) {
        return !cx.b.mem.is(Mem::NearestVisibleZombified, &Val::Entity(a));
    }
    false
}

/// `PiglinAi.wasHurtRecently`.
fn was_hurt_recently(cx: &Cx, id: i32) -> bool {
    mob_data(cx, id).and_then(|m| m.brain.as_ref()).is_some_and(|b| b.st.mem.has(Mem::HurtBy))
}

/// `PiglinAi.wantsToStopRiding`: the vehicle is no baby, is dead, either was hurt lately, or is a
/// piglin that itself rides nothing.
fn wants_to_stop_riding(cx: &Cx, vehicle: i32) -> bool {
    let Some(v) = cx.level.entity(vehicle) else { return false };
    let Some(vm) = mob::data(v) else { return false };
    let hurt_me = cx.b.mem.has(Mem::HurtBy);
    !vm.baby() || !v.is_alive() || hurt_me || was_hurt_recently(cx, vehicle) || (v.type_name == PIGLIN && v.vehicle.is_none())
}

fn look_at_player_holding_loved(cx: &Cx, id: i32) -> bool {
    player_holding_loved_item(cx, id)
}

fn look_is_player(cx: &Cx, id: i32) -> bool {
    living_now(cx, id).is_some_and(|l| l.type_name == PLAYER)
}

fn look_is_piglin(cx: &Cx, id: i32) -> bool {
    living_now(cx, id).is_some_and(|l| l.type_name == PIGLIN)
}

fn look_any(_cx: &Cx, _id: i32) -> bool {
    true
}

/// `PiglinAi.createLookBehaviors` (weights of one).
fn look_behaviors() -> Vec<(Box<dyn Control>, i32)> {
    vec![
        (set_entity_look_target(look_is_player, 8.0), 1),
        (set_entity_look_target(look_is_piglin, 8.0), 1),
        (set_entity_look_target(look_any, 8.0), 1),
    ]
}

/// `PiglinAi.createIdleLookBehaviors`.
fn idle_look_behaviors() -> Box<dyn Control> {
    let mut v = look_behaviors();
    v.push((DoNothing::new(30, 60), 1));
    Gate::run_one(v)
}

/// `PiglinAi.createIdleMovementBehaviors`.
fn idle_movement_behaviors() -> Box<dyn Control> {
    Gate::run_one(vec![
        (stroll(0.6, StrollKind::Land { avoid_water: true }), 2),
        (interact_with(PIGLIN, 8, 0.6, 2), 2),
        (trigger_if(does_not_see_any_player_holding_loved_item, set_walk_target_from_look_target(0.6, 3)), 2),
        (DoNothing::new(30, 60), 1),
    ])
}

fn avoid_repellent() -> Box<dyn Control> {
    SetWalkTargetAwayFrom::pos(Mem::NearestRepellent, 1.0, 8, false)
}

fn baby_avoid_nemesis() -> Box<dyn Control> {
    CopyMemoryWithExpiry::new(is_baby_cx, Mem::NearestVisibleNemesis, Mem::AvoidTarget, BABY_AVOID_NEMESIS_DURATION)
}

fn avoid_zombified() -> Box<dyn Control> {
    CopyMemoryWithExpiry::new(is_near_zombified, Mem::NearestVisibleZombified, Mem::AvoidTarget, AVOID_ZOMBIFIED_DURATION)
}

fn baby_sometimes_ride_baby_hoglin() -> Box<dyn Control> {
    let ticker = RideTicker { ticks: 0 };
    Shot::new(ticker)
}

/// `babySometimesRideBabyHoglin`: `CopyMemoryWithExpiry` whose predicate is a baby and the
/// `SetEntityLookTargetSometimes.Ticker` of `RIDE_START_INTERVAL`.
#[derive(Clone, Debug)]
struct RideTicker {
    ticks: i32,
}

impl ShotBehavior for RideTicker {
    fn name(&self) -> &'static str {
        "CopyMemoryWithExpiry"
    }
    fn entry(&self) -> &'static [(Mem, Status)] {
        &[(Mem::NearestVisibleBabyHoglin, ValuePresent), (Mem::RideTarget, ValueAbsent)]
    }
    fn trigger(&mut self, cx: &mut Cx) -> bool {
        if !cx.m.baby() {
            return false;
        }
        // `Ticker.tickDownAndCheck(level.getRandom())`.
        let fire = if self.ticks == 0 {
            self.ticks = uniform_ticks(cx, RIDE_START_INTERVAL) - 1;
            false
        } else {
            self.ticks -= 1;
            self.ticks == 0
        };
        if !fire {
            return false;
        }
        let Some(v) = cx.b.mem.get(Mem::NearestVisibleBabyHoglin).cloned() else { return false };
        let n = sample(cx.rng(), RIDE_DURATION.0, RIDE_DURATION.1) as i64;
        cx.b.mem.set_expiring(Mem::RideTarget, v, n);
        true
    }
    fn box_clone(&self) -> Box<dyn ShotBehavior> {
        Box::new(self.clone())
    }
}

fn uniform_ticks(cx: &mut Cx, r: (i32, i32)) -> i32 {
    sample(cx.rng(), r.0, r.1)
}

fn numbered(start: i32, list: Vec<Box<dyn Control>>) -> Vec<(i32, Box<dyn Control>)> {
    list.into_iter().enumerate().map(|(i, b)| (start + i as i32, b)).collect()
}

/// `PiglinAi.getActivities` and the sensors of `Piglin.BRAIN_PROVIDER`.
fn make_brain(random: &mut dyn RandomSource) -> Brain {
    let memories = [Mem::UniversalAnger, Mem::AteRecently, Mem::SpearFleeingTime, Mem::SpearFleeingPosition, Mem::SpearChargePosition, Mem::SpearEngageTime];
    let sensors: Vec<Box<dyn brain::Sensor>> = vec![
        Box::new(sensors::NearestLivingEntities),
        Box::new(sensors::Players),
        Box::new(NearestItems { wants: sensor_wants }),
        Box::new(sensors::HurtBy),
        Box::new(PiglinSpecific),
    ];
    let core = ActivityData::create(
        Activity::Core,
        0,
        vec![
            LookAtTargetSink::new(45, 90),
            MoveToTargetSink::new(),
            InteractWithDoor::new(),
            baby_avoid_nemesis(),
            avoid_zombified(),
            stop_holding_item_if_no_longer_admiring(),
            start_admiring_item_if_seen(ADMIRE_DURATION),
            start_celebrating_if_target_dead(300, wants_to_dance),
            stop_being_angry_if_target_dead(),
        ],
    );
    let idle = ActivityData::create(
        Activity::Idle,
        10,
        vec![
            set_entity_look_target(look_at_player_holding_loved, 14.0),
            start_attacking(is_adult, find_nearest_valid_attack_target),
            trigger_if(can_hunt_cx, start_hunting_hoglin()),
            avoid_repellent(),
            baby_sometimes_ride_baby_hoglin(),
            idle_look_behaviors(),
            idle_movement_behaviors(),
            set_look_and_interact(PLAYER, 4),
        ],
    );
    let admire = ActivityData::full(
        Activity::AdmireItem,
        numbered(
            10,
            vec![
                go_to_wanted_item(is_not_holding_loved_item_in_offhand_cx, 1.0, true, 9),
                stop_admiring_if_item_too_far_away(9),
                stop_admiring_if_tired_of_trying_to_reach_item(200, 200),
            ],
        ),
        &[(Mem::AdmiringItem, ValuePresent)],
        &[Mem::AdmiringItem],
    );
    let fight = ActivityData::full(
        Activity::Fight,
        numbered(
            10,
            vec![
                stop_attacking_if_target_invalid(|cx, t| find_nearest_valid_attack_target(cx) != Some(t.id), |_, _| {}, true),
                trigger_if(has_crossbow, back_up_if_too_close(5, 0.75)),
                set_walk_target_from_attack_target_if_out_of_reach(|_| 1.0),
                super::spear_brain::SpearApproach::new(1.0, 10.0),
                super::spear_brain::SpearAttack::new(1.0, 1.0, 2.0),
                super::spear_brain::SpearRetreat::new(1.0),
                melee_attack(20),
                CrossbowAttack::new(),
                remember_if_hoglin_was_killed(),
                erase_memory_if(is_near_zombified, Mem::AttackTarget),
            ],
        ),
        &[(Mem::AttackTarget, ValuePresent)],
        &[Mem::AttackTarget],
    );
    let celebrate = ActivityData::full(
        Activity::Celebrate,
        numbered(
            10,
            vec![
                avoid_repellent(),
                set_entity_look_target(look_at_player_holding_loved, 14.0),
                start_attacking(is_adult, find_nearest_valid_attack_target),
                trigger_if(is_not_dancing, go_to_target_location(Mem::CelebrateLocation, 2, 1.0)),
                trigger_if(is_dancing, go_to_target_location(Mem::CelebrateLocation, 4, 0.6)),
                Gate::run_one(vec![
                    (set_entity_look_target(look_is_piglin, 8.0), 1),
                    (stroll(0.6, StrollKind::LandRange { h: 2, v: 1 }), 1),
                    (DoNothing::new(10, 20), 1),
                ]),
            ],
        ),
        &[(Mem::CelebrateLocation, ValuePresent)],
        &[Mem::CelebrateLocation],
    );
    let avoid = ActivityData::full(
        Activity::Avoid,
        numbered(
            10,
            vec![
                SetWalkTargetAwayFrom::entity(Mem::AvoidTarget, 1.0, 12, true),
                idle_look_behaviors(),
                idle_movement_behaviors(),
                erase_memory_if(wants_to_stop_fleeing, Mem::AvoidTarget),
            ],
        ),
        &[(Mem::AvoidTarget, ValuePresent)],
        &[Mem::AvoidTarget],
    );
    let ride = ActivityData::full(
        Activity::Ride,
        numbered(
            10,
            vec![
                mount(0.8),
                set_entity_look_target(look_at_player_holding_loved, 8.0),
                trigger_if(is_passenger, TriggerGate::one_shuffled(ride_triggers())),
                dismount_or_skip_mounting(8, wants_to_stop_riding),
            ],
        ),
        &[(Mem::RideTarget, ValuePresent)],
        &[Mem::RideTarget],
    );
    Brain::new(&memories, sensors, vec![core, idle, admire, fight, celebrate, avoid, ride], random)
}

/// The riding piglin's look behaviours plus a trigger that always succeeds.
fn ride_triggers() -> Vec<(Box<dyn Control>, i32)> {
    let mut v = look_behaviors();
    v.push((trigger_if(always, shot("", &[], |_| true)), 1));
    v
}

// ---------------------------------------------------------------------------- AbstractPiglin

/// `AbstractPiglin.isConverting`.
pub fn is_converting(e: &Entity, m: &MobData, level: &dyn EntityLevel) -> bool {
    let _ = e;
    !state(m).is_some_and(|s| s.immune_to_zombification) && !m.no_ai && level.piglins_zombify()
}

/// `AbstractPiglin.customServerAiStep`'s conversion (`piglin`: the type's own drop of what it
/// carries first).
pub fn tick_conversion(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let converting = is_converting(e, m, &*level);
    let Some(st) = state_mut(m) else { return };
    if converting {
        st.time_in_overworld += 1;
    } else {
        st.time_in_overworld = 0;
    }
    if st.time_in_overworld <= CONVERSION_TIME {
        return;
    }
    if level.difficulty() != 0 {
        let sound = if m.kind == MobKind::PiglinBrute { "minecraft:entity.piglin_brute.converted_to_zombified" } else { "minecraft:entity.piglin.converted_to_zombified" };
        mob::make_sound(e, m, level, mob::sound_event(sound));
    }
    finish_conversion(e, m, level);
}

/// `Piglin.finishConversion` (`cancelAdmiring`, the inventory drops) then
/// `AbstractPiglin.finishConversion` (a zombified piglin with nausea).
fn finish_conversion(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if m.kind == MobKind::Piglin {
        if let Some(b) = m.brain.take() {
            cancel_admiring(e, m, level, &b.st.mem);
            m.brain = Some(b);
        }
        let items = state_mut(m).map(|s| std::mem::take(&mut s.inventory)).unwrap_or_default();
        for s in items {
            mob::spawn_at_location(e, level, s);
        }
        if let Some(s) = state_mut(m) {
            s.inventory = vec![ItemStack::empty(); 8];
        }
    }
    crate::mob::convert::convert_to(e, m, level, MobKind::ZombifiedPiglin, true, true, |ne, nm, level| {
        if let Some(fx) = crate::effect::Effect::named("minecraft:nausea", 200, 0) {
            crate::mob::effects::add(ne, nm, level, fx, None);
        }
    });
}

// ---------------------------------------------------------------------------- Kind

/// `PiglinAi.getSoundForActivity`.
fn sound_for_activity(e: &Entity, m: &MobData, level: &dyn EntityLevel, mem: &Memories, a: Activity) -> &'static str {
    let s = |n: &str| mob::sound_event(n);
    if a == Activity::Fight {
        return s("minecraft:entity.piglin.angry");
    }
    if is_converting(e, m, level) {
        return s("minecraft:entity.piglin.retreat");
    }
    let near_avoid = mem
        .entity(Mem::AvoidTarget)
        .and_then(|id| level.entity(id).map(|o| o.position()).or_else(|| level.player(id).map(|p| p.pos)))
        .is_some_and(|p| e.position().distance_to_sqr(p) < 144.0);
    if a == Activity::Avoid && near_avoid {
        return s("minecraft:entity.piglin.retreat");
    }
    if a == Activity::AdmireItem {
        return s("minecraft:entity.piglin.admiring_item");
    }
    if a == Activity::Celebrate {
        return s("minecraft:entity.piglin.celebrate");
    }
    if mem.has(Mem::NearestPlayerHoldingWantedItem) {
        return s("minecraft:entity.piglin.jealous");
    }
    if mem.has(Mem::NearestRepellent) {
        return s("minecraft:entity.piglin.retreat");
    }
    s("minecraft:entity.piglin.ambient")
}

/// `PiglinAi.updateActivity`.
fn update_activity(cx: &mut Cx) {
    let old = cx.b.active_non_core();
    cx.b.set_active_activity_to_first_valid(&[Activity::AdmireItem, Activity::Fight, Activity::Avoid, Activity::Celebrate, Activity::Ride, Activity::Idle]);
    let new = cx.b.active_non_core();
    if old != new
        && let Some(a) = new
    {
        let sound = sound_for_activity(cx.e, cx.m, &*cx.level, &cx.b.mem, a);
        mob::make_sound(cx.e, cx.m, cx.level, sound);
    }
    let aggressive = cx.b.mem.has(Mem::AttackTarget);
    cx.m.set_aggressive(aggressive);
    if !cx.b.mem.has(Mem::RideTarget) && is_baby_riding_baby(cx) {
        stop_riding(cx);
    }
    if !cx.b.mem.has(Mem::CelebrateLocation) {
        cx.b.mem.erase(Mem::Dancing);
    }
    let dancing = cx.b.mem.has(Mem::Dancing);
    if let Some(s) = state_mut(cx.m) {
        s.dancing = dancing;
    }
}

/// `PiglinAi.isBabyRidingBaby`.
fn is_baby_riding_baby(cx: &Cx) -> bool {
    if !cx.m.baby() {
        return false;
    }
    let Some(v) = cx.e.vehicle.and_then(|id| cx.level.entity(id)) else { return false };
    (v.type_name == PIGLIN || v.type_name == HOGLIN) && mob::data(v).is_some_and(|m| m.baby())
}

/// Reacts to the hits recorded for the next brain tick.
fn process_pending_hurt(cx: &mut Cx) {
    let pending = state_mut(cx.m).map(|s| std::mem::take(&mut s.pending_hurt)).unwrap_or_default();
    for id in pending {
        if let Some(a) = living_now(cx, id) {
            if cx.m.kind == MobKind::PiglinBrute {
                crate::mob::kinds::piglin_brute::was_hurt_by(cx, &a);
            } else {
                was_hurt_by(cx, &a);
            }
        }
    }
}

/// `Piglin.hurtServer`'s reaction, now or (for an attacker in the middle of its own tick) at the
/// start of the next brain tick.
pub fn on_hurt(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, hurt: bool) {
    if !hurt {
        return;
    }
    let Some(a) = source.attacker else { return };
    let Some(attacker) = living_or_ticking(&*level, a) else {
        // The attacker is not in the level (it is being ticked): react next tick, unless it is
        // not a living entity at all.
        if let Some(s) = state_mut(m) {
            s.pending_hurt.push(a);
        }
        return;
    };
    let Some(mut brain) = m.brain.take() else { return };
    {
        let time = level.game_time();
        let mut cx = Cx { e, m, level, b: &mut brain.st, time };
        if cx.m.kind == MobKind::PiglinBrute {
            crate::mob::kinds::piglin_brute::was_hurt_by(&mut cx, &attacker);
        } else {
            was_hurt_by(&mut cx, &attacker);
        }
    }
    m.brain = Some(brain);
}

/// `PiglinAi.createSpawnWeapon` (from the piglin's own random).
fn spawn_weapon(random: &mut LegacyRandom) -> &'static str {
    if (random.next_float() as f64) < 0.5 {
        return "minecraft:crossbow";
    }
    if random.next_int_bounded(10) == 0 { "minecraft:golden_spear" } else { "minecraft:golden_sword" }
}

impl Kind for Piglin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.can_pick_up_loot = true;
        // `AbstractPiglin`: opens doors, fears fire.
        m.nav.can_open_doors = true;
        crate::mob::kinds::tame::set_malus(m, crate::mob::path::PathType::FireInNeighbor, 16.0);
        crate::mob::kinds::tame::set_malus(m, crate::mob::path::PathType::Fire, -1.0);
        Some(Box::new(PiglinState { inventory: vec![ItemStack::empty(); 8], ..PiglinState::default() }))
    }

    /// No goals: the brain does it all.
    fn register_goals(&self, _m: &mut MobData) {}

    fn make_brain(&self, _m: &MobData, random: &mut dyn RandomSource) -> Option<Brain> {
        Some(make_brain(random))
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        pick_up_loot(e, m, level);
    }

    /// The brain, `PiglinAi.updateActivity`, then `AbstractPiglin`'s zombification.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                process_pending_hurt(&mut cx);
            }
            m.brain = Some(b);
        }
        set_ticking(Some((&*e, &*m)));
        brain::tick_brain(e, m, level);
        set_ticking(None);
        if let Some(mut b) = m.brain.take() {
            let time = level.game_time();
            {
                let mut cx = Cx { e, m, level, b: &mut b.st, time };
                update_activity(&mut cx);
            }
            m.brain = Some(b);
        }
        sync_target(m, &*level);
        tick_conversion(e, m, level);
    }

    fn update_using_item(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        crate::mob::kinds::pillager::crossbow_use_tick(e, m, level);
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        on_hurt(e, m, level, source, hurt);
        sync_target(m, &*level);
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, _group: &mut GroupData) {
        // Not a structure spawn: a baby, or an adult with a weapon.
        if r.next_float() < 0.2 {
            set_baby(e, m, true);
        } else if let Some(s) = ItemStack::of(spawn_weapon(&mut e.random), 1) {
            m.equipment[MAINHAND] = s;
        }
        // `PiglinAi.initMemories`: hunted recently for 30-120 s.
        let hunted = sample(r, TIME_BETWEEN_HUNTS.0, TIME_BETWEEN_HUNTS.1);
        if let Some(b) = m.brain.as_mut() {
            b.st.mem.set_expiring(Mem::HuntedRecently, Val::Bool(true), hunted as i64);
        }
        // `populateDefaultEquipmentSlots`: each gold armor piece at 10%.
        if !m.baby() {
            for (slot, name) in [(mob::HEAD, "minecraft:golden_helmet"), (mob::CHEST, "minecraft:golden_chestplate"), (mob::LEGS, "minecraft:golden_leggings"), (mob::FEET, "minecraft:golden_boots")] {
                if r.next_float() < 0.1
                    && let Some(s) = ItemStack::of(name, 1)
                {
                    m.equipment[slot] = s;
                }
            }
        }
        // `populateDefaultEquipmentEnchantments`.
        super::zombie::populate_enchantments(m, r, ctx);
        ext::mob_finalize(m, r);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let baby = r.bool_or("IsBaby", false);
        let cannot_hunt = r.bool_or("CannotHunt", false);
        let immune = r.bool_or("IsImmuneToZombification", false);
        let time = r.int_or("TimeInOverworld", 0);
        let loot = r.bool_or("CanPickUpLoot", true);
        let inventory = r.get("Inventory").and_then(Tag::as_list).map(|l| l.iter().filter_map(|t| ItemStack::from_nbt(t).ok()).collect::<Vec<_>>());
        set_baby(e, m, baby);
        m.can_pick_up_loot = loot;
        let Some(st) = state_mut(m) else { return };
        st.cannot_hunt = cannot_hunt;
        st.immune_to_zombification = immune;
        st.time_in_overworld = time;
        if let Some(items) = inventory {
            st.inventory = vec![ItemStack::empty(); 8];
            for s in items {
                let _ = add_to_inventory(st, s);
            }
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let Some(st) = state(m) else { return };
        if st.immune_to_zombification {
            o.put("IsImmuneToZombification", Tag::Byte(1));
        }
        o.put("TimeInOverworld", Tag::Int(st.time_in_overworld));
        if m.baby() {
            o.put("IsBaby", Tag::Byte(1));
        }
        if st.cannot_hunt {
            o.put("CannotHunt", Tag::Byte(1));
        }
        o.put("Inventory", Tag::List(st.inventory.iter().filter(|s| !s.is_empty()).map(ItemStack::to_nbt).collect()));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data;
        let st = state(m);
        d.set(data::abstract_piglin::IMMUNE_TO_ZOMBIFICATION, &DataValue::Boolean(st.is_some_and(|s| s.immune_to_zombification)));
        d.set(data::piglin::BABY, &DataValue::Boolean(m.baby()));
        d.set(data::piglin::IS_CHARGING_CROSSBOW, &DataValue::Boolean(st.is_some_and(|s| s.charging_crossbow)));
        d.set(data::piglin::IS_DANCING, &DataValue::Boolean(st.is_some_and(|s| s.dancing)));
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        // `PiglinAi.mobInteract`: an adult not admiring takes one gold ingot.
        let mut brain = m.brain.take()?;
        let can_admire = !brain.st.mem.has(Mem::AdmiringDisabled) && !brain.st.mem.has(Mem::AdmiringItem) && !m.baby() && is_item(stack, BARTERING_ITEM);
        if !can_admire {
            m.brain = Some(brain);
            return None;
        }
        hold_in_offhand(e, m, level, stack.with_count(1));
        admire_gold_item(&mut brain.st.mem);
        stop_walking(m, &mut brain.st.mem);
        m.brain = Some(brain);
        Some(Outcome { success: true, held: HeldChange::Consume(1), shear: None, player_sound: None, ride: false, open_container: false, sheared: None })
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.49, 0.98, 0.78) } else { base }
    }

    /// `AbstractPiglin.playAmbientSound`: only while idle; the sound follows the activity.
    fn ambient_sound(&self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let b = m.brain.as_ref()?;
        if !b.st.is_active(Activity::Idle) {
            return Some(None);
        }
        let a = b.st.active_non_core().unwrap_or(Activity::Idle);
        Some(Some(sound_for_activity(e, m, level, &b.st.mem, a)))
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        // `checkPiglinSpawnRules`: anywhere but on nether wart blocks.
        Some(crate::blocks::block_name(view.block(pos.below())) != "minecraft:nether_wart_block")
    }
}

/// The mob's target follows its brain's attack target (`Mob.getTargetFromBrain`).
pub fn sync_target(m: &mut MobData, level: &dyn EntityLevel) {
    let t = m.brain.as_ref().and_then(|b| b.st.mem.entity(Mem::AttackTarget));
    let _ = level;
    m.target = t;
}
