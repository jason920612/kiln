//! The levels (dimensions) a server runs and the dimension types of the loaded data packs.
//!
//! Vanilla has the overworld, the nether and the end; a data pack adds dimensions (`dimension/*.json`,
//! each with a dimension type) and dimension types (`dimension_type/*.json`) or overrides the vanilla
//! ones. Like the registries in [`crate::live`] these are process-wide tables that the simulation installs
//! when the world loads; without packs nothing is installed and they are the vanilla ones. The first
//! three levels are always the overworld, the nether and the end, in that order.

use crate::DimensionType;
use crate::registries;
use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

/// One level of the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    /// The level key (`minecraft:overworld`, `kiln:mine`), also the name of its save directory.
    pub key: &'static str,
    /// The name of the `minecraft:dimension_type` entry the level uses.
    pub dimension_type: &'static str,
    /// The biome chunks that nothing generates are filled with.
    pub biome: &'static str,
}

const VANILLA: &[Level] = &[
    Level { key: "minecraft:overworld", dimension_type: "minecraft:overworld", biome: "minecraft:plains" },
    Level { key: "minecraft:the_nether", dimension_type: "minecraft:the_nether", biome: "minecraft:nether_wastes" },
    Level { key: "minecraft:the_end", dimension_type: "minecraft:the_end", biome: "minecraft:the_end" },
];

static LEVELS: AtomicPtr<&'static [Level]> = AtomicPtr::new(std::ptr::null_mut());
static TYPES: AtomicPtr<&'static [DimensionType]> = AtomicPtr::new(std::ptr::null_mut());

fn load<T: 'static>(p: &AtomicPtr<&'static [T]>) -> Option<&'static [T]> {
    let p = p.load(Ordering::Acquire);
    // SAFETY: only ever set to a leaked value (`Box::leak`), never freed or mutated.
    (!p.is_null()).then(|| unsafe { *p })
}

/// The levels, in [`crate::levels::Level`] order: the vanilla three first.
pub fn levels() -> &'static [Level] {
    load(&LEVELS).unwrap_or(VANILLA)
}

/// A level's index in [`levels`].
pub fn level_id(key: &str) -> Option<usize> {
    levels().iter().position(|l| l.key == key)
}

/// The dimension type of the level with key `key`.
pub fn level_type(key: &str) -> Option<&'static DimensionType> {
    crate::dimension_type(levels().iter().find(|l| l.key == key)?.dimension_type)
}

fn interner() -> &'static Mutex<HashMap<String, &'static str>> {
    static I: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    I.get_or_init(Default::default)
}

fn leak_str(s: &str) -> &'static str {
    let mut map = interner().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(s) {
        return v;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    map.insert(s.to_owned(), leaked);
    leaked
}

/// Installs the levels (`(key, dimension type, default biome)`); the first three must be the vanilla
/// levels. Returns whether anything changed.
pub fn install_levels(levels: &[(String, String, String)]) -> bool {
    let new: Vec<Level> = levels
        .iter()
        .map(|(k, t, b)| Level { key: leak_str(k), dimension_type: leak_str(t), biome: leak_str(b) })
        .collect();
    if new.as_slice() == self::levels() {
        return false;
    }
    let list: &'static [Level] = Box::leak(new.into_boxed_slice());
    LEVELS.store(Box::leak(Box::new(list)), Ordering::Release);
    true
}

/// A dimension type, with the names a pack gave it leaked; see [`install_dimension_types`].
pub fn leak_name(name: &str) -> &'static str {
    leak_str(name)
}

/// Installs the dimension types the data packs add or override (by name); the others stay vanilla.
/// Returns whether anything changed.
pub fn install_dimension_types(types: Vec<DimensionType>) -> bool {
    if load(&TYPES).unwrap_or(&[]) == types.as_slice() {
        return false;
    }
    let list: &'static [DimensionType] = Box::leak(types.into_boxed_slice());
    TYPES.store(Box::leak(Box::new(list)), Ordering::Release);
    true
}

/// A dimension type of the loaded packs, else the vanilla one.
pub(crate) fn find_type(name: &str) -> Option<&'static DimensionType> {
    if let Some(list) = load(&TYPES)
        && let Some(t) = list.iter().find(|t| t.name == name)
    {
        return Some(t);
    }
    crate::dimension_types::DIMENSION_TYPES.iter().find(|d| d.name == name)
}

/// The names of the `minecraft:dimension_type` entries that are vanilla's, for tests.
pub fn vanilla_type_names() -> impl Iterator<Item = &'static str> {
    registries::SYNCHRONIZED
        .iter()
        .find(|(r, _)| *r == "minecraft:dimension_type")
        .map(|(_, e)| *e)
        .unwrap_or(&[])
        .iter()
        .copied()
}

/// Back to the vanilla levels and dimension types (tests).
pub fn reset() {
    LEVELS.store(std::ptr::null_mut(), Ordering::Release);
    TYPES.store(std::ptr::null_mut(), Ordering::Release);
}
