//! The registries and tags the running server uses.
//!
//! Vanilla builds the data-driven registries once when the world loads (`WorldLoader` →
//! `RegistryDataLoader`) from the enabled data packs, and the tags of every registry with the
//! reloadable resources (`TagLoader`, again on `/reload`). Kiln keeps the vanilla tables of
//! [`crate::registries`] as the starting point and lets the simulation replace them with what
//! the packs define: [`install_synced`] when the world loads, [`install_tags`] whenever the tags
//! (re)load. The replaced tables are leaked (`&'static`), interned, so a reload that changes
//! nothing leaks nothing and every user keeps the plain slices the static tables had.
//!
//! Without data packs nothing is installed and the tables are the static ones.
//!
//! One server per process: the tables are process-wide. Tests that install packs belong in
//! their own test binary and must serialize on [`exclusive`].

use crate::registries;
use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// `[(registry, entries by network id)]`, the shape of [`registries::SYNCHRONIZED`].
pub type Synced = &'static [(&'static str, &'static [&'static str])];
/// `[(registry, [(tag, member ids)])]`, the shape of [`registries::TAGS`].
pub type Tags = &'static [(&'static str, &'static [(&'static str, &'static [i32])])];

struct Tables {
    synced: Synced,
    tags: Tags,
    synced_index: HashMap<&'static str, HashMap<&'static str, i32>>,
    tag_index: HashMap<(&'static str, &'static str), &'static [i32]>,
    /// The entries a data pack defines or overrides: `(registry, name)` → the entry's JSON.
    data: HashMap<(&'static str, &'static str), &'static str>,
    /// Per registry whose entries moved, vanilla id → id now (`None`: the same).
    remap: HashMap<&'static str, &'static [i32]>,
    /// Whether data packs changed the registries / the tags.
    synced_custom: bool,
    tags_custom: bool,
}

impl Tables {
    fn new(synced: Synced, tags: Tags, data: HashMap<(&'static str, &'static str), &'static str>, synced_custom: bool, tags_custom: bool) -> Tables {
        let synced_index = synced
            .iter()
            .map(|(r, names)| (*r, names.iter().enumerate().map(|(i, n)| (*n, i as i32)).collect::<HashMap<_, _>>()))
            .collect();
        let tag_index = tags.iter().flat_map(|(r, ts)| ts.iter().map(move |(t, ids)| ((*r, *t), *ids))).collect();
        let mut remap = HashMap::new();
        for (registry, names) in registries::SYNCHRONIZED {
            let Some((_, now)) = synced.iter().find(|(r, _)| r == registry) else { continue };
            if now.len() >= names.len() && now[..names.len()] == **names {
                continue;
            }
            let ids: Vec<i32> = names.iter().map(|n| now.iter().position(|m| m == n).map_or(-1, |i| i as i32)).collect();
            remap.insert(*registry, leak_ids(&ids));
        }
        Tables { synced, tags, synced_index, tag_index, data, remap, synced_custom, tags_custom }
    }
}

static TABLES: AtomicPtr<Tables> = AtomicPtr::new(std::ptr::null_mut());
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REGISTRY_GENERATION: AtomicU64 = AtomicU64::new(0);

fn tables() -> &'static Tables {
    let p = TABLES.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: tables are only ever leaked (`Box::leak`), never freed or mutated.
        return unsafe { &*p };
    }
    let fresh = Box::leak(Box::new(Tables::new(registries::SYNCHRONIZED, registries::TAGS, HashMap::new(), false, false)));
    match TABLES.compare_exchange(std::ptr::null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => fresh,
        // SAFETY: as above.
        Err(other) => unsafe { &*other },
    }
}

fn publish(t: Tables, registries_changed: bool) {
    TABLES.store(Box::leak(Box::new(t)), Ordering::Release);
    GENERATION.fetch_add(1, Ordering::AcqRel);
    if registries_changed {
        REGISTRY_GENERATION.fetch_add(1, Ordering::AcqRel);
    }
}

/// Counts every change of the tables; caches of derived data compare it (see [`Cached`]).
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

/// Counts the changes of the registries (not of the tags): `/reload` does not move it.
pub fn registry_generation() -> u64 {
    REGISTRY_GENERATION.load(Ordering::Acquire)
}

/// Serializes tests that install tables (the tables are process-wide).
pub fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn interner() -> &'static Mutex<HashMap<String, &'static str>> {
    static I: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// `s` as a `&'static str`, one copy per distinct string.
pub fn leak_str(s: &str) -> &'static str {
    // The static names are interned first so tables that did not change share their pointers.
    let mut map = interner().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(s) {
        return v;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    map.insert(s.to_owned(), leaked);
    leaked
}

fn leak_ids(ids: &[i32]) -> &'static [i32] {
    static I: OnceLock<Mutex<HashMap<Vec<i32>, &'static [i32]>>> = OnceLock::new();
    let mut map = I.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(ids) {
        return v;
    }
    let leaked: &'static [i32] = Box::leak(ids.to_vec().into_boxed_slice());
    map.insert(ids.to_vec(), leaked);
    leaked
}

fn leak_names(names: &[String]) -> &'static [&'static str] {
    static I: OnceLock<Mutex<HashMap<Vec<String>, &'static [&'static str]>>> = OnceLock::new();
    let mut map = I.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(names) {
        return v;
    }
    let leaked: &'static [&'static str] = Box::leak(names.iter().map(|n| leak_str(n)).collect::<Vec<_>>().into_boxed_slice());
    map.insert(names.to_vec(), leaked);
    leaked
}

/// The synchronized registries now: `[(registry, entries by network id)]`.
pub fn synced() -> Synced {
    tables().synced
}

/// The tags now: `[(registry, [(tag, member ids)])]`.
pub fn tags() -> Tags {
    tables().tags
}

/// Entries of a synchronized registry now, by network id.
pub fn synced_entries(registry: &str) -> Option<&'static [&'static str]> {
    tables().synced.iter().find(|(r, _)| *r == registry).map(|(_, e)| *e)
}

/// Entries of a synchronized or built-in registry, by network id.
pub fn entries(registry: &str) -> Option<&'static [&'static str]> {
    synced_entries(registry).or_else(|| crate::builtin_entries(registry))
}

/// Entry count of a synchronized registry now (0 when unknown).
pub fn synced_len(registry: &str) -> usize {
    synced_entries(registry).map_or(0, <[_]>::len)
}

/// Name of entry `id` of a synchronized registry now.
pub fn synced_name(registry: &str, id: i32) -> Option<&'static str> {
    synced_entries(registry)?.get(usize::try_from(id).ok()?).copied()
}

/// Network id of `entry` in a synchronized registry now.
pub fn synced_id(registry: &str, entry: &str) -> Option<i32> {
    tables().synced_index.get(registry)?.get(entry).copied()
}

/// The id vanilla's registry gives entry `vanilla_id` of `registry`, as it is numbered now (a pack's
/// entries can shift the ones sorted after them). `None` for an entry that is gone.
pub fn from_vanilla_id(registry: &str, vanilla_id: i32) -> Option<i32> {
    match tables().remap.get(registry) {
        None => Some(vanilla_id),
        Some(map) => map.get(usize::try_from(vanilla_id).ok()?).copied().filter(|&i| i >= 0),
    }
}

/// The ids of tag `tag` (with namespace, no `#`) in `registry`, in the tag's own order.
pub fn tag(registry: &str, tag: &str) -> Option<&'static [i32]> {
    tables().tag_index.get(&(registry, tag)).copied()
}

/// Whether entry `id` is in the tag.
pub fn tag_contains(registry: &str, tag_name: &str, id: i32) -> bool {
    tag(registry, tag_name).is_some_and(|ids| ids.contains(&id))
}

/// The tags of `registry`.
pub fn tags_of(registry: &str) -> &'static [(&'static str, &'static [i32])] {
    tables().tags.iter().find(|(r, _)| *r == registry).map_or(&[][..], |(_, t)| *t)
}

/// The entry data of an entry a data pack defines or overrides (`None`: it is vanilla's).
pub fn entry_data(registry: &str, name: &str) -> Option<&'static str> {
    tables().data.get(&(registry, name)).copied()
}

/// The names of the entries of `registry` that data packs define or override, in registry order.
pub fn custom_entries(registry: &str) -> Vec<&'static str> {
    let t = tables();
    let Some((_, names)) = t.synced.iter().find(|(r, _)| *r == registry) else { return Vec::new() };
    names.iter().copied().filter(|n| t.data.contains_key(&(registry, *n))).collect()
}

/// Whether data packs changed any registry or tag.
pub fn is_custom() -> bool {
    let t = tables();
    t.synced_custom || t.tags_custom
}

/// Installs the synchronized registries the world loaded (`names` per registry, in network id
/// order) and the entries packs define (`data`). Registries not listed keep their vanilla entries.
/// The tags are reset to the static ones; install them again after this. Returns whether anything changed.
pub fn install_synced(registries: Vec<(String, Vec<String>)>, data: Vec<((String, String), String)>) -> bool {
    let old = tables();
    let mut table: Vec<(&'static str, &'static [&'static str])> = Vec::new();
    for (registry, names) in registries::SYNCHRONIZED {
        match registries.iter().find(|(r, _)| r == registry) {
            Some((_, list)) => {
                let same = list.len() == names.len() && list.iter().zip(*names).all(|(a, b)| a == b);
                table.push((*registry, if same { *names } else { leak_names(list) }));
            }
            None => table.push((*registry, *names)),
        }
    }
    let data: HashMap<(&'static str, &'static str), &'static str> =
        data.into_iter().map(|((r, n), j)| ((leak_str(&r), leak_str(&n)), leak_str(&j))).collect();
    let custom = !data.is_empty() || table.iter().zip(registries::SYNCHRONIZED).any(|(a, b)| a.1 != b.1);
    let same_synced = old.synced.len() == table.len() && old.synced.iter().zip(&table).all(|(a, b)| a.0 == b.0 && a.1 == b.1);
    if same_synced && old.data == data && !old.tags_custom {
        return false;
    }
    let table: Synced = Box::leak(table.into_boxed_slice());
    publish(Tables::new(table, registries::TAGS, data, custom, false), true);
    true
}

/// Installs the tags (`TagLoader` + `TagNetworkSerialization`): per registry, the tags with their
/// member ids in the tag's own order. Returns whether anything changed.
pub fn install_tags(tags: Vec<(String, Vec<(String, Vec<i32>)>)>) -> bool {
    let old = tables();
    let mut table: Vec<(&'static str, &'static [(&'static str, &'static [i32])])> = Vec::new();
    for (registry, list) in &tags {
        let items: Vec<(&'static str, &'static [i32])> = list.iter().map(|(t, ids)| (leak_str(t), leak_ids(ids))).collect();
        table.push((leak_str(registry), Box::leak(items.into_boxed_slice())));
    }
    if same_tags(old.tags, &table) {
        return false;
    }
    let custom = !same_tags(registries::TAGS, &table);
    let tags: Tags = Box::leak(table.into_boxed_slice());
    publish(Tables::new(old.synced, tags, old.data.clone(), old.synced_custom, custom), false);
    true
}

fn same_tags(a: Tags, b: &[(&'static str, &'static [(&'static str, &'static [i32])])]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| x.0 == y.0 && x.1.len() == y.1.len() && x.1.iter().zip(y.1.iter()).all(|(p, q)| p.0 == q.0 && p.1 == q.1))
}

/// Back to the vanilla tables (tests).
pub fn reset() {
    publish(Tables::new(registries::SYNCHRONIZED, registries::TAGS, HashMap::new(), false, false), true);
}

/// A value derived from the tables, rebuilt when they change; reads are a load and a compare. With
/// `REGISTRIES` it follows only the registries ([`registry_generation`]): tables built from entry names and
/// ids, not from tags.
pub struct Cached<T: 'static, const REGISTRIES: bool = false> {
    generation: AtomicU64,
    value: AtomicPtr<T>,
    build: Mutex<()>,
}

impl<T: 'static, const REGISTRIES: bool> Cached<T, REGISTRIES> {
    pub const fn new() -> Self {
        Cached { generation: AtomicU64::new(u64::MAX), value: AtomicPtr::new(std::ptr::null_mut()), build: Mutex::new(()) }
    }

    pub fn get(&self, build: impl FnOnce() -> T) -> &'static T {
        let now = if REGISTRIES { registry_generation() } else { generation() };
        if self.generation.load(Ordering::Acquire) == now {
            let p = self.value.load(Ordering::Acquire);
            // SAFETY: leaked values are never freed or mutated; non-null once the generation matched.
            return unsafe { &*p };
        }
        let _guard = self.build.lock().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(Ordering::Acquire) == now {
            // SAFETY: as above.
            return unsafe { &*self.value.load(Ordering::Acquire) };
        }
        let fresh: &'static mut T = Box::leak(Box::new(build()));
        self.value.store(fresh, Ordering::Release);
        self.generation.store(now, Ordering::Release);
        fresh
    }
}

impl<T: 'static, const REGISTRIES: bool> Default for Cached<T, REGISTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_tables_by_default() {
        let _x = exclusive();
        reset();
        assert!(!is_custom());
        assert_eq!(synced().len(), registries::SYNCHRONIZED.len());
        let painting = synced_entries("minecraft:painting_variant").unwrap();
        assert_eq!(synced_id("minecraft:painting_variant", painting[3]), Some(3));
        assert_eq!(from_vanilla_id("minecraft:painting_variant", 5), Some(5));
        reset();
    }

    #[test]
    fn installed_entries_shift_the_ones_after_them() {
        let _x = exclusive();
        reset();
        let g = generation();
        let mut names: Vec<String> = synced_entries("minecraft:painting_variant").unwrap().iter().map(|s| (*s).to_owned()).collect();
        let first = names[0].clone();
        names.insert(0, "kiln:aaa".into());
        let data = vec![(("minecraft:painting_variant".to_owned(), "kiln:aaa".to_owned()), "{\"width\":1}".to_owned())];
        assert!(install_synced(vec![("minecraft:painting_variant".into(), names)], data.clone()));
        assert!(generation() > g);
        assert!(is_custom());
        assert_eq!(synced_id("minecraft:painting_variant", "kiln:aaa"), Some(0));
        assert_eq!(synced_id("minecraft:painting_variant", &first), Some(1));
        assert_eq!(from_vanilla_id("minecraft:painting_variant", 0), Some(1));
        assert_eq!(entry_data("minecraft:painting_variant", "kiln:aaa"), Some("{\"width\":1}"));
        assert_eq!(custom_entries("minecraft:painting_variant"), ["kiln:aaa"]);
        // Installing the same thing again changes nothing.
        let g = generation();
        let names: Vec<String> = synced_entries("minecraft:painting_variant").unwrap().iter().map(|s| (*s).to_owned()).collect();
        assert!(!install_synced(vec![("minecraft:painting_variant".into(), names)], data));
        assert_eq!(generation(), g);
        reset();
    }

    #[test]
    fn installed_tags_replace_the_static_ones() {
        let _x = exclusive();
        reset();
        assert!(install_tags(vec![("minecraft:block".into(), vec![("minecraft:kiln".into(), vec![1, 2, 3])])]));
        assert_eq!(tag("minecraft:block", "minecraft:kiln"), Some(&[1, 2, 3][..]));
        assert_eq!(tag("minecraft:block", "minecraft:logs"), None);
        assert!(tag_contains("minecraft:block", "minecraft:kiln", 2));
        reset();
        assert!(tag("minecraft:block", "minecraft:logs").is_some());
    }

    #[test]
    fn cached_values_follow_the_generation() {
        let _x = exclusive();
        reset();
        static C: Cached<usize> = Cached::new();
        let a = *C.get(|| tags_of("minecraft:block").len());
        assert!(a > 100);
        assert_eq!(*C.get(|| unreachable!()), a);
        install_tags(vec![("minecraft:block".into(), vec![("minecraft:kiln".into(), vec![1])])]);
        assert_eq!(*C.get(|| tags_of("minecraft:block").len()), 1);
        reset();
    }
}
