//! The data-driven (synchronized) registries of the enabled data packs
//! (`RegistryDataLoader` + `RegistrySynchronization`).
//!
//! Vanilla loads these registries once, when the world loads; `/reload` does not change them.
//! Each registry's entries are the files `data/<namespace>/<registry path>/**.json` of the packs
//! (a later pack's file replaces an earlier one's), numbered in the order the loader registers
//! them: sorted by identifier, which compares the *path* first and the namespace second
//! (`Identifier.compareTo`). So a pack's new entry shifts the ids of the entries sorted after it.
//!
//! The client takes the entries of `minecraft:core` from its own copy; Kiln sends the data
//! (`RegistryData` with the entry's NBT) of every entry that comes from another pack, which
//! includes the vanilla entries a pack overrides. Registries whose network codec leaves fields
//! out (the variants' `spawn_conditions`, a biome's features and spawners) send the rest.

use kiln_loot::Json;
use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tracing::{error, warn};

/// What the packs add to the synchronized registries.
#[derive(Default)]
pub(crate) struct Loaded {
    /// Every synchronized registry whose entries changed: names by network id.
    pub names: Vec<(String, Vec<String>)>,
    /// The JSON of the entries a pack defines or overrides: `((registry, name), text)`.
    pub data: Vec<((String, String), String)>,
    /// The `RegistryData` packets of all synchronized registries, in sending order.
    pub packets: Vec<bytes::Bytes>,
}

/// `Identifier.compareTo`: path, then namespace.
fn identifier_order(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &'_ str| match s.split_once(':') {
        Some((ns, path)) => (path.to_owned(), ns.to_owned()),
        None => (s.to_owned(), "minecraft".to_owned()),
    };
    split(a).cmp(&split(b))
}

/// The registries with `roots` (the enabled packs other than the vanilla data, in load order).
pub(crate) fn load(roots: &[PathBuf]) -> Loaded {
    let mut out = Loaded::default();
    let mut entries: Vec<(&str, Vec<(String, Option<Tag>)>)> = Vec::new();
    let mut changed = false;
    for (registry, vanilla) in kiln_data::registries::SYNCHRONIZED {
        let dir = registry.strip_prefix("minecraft:").unwrap_or(registry);
        let mut from_packs: BTreeMap<String, String> = BTreeMap::new();
        for root in roots {
            for (ns, rel, path) in crate::datapacks::pack_files(root, dir, ".json") {
                let name = format!("{ns}:{rel}");
                if kiln_item::Identifier::parse(&name).is_none() {
                    warn!("ignoring {registry} entry with the invalid id {name}");
                    continue;
                }
                let text = match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        error!("Failed to read {registry}/{name}: {e}");
                        continue;
                    }
                };
                match Json::parse(text.trim_start_matches('\u{feff}')) {
                    Err(e) => {
                        error!("Failed to parse {registry}/{name}: {e}");
                        continue;
                    }
                    Ok(json) => {
                        // Vanilla stops loading on an entry its codec rejects; a client that gets one disconnects. Entries
                        // without the fields the codec requires are left out here, with the same kind of message.
                        let missing: Vec<&str> = required(registry).iter().copied().filter(|k| json.get(k).is_none()).collect();
                        if !missing.is_empty() {
                            error!("Failed to load {registry}/{name}: missing required fields {}", missing.join(", "));
                            continue;
                        }
                        if *registry == "minecraft:dimension_type" {
                            let checked = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()).and_then(|v| crate::dimensions::dimension_type_from_json(&name, &v, &|_| Vec::new()));
                            if let Err(e) = checked {
                                error!("Failed to load {registry}/{name}: {e}");
                                continue;
                            }
                        }
                    }
                }
                from_packs.insert(name, text);
            }
        }
        let mut names: Vec<String> = vanilla.iter().map(|n| (*n).to_owned()).collect();
        for name in from_packs.keys() {
            if !vanilla.contains(&name.as_str()) {
                names.push(name.clone());
            }
        }
        names.sort_by(|a, b| identifier_order(a, b));
        let list: Vec<(String, Option<Tag>)> = names
            .iter()
            .map(|n| {
                let tag = from_packs.get(n).and_then(|text| {
                    let json = Json::parse(text.trim_start_matches('\u{feff}')).ok()?;
                    Some(network_entry(registry, &json))
                });
                (n.clone(), tag)
            })
            .collect();
        if !from_packs.is_empty() {
            changed = true;
        }
        if names.len() != vanilla.len() || names.iter().zip(*vanilla).any(|(a, b)| a != b) {
            out.names.push(((*registry).to_owned(), names));
        }
        out.data.extend(from_packs.into_iter().map(|(n, t)| (((*registry).to_owned(), n), t)));
        entries.push((registry, list));
    }
    if changed {
        out.packets = entries
            .iter()
            .map(|(registry, list)| {
                let refs: Vec<(&str, Option<&Tag>)> = list.iter().map(|(n, t)| (n.as_str(), t.as_ref())).collect();
                kiln_proto::packets::registry_data_entries(registry, &refs)
            })
            .collect();
    }
    out
}

/// The fields a registry's codec requires of every entry (`fieldOf`, not `optionalFieldOf`), for the registries Kiln reads.
fn required(registry: &str) -> &'static [&'static str] {
    match registry {
        "minecraft:painting_variant" => &["asset_id", "width", "height"],
        "minecraft:damage_type" => &["message_id", "scaling", "exhaustion"],
        "minecraft:jukebox_song" => &["sound_event", "description", "length_in_seconds", "comparator_output"],
        "minecraft:instrument" => &["sound_event", "use_duration", "range", "description"],
        "minecraft:trim_pattern" => &["asset_id", "description"],
        "minecraft:trim_material" => &["description"],
        "minecraft:banner_pattern" => &["asset_id", "translation_key"],
        "minecraft:decorated_pot_pattern" => &["asset_id"],
        "minecraft:wolf_variant" => &["assets", "baby_assets", "spawn_conditions"],
        "minecraft:cat_variant" | "minecraft:pig_variant" | "minecraft:cow_variant" | "minecraft:chicken_variant" => &["asset_id", "baby_asset_id", "spawn_conditions"],
        "minecraft:frog_variant" | "minecraft:zombie_nautilus_variant" => &["asset_id", "spawn_conditions"],
        "minecraft:chat_type" => &["chat", "narration"],
        "minecraft:dimension_type" => &["has_skylight", "has_ceiling", "has_ender_dragon_fight", "coordinate_scale", "min_y", "height", "logical_height", "infiniburn", "ambient_light"],
        "minecraft:dialog" => &["type"],
        _ => &[],
    }
}

/// The fields a registry's network codec leaves out (`NETWORK_CODEC` against `DIRECT_CODEC`).
fn omitted(registry: &str) -> &'static [&'static str] {
    match registry {
        "minecraft:wolf_variant"
        | "minecraft:pig_variant"
        | "minecraft:frog_variant"
        | "minecraft:cat_variant"
        | "minecraft:cow_variant"
        | "minecraft:chicken_variant"
        | "minecraft:zombie_nautilus_variant" => &["spawn_conditions"],
        "minecraft:worldgen/biome" => &["carvers", "features", "spawners", "spawn_costs", "creature_spawn_probability"],
        _ => &[],
    }
}

/// An entry as the network sends it.
pub(crate) fn network_entry(registry: &str, json: &Json) -> Tag {
    let mut tag = json_to_nbt(json);
    if let Tag::Compound(fields) = &mut tag {
        let skip = omitted(registry);
        fields.retain(|(k, _)| !skip.contains(&k.as_str()));
    }
    tag
}

/// JSON as NBT the way `JsonOps` → `NbtOps` conversion types it: whole numbers are ints (longs
/// when they do not fit), others doubles, booleans bytes. A list of numbers has one type; a list of
/// other mixed values wraps each non-compound in `{"": value}` (`NbtOps`' heterogeneous lists).
/// The client's codecs read any numeric tag as the number they ask for.
pub(crate) fn json_to_nbt(json: &Json) -> Tag {
    match json {
        Json::Null => Tag::Compound(Vec::new()),
        Json::Bool(b) => Tag::Byte(i8::from(*b)),
        Json::Num(text) => number(text),
        Json::Str(s) => Tag::String(s.clone()),
        Json::Arr(items) => {
            let tags: Vec<Tag> = items.iter().map(json_to_nbt).collect();
            Tag::List(unify(tags))
        }
        Json::Obj(fields) => Tag::Compound(fields.iter().map(|(k, v)| (k.clone(), json_to_nbt(v))).collect()),
    }
}

fn number(text: &str) -> Tag {
    if !text.contains(['.', 'e', 'E']) {
        if let Ok(v) = text.parse::<i64>() {
            return match i32::try_from(v) {
                Ok(i) => Tag::Int(i),
                Err(_) => Tag::Long(v),
            };
        }
    }
    Tag::Double(text.parse::<f64>().unwrap_or(0.0))
}

fn kind(tag: &Tag) -> u8 {
    match tag {
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 3,
        Tag::Long(_) => 4,
        Tag::Float(_) => 5,
        Tag::Double(_) => 6,
        Tag::ByteArray(_) => 7,
        Tag::String(_) => 8,
        Tag::List(_) => 9,
        Tag::Compound(_) => 10,
        Tag::IntArray(_) => 11,
        Tag::LongArray(_) => 12,
    }
}

/// Makes a list's elements one type (see [`json_to_nbt`]).
fn unify(tags: Vec<Tag>) -> Vec<Tag> {
    let Some(first) = tags.first() else { return tags };
    let same = tags.iter().all(|t| kind(t) == kind(first));
    if same {
        return tags;
    }
    let numeric = |t: &Tag| matches!(t, Tag::Int(_) | Tag::Long(_) | Tag::Double(_));
    if tags.iter().all(numeric) {
        if tags.iter().any(|t| matches!(t, Tag::Double(_))) {
            return tags
                .into_iter()
                .map(|t| match t {
                    Tag::Int(i) => Tag::Double(f64::from(i)),
                    Tag::Long(l) => Tag::Double(l as f64),
                    other => other,
                })
                .collect();
        }
        return tags.into_iter().map(|t| if let Tag::Int(i) = t { Tag::Long(i64::from(i)) } else { t }).collect();
    }
    tags.into_iter().map(|t| if matches!(t, Tag::Compound(_)) { t } else { Tag::Compound(vec![(String::new(), t)]) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_sort_by_path_then_namespace() {
        let mut v = vec!["minecraft:b", "kiln:b", "kiln:a", "minecraft:a", "zeta:a/b", "minecraft:a_b"];
        v.sort_by(|a, b| identifier_order(a, b));
        assert_eq!(v, ["kiln:a", "minecraft:a", "zeta:a/b", "minecraft:a_b", "kiln:b", "minecraft:b"]);
    }

    #[test]
    fn json_numbers_keep_whole_and_fractional_apart() {
        let j = Json::parse(r#"{"a":1,"b":2.5,"c":[1,2.5],"d":true,"e":["x",{"y":1}],"f":5000000000}"#).unwrap();
        let Tag::Compound(f) = json_to_nbt(&j) else { panic!() };
        assert_eq!(f[0].1, Tag::Int(1));
        assert_eq!(f[1].1, Tag::Double(2.5));
        assert_eq!(f[2].1, Tag::List(vec![Tag::Double(1.0), Tag::Double(2.5)]));
        assert_eq!(f[3].1, Tag::Byte(1));
        assert_eq!(f[4].1, Tag::List(vec![Tag::Compound(vec![(String::new(), Tag::String("x".into()))]), Tag::Compound(vec![("y".into(), Tag::Int(1))])]));
        assert_eq!(f[5].1, Tag::Long(5_000_000_000));
    }

    #[test]
    fn variants_lose_their_spawn_conditions() {
        let j = Json::parse(r#"{"assets":{"asset_id":"a:b"},"spawn_conditions":[{"priority":0}]}"#).unwrap();
        let Tag::Compound(f) = network_entry("minecraft:wolf_variant", &j) else { panic!() };
        assert_eq!(f.len(), 1);
        let Tag::Compound(f) = network_entry("minecraft:painting_variant", &j) else { panic!() };
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn entries_the_codec_rejects_are_left_out() {
        let dir = std::env::temp_dir().join(format!("kiln-regdata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paintings = dir.join("data/t/painting_variant");
        std::fs::create_dir_all(&paintings).unwrap();
        std::fs::write(paintings.join("good.json"), r#"{"asset_id":"t:good","width":1,"height":1}"#).unwrap();
        std::fs::write(paintings.join("no_width.json"), r#"{"asset_id":"t:bad","height":1}"#).unwrap();
        std::fs::write(paintings.join("not_json.json"), "{").unwrap();
        let loaded = load(std::slice::from_ref(&dir));
        let (_, names) = loaded.names.iter().find(|(r, _)| r == "minecraft:painting_variant").unwrap();
        assert!(names.contains(&"t:good".to_owned()));
        assert!(!names.contains(&"t:no_width".to_owned()) && !names.contains(&"t:not_json".to_owned()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
