//! The menus of container entities (`ContainerEntity` as a `MenuProvider`: chest and hopper
//! minecarts, chest boats): opening one from a click on the entity, and keeping the menu's
//! slots and the entity's in step.
//!
//! The slots live in the entity, so the menu works on a copy the player carries
//! ([`PlayerContainers::cart`](crate::container::open::PlayerContainers)): the region copies
//! the minecart's slots into it before a menu operation ([`pull`]) and back afterwards
//! ([`push`]), so what hoppers and other viewers did shows and what the player did sticks.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::container::open::OpenBlock;
use crate::entities::{Entities, Spawn};
use kiln_entity::ext_entity::minecart::Contents;
use kiln_inventory::{Menu, SimpleContainer};
use kiln_proto::nbt::Tag;

/// The slots of the minecart with id `id`.
fn contents(entities: &Entities, id: i32) -> Option<&Contents> {
    let idx = entities.list.binary_search_by_key(&id, |e| e.id).ok()?;
    let e = &entities.list[idx];
    if e.removed {
        return None;
    }
    kiln_entity::ext_entity::container(e.phys.as_ref()?)
}

fn contents_mut(entities: &mut Entities, id: i32) -> Option<&mut Contents> {
    let idx = entities.list.binary_search_by_key(&id, |e| e.id).ok()?;
    let e = &mut entities.list[idx];
    if e.removed {
        return None;
    }
    kiln_entity::ext_entity::container_mut(e.phys.as_mut()?)
}

/// Before a menu operation of `p`: its open minecart's slots into the player's copy. Returns
/// the minecart's id for [`push`].
pub(crate) fn pull(entities: &Entities, p: &mut Player) -> Option<i32> {
    let Some(OpenBlock::Cart { entity }) = p.containers.open else { return None };
    if p.containers.cart_event_pos.is_some()
        && let Some(at) = position_of(entities, entity)
    {
        p.containers.cart_event_pos = Some(at);
    }
    if let Some(c) = contents(entities, entity)
        && c.items.len() == p.containers.cart.items.len()
    {
        p.containers.cart.items.clone_from(&c.items);
    }
    Some(entity)
}

/// After the operation: the player's copy back into the minecart.
pub(crate) fn push(entities: &mut Entities, p: &Player, cart: Option<i32>) {
    let Some(id) = cart else { return };
    if let Some(c) = contents_mut(entities, id)
        && c.items.len() == p.containers.cart.items.len()
    {
        c.items.clone_from(&p.containers.cart.items);
    }
}

/// Whether closing the menu of this entity type posts `container_close` (`MinecartChest` and
/// the chest boats override `stopOpen`; the hopper minecart keeps the default).
fn posts_close(type_name: &str) -> bool {
    type_name == "minecraft:chest_minecart" || type_name.ends_with("_chest_boat") || type_name.ends_with("_chest_raft")
}

fn position_of(entities: &Entities, id: i32) -> Option<[f64; 3]> {
    let idx = entities.list.binary_search_by_key(&id, |e| e.id).ok()?;
    let p = entities.list[idx].phys.as_ref()?.position();
    Some([p.x, p.y, p.z])
}

/// `Entity.getDisplayName`: the custom name, or the type's.
fn title(phys: &kiln_entity::Entity) -> Tag {
    if let Some((_, name)) = phys.extra.iter().find(|(k, _)| k == "CustomName") {
        return name.clone();
    }
    let path = phys.type_name.strip_prefix("minecraft:").unwrap_or(phys.type_name);
    Tag::Compound(vec![("translate".into(), Tag::String(format!("entity.minecraft.{path}")))])
}

/// `interactWithContainerVehicle` → `Player.openMenu`: opens the menu of minecart `target`
/// for `p` (another open screen closes first).
pub(crate) fn open(entities: &Entities, level: &mut RegionLevel, p: &mut Player, target: i32, spawns: &mut Vec<Spawn>) {
    let Ok(idx) = entities.list.binary_search_by_key(&target, |e| e.id) else { return };
    let Some(phys) = entities.list[idx].phys.as_ref() else { return };
    let Some(c) = contents(entities, target) else { return };
    let (items, hopper, title) = (c.items.clone(), phys.type_name == "minecraft:hopper_minecart", title(phys));
    let rules = level.env.menus.clone();
    if p.open_menu.is_some() {
        p.close_block_menu(&rules, spawns, level, true);
    }
    let id = kiln_inventory::click::next_container_id(&mut p.containers.counter);
    let menu = if hopper { Menu::hopper(id) } else { Menu::generic(id, 3) };
    let Some(ty) = menu.kind.menu_type_id() else { return };
    p.send(kiln_inventory::effect::open_screen(id, ty, &title));
    p.containers.cart = SimpleContainer::from_items(items);
    p.containers.cart_event_pos = posts_close(phys.type_name).then(|| [phys.x(), phys.y(), phys.z()]);
    p.containers.open = Some(OpenBlock::Cart { entity: target });
    p.open_menu = Some(menu);
    p.with_menu_at(&rules, spawns, None, |menu, _, env| menu.open(env));
}

/// `ContainerEntity.isChestVehicleStillValid` each tick: the menu closes when its minecart
/// is gone or farther than the interaction range plus 4.
pub(crate) fn check_menus(entities: &Entities, players: &mut [&mut Player], rules: &kiln_inventory::Rules, spawns: &mut Vec<Spawn>) {
    for p in players.iter_mut() {
        let Some(OpenBlock::Cart { entity }) = p.containers.open else { continue };
        let valid = contents(entities, entity).is_some()
            && entities.list.binary_search_by_key(&entity, |e| e.id).ok().and_then(|i| entities.list[i].phys.as_ref()).is_some_and(|phys| {
                let bb = phys.bounding_box();
                let eye = p.eye_position();
                let d = |v: f64, lo: f64, hi: f64| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
                let (dx, dy, dz) = (d(eye[0], bb.min_x, bb.max_x), d(eye[1], bb.min_y, bb.max_y), d(eye[2], bb.min_z, bb.max_z));
                let range = p.attribute(crate::combat::ENTITY_INTERACTION_RANGE) + 4.0;
                dx * dx + dy * dy + dz * dz < range * range
            });
        if valid && p.containers.cart_event_pos.is_some() {
            p.containers.cart_event_pos = position_of(entities, entity);
        }
        if !valid && !p.dead {
            p.containers.cart_closed = p.containers.cart_event_pos.take();
            // `ServerPlayer.closeContainer`.
            if let Some(id) = p.open_menu.as_ref().map(|m| m.container_id) {
                p.send(kiln_inventory::effect::container_close(id));
            }
            p.with_menu_at(rules, spawns, None, |open, inventory_menu, env| kiln_inventory::click::close_container(open, inventory_menu, env));
            p.open_menu = None;
            p.containers.open = None;
        }
    }
}
