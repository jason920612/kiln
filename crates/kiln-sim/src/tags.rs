//! Tags of the enabled data packs for the network (`TagLoader` then
//! `TagNetworkSerialization.serializeTagsToNetwork`): the tag files of every synchronized
//! registry, merged in pack order (`replace` clears what earlier packs added), resolved to
//! network ids. `/reload` broadcasts them (`PlayerList.reloadResources`) and logins get them in
//! the configuration phase.
//!
//! The vanilla pack's tags come from its files when Kiln has them, otherwise from the built-in
//! copy (`kiln_data::registries::TAGS`, the same tags already resolved).

use kiln_loot::Json;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tracing::error;

/// Tags per registry, as network ids: `[(registry, [(tag, [id])])]`.
pub(crate) type NetworkTags = Vec<(String, Vec<(String, Vec<i32>)>)>;

/// A pack's contribution to the tags.
pub(crate) enum TagSource<'a> {
    /// A pack directory holding `data/`.
    Dir(&'a Path),
    /// The vanilla pack without its files: the built-in tags.
    BuiltIn,
}

struct Entry {
    id: String,
    required: bool,
}

fn entries(json: &Json) -> Vec<Entry> {
    let values = json.get("values").and_then(Json::as_array).unwrap_or(&[]);
    values
        .iter()
        .filter_map(|v| match v.as_str() {
            Some(s) => Some(Entry { id: s.to_owned(), required: true }),
            None => Some(Entry {
                id: v.get("id")?.as_str()?.to_owned(),
                required: v.get("required").and_then(Json::as_bool).unwrap_or(true),
            }),
        })
        .collect()
}

/// `minecraft:x` for `x`.
fn full_id(id: &str) -> String {
    if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") }
}

/// The entries of a registry by network id: synchronized registries first, then built-in ones.
fn registry_entries(registry: &str) -> Option<&'static [&'static str]> {
    kiln_data::registries::SYNCHRONIZED
        .iter()
        .find(|(r, _)| *r == registry)
        .map(|(_, e)| *e)
        .or_else(|| kiln_data::builtin_entries(registry))
}

/// Resolves `tag` (`TagLoader.build`): nested tags in place, elements in order, no repeats;
/// `None` when a required reference is missing.
fn resolve(
    tag: &str,
    raw: &BTreeMap<String, Vec<Entry>>,
    ids: &HashMap<&str, i32>,
    done: &mut HashMap<String, Option<Vec<i32>>>,
    visiting: &mut Vec<String>,
) -> Option<Vec<i32>> {
    if let Some(r) = done.get(tag) {
        return r.clone();
    }
    if visiting.iter().any(|v| v == tag) {
        return None;
    }
    visiting.push(tag.to_owned());
    let mut out: Vec<i32> = Vec::new();
    let mut ok = true;
    for e in raw.get(tag).map(Vec::as_slice).unwrap_or(&[]) {
        let found = match e.id.strip_prefix('#') {
            Some(t) => {
                let t = full_id(t);
                if raw.contains_key(&t) { resolve(&t, raw, ids, done, visiting) } else { None }
            }
            None => ids.get(full_id(&e.id).as_str()).map(|&i| vec![i]),
        };
        match found {
            Some(list) => {
                for i in list {
                    if !out.contains(&i) {
                        out.push(i);
                    }
                }
            }
            None if e.required => ok = false,
            None => {}
        }
    }
    visiting.pop();
    let result = ok.then_some(out);
    done.insert(tag.to_owned(), result.clone());
    result
}

/// The network tags of the packs in load order.
pub(crate) fn load(packs: &[TagSource]) -> NetworkTags {
    let mut out = Vec::new();
    for (registry, builtin) in kiln_data::registries::TAGS {
        let Some(names) = registry_entries(registry) else { continue };
        let ids: HashMap<&str, i32> = names.iter().enumerate().map(|(i, n)| (*n, i as i32)).collect();
        let dir = format!("tags/{}", registry.strip_prefix("minecraft:").unwrap_or(registry));
        let mut raw: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
        for pack in packs {
            match pack {
                TagSource::BuiltIn => {
                    for (tag, members) in *builtin {
                        let slot = raw.entry((*tag).to_owned()).or_default();
                        slot.extend(members.iter().filter_map(|&i| names.get(i as usize)).map(|n| Entry { id: (*n).to_owned(), required: true }));
                    }
                }
                TagSource::Dir(root) => {
                    for (ns, rel, path) in crate::datapacks::pack_files(root, &dir, ".json") {
                        let id = format!("{ns}:{rel}");
                        let text = std::fs::read_to_string(&path).unwrap_or_default();
                        let Ok(json) = Json::parse(&text) else {
                            error!("Couldn't read tag list {id} from {}", path.display());
                            continue;
                        };
                        let slot = raw.entry(id).or_default();
                        if json.get("replace").and_then(Json::as_bool).unwrap_or(false) {
                            slot.clear();
                        }
                        slot.extend(entries(&json));
                    }
                }
            }
        }
        let mut done = HashMap::new();
        let mut tags = Vec::new();
        for tag in raw.keys() {
            match resolve(tag, &raw, &ids, &mut done, &mut Vec::new()) {
                Some(list) => tags.push((tag.clone(), list)),
                None => error!("Couldn't load tag {tag} as it is missing following references"),
            }
        }
        out.push(((*registry).to_owned(), tags));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin() -> NetworkTags {
        kiln_data::registries::TAGS
            .iter()
            .map(|(r, tags)| ((*r).to_owned(), tags.iter().map(|(t, m)| ((*t).to_owned(), m.to_vec())).collect()))
            .collect()
    }

    fn sorted(mut tags: NetworkTags) -> NetworkTags {
        for (_, list) in &mut tags {
            list.sort();
        }
        tags
    }

    #[test]
    fn builtin_source_gives_the_builtin_tags() {
        assert_eq!(sorted(load(&[TagSource::BuiltIn])), sorted(builtin()));
    }

    /// The vanilla pack's tag files resolve to the tags vanilla sends.
    #[test]
    fn vanilla_files_match_the_builtin_tags() {
        let root = crate::datapack_dir(None);
        let root = if root.is_absolute() { root } else { Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(root) };
        if !root.join("data/minecraft/tags/block").is_dir() {
            eprintln!("skipped: no vanilla data at {}", root.display());
            return;
        }
        let loaded = load(&[TagSource::Dir(&root)]);
        let want = builtin();
        for ((reg, got), (_, want)) in loaded.iter().zip(&want) {
            let got: BTreeMap<_, _> = got.iter().map(|(t, m)| (t.clone(), m.clone())).collect();
            let want: BTreeMap<_, _> = want.iter().map(|(t, m)| (t.clone(), m.clone())).collect();
            assert_eq!(got.keys().collect::<Vec<_>>(), want.keys().collect::<Vec<_>>(), "{reg}");
            for (tag, members) in &want {
                let mut a = got[tag].clone();
                let mut b = members.clone();
                a.sort();
                b.sort();
                assert_eq!(a, b, "{reg} {tag}");
            }
        }
    }

    #[test]
    fn later_packs_add_and_replace() {
        let dir = std::env::temp_dir().join(format!("kiln-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tags = dir.join("data/minecraft/tags/block");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(tags.join("logs.json"), r#"{"values":["minecraft:stone"]}"#).unwrap();
        std::fs::write(tags.join("dirt.json"), r##"{"replace":true,"values":["minecraft:stone","#minecraft:logs",{"id":"minecraft:nope","required":false}]}"##).unwrap();
        std::fs::write(tags.join("broken.json"), r#"{"values":["minecraft:nope"]}"#).unwrap();
        let loaded = load(&[TagSource::BuiltIn, TagSource::Dir(&dir)]);
        let blocks = &loaded.iter().find(|(r, _)| r == "minecraft:block").unwrap().1;
        let get = |t: &str| blocks.iter().find(|(n, _)| n == t).map(|(_, m)| m.clone());
        let stone = kiln_data::builtin_id("minecraft:block", "minecraft:stone").unwrap();
        let oak = kiln_data::builtin_id("minecraft:block", "minecraft:oak_log").unwrap();
        let logs = get("minecraft:logs").unwrap();
        assert!(logs.contains(&oak) && logs.contains(&stone));
        let dirt = get("minecraft:dirt").unwrap();
        assert_eq!(dirt[0], stone);
        assert!(dirt.contains(&oak));
        assert!(!dirt.contains(&kiln_data::builtin_id("minecraft:block", "minecraft:dirt").unwrap()));
        assert_eq!(get("minecraft:broken"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
