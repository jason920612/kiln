//! A player's right click on a mob (`Player.interactOn` → `Mob.interact` → `mobInteract`, then
//! the held item's `interactLivingEntity`): feeding animals, milking cows, shearing and dyeing
//! sheep. What happens to the held item is returned for the simulation to apply to the
//! player's inventory.

use super::{MobData, MobKind, Species, breed};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use kiln_item::ItemStack;

/// The player doing the clicking.
#[derive(Clone, Copy, Debug)]
pub struct Interactor {
    pub id: i32,
    /// `hasInfiniteMaterials` (creative): consumed items are not taken.
    pub creative: bool,
    /// `isSecondaryUseActive` (sneaking).
    pub sneaking: bool,
    /// Spectator mode.
    pub spectator: bool,
    /// Where the click hit the entity, relative to its position.
    pub hit: crate::math::Vec3,
}

/// What becomes of the held stack.
#[derive(Clone, Debug, PartialEq)]
pub enum HeldChange {
    None,
    /// `ItemStack.consume(n, player)`: taken unless the player is creative.
    Consume(i32),
    /// `ItemUtils.createFilledResult`: one of the held items becomes this (a milk bucket).
    Fill(ItemStack),
    /// `hurtAndBreak(n)`: durability lost (none in creative).
    Damage(i32),
    /// `ItemStack.shrink(n)`: taken in every game mode (a lead put on a mob).
    Shrink(i32),
    /// `Player.setItemInHand`: the held stack becomes this (an armor stand's swap).
    Replace(ItemStack),
}

/// `InteractionResult`, as far as the caller cares.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// `consumesAction`: the click did something (the hand swings).
    pub success: bool,
    pub held: HeldChange,
    /// A sheep was sheared: the shearing loot table to drop (see [`super::species::shear`]).
    pub shear: Option<String>,
    /// The player to play a sound to (`Player.playSound`: the milking sound).
    pub player_sound: Option<&'static str>,
    /// The player gets on the mob (`startRiding`).
    pub ride: bool,
    /// The player opens the entity's container menu (a chest or hopper minecart).
    pub open_container: bool,
    /// `Mob.shearItem`: the equipment the shears took off (for the `player_sheared_equipment` trigger).
    pub sheared: Option<ItemStack>,
}

impl Outcome {
    pub const PASS: Outcome = Outcome { success: false, held: HeldChange::None, shear: None, player_sound: None, ride: false, open_container: false, sheared: None };

    pub fn success(held: HeldChange) -> Outcome {
        Outcome { success: true, held, shear: None, player_sound: None, ride: false, open_container: false, sheared: None }
    }
}

fn is(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && super::item_name(stack) == name
}

/// `Player.interactOn` for a mob: the mob's own handler first, then the held item's.
pub fn interact(e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Outcome {
    // `Mob.checkAndHandleImportantInteractions`: a named name tag names the mob before anything else reacts.
    if let Some(out) = name_tag(e, stack) {
        level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(who.id) });
        return out;
    }
    // `Entity.interact`'s share (leads and shears) comes before the type's own handler, for
    // living mobs and for boats.
    if crate::leash::is_leashable(e) && super::data(e).is_none_or(|m| super::is_alive(e, m)) && let Some(out) = crate::leash::interact(e, level, who, stack) {
        if out.success {
            level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(who.id) });
        }
        return out;
    }
    if super::data(e).is_none() {
        // Extension entities with a click of their own (boats).
        let placeholder = crate::entity::EntityKind::Other { type_name: e.type_name };
        if let crate::entity::EntityKind::Ext(mut x) = std::mem::replace(&mut e.kind, placeholder) {
            let out = x.interact(e, level, who, stack);
            e.kind = crate::entity::EntityKind::Ext(x);
            if let Some(out) = out {
                if out.success {
                    level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(who.id) });
                }
                return out;
            }
        }
        return Outcome::PASS;
    }
    let mut m = super::take(e);
    // `Entity.interact`: shears take off the equipment that can be sheared (after the leads).
    let sheared = shear_equipment(e, &mut m, level, who, stack);
    let mut out = match sheared {
        Some(out) => out,
        None => mob_interact(e, &mut m, level, who, stack),
    };
    if !out.success && !stack.is_empty() {
        out = item_interact(e, &mut m, level, who, stack);
    }
    if out.success {
        level.emit(Event::GameEvent { event: "minecraft:entity_interact", pos: e.position(), entity: Some(who.id) });
    }
    super::put(e, m);
    out
}

/// `Mob.interact` → the types' `mobInteract`.
fn mob_interact(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Outcome {
    if !super::is_alive(e, m) {
        return Outcome::PASS;
    }
    if let Some(o) = m.kind.ext().and_then(|k| k.interact(e, m, level, who, stack)) {
        return o;
    }
    match m.kind {
        MobKind::Cow if is(stack, "minecraft:bucket") && !m.baby() => {
            let mut out = Outcome::success(HeldChange::Fill(ItemStack::of("minecraft:milk_bucket", 1).unwrap_or_else(ItemStack::empty)));
            out.player_sound = Some("minecraft:entity.cow.milk");
            return out;
        }
        MobKind::Sheep if is(stack, "minecraft:shears") => {
            let ready = matches!(m.species, Species::Sheep { sheared: false, .. }) && !m.baby();
            if !ready {
                // `CONSUME`: nothing happens, but the click is taken.
                return Outcome { success: true, held: HeldChange::None, shear: None, player_sound: None, ride: false, open_container: false, sheared: None };
            }
            let table = super::species::shear_table(m);
            level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.sheep.shear", source: "players", volume: 1.0, pitch: 1.0 });
            level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
            let mut out = Outcome::success(HeldChange::Damage(1));
            out.shear = table;
            return out;
        }
        _ => {}
    }
    if breed::is_animal(m.kind) {
        return animal_interact(e, m, level, who, stack);
    }
    Outcome::PASS
}

/// `Animal.mobInteract`: food makes an adult fall in love, or a baby grow up faster.
pub fn animal_interact(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Outcome {
    if stack.is_empty() || !breed::is_food(m.kind, stack.item()) {
        return Outcome::PASS;
    }
    let age = m.age;
    if age == 0 && m.in_love <= 0 {
        breed::set_in_love(e, m, level, Some(who.id));
        play_eating_sound(e, m, level);
        return Outcome::success(HeldChange::Consume(1));
    }
    if age < 0 && !m.age_locked {
        let seconds = breed::speed_up_seconds_when_feeding(-age);
        super::age_up(e, m, seconds, true);
        play_eating_sound(e, m, level);
        return Outcome::success(HeldChange::Consume(1));
    }
    Outcome::PASS
}

/// `Animal.playEatingSound` (pigs grunt their eating sound; the others are silent).
fn play_eating_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel) {
    if m.kind == MobKind::Pig {
        let sound = super::sound_event(if m.baby() { "minecraft:entity.baby_pig.eat" } else { "minecraft:entity.pig.eat" });
        super::make_sound(e, m, level, sound);
    }
    // `Nautilus.playEatingSound`.
    if matches!(m.kind, MobKind::Nautilus | MobKind::ZombieNautilus) {
        super::kinds::nautilus::play_eating_sound(e, m, level);
    }
    // `Frog.playEatingSound`: a loud gulp, by the level.
    if m.kind == MobKind::Frog {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.frog.eat", source: "neutral", volume: 2.0, pitch: 1.0 });
    }
}

/// `NameTagItem.interactLivingEntity` of a named tag on a living mob, which then never despawns.
fn name_tag(e: &mut Entity, stack: &ItemStack) -> Option<Outcome> {
    if !is(stack, "minecraft:name_tag") {
        return None;
    }
    let name = stack.get(kiln_item::keys::CUSTOM_NAME)?.nbt().clone();
    let alive = e.is_alive();
    let m = super::data_mut(e)?;
    if !alive || m.health <= 0.0 {
        return None;
    }
    m.persistence_required = true;
    e.extra.retain(|(k, _)| k != "CustomName");
    e.extra.push(("CustomName".into(), name));
    Some(Outcome::success(HeldChange::Consume(1)))
}

/// `Mob.attemptToShearEquipment` for shears in hand: the first worn piece that can be sheared
/// comes off (and drops where the mob's passengers would sit), the shears wear 1 and the shear
/// sound plays. `None`: nothing to take, or the type does not let this player.
fn shear_equipment(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
    use kiln_item::component::EquipmentSlot;
    // (A dispenser, who.id < 0, only shears what is `Shearable`.)
    if !is(stack, "minecraft:shears") || !super::is_alive(e, m) || who.sneaking || who.id < 0 {
        return None;
    }
    // `canShearEquipment`: a wolf lets only its owner, the others unless they carry a passenger.
    let allowed = match m.kind.ext().and_then(|k| k.can_shear_equipment(m, &*level, who.id)) {
        Some(allowed) => allowed,
        None => e.passengers.is_empty(),
    };
    if !allowed {
        return None;
    }
    let curse = kiln_item::registry::ENCHANTMENT.id("minecraft:binding_curse");
    for slot in [
        EquipmentSlot::MainHand,
        EquipmentSlot::OffHand,
        EquipmentSlot::Feet,
        EquipmentSlot::Legs,
        EquipmentSlot::Chest,
        EquipmentSlot::Head,
        EquipmentSlot::Body,
        EquipmentSlot::Saddle,
    ] {
        let index = slot as usize;
        let worn = if index < 6 {
            m.equipment[index].clone()
        } else {
            m.kind.ext().and_then(|k| k.extra_equipment(m).into_iter().find(|(s, _)| *s as usize == index)).map(|(_, s)| s).unwrap_or_default()
        };
        let Some(equippable) = worn.get(kiln_item::keys::EQUIPPABLE) else { continue };
        if !equippable.can_be_sheared {
            continue;
        }
        let locked = curse.is_some_and(|c| worn.get(kiln_item::keys::ENCHANTMENTS).is_some_and(|en| en.level(c) > 0));
        if locked && !who.creative {
            continue;
        }
        let sound = match &equippable.shearing_sound {
            kiln_item::Holder::Reference(id) => kiln_data::builtin_entries("minecraft:sound_event").and_then(|n| n.get(*id as usize).copied()),
            kiln_item::Holder::Direct(_) => None,
        };
        // `shearItem`: `setItemSlot(slot, EMPTY)` (the unequip game event), the shear game event, the piece drops.
        if index < 6 {
            m.equipment[index] = ItemStack::empty();
            super::sync_equipment_modifiers(m);
        } else if let Some(k) = m.kind.ext() {
            k.remove_extra_equipment(m, index as u8);
        }
        level.emit(Event::GameEvent { event: "minecraft:unequip", pos: e.position(), entity: Some(e.id) });
        level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
        super::spawn_at(e.position() + crate::ride::passenger_attachment_unrotated(e, m), level, worn.clone(), 0.0);
        if let Some(sound) = sound
            && !e.silent
        {
            level.emit(Event::Sound { pos: e.position(), sound, source: m.kind.sound_source(), volume: 1.0, pitch: 1.0 });
        }
        let mut out = Outcome::success(HeldChange::Damage(1));
        out.sheared = Some(worn);
        return Some(out);
    }
    None
}

/// The held item's `interactLivingEntity` (`DyeItem` on sheep).
fn item_interact(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Outcome {
    let _ = who;
    if let Some(color) = dye_color(stack)
        && let Species::Sheep { color: c, sheared: false } = &mut m.species
        && *c != color
        && e.is_alive()
        && m.health > 0.0
    {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:item.dye.use", source: "players", volume: 1.0, pitch: 1.0 });
        *c = color;
        return Outcome::success(HeldChange::Consume(1));
    }
    Outcome::PASS
}

/// `DataComponents.DYE` of the vanilla dyes.
pub fn dye_color(stack: &ItemStack) -> Option<u8> {
    const NAMES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    if stack.is_empty() {
        return None;
    }
    let name = super::item_name(stack).strip_prefix("minecraft:")?.strip_suffix("_dye")?;
    NAMES.iter().position(|n| *n == name).map(|i| i as u8)
}
