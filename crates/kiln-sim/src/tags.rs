//! Tags of the enabled data packs (`TagLoader` then `TagNetworkSerialization.serializeTagsToNetwork`):
//! the tag files of every registry, merged in pack order (`replace` clears what earlier packs
//! added), resolved to network ids. They become the tags the game logic reads
//! (`kiln_data::live::install_tags`), `/reload` broadcasts them (`PlayerList.reloadResources`)
//! and logins get them in the configuration phase.
//!
//! The vanilla pack's tags come from its files when Kiln has them, otherwise from the built-in
//! copy (`kiln_data::registries::TAGS`, the same tags already resolved).

use kiln_loot::tags::Tags;
pub(crate) use kiln_loot::tags::TagSource;

/// Tags per registry, as network ids: `[(registry, [(tag, [id])])]`.
pub(crate) type NetworkTags = Vec<(String, Vec<(String, Vec<i32>)>)>;

/// Whether registry entry `id` exists (a tag naming a missing entry does not load).
fn known(registry: &str, id: &kiln_item::Identifier) -> bool {
    if kiln_loot::tags::has_id_table(registry) {
        kiln_data::builtin_id(registry, id.as_str()).is_some() || kiln_data::live::synced_id(registry, id.as_str()).is_some()
    } else {
        true
    }
}

/// The tags of the packs in load order, and what did not load.
pub(crate) fn load(packs: &[TagSource]) -> (NetworkTags, Vec<String>) {
    let tags = match Tags::load_sources(packs, known) {
        Ok(tags) => tags,
        Err(e) => return (Vec::new(), vec![e]),
    };
    (network(&tags), tags.errors.clone())
}

/// The network form of resolved tags: the registries Kiln's vanilla tables list, in their order,
/// then any other registry a pack has tags for.
pub(crate) fn network(tags: &Tags) -> NetworkTags {
    let mut registries: Vec<&str> = kiln_data::registries::TAGS.iter().map(|(r, _)| *r).collect();
    let mut extra: Vec<&str> = tags.registries().filter(|r| !registries.contains(r) && kiln_loot::tags::has_id_table(r)).collect();
    extra.sort_unstable();
    registries.extend(extra);
    registries
        .into_iter()
        .map(|registry| {
            let mut list: Vec<(String, Vec<i32>)> =
                tags.all_ids(registry).map(|(tag, set)| (tag.as_str().to_owned(), set.ids().to_vec())).collect();
            list.sort();
            (registry.to_owned(), list)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

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
        assert_eq!(sorted(load(&[TagSource::Builtin]).0), sorted(builtin()));
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
        let loaded = load(&[TagSource::Dir(&root)]).0;
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
        let (loaded, errors) = load(&[TagSource::Builtin, TagSource::Dir(&dir)]);
        assert!(errors.iter().any(|e| e.contains("minecraft:broken")), "{errors:?}");
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
