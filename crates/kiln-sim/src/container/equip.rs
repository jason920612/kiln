//! What a dispenser does to the living things in front of it: `EquipmentDispenseItemBehavior.dispenseEquipment`
//! (armor on players, armor stands and mobs that can wear it, saddles and horse armor on mounts), a chest on a
//! pack animal (`DispenseItemBehavior$2`) and an item swallowed by a sulfur cube.
//!
//! The behaviour decides from the bodies as they stood when the phase began ([`Wear`]) and queues what it did
//! ([`DispenseOp`]); the region carries it out on the entities once the block work is done.

use crate::Player;
use crate::blocks::{DispenseOp, RegionLevel, Wear};
use crate::entities::{Body, Entities, Spawn};
use kiln_blocks::{BlockPos, Direction};
use kiln_entity::mob::dispense as rules;
use kiln_inventory::stack::StackExt;
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};

const SLOTS: [EquipmentSlot; 8] = [
    EquipmentSlot::MainHand,
    EquipmentSlot::OffHand,
    EquipmentSlot::Feet,
    EquipmentSlot::Legs,
    EquipmentSlot::Chest,
    EquipmentSlot::Head,
    EquipmentSlot::Body,
    EquipmentSlot::Saddle,
];

/// A player as a dispenser sees it.
pub(crate) fn wear_of_player(p: &Player) -> Wear {
    let mut accepts = 0u8;
    for (i, slot) in SLOTS.iter().enumerate().take(6) {
        if p.inv.equipped(*slot).is_empty() {
            accepts |= 1 << i;
        }
    }
    Wear { id: p.entity_id, type_name: "minecraft:player", open: !p.dead && p.game_mode != 3, accepts, mob: None, shearable: false, leads: false }
}

/// An armor stand or a mob as a dispenser sees it.
pub(crate) fn wear_of(e: &kiln_entity::Entity) -> Option<Wear> {
    if let Some(stand) = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::armor_stand::ArmorStand>(e) {
        let mut accepts = 0u8;
        for (i, slot) in SLOTS.iter().enumerate() {
            if stand.dispenser_accepts(*slot) {
                accepts |= 1 << i;
            }
        }
        return Some(Wear { id: e.id, type_name: e.type_name, open: e.is_alive(), accepts, mob: None, shearable: false, leads: false });
    }
    // A knot on a fence holds the leads (`Entity.shearOffAllLeashConnections`).
    if e.type_name == kiln_entity::leash::KNOT {
        return Some(Wear { id: e.id, type_name: e.type_name, open: true, accepts: 0, mob: None, shearable: false, leads: false });
    }
    let facts = rules::facts(e)?;
    let mut accepts = 0u8;
    for (i, slot) in SLOTS.iter().enumerate() {
        if facts.worn & (1 << i) == 0 && rules::can_use_slot(&facts, *slot) && rules::can_dispenser_equip_into(&facts, *slot) {
            accepts |= 1 << i;
        }
    }
    let alive = rules::mob(e).is_some_and(|m| kiln_entity::mob::is_alive(e, m));
    Some(Wear { id: e.id, type_name: e.type_name, open: alive, accepts, mob: Some(facts), shearable: rules::shearable(e), leads: false })
}

/// The living things whose box meets the block `target`, in id order.
fn in_front<'a>(level: &'a RegionLevel, target: BlockPos) -> Vec<&'a Wear> {
    let (min, max) = ([target.x as f64, target.y as f64, target.z as f64], [target.x as f64 + 1.0, target.y as f64 + 1.0, target.z as f64 + 1.0]);
    let mut found: Vec<&Wear> = level.bodies.iter().filter(|b| b.intersects(min, max)).filter_map(|b| b.wear.as_ref()).collect();
    found.sort_by_key(|w| w.id);
    found
}

/// Whether something queued this phase already took `slot` of the entity `id`.
fn taken(level: &RegionLevel, id: i32, slot: EquipmentSlot) -> bool {
    level.out.dispenses.iter().any(|op| matches!(op, DispenseOp::Equip { id: i, slot: s, .. } if *i == id && *s == slot))
}

/// `EquipmentDispenseItemBehavior.dispenseEquipment`: one of `stack` is put on the first living thing in front of
/// the dispenser that can wear it. Whether it was.
pub(super) fn dispense_equipment(level: &mut RegionLevel, pos: BlockPos, facing: Direction, stack: &mut ItemStack) -> bool {
    let target = pos.relative(facing);
    let Some(eq) = stack.get(keys::EQUIPPABLE).cloned() else { return false };
    if !eq.dispensable {
        return false;
    }
    let i = eq.slot as usize;
    let found = in_front(level, target).into_iter().find(|w| {
        w.open
            && w.accepts & (1 << i) != 0
            && !taken(level, w.id, eq.slot)
            && (w.mob.is_none() || kiln_entity::mob::kinds::horse::equippable_in_slot(stack, eq.slot, w.type_name))
            && (w.mob.is_some() || equippable_by(stack, eq.slot, w.type_name))
    });
    let Some(w) = found else { return false };
    let id = w.id;
    let one = stack.split_count(1);
    level.out.dispenses.push(DispenseOp::Equip { id, slot: eq.slot, stack: one });
    true
}

/// `Equippable.canBeEquippedBy` for a player or a stand.
fn equippable_by(stack: &ItemStack, slot: EquipmentSlot, type_name: &str) -> bool {
    kiln_entity::mob::kinds::horse::equippable_in_slot(stack, slot, type_name)
}

/// `DispenseItemBehavior$2`: a chest onto the first tame pack animal in front. Whether one took it.
pub(super) fn dispense_chest(level: &mut RegionLevel, pos: BlockPos, facing: Direction, stack: &mut ItemStack) -> bool {
    use kiln_entity::mob::MobKind;
    let target = pos.relative(facing);
    let found = in_front(level, target)
        .into_iter()
        .find(|w| w.mob.is_some_and(|f| matches!(f.kind, MobKind::Donkey | MobKind::Mule | MobKind::Llama | MobKind::TraderLlama) && f.tamed));
    let Some(w) = found else { return false };
    let id = w.id;
    stack.shrink_count(1);
    level.out.dispenses.push(DispenseOp::Chest { id });
    true
}

/// `SulfurCubeBlockDispenseItemBehavior.dispenseBlock`: the first grown sulfur cube in front that does not hold the
/// same item swallows it. Whether one did.
pub(super) fn dispense_swallow(level: &mut RegionLevel, pos: BlockPos, facing: Direction, stack: &mut ItemStack) -> bool {
    use kiln_entity::mob::MobKind;
    let target = pos.relative(facing);
    let item = stack.item();
    let found = in_front(level, target).into_iter().find(|w| w.open && w.mob.is_some_and(|f| f.kind == MobKind::SulfurCube && !f.baby && f.body_item != item));
    let Some(w) = found else { return false };
    let id = w.id;
    let one = stack.split_count(1);
    level.out.dispenses.push(DispenseOp::Swallow { id, stack: one });
    true
}

/// `DispenseItemBehavior$11`: the first grown armadillo in front gives up a scute to the brush. Whether one did.
pub(super) fn dispense_brush(level: &mut RegionLevel, pos: BlockPos, facing: Direction, tool: &ItemStack) -> bool {
    let target = pos.relative(facing);
    let found = in_front(level, target).into_iter().find(|w| w.mob.is_some_and(|f| f.kind == kiln_entity::mob::MobKind::Armadillo && !f.baby));
    let Some(w) = found else { return false };
    let id = w.id;
    level.out.dispenses.push(DispenseOp::Shear { id, tool: tool.clone() });
    true
}

/// `ShearsDispenseItemBehavior.tryShearEntity`: the first shearable thing in front is sheared. Whether one was.
pub(super) fn dispense_shear(level: &mut RegionLevel, pos: BlockPos, facing: Direction, tool: &ItemStack) -> bool {
    let target = pos.relative(facing);
    let found = in_front(level, target).into_iter().find(|w| w.leads || (w.open && w.shearable));
    let Some(w) = found else { return false };
    let id = w.id;
    level.out.dispenses.push(DispenseOp::Shear { id, tool: tool.clone() });
    true
}

/// Carries out what the dispensers decided; the shears (which need the level) are returned for the caller.
pub(crate) fn apply(ops: Vec<DispenseOp>, entities: &mut Entities, players: &mut [&mut Player], spawns: &mut Vec<Spawn>) -> Vec<(i32, ItemStack)> {
    let mut shears = Vec::new();
    for op in ops {
        match op {
            DispenseOp::Shear { id, tool } => shears.push((id, tool)),
            DispenseOp::Equip { id, slot, stack } => {
                if let Some(p) = players.iter_mut().find(|p| p.entity_id == id) {
                    let at = kiln_inventory::inventory::equipment_index(slot, p.inv.selected);
                    *kiln_inventory::Container::item_mut(&mut p.inv, at) = stack.clone();
                    p.inv.times_changed += 1;
                    p.on_equip_item(slot, &ItemStack::empty(), &stack);
                    continue;
                }
                let Some(e) = entity_mut(entities, id) else { continue };
                if let Some(stand) = kiln_entity::ext_entity::get_mut::<kiln_entity::ext_entity::armor_stand::ArmorStand>(e) {
                    stand.dispenser_put(slot, stack);
                    e.needs_sync = true;
                } else if rules::equip(e, slot, stack) {
                    e.needs_sync = true;
                }
            }
            DispenseOp::Chest { id } => {
                if let Some(e) = entity_mut(entities, id) {
                    rules::put_chest(e);
                    e.needs_sync = true;
                }
            }
            DispenseOp::Swallow { id, stack } => {
                let Some(e) = entity_mut(entities, id) else { continue };
                // `SulfurCube.equipItem`: what it held pops out above it.
                let old = kiln_entity::mob::data_mut(e).and_then(|m| m.kind.ext().map(|k| k.extra_equipment(m))).and_then(|v| v.into_iter().find(|(s, _)| *s == 6)).map(|(_, s)| s);
                let top = [e.x(), e.y() + e.height as f64, e.z()];
                if rules::equip(e, EquipmentSlot::Body, stack) {
                    e.needs_sync = true;
                }
                if let Some(old) = old {
                    spawns.push(Spawn {
                        kind: &kiln_data::entities::types::ITEM,
                        pos: top,
                        vel: [0.0; 3],
                        body: Body::Item { stack: old, pickup_delay: 10, thrower: None },
                    });
                }
            }
        }
    }
    shears
}

/// `Shearable.shear(level, BLOCKS, tool)` of the thing a dispenser's shears found.
pub(crate) fn shear(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    id: i32,
    tool: &ItemStack,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<crate::health::Death>,
) {
    use kiln_entity::mob::interact::{Interactor, interact};
    let who = Interactor { id: -1, creative: false, sneaking: false, spectator: false, hit: kiln_entity::math::Vec3::ZERO };
    crate::entities::with_entity(entities, level, players, id, spawns, deaths, 0x7368_6172, |e, lv| {
        // `Entity.shearOffAllLeashConnections(null)` comes first.
        if tool.item_name() == "minecraft:shears" && kiln_entity::leash::shear_off_all(e, lv, -1) {
            return;
        }
        let out = interact(e, lv, &who, tool);
        // A sheep's wool: the loot table comes back with the outcome (a snow golem's and the others' drop through events).
        if let Some(table) = out.shear {
            let p = e.position();
            lv.emit(kiln_entity::level::Event::ShearLoot { entity: e.id, table, pos: kiln_entity::math::Vec3::new(p.x, p.y + 1.0, p.z) });
        }
    });
}

fn entity_mut(entities: &mut Entities, id: i32) -> Option<&mut kiln_entity::Entity> {
    let i = entities.list.binary_search_by_key(&id, |e| e.id).ok()?;
    entities.list[i].phys.as_deref_mut()
}
