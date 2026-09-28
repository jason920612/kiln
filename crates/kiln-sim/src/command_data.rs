//! What `/data`, `/tag` and the other entity and item commands ask of the simulation: saved
//! entity data, entity tags, block entity data and item slots.

use crate::commands::PlayerRef;
use crate::{Player, Sim, entities};
use kiln_command::CommandError;
use kiln_proto::nbt::Tag;

/// `Entity.addTag`'s limit.
const MAX_TAGS: usize = 1024;

/// The `Tags` list of a saved compound.
pub(crate) fn tags_in(tag: &Tag) -> Vec<String> {
    match tag.get("Tags") {
        Some(Tag::List(items)) => items.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

/// Replaces (or with an empty list removes) the `Tags` field of a compound's fields.
fn put_tags(fields: &mut Vec<(String, Tag)>, tags: &[String]) {
    fields.retain(|(k, _)| k != "Tags");
    if !tags.is_empty() {
        fields.push(("Tags".into(), Tag::List(tags.iter().map(|t| Tag::String(t.clone())).collect())));
    }
}

/// Adds or removes `tag`; whether the set changed (`Entity.addTag` / `removeTag`).
fn change_tag(tags: &mut Vec<String>, tag: &str, add: bool) -> bool {
    if add {
        if tags.len() >= MAX_TAGS || tags.iter().any(|t| t == tag) {
            return false;
        }
        tags.push(tag.to_owned());
        true
    } else {
        let before = tags.len();
        tags.retain(|t| t != tag);
        tags.len() != before
    }
}

impl Player {
    pub(crate) fn tags(&self) -> Vec<String> {
        tags_in(self.saved.raw())
    }
}

impl Sim {
    /// The non-player entity a selector found.
    pub(crate) fn entity_mut(&mut self, target: &PlayerRef) -> Option<&mut entities::Entity> {
        let id = target.entity?;
        let dim = crate::dim_id(target.dim)?;
        self.dims[dim].regions.iter_mut().find_map(|r| r.part_mut().0.list.iter_mut().find(|e| e.id == id))
    }

    /// `Entity.addTag` / `removeTag` on a player or entity.
    pub(crate) fn change_entity_tag(&mut self, target: &PlayerRef, tag: &str, add: bool) -> bool {
        if target.entity.is_none() {
            let Some(p) = self.players.get_mut(&target.conn) else { return false };
            let mut tags = p.tags();
            if !change_tag(&mut tags, tag, add) {
                return false;
            }
            if let Tag::Compound(fields) = p.saved.raw_mut() {
                put_tags(fields, &tags);
            }
            return true;
        }
        let Some(phys) = self.entity_mut(target).and_then(|e| e.phys.as_mut()) else { return false };
        let mut tags = tags_in(&Tag::Compound(phys.extra.clone()));
        if !change_tag(&mut tags, tag, add) {
            return false;
        }
        put_tags(&mut phys.extra, &tags);
        true
    }

    /// `NbtPredicate.getEntityTagToCompare`: `saveWithoutId`, and a player's selected item.
    pub(crate) fn entity_saved_data(&mut self, target: &PlayerRef) -> Option<Tag> {
        if target.entity.is_none() {
            let p = self.players.get(&target.conn)?;
            let mut nbt = self.player_nbt(p);
            let selected = p.inv.selected_item().clone();
            if let Tag::Compound(fields) = &mut nbt {
                // Saved-file bookkeeping that `saveWithoutId` does not write.
                fields.retain(|(k, _)| k != "DataVersion");
                if !selected.is_empty() {
                    fields.push(("SelectedItem".into(), selected.to_nbt()));
                }
            }
            return Some(nbt);
        }
        let e = self.entity_mut(target)?;
        let mut nbt = e.save(&|_| None);
        if let Tag::Compound(fields) = &mut nbt {
            fields.retain(|(k, _)| k != "id");
        }
        Some(nbt)
    }

    /// `EntityDataAccessor.setData` for a non-player: the entity reloaded from `data` with its
    /// UUID, network id and type kept.
    pub(crate) fn load_entity_data(&mut self, target: &PlayerRef, data: &Tag) -> Result<(), CommandError> {
        let Some(e) = self.entity_mut(target) else { return Ok(()) };
        let Tag::Compound(fields) = data else { return Ok(()) };
        let mut fields = fields.clone();
        fields.retain(|(k, _)| k != "id" && k != "UUID");
        fields.push(("id".into(), Tag::String(e.kind.name.to_owned())));
        let seed = crate::entities::seed_for_uuid(e.uuid.as_u128());
        match kiln_entity::persist::load(&Tag::Compound(fields), e.id, seed) {
            Ok(mut loaded) => {
                loaded.uuid = e.uuid.as_u128();
                e.phys = Some(loaded);
                e.sync();
                Ok(())
            }
            Err(_) => Err(CommandError::unsupported("Changing this entity's data")),
        }
    }

    /// `Entity.forceSetRotation`: players get a teleport to where they are.
    pub(crate) fn rotate_target(&mut self, target: &PlayerRef, rot: [f32; 2]) {
        let rot = [rot[0], rot[1].clamp(-90.0, 90.0)];
        if target.entity.is_none() {
            let now = self.game_time;
            if let Some(p) = self.players.get_mut(&target.conn) {
                let pos = p.pos;
                p.teleport(pos, rot, now);
            }
            return;
        }
        let Some(phys) = self.entity_mut(target).and_then(|e| e.phys.as_mut()) else { return };
        phys.y_rot = rot[0];
        phys.x_rot = rot[1];
        if let Some(m) = kiln_entity::mob::data_mut(phys) {
            m.y_head_rot = rot[0];
            m.y_body_rot = rot[0];
        }
    }

    /// `LivingEntity.swing`: the swing animation to the entity's viewers (and a player).
    pub(crate) fn swing_target(&mut self, target: &PlayerRef, offhand: bool, animation: &str, duration: i32) -> bool {
        let kind = match animation {
            "none" => kiln_proto::packets::entity::swing::NONE,
            "stab" => kiln_proto::packets::entity::swing::STAB,
            _ => kiln_proto::packets::entity::swing::WHACK,
        };
        let (id, viewers) = match target.entity {
            None => {
                let Some(p) = self.players.get(&target.conn) else { return false };
                let mut v = p.seen_by.clone();
                v.push(target.conn);
                (p.entity_id, v)
            }
            Some(id) => {
                let Some(e) = self.entity_mut(target) else { return false };
                if e.phys.as_ref().and_then(kiln_entity::mob::data).is_none() {
                    return false;
                }
                (id, e.seen_by.clone())
            }
        };
        let pkt = kiln_proto::packets::entity::swing_animation(id, offhand, kind, duration);
        for c in viewers {
            if let Some(p) = self.players.get_mut(&c) {
                p.send(pkt.clone());
            }
        }
        true
    }

    /// The network id of what `target` rides.
    fn vehicle_id(&mut self, target: &PlayerRef) -> Option<i32> {
        match target.entity {
            None => self.players.get(&target.conn)?.vehicle,
            Some(_) => self.entity_mut(target)?.phys.as_ref()?.vehicle,
        }
    }

    /// `Entity.getVehicle`.
    pub(crate) fn vehicle_of_target(&mut self, target: &PlayerRef) -> Option<PlayerRef> {
        let id = self.vehicle_id(target)?;
        kiln_command::selector::SelectorWorld::entities(self, Some(target.dim), None).into_iter().find(|e| e.entity == Some(id))
    }

    /// The entity and everything riding it, recursively.
    pub(crate) fn self_and_passengers_of(&mut self, target: &PlayerRef) -> Vec<PlayerRef> {
        let all = kiln_command::selector::SelectorWorld::entities(self, Some(target.dim), None);
        // Network ids of the candidates (players by their entity id).
        let ids: Vec<i32> =
            all.iter().map(|e| e.entity.unwrap_or_else(|| self.players.get(&e.conn).map_or(-1, |p| p.entity_id))).collect();
        let mut out = vec![target.clone()];
        let mut i = 0;
        while i < out.len() {
            if out[i].entity.is_some() {
                let passengers = self.entity_mut(&out[i]).and_then(|e| e.phys.as_ref().map(|p| p.passengers.clone())).unwrap_or_default();
                for pid in passengers {
                    if let Some(k) = ids.iter().position(|&id| id == pid) {
                        out.push(all[k].clone());
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// `startRiding(vehicle, force)`: a player or entity onto an entity of the same level.
    pub(crate) fn start_riding_target(&mut self, target: &PlayerRef, vehicle: &PlayerRef) -> bool {
        let Some(vid) = vehicle.entity else { return false };
        match target.entity {
            None => {
                let Some(pid) = self.players.get(&target.conn).map(|p| p.entity_id) else { return false };
                let first_is_player = {
                    let players: Vec<i32> = self.players.values().map(|p| p.entity_id).collect();
                    let Some(phys) = self.entity_mut(vehicle).and_then(|e| e.phys.as_ref()) else { return false };
                    phys.passengers.first().is_some_and(|f| players.contains(f))
                };
                let Some(phys) = self.entity_mut(vehicle).and_then(|e| e.phys.as_mut()) else { return false };
                kiln_entity::ride::add_passenger(phys, pid, true, first_is_player);
                let at = phys.passengers.iter().position(|&x| x == pid).unwrap_or(0);
                let seat = kiln_entity::ride::rider_position(phys, at, "minecraft:player", 1.0);
                let (rot, type_name) = ([phys.y_rot, phys.x_rot], phys.type_name);
                let now = self.game_time;
                let Some(p) = self.players.get_mut(&target.conn) else { return false };
                p.vehicle = Some(vid);
                p.vehicle_type = Some(type_name);
                p.teleport([seat.x, seat.y, seat.z], rot, now);
                true
            }
            Some(_) => {
                let dim = crate::dim_id(target.dim).unwrap_or(crate::OVERWORLD_ID);
                let Some(tid) = target.entity else { return false };
                for r in self.dims[dim].regions.iter_mut() {
                    let list = &mut r.part_mut().0.list;
                    let (Some(ti), Some(vi)) = (list.iter().position(|e| e.id == tid), list.iter().position(|e| e.id == vid)) else {
                        continue;
                    };
                    let mut rider = list[ti].phys.take().expect("rider state");
                    let ok = list[vi].phys.as_mut().is_some_and(|v| kiln_entity::ride::start_riding(&mut rider, v, false));
                    list[ti].phys = Some(rider);
                    list[ti].sync();
                    return ok;
                }
                false
            }
        }
    }

    /// `Entity.stopRiding`.
    pub(crate) fn stop_riding_target(&mut self, target: &PlayerRef) {
        let Some(vehicle) = self.vehicle_of_target(target) else { return };
        let rider = match target.entity {
            None => {
                let Some(p) = self.players.get_mut(&target.conn) else { return };
                p.vehicle = None;
                p.vehicle_type = None;
                p.entity_id
            }
            Some(id) => {
                if let Some(phys) = self.entity_mut(target).and_then(|e| e.phys.as_mut()) {
                    phys.vehicle = None;
                }
                id
            }
        };
        if let Some(phys) = self.entity_mut(&vehicle).and_then(|e| e.phys.as_mut()) {
            kiln_entity::ride::remove_passenger(phys, rider);
        }
    }

    /// `/damage` on a player (`hurtServer` with a source of `damage_type`).
    pub(crate) fn damage_target(&mut self, target: &PlayerRef, amount: f32, damage_type: &str) -> Result<bool, CommandError> {
        if target.entity.is_some() {
            return Err(CommandError::unsupported("Damaging entities by command"));
        }
        let ty = kiln_data::registries::SYNCHRONIZED
            .iter()
            .find(|(r, _)| *r == "minecraft:damage_type")
            .and_then(|(_, ids)| ids.iter().copied().find(|t| *t == damage_type))
            .unwrap_or("minecraft:generic");
        let (rules, game_time) = (self.damage_rules(), self.game_time);
        let Some(p) = self.players.get_mut(&target.conn) else { return Ok(false) };
        let (mut spawns, mut deaths) = (Vec::new(), Vec::new());
        let mut ctx = crate::health::DamageCtx { rules, game_time, spawns: &mut spawns, deaths: &mut deaths, level_rng: None };
        let cause = if ty == "minecraft:generic_kill" { crate::health::Cause::Kill } else { crate::health::Cause::Other(ty) };
        let hurt = p.hurt(amount, &cause.into(), &mut ctx);
        let dim = p.dim;
        self.dims[dim].spawns.extend(spawns);
        self.announce_deaths(deaths);
        Ok(hurt)
    }

    /// `ServerPlayer.setCamera`.
    pub(crate) fn set_camera_of(&mut self, player: &PlayerRef, target: Option<&PlayerRef>) {
        let id = match target {
            None => self.players.get(&player.conn).map(|p| p.entity_id),
            Some(t) if t.entity.is_none() => self.players.get(&t.conn).map(|p| p.entity_id),
            Some(t) => t.entity,
        };
        if let (Some(id), Some(p)) = (id, self.players.get_mut(&player.conn)) {
            p.send(kiln_proto::packets::player::set_camera(id));
        }
    }

    /// A player's stack in a [`kiln_command::slots`] slot; `None` if the slot does not exist.
    fn player_slot(p: &Player, slot: i32) -> Option<kiln_item::ItemStack> {
        let inv = &p.inv;
        Some(match slot {
            0..36 => inv.items[slot as usize].clone(),
            98 => inv.selected_item().clone(),
            99..=103 | 105 | 106 => inv.equipment[equipment_index(slot)?].clone(),
            200..227 => p.containers.ender.items.get((slot - 200) as usize)?.clone(),
            _ => return None,
        })
    }

    /// `SlotAccess` of an entity or a container block, as item stack NBT.
    pub(crate) fn slot_item_nbt(&mut self, holder: &kiln_command::host::ItemHolder<PlayerRef>, slot: i32) -> Option<Option<Tag>> {
        use kiln_command::host::ItemHolder;
        match holder {
            ItemHolder::Entity(e) if e.entity.is_none() => {
                let p = self.players.get(&e.conn)?;
                let stack = Self::player_slot(p, slot)?;
                Some((!stack.is_empty()).then(|| stack.to_nbt()))
            }
            ItemHolder::Entity(e) => {
                let phys = self.entity_mut(e)?.phys.as_ref()?;
                let m = kiln_entity::mob::data(phys)?;
                let stack = mob_slot(m, slot)?;
                Some((!stack.is_empty()).then(|| stack.to_nbt()))
            }
            ItemHolder::Block { dimension, pos } => {
                let data = kiln_command::Host::block_entity(self, dimension, *pos)?;
                let size = container_size(data.get("id")?.as_str()?)?;
                if !(0..size).contains(&slot) {
                    return None;
                }
                let item = match data.get("Items") {
                    Some(Tag::List(items)) => items.iter().find(|i| matches!(i.get("Slot"), Some(Tag::Byte(s)) if *s as u8 as i32 == slot)).cloned(),
                    _ => None,
                };
                Some(item.map(|mut i| {
                    if let Tag::Compound(f) = &mut i {
                        f.retain(|(k, _)| k != "Slot");
                    }
                    i
                }))
            }
        }
    }

    /// Puts an item (stack NBT) into a slot; false if the slot does not exist.
    pub(crate) fn set_slot_item_nbt(&mut self, holder: &kiln_command::host::ItemHolder<PlayerRef>, slot: i32, item: Option<&Tag>) -> bool {
        use kiln_command::host::ItemHolder;
        let stack = match item {
            Some(t) => kiln_item::ItemStack::from_nbt(t).unwrap_or_else(|_| kiln_item::ItemStack::empty()),
            None => kiln_item::ItemStack::empty(),
        };
        match holder {
            ItemHolder::Entity(e) if e.entity.is_none() => {
                let Some(p) = self.players.get_mut(&e.conn) else { return false };
                let inv = &mut p.inv;
                match slot {
                    0..36 => inv.items[slot as usize] = stack,
                    98 => {
                        let sel = inv.selected;
                        inv.items[sel] = stack;
                    }
                    99..=103 | 105 | 106 => match equipment_index(slot) {
                        Some(i) => inv.equipment[i] = stack,
                        None => return false,
                    },
                    200..227 => {
                        p.containers.ender.items[(slot - 200) as usize] = stack;
                        p.containers.ender.changes += 1;
                        return true;
                    }
                    _ => return false,
                }
                inv.times_changed += 1;
                true
            }
            ItemHolder::Entity(e) => {
                let Some(m) = self.entity_mut(e).and_then(|en| en.phys.as_mut()).and_then(kiln_entity::mob::data_mut) else { return false };
                set_mob_slot(m, slot, stack)
            }
            ItemHolder::Block { dimension, pos } => {
                let Some(mut data) = kiln_command::Host::block_entity(self, dimension, *pos) else { return false };
                let Some(size) = data.get("id").and_then(Tag::as_str).and_then(container_size) else { return false };
                if !(0..size).contains(&slot) {
                    return false;
                }
                if let Tag::Compound(fields) = &mut data {
                    if !fields.iter().any(|(k, _)| k == "Items") {
                        fields.push(("Items".into(), Tag::List(Vec::new())));
                    }
                    if let Some((_, Tag::List(items))) = fields.iter_mut().find(|(k, _)| k == "Items") {
                        items.retain(|i| !matches!(i.get("Slot"), Some(Tag::Byte(s)) if *s as u8 as i32 == slot));
                        if !stack.is_empty() {
                            let mut t = stack.to_nbt();
                            if let Tag::Compound(f) = &mut t {
                                f.insert(0, ("Slot".into(), Tag::Byte(slot as i8)));
                            }
                            items.push(t);
                        }
                    }
                }
                self.set_block_entity_nbt(dimension, *pos, &data);
                true
            }
        }
    }

    /// Whether the block entity at `pos` is a container.
    pub(crate) fn is_container_at(&mut self, dimension: &str, pos: [i32; 3]) -> bool {
        kiln_command::Host::block_entity(self, dimension, pos)
            .and_then(|d| d.get("id").and_then(Tag::as_str).map(str::to_owned))
            .is_some_and(|id| container_size(&id).is_some())
    }

    /// Sends a player's inventory after commands changed it.
    pub(crate) fn broadcast_inventory(&mut self, player: &PlayerRef) {
        let rules = self.rules.clone();
        let Some(p) = self.players.get_mut(&player.conn) else { return };
        let dim = p.dim;
        let mut spawns = Vec::new();
        p.with_menu(&rules, &mut spawns, |menu, _, env| menu.broadcast_changes(env));
        self.dims[dim].spawns.extend(spawns);
    }

    /// `max_level` of an enchantment of the loaded data.
    pub(crate) fn enchantment_max(&self, enchantment: &str) -> Option<i32> {
        let id = kiln_data::synced_id("minecraft:enchantment", enchantment)?;
        let loot = self.loot.clone()?;
        loot.enchantment(id).map(|e| e.max_level)
    }

    /// `EnchantCommand` on one target's main hand item.
    pub(crate) fn enchant_target(&mut self, target: &PlayerRef, enchantment: &str, level: i32) -> kiln_command::host::EnchantOutcome {
        use kiln_command::host::EnchantOutcome;
        let Some(id) = kiln_data::synced_id("minecraft:enchantment", enchantment) else { return EnchantOutcome::NotLiving };
        let Some(loot) = self.loot.clone() else { return EnchantOutcome::NotLiving };
        let Some(ench) = loot.enchantment(id) else { return EnchantOutcome::NotLiving };
        let apply = |stack: &mut kiln_item::ItemStack| {
            if stack.is_empty() {
                return EnchantOutcome::NoItem;
            }
            let key = if stack.item_name() == "minecraft:enchanted_book" {
                kiln_item::component::keys::STORED_ENCHANTMENTS
            } else {
                kiln_item::component::keys::ENCHANTMENTS
            };
            let existing: Vec<i32> = stack.get(key).map(|e| e.0.iter().map(|(id, _)| *id).collect()).unwrap_or_default();
            let compatible =
                existing.iter().all(|&other| loot.enchantment(other).is_none_or(|o| kiln_loot::enchant::compatible(o, ench)));
            if !ench.can_enchant(stack) || !compatible {
                return EnchantOutcome::Incompatible(hover_name(stack));
            }
            kiln_loot::enchant::enchant(stack, id, level);
            EnchantOutcome::Applied
        };
        if target.entity.is_some() {
            let Some(m) = self.entity_mut(target).and_then(|e| e.phys.as_mut()).and_then(kiln_entity::mob::data_mut) else {
                return EnchantOutcome::NotLiving;
            };
            return apply(&mut m.equipment[0]);
        }
        let Some(p) = self.players.get_mut(&target.conn) else { return EnchantOutcome::NotLiving };
        let sel = p.inv.selected;
        let out = apply(&mut p.inv.items[sel]);
        if out == EnchantOutcome::Applied {
            p.inv.times_changed += 1;
            let r = target.clone();
            self.broadcast_inventory(&r);
        }
        out
    }

    /// `BlockDataAccessor.setData`.
    pub(crate) fn set_block_entity_nbt(&mut self, dimension: &str, pos: [i32; 3], data: &Tag) {
        let Some(dim) = crate::dim_id(dimension) else { return };
        if let Tag::Compound(fields) = data {
            let mut fields = fields.clone();
            normalize_items(&mut fields);
            self.load_block_entity(dim, pos, &fields);
        }
    }
}

/// `ContainerHelper.loadAllItems` then `saveAllItems`: `Items` as the block entity will save
/// them (each stack re-encoded, unreadable and empty ones dropped, in slot order).
fn normalize_items(fields: &mut [(String, Tag)]) {
    let Some((_, Tag::List(items))) = fields.iter_mut().find(|(k, _)| k == "Items") else { return };
    let mut out: Vec<(i8, Tag)> = Vec::new();
    for item in items.iter() {
        let slot = match item.get("Slot") {
            Some(Tag::Byte(s)) => *s,
            _ => 0,
        };
        let Ok(stack) = kiln_item::ItemStack::from_nbt(item) else { continue };
        if stack.is_empty() {
            continue;
        }
        let mut t = stack.to_nbt();
        if let Tag::Compound(f) = &mut t {
            f.insert(0, ("Slot".into(), Tag::Byte(slot)));
        }
        out.retain(|(s, _)| *s != slot);
        out.push((slot, t));
    }
    out.sort_by_key(|(s, _)| *s as u8);
    *items = out.into_iter().map(|(_, t)| t).collect();
}

/// `Player.createAttributes` (with `LivingEntity.createLivingAttributes`): every attribute a
/// player has, with its default base and range.
const PLAYER_ATTRIBUTES: [crate::combat::Attr; 36] = {
    use crate::combat::Attr as A;
    [
        A::new("minecraft:max_health", 20.0, 1.0, 1024.0),
        A::new("minecraft:knockback_resistance", 0.0, 0.0, 1.0),
        A::new("minecraft:movement_speed", 0.10000000149011612, 0.0, 1024.0),
        A::new("minecraft:armor", 0.0, 0.0, 30.0),
        A::new("minecraft:armor_toughness", 0.0, 0.0, 20.0),
        A::new("minecraft:max_absorption", 0.0, 0.0, 2048.0),
        A::new("minecraft:step_height", 0.6, 0.0, 10.0),
        A::new("minecraft:scale", 1.0, 0.0625, 16.0),
        A::new("minecraft:gravity", 0.08, -1.0, 1.0),
        A::new("minecraft:safe_fall_distance", 3.0, -1024.0, 1024.0),
        A::new("minecraft:fall_damage_multiplier", 1.0, 0.0, 100.0),
        A::new("minecraft:jump_strength", 0.41999998688697815, 0.0, 32.0),
        A::new("minecraft:entity_interaction_range", 3.0, 0.0, 64.0),
        A::new("minecraft:oxygen_bonus", 0.0, 0.0, 1024.0),
        A::new("minecraft:burning_time", 1.0, 0.0, 1024.0),
        A::new("minecraft:explosion_knockback_resistance", 0.0, 0.0, 1.0),
        A::new("minecraft:water_movement_efficiency", 0.0, 0.0, 1.0),
        A::new("minecraft:movement_efficiency", 0.0, 0.0, 1.0),
        A::new("minecraft:attack_knockback", 0.0, 0.0, 5.0),
        A::new("minecraft:camera_distance", 4.0, 0.0, 32.0),
        A::new("minecraft:waypoint_transmit_range", 6.0e7, 0.0, 6.0e7),
        A::new("minecraft:bounciness", 0.0, 0.0, 1.0),
        A::new("minecraft:air_drag_modifier", 1.0, 0.0, 2048.0),
        A::new("minecraft:friction_modifier", 1.0, 0.0, 2048.0),
        A::new("minecraft:name_tag_distance", 64.0, 0.0, 512.0),
        A::new("minecraft:below_name_distance", 10.0, 0.0, 512.0),
        A::new("minecraft:attack_damage", 1.0, 0.0, 2048.0),
        A::new("minecraft:attack_speed", 4.0, 0.0, 1024.0),
        A::new("minecraft:luck", 0.0, -1024.0, 1024.0),
        A::new("minecraft:block_interaction_range", 4.5, 0.0, 64.0),
        A::new("minecraft:block_break_speed", 1.0, 0.0, 1024.0),
        A::new("minecraft:submerged_mining_speed", 0.2, 0.0, 20.0),
        A::new("minecraft:sneaking_speed", 0.3, 0.0, 1.0),
        A::new("minecraft:mining_efficiency", 0.0, 0.0, 1024.0),
        A::new("minecraft:sweeping_damage_ratio", 0.0, 0.0, 1.0),
        A::new("minecraft:waypoint_receive_range", 6.0e7, 0.0, 6.0e7),
    ]
};

fn op_of(op: u8) -> kiln_item::component::AttributeOperation {
    use kiln_item::component::AttributeOperation as O;
    match op {
        0 => O::AddValue,
        1 => O::AddMultipliedBase,
        _ => O::AddMultipliedTotal,
    }
}

impl Sim {
    /// `/attribute`'s view of an attribute (`Err` for entities that are not living).
    pub(crate) fn attribute_state(&mut self, target: &PlayerRef, name: &str) -> Result<Option<kiln_command::host::AttributeState>, ()> {
        use kiln_command::host::AttributeState;
        if target.entity.is_none() {
            let p = self.players.get(&target.conn).ok_or(())?;
            let Some(attr) = PLAYER_ATTRIBUTES.iter().find(|a| a.name() == name) else { return Ok(None) };
            let with_base = p.with_base(*attr);
            let modifiers = p.attribute_modifiers(with_base).into_iter().map(|(id, amount, _)| (id, amount)).collect();
            return Ok(Some(AttributeState { base: with_base.default_base(), value: p.attribute(*attr), modifiers }));
        }
        let m = self.entity_mut(target).and_then(|e| e.phys.as_ref()).and_then(kiln_entity::mob::data).ok_or(())?;
        let Some(attr) = kiln_entity::mob::attributes::Attr::by_name(name) else { return Ok(None) };
        let Some(i) = m.attrs.get(attr) else { return Ok(None) };
        Ok(Some(AttributeState { base: i.base, value: i.value(), modifiers: i.modifiers.iter().map(|m| (m.id.clone(), m.amount)).collect() }))
    }

    /// Changes an attribute: `Some(base)` sets the base (`None` resets it), or adds
    /// (`Some`) or removes (`None`) a modifier.
    pub(crate) fn change_attribute(&mut self, target: &PlayerRef, name: &str, change: AttributeChange) -> bool {
        if target.entity.is_none() {
            let Some(p) = self.players.get_mut(&target.conn) else { return false };
            let Some(attr) = PLAYER_ATTRIBUTES.iter().find(|a| a.name() == name) else { return false };
            let c = &mut p.command_attributes;
            let changed = match change {
                AttributeChange::Base(v) => {
                    c.bases.retain(|(a, _)| *a != attr.name());
                    if let Some(v) = v {
                        c.bases.push((attr.name(), v));
                    }
                    true
                }
                AttributeChange::AddModifier(id, amount, op) => {
                    c.modifiers.push((attr.name(), id, amount, op_of(op)));
                    true
                }
                AttributeChange::RemoveModifier(id) => {
                    let before = c.modifiers.len();
                    c.modifiers.retain(|(a, i, _, _)| !(*a == attr.name() && *i == id));
                    before != c.modifiers.len()
                }
            };
            p.attributes_dirty |= changed;
            return changed;
        }
        let Some(m) = self.entity_mut(target).and_then(|e| e.phys.as_mut()).and_then(kiln_entity::mob::data_mut) else { return false };
        let Some(attr) = kiln_entity::mob::attributes::Attr::by_name(name) else { return false };
        let default = m.kind.attributes().base(attr);
        let Some(i) = m.attrs.get_mut(attr) else { return false };
        use kiln_entity::mob::attributes::{Modifier, Op};
        match change {
            AttributeChange::Base(v) => {
                i.base = v.unwrap_or(default);
                true
            }
            AttributeChange::AddModifier(id, amount, op) => {
                let op = match op {
                    0 => Op::AddValue,
                    1 => Op::AddMultipliedBase,
                    _ => Op::AddMultipliedTotal,
                };
                i.modifiers.push(Modifier { id, amount, op });
                true
            }
            AttributeChange::RemoveModifier(id) => {
                let before = i.modifiers.len();
                i.modifiers.retain(|m| m.id != id);
                before != i.modifiers.len()
            }
        }
    }
}

/// The loot context of `/loot loot` and `/loot fish` (`CHEST` / `FISHING` parameter sets).
struct CommandLootContext {
    origin: [f64; 3],
    this: bool,
    tool: Option<kiln_item::ItemStack>,
}

impl kiln_loot::LootContext for CommandLootContext {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        self.this && target == kiln_loot::EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn tool(&self) -> Option<&kiln_item::ItemStack> {
        self.tool.as_ref()
    }
}

/// SNBT-decoded NBT as JSON text, the way `NbtOps` hands a definition to a codec.
fn nbt_json(tag: &Tag, out: &mut String) {
    use std::fmt::Write as _;
    match tag {
        Tag::Byte(v) => write!(out, "{v}").unwrap(),
        Tag::Short(v) => write!(out, "{v}").unwrap(),
        Tag::Int(v) => write!(out, "{v}").unwrap(),
        Tag::Long(v) => write!(out, "{v}").unwrap(),
        Tag::Float(v) => write!(out, "{v}").unwrap(),
        Tag::Double(v) => write!(out, "{v}").unwrap(),
        Tag::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Tag::List(items) => {
            out.push('[');
            for (i, t) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nbt_json(t, out);
            }
            out.push(']');
        }
        Tag::ByteArray(v) => nbt_json(&Tag::List(v.iter().map(|b| Tag::Byte(*b)).collect()), out),
        Tag::IntArray(v) => nbt_json(&Tag::List(v.iter().map(|b| Tag::Int(*b)).collect()), out),
        Tag::LongArray(v) => nbt_json(&Tag::List(v.iter().map(|b| Tag::Long(*b)).collect()), out),
        Tag::Compound(f) => {
            out.push('{');
            for (i, (k, v)) in f.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                nbt_json(&Tag::String(k.clone()), out);
                out.push(':');
                nbt_json(v, out);
            }
            out.push('}');
        }
    }
}

fn no_such_element(id: &str, registry: &str) -> CommandError {
    CommandError::new(kiln_command::tr!("argument.resource_or_id.no_such_element", id, registry))
}

fn stack_of(t: Option<&Tag>) -> kiln_item::ItemStack {
    t.and_then(|t| kiln_item::ItemStack::from_nbt(t).ok()).unwrap_or_else(kiln_item::ItemStack::empty)
}

impl Sim {
    /// A seed for a `/loot` roll: the world, the tick and a per-call counter.
    fn command_loot_seed(&mut self) -> i64 {
        self.commands.rng = self.commands.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        crate::mobs::loot_seed(self.commands.seed, self.game_time, 0, self.commands.rng)
    }

    /// Rolls `table` (an id or an inline definition) for `ctx`.
    fn roll_table(
        &mut self,
        table: &kiln_command::host::LootTableArg,
        ctx: &dyn kiln_loot::LootContext,
    ) -> Result<(Vec<kiln_item::ItemStack>, Option<String>), CommandError> {
        use kiln_command::host::LootTableArg;
        let Some(loot) = self.loot.clone() else { return Err(CommandError::unsupported("Loot tables")) };
        let seed = self.command_loot_seed();
        let (mut sequences, mut level) = (kiln_loot::RandomSequences::new(0), kiln_javamath::random::LegacyRandom::new(seed));
        match table {
            LootTableArg::Id(id) => {
                let ident = kiln_item::Identifier::parse(id).ok_or_else(|| no_such_element(id, "minecraft:loot_table"))?;
                let t = loot.table(&ident).ok_or_else(|| no_such_element(id, "minecraft:loot_table"))?;
                let mut rng = t.random(0, &mut sequences, &mut level);
                Ok((loot.random_items(&ident, ctx, rng.source()), Some(id.clone())))
            }
            LootTableArg::Inline(tag) => {
                let mut json = String::new();
                nbt_json(tag, &mut json);
                let t = loot.parse_table(&json).map_err(|e| {
                    CommandError::new(kiln_command::tr!("argument.resource_or_id.failed_to_parse", e.to_string()))
                })?;
                let t = std::sync::Arc::new(t);
                let mut rng = t.random(0, &mut sequences, &mut level);
                Ok((loot.random_items(&t, ctx, rng.source()), None))
            }
        }
    }

    /// `/loot`'s sources.
    pub(crate) fn roll_command_loot(&mut self, source: &kiln_command::host::LootSource<PlayerRef>) -> Result<(Vec<Tag>, Option<String>), CommandError> {
        use kiln_command::host::LootSource;
        let (items, table) = match source {
            LootSource::Table { table, origin, this, .. } => {
                let ctx = CommandLootContext { origin: *origin, this: this.is_some(), tool: None };
                let (items, _) = self.roll_table(table, &ctx)?;
                (items, None)
            }
            LootSource::Fish { table, pos, tool, this, .. } => {
                let ctx = CommandLootContext {
                    origin: [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5],
                    this: this.is_some(),
                    tool: Some(stack_of(tool.as_ref())),
                };
                let (items, _) = self.roll_table(table, &ctx)?;
                (items, None)
            }
            LootSource::Mine { pos, dimension, tool, this } => {
                let state = kiln_command::Host::block_state(self, dimension, *pos);
                let name = kiln_blocks::BlockId::of(state).name();
                let Some(loot) = self.loot.clone() else { return Err(CommandError::unsupported("Loot tables")) };
                let Some(table_id) = loot.block_table(name) else {
                    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
                    let block = kiln_command::Text::translate(format!("block.{ns}.{path}"), Vec::new());
                    return Err(CommandError::new(kiln_command::tr!("commands.drop.no_loot_table.block", block)));
                };
                let ctx = crate::blocks::BreakContext {
                    tool: stack_of(tool.as_ref()),
                    player: this.is_some(),
                    state,
                    origin: [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5],
                    block_entity: None,
                };
                let (items, _) = self.roll_table(&kiln_command::host::LootTableArg::Id(table_id.to_string()), &ctx)?;
                (items, Some(table_id.to_string()))
            }
            LootSource::Kill { target, origin, killer } => {
                let (table, type_name, baby) = if target.entity.is_none() {
                    ("minecraft:entities/player".to_owned(), "minecraft:player", false)
                } else {
                    let found = self.entity_mut(target).and_then(|e| e.phys.as_ref()).and_then(|p| {
                        let m = kiln_entity::mob::data(p)?;
                        Some((m.kind.ext().and_then(|k| k.loot_table(m)).unwrap_or_else(|| m.kind.loot_table()), p.type_name, m.baby()))
                    });
                    match found {
                        Some(f) => f,
                        None => return Err(CommandError::new(kiln_command::tr!("commands.drop.no_loot_table", kiln_command::SelectorTarget::display_name(target)))),
                    }
                };
                let weapon = killer.as_ref().filter(|k| k.entity.is_none()).and_then(|k| self.players.get(&k.conn)).map(|p| p.inv.selected_item().clone());
                let ctx = crate::mobs::DeathContext {
                    type_name,
                    origin: *origin,
                    on_fire: false,
                    baby,
                    killed_by_player: weapon.is_some(),
                    damage_type: "minecraft:magic",
                    weapon,
                };
                self.roll_table(&kiln_command::host::LootTableArg::Id(table), &ctx)?
            }
        };
        Ok((items.into_iter().filter(|s| !s.is_empty()).map(|s| s.to_nbt()).collect(), table))
    }

    /// `getItemBySlot(MAINHAND / OFFHAND)`.
    pub(crate) fn hand_item_nbt(&mut self, target: &PlayerRef, offhand: bool) -> Option<Option<Tag>> {
        let stack = if target.entity.is_none() {
            let p = self.players.get(&target.conn)?;
            if offhand { p.inv.equipment[4].clone() } else { p.inv.selected_item().clone() }
        } else {
            let m = self.entity_mut(target)?.phys.as_ref().and_then(kiln_entity::mob::data)?;
            m.equipment[offhand as usize].clone()
        };
        Some((!stack.is_empty()).then(|| stack.to_nbt()))
    }

    /// `Inventory.add(copy)`.
    pub(crate) fn give_stack_nbt(&mut self, player: &PlayerRef, item: &Tag) -> bool {
        let Some(p) = self.players.get_mut(&player.conn) else { return false };
        let mut stack = stack_of(Some(item));
        let creative = p.game_mode == 1;
        let added = p.inv.add(None, &mut stack, creative);
        if added {
            p.inv.times_changed += 1;
        }
        added
    }

    /// A dropped item entity (`ItemEntity` with the default pickup delay).
    pub(crate) fn spawn_item_nbt(&mut self, dimension: &str, pos: [f64; 3], item: &Tag) {
        let Some(dim) = crate::dim_id(dimension) else { return };
        let stack = stack_of(Some(item));
        if stack.is_empty() {
            return;
        }
        let h = self.command_loot_seed() as u64;
        let spawn = crate::mobs::drop_item(stack, pos, h);
        self.dims[dim].spawns.push(spawn);
    }

    pub(crate) fn container_size_at(&mut self, dimension: &str, pos: [i32; 3]) -> Option<i32> {
        kiln_command::Host::block_entity(self, dimension, pos).and_then(|d| d.get("id").and_then(Tag::as_str).and_then(container_size))
    }
}

pub(crate) enum AttributeChange {
    Base(Option<f64>),
    AddModifier(String, f64, u8),
    RemoveModifier(String),
}

/// The player inventory index of an equipment slot id (`Inventory.EQUIPMENT_SLOT_MAPPING`:
/// feet, legs, chest, head, off hand, body, saddle).
fn equipment_index(slot: i32) -> Option<usize> {
    Some(match slot {
        100 => 0,
        101 => 1,
        102 => 2,
        103 => 3,
        99 => 4,
        105 => 5,
        106 => 6,
        _ => return None,
    })
}

/// A mob's equipment slot (main hand, off hand, feet, legs, chest, head).
fn mob_equipment_index(slot: i32) -> Option<usize> {
    Some(match slot {
        98 => 0,
        99 => 1,
        100..=103 => (slot - 98) as usize,
        _ => return None,
    })
}

fn mob_slot(m: &kiln_entity::mob::MobData, slot: i32) -> Option<kiln_item::ItemStack> {
    Some(m.equipment[mob_equipment_index(slot)?].clone())
}

fn set_mob_slot(m: &mut kiln_entity::mob::MobData, slot: i32, stack: kiln_item::ItemStack) -> bool {
    match mob_equipment_index(slot) {
        Some(i) => {
            m.equipment[i] = stack;
            true
        }
        None => false,
    }
}

/// `Container.getContainerSize` of the block entities that are containers.
fn container_size(id: &str) -> Option<i32> {
    Some(match id.strip_prefix("minecraft:").unwrap_or(id) {
        "chest" | "trapped_chest" | "barrel" | "shulker_box" => 27,
        "dispenser" | "dropper" | "crafter" => 9,
        "hopper" | "brewing_stand" => 5,
        "furnace" | "blast_furnace" | "smoker" => 3,
        "chiseled_bookshelf" => 6,
        "shelf" => 3,
        "decorated_pot" | "jukebox" => 1,
        _ => return None,
    })
}

/// `ItemStack.getHoverName`: the item's name (custom names are not read yet).
fn hover_name(stack: &kiln_item::ItemStack) -> kiln_command::Text {
    let name = stack.item_name();
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let kind = if kiln_data::builtin_id("minecraft:block", name).is_some() { "block" } else { "item" };
    kiln_command::Text::translate(format!("{kind}.{ns}.{path}"), Vec::new())
}
