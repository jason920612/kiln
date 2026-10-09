//! What a dispenser may do to a mob in front of it: the rules of `LivingEntity.canEquipWithDispenser`
//! (`canUseSlot`, `canDispenserEquipIntoSlot`) per type, and the changes it makes.

use super::{MobData, MobKind, data, data_mut};
use crate::entity::Entity;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;

/// What the rules ask of a mob.
#[derive(Clone, Copy, Debug)]
pub struct Facts {
    pub kind: MobKind,
    pub baby: bool,
    pub tamed: bool,
    pub pick_up_loot: bool,
    /// A bit per `EquipmentSlot` ordinal that holds something.
    pub worn: u8,
    /// The item id in the body slot (a sulfur cube's), 0 for none.
    pub body_item: i32,
}

fn is_horse_like(kind: MobKind) -> bool {
    use MobKind::*;
    matches!(kind, Horse | Donkey | Mule | SkeletonHorse | ZombieHorse | Llama | TraderLlama | Camel | CamelHusk)
}

/// The facts about the mob `e`, `None` for anything else.
pub fn facts(e: &Entity) -> Option<Facts> {
    let m = data(e)?;
    let mut worn = 0u8;
    for (i, s) in m.equipment.iter().enumerate() {
        if !s.is_empty() {
            worn |= 1 << i;
        }
    }
    let mut body_item = 0;
    if let Some(k) = m.kind.ext() {
        for (slot, stack) in k.extra_equipment(m) {
            worn |= 1 << slot;
            if slot == 6 {
                body_item = stack.item();
            }
        }
    }
    let tamed = is_horse_like(m.kind) && super::kinds::horse::is_tamed(m);
    Some(Facts { kind: m.kind, baby: m.baby(), tamed, pick_up_loot: m.can_pick_up_loot, worn, body_item })
}

/// `canUseSlot(slot)` of the type (for a living mob that is alive).
pub fn can_use_slot(f: &Facts, slot: EquipmentSlot) -> bool {
    use MobKind::*;
    match (f.kind, slot) {
        (Pig | Strider, EquipmentSlot::Saddle) => !f.baby,
        (HappyGhast | SulfurCube, EquipmentSlot::Body) => !f.baby,
        // (`Horse`, `ZombieHorse`, `SkeletonHorse` and `Llama` say yes to every slot.)
        (Horse | ZombieHorse | SkeletonHorse | Llama | TraderLlama, _) => true,
        (Donkey | Mule | Camel | CamelHusk, EquipmentSlot::Saddle) => !f.baby && f.tamed,
        (Nautilus | ZombieNautilus, EquipmentSlot::Saddle | EquipmentSlot::Body) => !f.baby && f.tamed,
        _ => true,
    }
}

/// `canDispenserEquipIntoSlot(slot)` of the type.
pub fn can_dispenser_equip_into(f: &Facts, slot: EquipmentSlot) -> bool {
    use MobKind::*;
    match f.kind {
        Allay => false,
        Pig | Strider => slot == EquipmentSlot::Saddle || f.pick_up_loot,
        HappyGhast | SulfurCube => slot == EquipmentSlot::Body,
        Panda | Fox | Dolphin => slot == EquipmentSlot::MainHand && f.pick_up_loot,
        Nautilus | ZombieNautilus => matches!(slot, EquipmentSlot::Body | EquipmentSlot::Saddle) || f.pick_up_loot,
        k if is_horse_like(k) => (matches!(slot, EquipmentSlot::Body | EquipmentSlot::Saddle) && f.tamed) || f.pick_up_loot,
        _ => f.pick_up_loot,
    }
}

/// `setItemSlot` + `setGuaranteedDrop` + `setPersistenceRequired` of a dispenser's `EquipmentDispenseItemBehavior`.
pub fn equip(e: &mut Entity, slot: EquipmentSlot, stack: ItemStack) -> bool {
    let Some(m) = data_mut(e) else { return false };
    let i = slot as usize;
    if i < 6 {
        m.equipment[i] = stack;
        m.drop_chances[i] = 2.0;
        m.persistence_required = true;
        super::sync_equipment_modifiers(m);
        return true;
    }
    let done = { let k = m.kind.ext(); k.is_some_and(|k| k.set_extra_equipment(m, i as u8, stack)) };
    if done {
        m.persistence_required = true;
    }
    done
}

/// `Shearable.readyForShearing` for a living mob: a sheep with its wool, a grown mooshroom, a snow golem with its
/// pumpkin, a bogged with its mushrooms.
pub fn shearable(e: &Entity) -> bool {
    let Some(m) = data(e) else { return false };
    if !super::is_alive(e, m) {
        return false;
    }
    match m.kind {
        MobKind::Sheep => matches!(m.species, super::Species::Sheep { sheared: false, .. }) && !m.baby(),
        k => k.ext().is_some_and(|x| x.ready_for_shearing(m)),
    }
}

/// A chest onto a tame pack animal (`DispenseItemBehavior$2`): whether it took it.
pub fn put_chest(e: &mut Entity) -> bool {
    let Some(m) = data_mut(e) else { return false };
    if !is_horse_like(m.kind) || !super::kinds::horse::is_tamed(m) {
        return false;
    }
    { let k = m.kind.ext(); k.is_some_and(|k| k.put_chest(m)) }
}

/// The data of a mob, for the callers that only read.
pub fn mob(e: &Entity) -> Option<&MobData> {
    data(e)
}
