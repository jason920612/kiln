//! Block entities: each keeps its saved NBT losslessly; what clients see (the vanilla update
//! tag) is derived from it per type.

use kiln_proto::nbt::Tag;

#[derive(Debug, Clone, PartialEq)]
pub struct BlockEntity {
    /// Protocol id in `minecraft:block_entity_type`.
    pub kind: u16,
    /// The saved form as in chunk NBT (`id`, `components` and the type's own fields). The
    /// position fields are rewritten from the block entity's place in the chunk when saved.
    pub nbt: Tag,
}

/// Keys vanilla's `saveWithFullMetadata` adds around the type's own fields.
const METADATA: [&str; 5] = ["id", "x", "y", "z", "keepPacked"];

/// What `getUpdateTag` returns per type, read off the 26.3 bytecode.
enum UpdateTag {
    /// Not overridden: empty.
    Empty,
    /// `saveCustomOnly`: the type's fields without components.
    Custom,
    /// `saveWithoutMetadata`: the type's fields and components.
    WithComponents,
    /// `saveCustomOnly` minus some fields (spawners drop `SpawnPotentials`).
    CustomWithout(&'static [&'static str]),
    /// Selected fields; `true` marks a list the type always writes, even when empty.
    Fields(&'static [(&'static str, bool)]),
    /// Trial spawners: `next_mob_spawns_at` while active, and the next spawn's data.
    TrialSpawner,
}

fn update_rule(type_name: &str) -> UpdateTag {
    match type_name.strip_prefix("minecraft:").unwrap_or(type_name) {
        "banner" => UpdateTag::WithComponents,
        "beacon" | "conduit" | "creaking_heart" | "decorated_pot" | "end_gateway" | "hanging_sign" | "jigsaw"
        | "piston" | "sign" | "skull" | "structure_block" | "test_block" | "test_instance_block" => UpdateTag::Custom,
        "mob_spawner" => UpdateTag::CustomWithout(&["SpawnPotentials"]),
        "campfire" => UpdateTag::Fields(&[("Items", true)]),
        "shelf" => UpdateTag::Fields(&[("Items", true), ("align_items_to_bottom", false)]),
        "brushable_block" => UpdateTag::Fields(&[("item", false)]),
        "vault" => UpdateTag::Fields(&[("shared_data", false)]),
        "trial_spawner" => UpdateTag::TrialSpawner,
        _ => UpdateTag::Empty,
    }
}

/// Protocol id of a block entity type by name.
pub fn type_id(name: &str) -> Option<u16> {
    kiln_data::builtin_id("minecraft:block_entity_type", name).map(|i| i as u16)
}

pub fn type_name(kind: u16) -> &'static str {
    kiln_data::builtin_entries("minecraft:block_entity_type")
        .and_then(|e| e.get(kind as usize))
        .copied()
        .unwrap_or("minecraft:unknown")
}

/// Whether vanilla sends Block Entity Data when a block of this type changes
/// (`getUpdatePacket` is overridden).
pub fn sends_updates(kind: u16) -> bool {
    matches!(
        type_name(kind).strip_prefix("minecraft:").unwrap_or(""),
        "banner"
            | "beacon"
            | "brushable_block"
            | "campfire"
            | "conduit"
            | "copper_golem_statue"
            | "creaking_heart"
            | "decorated_pot"
            | "end_gateway"
            | "hanging_sign"
            | "jigsaw"
            | "mob_spawner"
            | "shelf"
            | "sign"
            | "skull"
            | "structure_block"
            | "test_block"
            | "test_instance_block"
            | "trial_spawner"
            | "vault"
    )
}

impl BlockEntity {
    /// A block entity with default contents: vanilla loads missing fields as defaults.
    pub fn new(kind: u16) -> Self {
        let name = type_name(kind);
        let mut fields = vec![("id".into(), Tag::String(name.to_owned()))];
        // A new sign saves both sides empty, black and unwaxed (`SignBlockEntity.saveAdditional`).
        if matches!(name, "minecraft:sign" | "minecraft:hanging_sign") {
            let side = || {
                Tag::Compound(vec![
                    ("color".into(), Tag::String("black".into())),
                    ("has_glowing_text".into(), Tag::Byte(0)),
                    ("messages".into(), Tag::List(vec![Tag::String(String::new()), Tag::String(String::new()), Tag::String(String::new()), Tag::String(String::new())])),
                ])
            };
            fields.push(("front_text".into(), side()));
            fields.push(("back_text".into(), side()));
            fields.push(("is_waxed".into(), Tag::Byte(0)));
            fields.push(("components".into(), Tag::Compound(Vec::new())));
        }
        Self { kind, nbt: Tag::Compound(fields) }
    }

    /// From a chunk's `block_entities` entry; `None` if its `id` is not a known type.
    pub fn from_saved(nbt: Tag) -> Option<Self> {
        let kind = type_id(nbt.get("id")?.as_str()?)?;
        Some(Self { kind, nbt })
    }

    /// The saved form at `pos` (absolute block coordinates).
    pub fn saved(&self, pos: [i32; 3]) -> Tag {
        let mut fields = match &self.nbt {
            Tag::Compound(f) => f.clone(),
            _ => Vec::new(),
        };
        fields.retain(|(k, _)| !matches!(k.as_str(), "id" | "x" | "y" | "z"));
        let mut out = Vec::with_capacity(fields.len() + 4);
        out.push(("id".to_owned(), Tag::String(type_name(self.kind).to_owned())));
        out.push(("x".to_owned(), Tag::Int(pos[0])));
        out.push(("y".to_owned(), Tag::Int(pos[1])));
        out.push(("z".to_owned(), Tag::Int(pos[2])));
        out.extend(fields);
        Tag::Compound(out)
    }

    /// The tag clients get in chunk data and Block Entity Data (`None` when empty). `state` is
    /// the block state at the block entity (trial spawners depend on it).
    pub fn update_tag(&self, state: u16) -> Option<Tag> {
        let Tag::Compound(fields) = &self.nbt else { return None };
        let own = || fields.iter().filter(|(k, _)| !METADATA.contains(&k.as_str()));
        let out: Vec<(String, Tag)> = match update_rule(type_name(self.kind)) {
            UpdateTag::Empty => Vec::new(),
            UpdateTag::WithComponents => own().cloned().collect(),
            UpdateTag::Custom => own().filter(|(k, _)| k != "components").cloned().collect(),
            UpdateTag::CustomWithout(drop) => {
                own().filter(|(k, _)| k != "components" && !drop.contains(&k.as_str())).cloned().collect()
            }
            UpdateTag::Fields(keys) => keys
                .iter()
                .filter_map(|&(k, always)| match self.nbt.get(k) {
                    Some(v) => Some((k.to_owned(), v.clone())),
                    None => always.then(|| (k.to_owned(), Tag::List(Vec::new()))),
                })
                .collect(),
            UpdateTag::TrialSpawner => {
                let mut out = Vec::new();
                let block = kiln_data::blocks_types::block_of(state);
                if block.property(state, "trial_spawner_state") == Some("active") {
                    let at = self.nbt.get("next_mob_spawns_at").and_then(Tag::as_i64).unwrap_or(0);
                    out.push(("next_mob_spawns_at".to_owned(), Tag::Long(at)));
                }
                if let Some(d) = self.nbt.get("spawn_data") {
                    out.push(("spawn_data".to_owned(), d.clone()));
                }
                out
            }
        };
        (!out.is_empty()).then_some(Tag::Compound(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as block;

    fn compound(fields: &[(&str, Tag)]) -> Tag {
        Tag::Compound(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn update_tags_follow_the_type() {
        let text = compound(&[("messages", Tag::List(vec![Tag::String("hi".into())]))]);
        let sign = BlockEntity::from_saved(compound(&[
            ("id", Tag::String("minecraft:sign".into())),
            ("x", Tag::Int(1)),
            ("y", Tag::Int(2)),
            ("z", Tag::Int(3)),
            ("front_text", text.clone()),
            ("is_waxed", Tag::Byte(0)),
            ("components", compound(&[])),
        ]))
        .unwrap();
        assert_eq!(sign.update_tag(block::OAK_SIGN), Some(compound(&[("front_text", text), ("is_waxed", Tag::Byte(0))])));

        let lore = compound(&[("minecraft:custom_name", Tag::String("x".into()))]);
        let banner = BlockEntity::from_saved(compound(&[
            ("id", Tag::String("minecraft:banner".into())),
            ("patterns", Tag::List(Vec::new())),
            ("components", lore.clone()),
        ]))
        .unwrap();
        assert_eq!(banner.update_tag(block::WHITE_BANNER), Some(compound(&[("patterns", Tag::List(Vec::new())), ("components", lore)])));

        let chest = BlockEntity::from_saved(compound(&[
            ("id", Tag::String("minecraft:chest".into())),
            ("Items", Tag::List(Vec::new())),
        ]))
        .unwrap();
        assert_eq!(chest.update_tag(block::CHEST), None);

        let campfire = BlockEntity::new(type_id("minecraft:campfire").unwrap());
        assert_eq!(campfire.update_tag(block::CAMPFIRE), Some(compound(&[("Items", Tag::List(Vec::new()))])));

        let spawner = BlockEntity::from_saved(compound(&[
            ("id", Tag::String("minecraft:mob_spawner".into())),
            ("Delay", Tag::Short(20)),
            ("SpawnPotentials", Tag::List(Vec::new())),
        ]))
        .unwrap();
        assert_eq!(spawner.update_tag(block::SPAWNER), Some(compound(&[("Delay", Tag::Short(20))])));
    }

    #[test]
    fn trial_spawner_depends_on_the_state() {
        let be = BlockEntity::from_saved(compound(&[
            ("id", Tag::String("minecraft:trial_spawner".into())),
            ("next_mob_spawns_at", Tag::Long(99)),
            ("total_mobs_spawned", Tag::Int(3)),
        ]))
        .unwrap();
        let info = kiln_data::blocks_types::block_by_name("minecraft:trial_spawner").unwrap();
        let active = info.with_property(info.default, "trial_spawner_state", "active").unwrap();
        assert_eq!(be.update_tag(info.default), None);
        assert_eq!(be.update_tag(active), Some(compound(&[("next_mob_spawns_at", Tag::Long(99))])));
    }

    #[test]
    fn saved_form_takes_the_position_given() {
        let be = BlockEntity::new(type_id("minecraft:chest").unwrap());
        let saved = be.saved([-5, 70, 12]);
        assert_eq!(saved.get("id").and_then(Tag::as_str), Some("minecraft:chest"));
        assert_eq!(saved.get("x").and_then(Tag::as_i64), Some(-5));
        assert_eq!(saved.get("z").and_then(Tag::as_i64), Some(12));
        assert!(sends_updates(type_id("minecraft:sign").unwrap()));
        assert!(!sends_updates(type_id("minecraft:chest").unwrap()));
    }
}
