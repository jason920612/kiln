//! What the saddled mounts that are steered with an item on a stick have in common (pigs with a
//! carrot on a stick, striders with a warped fungus on a stick): the saddle slot, `ItemBasedSteering`
//! (the boost a use of the stick gives) and `Equippable.equipOnTarget` (a saddle, or armor, put on
//! by a click).

use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `Mob.dropChances` of a slot nobody filled by hand (`DEFAULT_EQUIPMENT_DROP_CHANCE`).
pub const DEFAULT_DROP: f32 = 0.085;

/// `EquipmentSlot.SADDLE` and its drop chance.
#[derive(Clone, Debug)]
pub struct Saddle {
    pub stack: ItemStack,
    pub drop: f32,
}

impl Default for Saddle {
    fn default() -> Saddle {
        Saddle { stack: ItemStack::default(), drop: DEFAULT_DROP }
    }
}

impl Saddle {
    pub fn is_saddled(&self) -> bool {
        !self.stack.is_empty()
    }

    /// `setItemSlot(SADDLE, stack)` followed by `setGuaranteedDrop(SADDLE)`.
    pub fn put_guaranteed(&mut self, stack: ItemStack) {
        self.stack = stack;
        self.drop = 2.0;
    }

    /// The saddle and its drop chance from the saved `equipment` and `drop_chances` compounds.
    pub fn load(&mut self, r: &mut Input) {
        if let Some(Tag::Compound(eq)) = r.get("equipment")
            && let Some(sd) = eq.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| ItemStack::from_nbt(v).ok())
        {
            self.stack = sd;
        }
        if let Some(Tag::Compound(dc)) = r.get("drop_chances")
            && let Some(f) = dc.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| v.as_f64())
        {
            self.drop = f as f32;
        }
    }

    /// The saddle into the saved `equipment` and `drop_chances` compounds.
    pub fn save(&self, o: &mut Output) {
        if !self.stack.is_empty() {
            let entry = ("saddle".to_owned(), self.stack.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        if self.drop != DEFAULT_DROP {
            let entry = ("saddle".to_owned(), Tag::Float(self.drop));
            match o.0.iter_mut().find(|(k, _)| k == "drop_chances") {
                Some((_, Tag::Compound(dc))) => dc.push(entry),
                _ => o.put("drop_chances", Tag::Compound(vec![entry])),
            }
        }
    }
}

/// `ItemBasedSteering`: the boost a use of the stick gives while riding. The boost's length is
/// what `DATA_BOOST_TIME` says; the client shapes the speed with it.
#[derive(Clone, Debug, Default)]
pub struct Steering {
    boosting: bool,
    time: i32,
    /// `DATA_BOOST_TIME`.
    pub total: i32,
}

impl Steering {
    /// `ItemBasedSteering.boost`: starts a boost of 140 to 980 ticks unless one is going.
    pub fn boost(&mut self, r: &mut dyn RandomSource) -> bool {
        if self.boosting {
            return false;
        }
        self.boosting = true;
        self.time = 0;
        self.total = r.next_int_bounded(841) + 140;
        true
    }

    /// `ItemBasedSteering.tickBoost` (`boostTime++ > total`).
    pub fn tick_boost(&mut self) {
        if self.boosting {
            let before = self.time;
            self.time += 1;
            if before > self.total {
                self.boosting = false;
            }
        }
    }
}

/// `LivingEntity.onEquipItem` for a stack that went into the empty `slot`: the equip sound
/// (`sound`, else the item's own), whose seed is drawn from the mob's random, and the `equip` game
/// event. Nothing for a silent mob or a mob that has not ticked yet.
pub fn on_equip_item(e: &mut Entity, level: &mut dyn EntityLevel, slot: EquipmentSlot, new: &ItemStack, sound: Option<&'static str>) {
    let Some(equippable) = new.get(kiln_item::keys::EQUIPPABLE) else { return };
    if e.first_tick || e.silent || equippable.slot != slot {
        return;
    }
    let sound = sound.unwrap_or_else(|| match &equippable.equip_sound {
        kiln_item::Holder::Reference(id) => kiln_data::builtin_entries("minecraft:sound_event").and_then(|n| n.get(*id as usize).copied()).unwrap_or("minecraft:item.armor.equip_generic"),
        kiln_item::Holder::Direct(_) => "minecraft:item.armor.equip_generic",
    });
    // `playSeededSound(..., random.nextLong())`.
    e.random.next_long();
    level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch: 1.0 });
    level.emit(Event::GameEvent { event: "minecraft:equip", pos: e.position(), entity: Some(e.id) });
}

/// `Equippable.equipOnTarget` of the held `stack` for `slot` of the mob `e`, whose `canUseSlot`,
/// empty slot and life the caller has checked: whether the item is equippable there, and then the
/// single item that went on.
pub fn equip_on_target(e: &mut Entity, level: &mut dyn EntityLevel, stack: &ItemStack, slot: EquipmentSlot, sound: Option<&'static str>) -> Option<ItemStack> {
    if stack.is_empty() || !super::horse::equippable_in_slot(stack, slot, e.type_name) {
        return None;
    }
    let mut one = stack.clone();
    one.set_count(1);
    on_equip_item(e, level, slot, &one, sound);
    Some(one)
}
