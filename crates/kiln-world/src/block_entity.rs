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
        "brushable_block" => UpdateTag::Fields(&[("hit_direction", false), ("item", false)]),
        "vault" => UpdateTag::Fields(&[("shared_data", false)]),
        "trial_spawner" => UpdateTag::TrialSpawner,
        _ => UpdateTag::Empty,
    }
}

/// The fields a vanilla block entity of this type saves from the start (26.3: `data get block` of one placed with
/// `/setblock`), which a block entity that nothing has touched leaves out of its stored form here. `Items` is
/// left out where a loot table stands in for it.
fn always_saved(type_name: &str) -> Vec<(&'static str, Tag)> {
    let int = |v: i32| Tag::Int(v);
    let items = || ("Items", Tag::List(Vec::new()));
    let listener = || {
        Tag::Compound(vec![
            ("selector".to_owned(), Tag::Compound(vec![("tick".to_owned(), Tag::Long(-1))])),
            ("event_delay".to_owned(), Tag::Int(0)),
        ])
    };
    match type_name.strip_prefix("minecraft:").unwrap_or(type_name) {
        "chest" | "trapped_chest" | "barrel" | "dropper" | "dispenser" | "copper_chest" => vec![items()],
        "hopper" => vec![items(), ("TransferCooldown", int(-1))],
        "furnace" | "blast_furnace" | "smoker" => vec![
            items(),
            ("RecipesUsed", Tag::Compound(Vec::new())),
            ("lit_time_remaining", int(0)),
            ("lit_total_time", int(0)),
            ("cooking_time_spent", int(0)),
            ("cooking_total_time", int(0)),
            ("speed_multiplier", Tag::Float(1.0)),
        ],
        "brewing_stand" => vec![
            items(),
            ("BrewTime", int(0)),
            ("Fuel", int(0)),
            ("speed_multiplier", Tag::Float(1.0)),
            ("total_brew_time", int(0)),
            ("total_fuel", int(0)),
        ],
        "campfire" | "soul_campfire" => vec![items(), ("CookingTimes", Tag::IntArray(vec![0; 4])), ("CookingTotalTimes", Tag::IntArray(vec![0; 4]))],
        "chiseled_bookshelf" => vec![items(), ("last_interacted_slot", int(-1))],
        "end_gateway" => vec![("Age", Tag::Long(0))],
        "vault" => vec![
            (
                "config",
                Tag::Compound(vec![(
                    "key_item".to_owned(),
                    Tag::Compound(vec![("count".to_owned(), Tag::Int(1)), ("id".to_owned(), Tag::String("minecraft:trial_key".into()))]),
                )]),
            ),
            ("server_data", Tag::Compound(Vec::new())),
            ("shared_data", Tag::Compound(Vec::new())),
        ],
        "sculk_sensor" | "calibrated_sculk_sensor" => vec![("last_vibration_frequency", int(0)), ("listener", listener())],
        "sculk_shrieker" => vec![("warning_level", int(0)), ("listener", listener())],
        "sculk_catalyst" => vec![("cursors", Tag::List(Vec::new()))],
        "bee_nest" | "beehive" => vec![("bees", Tag::List(Vec::new()))],
        // `CommandBlockEntity.saveAdditional` .
        "command_block" | "chain_command_block" | "repeating_command_block" => vec![
            ("Command", Tag::String(String::new())),
            ("SuccessCount", int(0)),
            ("TrackOutput", Tag::Byte(1)),
            ("UpdateLastExecution", Tag::Byte(1)),
            ("powered", Tag::Byte(0)),
            ("conditionMet", Tag::Byte(0)),
            // (`auto` of a chain block, which starts automatic, is set where the block is placed.)
            ("auto", Tag::Byte(0)),
        ],
        _ => Vec::new(),
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
        // What a new block entity saves before anything has happened to it (read off the 26.3 `saveAdditional`s).
        let int = |k: &str, v: i32| (k.to_owned(), Tag::Int(v));
        match name {
            // `PotentSulfurBlockEntity.waitingCountdown`.
            "minecraft:potent_sulfur" => fields.push(int("countdown", -1)),
            // `ShelfBlockEntity`: its three slots start empty, and the items stand in the middle.
            "minecraft:shelf" => {
                fields.push(("Items".into(), Tag::List(Vec::new())));
                fields.push(("align_items_to_bottom".into(), Tag::Byte(0)));
            }
            // `ComparatorBlockEntity.output`.
            "minecraft:comparator" => fields.push(int("OutputSignal", 0)),
            // `BeaconBlockEntity.levels` (clients are told it).
            "minecraft:beacon" => fields.push(int("Levels", 0)),
            "minecraft:test_block" => {
                fields.push(("message".into(), Tag::String(String::new())));
                fields.push(("mode".into(), Tag::String("start".into())));
                fields.push(("powered".into(), Tag::Byte(0)));
            }
            "minecraft:test_instance_block" => fields.push((
                "data".into(),
                Tag::Compound(vec![
                    ("ignore_entities".into(), Tag::Byte(0)),
                    ("rotation".into(), Tag::String("none".into())),
                    ("size".into(), Tag::IntArray(vec![0, 0, 0])),
                    ("status".into(), Tag::String("cleared".into())),
                ]),
            )),
            _ => {}
        }
        Self { kind, nbt: Tag::Compound(fields) }
    }

    /// Puts `fields` into the saved form: a field of the same name replaces the one that is there (the defaults of a new block entity).
    pub fn merge_fields(&mut self, fields: Vec<(String, Tag)>) {
        if let Tag::Compound(out) = &mut self.nbt {
            out.retain(|(k, _)| !fields.iter().any(|(f, _)| f == k));
            out.extend(fields);
        }
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
        // (A suspicious block's `hit_direction` is for the clients' update tag only.)
        if type_name(self.kind) == "minecraft:brushable_block" {
            fields.retain(|(k, _)| k != "hit_direction");
        }
        // What `saveAdditional` writes however little has happened to the block entity, and the components every
        // block entity saves (`saveWithoutMetadata`), even when it has none: `components: {}`.
        let loot = fields.iter().any(|(k, _)| k == "LootTable");
        for (key, value) in always_saved(type_name(self.kind)) {
            if !fields.iter().any(|(k, _)| k == key) && !(key == "Items" && loot) {
                fields.push((key.to_owned(), value));
            }
        }
        if !fields.iter().any(|(k, _)| k == "components") {
            fields.push(("components".to_owned(), Tag::Compound(Vec::new())));
        }
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

    /// What a block entity placed with `/setblock` saves in 26.3 (`data get block`, read off a vanilla server), by block:
    /// the fields of its type that `saveAdditional` always writes, and `components`, even when empty.
    #[test]
    fn a_new_block_entity_saves_what_vanilla_saves() {
        let table: &[(&str, &[&str])] = &[
        ("chest", &["Items", "components"]),
        ("trapped_chest", &["Items", "components"]),
        ("ender_chest", &["components"]),
        ("barrel", &["Items", "components"]),
        ("furnace", &["Items", "RecipesUsed", "components", "cooking_time_spent", "cooking_total_time", "lit_time_remaining", "lit_total_time", "speed_multiplier"]),
        ("blast_furnace", &["Items", "RecipesUsed", "components", "cooking_time_spent", "cooking_total_time", "lit_time_remaining", "lit_total_time", "speed_multiplier"]),
        ("smoker", &["Items", "RecipesUsed", "components", "cooking_time_spent", "cooking_total_time", "lit_time_remaining", "lit_total_time", "speed_multiplier"]),
        ("hopper", &["Items", "TransferCooldown", "components"]),
        ("dropper", &["Items", "components"]),
        ("dispenser", &["Items", "components"]),
        ("brewing_stand", &["BrewTime", "Fuel", "Items", "components", "speed_multiplier", "total_brew_time", "total_fuel"]),
        ("jukebox", &["components"]),
        ("enchanting_table", &["components"]),
        ("lectern", &["components"]),
        ("bell", &["components"]),
        ("campfire", &["CookingTimes", "CookingTotalTimes", "Items", "components"]),
        ("soul_campfire", &["CookingTimes", "CookingTotalTimes", "Items", "components"]),
        ("beacon", &["Levels", "components"]),
        ("comparator", &["OutputSignal", "components"]),
        ("daylight_detector", &["components"]),
        ("end_portal", &["components"]),
        ("end_gateway", &["Age", "components"]),
        ("trial_spawner", &["components"]),
        ("vault", &["components", "config", "server_data", "shared_data"]),
        ("oak_sign", &["back_text", "components", "front_text", "is_waxed"]),
        ("oak_wall_sign", &["back_text", "components", "front_text", "is_waxed"]),
        ("oak_hanging_sign", &["back_text", "components", "front_text", "is_waxed"]),
        ("red_banner", &["components"]),
        ("white_wall_banner", &["components"]),
        ("player_head", &["components"]),
        ("skeleton_skull", &["components"]),
        ("conduit", &["components"]),
        ("sculk_sensor", &["components", "last_vibration_frequency", "listener"]),
        ("calibrated_sculk_sensor", &["components", "last_vibration_frequency", "listener"]),
        ("sculk_shrieker", &["components", "listener", "warning_level"]),
        ("sculk_catalyst", &["components", "cursors"]),
        ("decorated_pot", &["components"]),
        ("chiseled_bookshelf", &["Items", "components", "last_interacted_slot"]),
        ("suspicious_sand", &["components"]),
        ("suspicious_gravel", &["components"]),
        ("command_block", &["Command", "SuccessCount", "TrackOutput", "UpdateLastExecution", "auto", "components", "conditionMet", "powered"]),
        ("chain_command_block", &["Command", "SuccessCount", "TrackOutput", "UpdateLastExecution", "auto", "components", "conditionMet", "powered"]),
        ("repeating_command_block", &["Command", "SuccessCount", "TrackOutput", "UpdateLastExecution", "auto", "components", "conditionMet", "powered"]),
        ("creaking_heart", &["components"]),
        ("white_shulker_box", &["components"]),
        ("shulker_box", &["components"]),
        ("bee_nest", &["bees", "components"]),
        ("beehive", &["bees", "components"]),
        ("cherry_shelf", &["Items", "align_items_to_bottom", "components"]),
        ("copper_chest", &["Items", "components"]),
        ("copper_golem_statue", &["components"]),
        ];
        // (A structure block, a jigsaw and a test block start from fields their own modules fill in.)
        for (name, want) in table {
            let info = kiln_data::blocks_types::block_by_name(&format!("minecraft:{name}")).unwrap_or_else(|| panic!("block {name}"));
            let kind = kiln_data::block_props::block_entity_type(info.default).unwrap_or_else(|| panic!("{name} has no block entity"));
            let saved = BlockEntity::new(kind).saved([1, 2, 3]);
            let Tag::Compound(fields) = &saved else { panic!() };
            let mut keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).filter(|k| !matches!(*k, "id" | "x" | "y" | "z")).collect();
            keys.sort_unstable();
            let mut want = want.to_vec();
            want.sort_unstable();
            assert_eq!(keys, want, "{name}");
        }
        let chest = BlockEntity::new(type_id("minecraft:chest").unwrap());
        let tag = chest.saved([0, 0, 0]);
        assert_eq!(tag.get("Items"), Some(&Tag::List(Vec::new())));
        assert_eq!(tag.get("components"), Some(&Tag::Compound(Vec::new())));
        // A loot table stands in for the items (`trySaveLootTable`).
        let mut loot = BlockEntity::new(type_id("minecraft:chest").unwrap());
        loot.merge_fields(vec![("LootTable".into(), Tag::String("minecraft:chests/simple_dungeon".into()))]);
        assert!(loot.saved([0, 0, 0]).get("Items").is_none());
        let hopper = BlockEntity::new(type_id("minecraft:hopper").unwrap()).saved([0, 0, 0]);
        assert_eq!(hopper.get("TransferCooldown"), Some(&Tag::Int(-1)));
        let furnace = BlockEntity::new(type_id("minecraft:furnace").unwrap()).saved([0, 0, 0]);
        assert_eq!(furnace.get("speed_multiplier"), Some(&Tag::Float(1.0)));
        assert_eq!(furnace.get("RecipesUsed"), Some(&Tag::Compound(Vec::new())));
        let gateway = BlockEntity::new(type_id("minecraft:end_gateway").unwrap()).saved([0, 0, 0]);
        assert_eq!(gateway.get("Age"), Some(&Tag::Long(0)));
        assert!(gateway.get("ExactTeleport").is_none(), "only written when exact");
        // What the block entity holds stays as it is.
        let mut full = BlockEntity::new(type_id("minecraft:chest").unwrap());
        full.merge_fields(vec![("Items".into(), Tag::List(vec![Tag::Int(7)])), ("components".into(), Tag::Compound(vec![("minecraft:custom_name".into(), Tag::String("x".into()))]))]);
        let saved = full.saved([0, 0, 0]);
        assert_eq!(saved.get("Items"), Some(&Tag::List(vec![Tag::Int(7)])));
        assert!(matches!(saved.get("components"), Some(Tag::Compound(c)) if c.len() == 1));
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
