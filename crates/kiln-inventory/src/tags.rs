//! Registry tags of the running server (`kiln_data::live`: the enabled data packs' tags, the vanilla ones before they load).

use std::collections::HashMap;

type Index = HashMap<(&'static str, &'static str), &'static [i32]>;

fn index() -> &'static Index {
    static INDEX: kiln_data::live::Cached<Index> = kiln_data::live::Cached::new();
    INDEX.get(|| {
        let mut m = HashMap::new();
        for (registry, tags) in kiln_data::live::tags() {
            for (tag, ids) in *tags {
                m.insert((*registry, *tag), *ids);
            }
        }
        m
    })
}

/// Entries of `tag` (such as `minecraft:planks`) in `registry`; `None` for an unknown tag.
pub fn entries(registry: &str, tag: &str) -> Option<&'static [i32]> {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    let full;
    let tag = if tag.contains(':') {
        tag
    } else {
        full = format!("minecraft:{tag}");
        &full
    };
    index().get(&(registry, tag)).copied()
}

pub fn contains(registry: &str, tag: &str, id: i32) -> bool {
    entries(registry, tag).is_some_and(|ids| ids.contains(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_tags_resolve() {
        let oak = kiln_item::registry::ITEM.id("oak_planks").unwrap();
        assert!(contains("minecraft:item", "minecraft:planks", oak));
        assert!(contains("minecraft:item", "#planks", oak));
        assert!(entries("minecraft:item", "minecraft:nope").is_none());
    }
}
