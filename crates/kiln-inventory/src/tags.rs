//! Registry tags from kiln-data's generated tables (the vanilla datapack's tags, resolved).

use std::collections::HashMap;
use std::sync::OnceLock;

type Index = HashMap<(&'static str, &'static str), &'static [i32]>;

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut m = HashMap::new();
        for (registry, tags) in kiln_data::registries::TAGS {
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
