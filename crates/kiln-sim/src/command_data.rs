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
    out.sort_by_key(|(s, _)| (*s as u8));
    *items = out.into_iter().map(|(_, t)| t).collect();
}
