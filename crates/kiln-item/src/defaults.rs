//! Default (prototype) components of every item, decoded on first use from the generated
//! table (network-encoded values extracted from the vanilla jar).

use crate::component::{Component, ComponentId};
use kiln_proto::Reader;
use std::sync::OnceLock;

static TABLE: &[u8] = include_bytes!("gen/item_defaults.bin");

/// An item's default components, sorted by type.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComponentMap(Vec<Component>);

impl ComponentMap {
    pub fn get(&self, id: ComponentId) -> Option<&Component> {
        self.0.binary_search_by_key(&id, Component::id).ok().map(|i| &self.0[i])
    }

    pub fn contains(&self, id: ComponentId) -> bool {
        self.get(id).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Component> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

struct Defaults {
    maps: Vec<ComponentMap>,
    /// (item id, component type) of values that failed to decode.
    failures: Vec<(i32, ComponentId)>,
}

fn load() -> &'static Defaults {
    static DEFAULTS: OnceLock<Defaults> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let u16_at = |p: usize| u16::from_le_bytes([TABLE[p], TABLE[p + 1]]);
        let u32_at = |p: usize| u32::from_le_bytes(TABLE[p..p + 4].try_into().unwrap()) as usize;
        assert_eq!(&TABLE[..4], b"KID1", "item_defaults.bin: bad magic");
        let items = u32_at(4);
        let count = u32_at(8);
        let mut p = 12;
        let mut values: Vec<(ComponentId, Option<Component>)> = Vec::with_capacity(count);
        for _ in 0..count {
            let ty = u16_at(p);
            let len = u32_at(p + 2);
            let bytes = &TABLE[p + 6..p + 6 + len];
            p += 6 + len;
            let mut r = Reader::new(bytes);
            let value = Component::read(ty, &mut r).ok().filter(|_| r.remaining() == 0);
            values.push((ty, value));
        }
        let mut maps = Vec::with_capacity(items);
        let mut failures = Vec::new();
        for item in 0..items {
            let n = u16_at(p) as usize;
            p += 2;
            let mut comps = Vec::with_capacity(n);
            for _ in 0..n {
                let (ty, value) = &values[u16_at(p) as usize];
                p += 2;
                match value {
                    Some(c) => comps.push(c.clone()),
                    None => failures.push((item as i32, *ty)),
                }
            }
            comps.sort_by_key(Component::id);
            maps.push(ComponentMap(comps));
        }
        Defaults { maps, failures }
    })
}

/// The default components of `item` (a `minecraft:item` network id); empty for unknown ids.
pub fn default_components(item: i32) -> &'static ComponentMap {
    static EMPTY: ComponentMap = ComponentMap(Vec::new());
    usize::try_from(item).ok().and_then(|i| load().maps.get(i)).unwrap_or(&EMPTY)
}

/// Default values the typed codecs could not decode, as (item id, component type).
/// Empty once every component type has a model.
pub fn decode_failures() -> &'static [(i32, ComponentId)] {
    &load().failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{ids, keys};
    use crate::registry::ITEM;

    #[test]
    fn every_default_value_decodes() {
        let failures: Vec<String> = decode_failures()
            .iter()
            .map(|&(item, ty)| format!("{} {}", ITEM.name(item).unwrap_or("?"), crate::component::name(ty)))
            .collect();
        assert!(failures.is_empty(), "default components that failed to decode: {failures:?}");
        let sword = default_components(ITEM.id("diamond_sword").unwrap());
        assert_eq!(sword.len(), 19);
        assert_eq!(sword.get(ids::MAX_DAMAGE).and_then(|c| keys::MAX_DAMAGE.get(c)), Some(&1561));
        assert!(default_components(-1).is_empty());
    }
}
