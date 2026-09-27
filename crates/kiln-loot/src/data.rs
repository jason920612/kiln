//! Loading a datapack's loot registries (`loot_table/`, `predicate/`, `item_modifier/`,
//! `slot_source/`, `context_int_provider/`, `context_float_provider/`) plus the enchantment
//! definitions and tags they depend on.

use crate::condition::Condition;
use crate::enchant::Enchantment;
use crate::function::Function;
use crate::json::Json;
use crate::number::{FloatProvider, IntProvider};
use crate::parse::{IdSet, ParseError, Parser};
use crate::slot::SlotSource;
use crate::table::LootTable;
use crate::tags::{self, Tags};
use kiln_item::Identifier;
use kiln_item::registry;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

/// The loot registries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Table,
    Predicate,
    Modifier,
    SlotSource,
    IntProvider,
    FloatProvider,
}

impl Kind {
    pub const ALL: [Kind; 6] =
        [Kind::Table, Kind::Predicate, Kind::Modifier, Kind::SlotSource, Kind::IntProvider, Kind::FloatProvider];

    /// The datapack directory (and registry path) of the kind.
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Table => "loot_table",
            Kind::Predicate => "predicate",
            Kind::Modifier => "item_modifier",
            Kind::SlotSource => "slot_source",
            Kind::IntProvider => "context_int_provider",
            Kind::FloatProvider => "context_float_provider",
        }
    }
}

/// Entry names of every loot registry, for resolving references while decoding.
#[derive(Debug, Default, Clone)]
pub struct Names {
    index: HashMap<Kind, HashMap<Identifier, usize>>,
    names: HashMap<Kind, Vec<Identifier>>,
}

impl Names {
    pub fn index(&self, kind: Kind, id: &Identifier) -> Option<usize> {
        self.index.get(&kind).and_then(|m| m.get(id)).copied()
    }

    pub fn names(&self, kind: Kind) -> &[Identifier] {
        self.names.get(&kind).map_or(&[], Vec::as_slice)
    }

    fn add(&mut self, kind: Kind, id: Identifier) -> usize {
        let list = self.names.entry(kind).or_default();
        let i = list.len();
        list.push(id.clone());
        self.index.entry(kind).or_default().insert(id, i);
        i
    }
}

/// One registry's decoded values, by index (a value that failed to decode is `None`).
#[derive(Debug, Clone)]
pub struct Registry<T> {
    values: Vec<Option<T>>,
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Registry { values: Vec::new() }
    }
}

impl<T> Registry<T> {
    pub fn get(&self, i: usize) -> Option<&T> {
        self.values.get(i).and_then(Option::as_ref)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// A file that failed to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    /// `loot_table/minecraft:blocks/stone`-style name of the element.
    pub element: String,
    pub error: String,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.element, self.error)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("reading {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("tags: {0}")]
    Tags(String),
    #[error("{} files failed to load; first: {}", .0.len(), .0.first().map(ToString::to_string).unwrap_or_default())]
    Files(Vec<FileError>),
}

/// Every loot table, predicate, item modifier, slot source and number provider of a datapack,
/// decoded, with the enchantments and tags they use. Immutable and shareable across threads.
#[derive(Debug, Default)]
pub struct LootData {
    pub(crate) names: Names,
    pub(crate) tables: Registry<LootTable>,
    pub(crate) predicates: Registry<Condition>,
    pub(crate) modifiers: Registry<Function>,
    pub(crate) slot_sources: Registry<SlotSource>,
    pub(crate) int_providers: Registry<IntProvider>,
    pub(crate) float_providers: Registry<FloatProvider>,
    /// By `minecraft:enchantment` network id.
    pub(crate) enchantments: Vec<Option<Enchantment>>,
    pub tags: Tags,
    /// Files that failed to decode (see [`LootData::load_lenient`]).
    pub errors: Vec<FileError>,
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> LoadError + '_ {
    move |source| LoadError::Io { path: path.display().to_string(), source }
}

/// `(id, file)` of every JSON file under `data/<namespace>/<dir>/`.
/// Files of `dir` across packs: a later pack's file replaces an earlier one with the same id.
fn list_pack_files(packs: &[&Path], dir: &str) -> Result<Vec<(Identifier, std::path::PathBuf)>, LoadError> {
    let mut by_id: std::collections::HashMap<Identifier, std::path::PathBuf> = std::collections::HashMap::new();
    for pack in packs {
        for (id, path) in list_files(pack, dir)? {
            by_id.insert(id, path);
        }
    }
    let mut out: Vec<(Identifier, std::path::PathBuf)> = by_id.into_iter().collect();
    out.sort_by(|a, b| (a.0.path(), a.0.namespace()).cmp(&(b.0.path(), b.0.namespace())));
    Ok(out)
}

fn list_files(datapack: &Path, dir: &str) -> Result<Vec<(Identifier, std::path::PathBuf)>, LoadError> {
    let data = datapack.join("data");
    let mut out = Vec::new();
    let Ok(namespaces) = std::fs::read_dir(&data) else { return Ok(out) };
    for ns in namespaces.flatten() {
        let Some(namespace) = ns.file_name().to_str().map(str::to_owned) else { continue };
        let root = ns.path().join(dir);
        if !root.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        tags::collect(&root, "", &mut files).map_err(io(&root))?;
        for rel in files {
            let path = rel.strip_suffix(".json").unwrap_or(&rel).to_owned();
            if let Some(id) = Identifier::parse(&format!("{namespace}:{path}")) {
                out.push((id, root.join(&rel)));
            }
        }
    }
    // `Identifier.compareTo`: path, then namespace.
    out.sort_by(|a, b| (a.0.path(), a.0.namespace()).cmp(&(b.0.path(), b.0.namespace())));
    Ok(out)
}

impl LootData {
    /// Loads `datapack` (a directory containing `data/`); fails if any file does not decode.
    pub fn load(datapack: &Path) -> Result<LootData, LoadError> {
        let data = LootData::load_lenient(datapack)?;
        if data.errors.is_empty() { Ok(data) } else { Err(LoadError::Files(data.errors)) }
    }

    /// Loads `datapack`, keeping the files that decode and listing the others in `errors`
    /// (references to them resolve to nothing).
    pub fn load_lenient(datapack: &Path) -> Result<LootData, LoadError> {
        LootData::load_lenient_packs(&[datapack])
    }

    /// [`load_lenient`](Self::load_lenient) over several packs in order: later packs replace
    /// files with the same id and add to (or, with `replace`, replace) tags.
    pub fn load_lenient_packs(packs: &[&Path]) -> Result<LootData, LoadError> {
        let known = |registry: &str, id: &Identifier| {
            if tags::has_id_table(registry) {
                kiln_data::builtin_entries(registry)
                    .or_else(|| kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == registry).map(|(_, e)| *e))
                    .is_some_and(|entries| entries.contains(&id.as_str()))
            } else {
                true
            }
        };
        let tags = Tags::load_packs(packs, known).map_err(LoadError::Tags)?;
        let mut names = Names::default();
        let mut files: Vec<(Kind, Identifier, std::path::PathBuf)> = Vec::new();
        for kind in Kind::ALL {
            for (id, path) in list_pack_files(packs, kind.dir())? {
                names.add(kind, id.clone());
                files.push((kind, id, path));
            }
        }
        let mut data = LootData { names, tags, ..LootData::default() };
        for kind in Kind::ALL {
            let n = data.names.names(kind).len();
            match kind {
                Kind::Table => data.tables.values = (0..n).map(|_| None).collect(),
                Kind::Predicate => data.predicates.values = (0..n).map(|_| None).collect(),
                Kind::Modifier => data.modifiers.values = (0..n).map(|_| None).collect(),
                Kind::SlotSource => data.slot_sources.values = (0..n).map(|_| None).collect(),
                Kind::IntProvider => data.int_providers.values = (0..n).map(|_| None).collect(),
                Kind::FloatProvider => data.float_providers.values = (0..n).map(|_| None).collect(),
            }
        }

        // Enchantment definitions.
        let enchantment_files = list_pack_files(packs, "enchantment")?;
        let mut enchantments: Vec<Option<Enchantment>> = vec![None; registry::ENCHANTMENT.len()];
        let mut errors = Vec::new();
        {
            let parser = Parser { names: &data.names, tags: &data.tags };
            for (id, path) in enchantment_files {
                let text = std::fs::read_to_string(&path).map_err(io(&path))?;
                let result = Json::parse(&text).map_err(|e| ParseError::new(e.to_string())).and_then(|j| {
                    let net = crate::parse::registry_id(registry::ENCHANTMENT, &id)?;
                    Enchantment::parse(&parser, net, &j)
                });
                match result {
                    Ok(e) => {
                        let slot = e.id as usize;
                        enchantments[slot] = Some(e);
                    }
                    Err(e) => errors.push(FileError { element: format!("enchantment/{id}"), error: e.to_string() }),
                }
            }
        }
        data.enchantments = enchantments;

        // Loot registries.
        for (kind, id, path) in files {
            let text = std::fs::read_to_string(&path).map_err(io(&path))?;
            let json = match Json::parse(&text) {
                Ok(j) => j,
                Err(e) => {
                    errors.push(FileError { element: format!("{}/{id}", kind.dir()), error: e.to_string() });
                    continue;
                }
            };
            let index = data.names.index(kind, &id).expect("listed");
            let parser = Parser { names: &data.names, tags: &data.tags };
            let fail = |e: ParseError| FileError { element: format!("{}/{id}", kind.dir()), error: e.to_string() };
            match kind {
                Kind::Table => match LootTable::parse(&parser, &json) {
                    Ok(v) => data.tables.values[index] = Some(v),
                    Err(e) => errors.push(fail(e)),
                },
                Kind::Predicate => match Condition::parse(&parser, &json) {
                    Ok(v) => data.predicates.values[index] = Some(v),
                    Err(e) => errors.push(fail(e)),
                },
                Kind::Modifier => match Function::parse(&parser, &json) {
                    Ok(v) => data.modifiers.values[index] = Some(v),
                    Err(e) => errors.push(fail(e)),
                },
                Kind::SlotSource => match SlotSource::parse(&parser, &json) {
                    Ok(v) => data.slot_sources.values[index] = Some(v),
                    Err(e) => errors.push(fail(e)),
                },
                Kind::IntProvider => match IntProvider::parse(&parser, &json) {
                    Ok(v) => data.int_providers.values[index] = Some(v),
                    Err(e) => errors.push(fail(e)),
                },
                Kind::FloatProvider => match FloatProvider::parse(&parser, &json) {
                    Ok(v) => data.float_providers.values[index] = Some(v),
                    Err(e) => errors.push(fail(e)),
                },
            }
        }
        data.errors = errors;
        Ok(data)
    }

    /// Decodes one loot table from JSON against this data (for tables built at run time, such
    /// as `/loot` with an inline table).
    pub fn parse_table(&self, json: &str) -> Result<LootTable, ParseError> {
        let j = Json::parse(json).map_err(|e| ParseError::new(e.to_string()))?;
        LootTable::parse(&Parser { names: &self.names, tags: &self.tags }, &j)
    }

    /// Decodes an item modifier from JSON against this data.
    pub fn parse_modifier(&self, json: &str) -> Result<Function, ParseError> {
        let j = Json::parse(json).map_err(|e| ParseError::new(e.to_string()))?;
        Function::parse(&Parser { names: &self.names, tags: &self.tags }, &j)
    }

    /// Decodes a predicate (loot condition) from JSON against this data.
    pub fn parse_predicate(&self, json: &str) -> Result<Condition, ParseError> {
        let j = Json::parse(json).map_err(|e| ParseError::new(e.to_string()))?;
        Condition::parse(&Parser { names: &self.names, tags: &self.tags }, &j)
    }

    pub fn table_index(&self, id: &Identifier) -> Option<usize> {
        self.names.index(Kind::Table, id)
    }

    /// A loaded loot table.
    pub fn table(&self, id: &Identifier) -> Option<&LootTable> {
        self.table_index(id).and_then(|i| self.tables.get(i))
    }

    /// The loot table a block drops from (`BlockBehaviour.getLootTable`): `blocks/<name>`, or for
    /// wall-mounted variants (torches, signs, banners, heads, coral fans), the table of the
    /// standing block they drop like. `None` for blocks without one (air, fluids, portals...).
    pub fn block_table(&self, block: &str) -> Option<Identifier> {
        let (ns, path) = block.split_once(':').unwrap_or(("minecraft", block));
        let own = Identifier::new_unchecked(format!("{ns}:blocks/{path}"));
        if self.table_index(&own).is_some() {
            return Some(own);
        }
        let standing = Identifier::new_unchecked(format!("{ns}:blocks/{}", path.replacen("wall_", "", 1)));
        self.table_index(&standing).map(|_| standing)
    }

    /// Ids of every loot table, in registry order.
    pub fn table_ids(&self) -> &[Identifier] {
        self.names.names(Kind::Table)
    }

    /// Ids of the entries of a loot registry.
    pub fn ids(&self, kind: Kind) -> &[Identifier] {
        self.names.names(kind)
    }

    /// A loaded predicate (`predicate/`).
    pub fn predicate(&self, id: &Identifier) -> Option<&Condition> {
        self.names.index(Kind::Predicate, id).and_then(|i| self.predicates.get(i))
    }

    /// A loaded item modifier (`item_modifier/`).
    pub fn modifier(&self, id: &Identifier) -> Option<&Function> {
        self.names.index(Kind::Modifier, id).and_then(|i| self.modifiers.get(i))
    }

    /// The definition of a `minecraft:enchantment` id.
    pub fn enchantment(&self, id: i32) -> Option<&Enchantment> {
        usize::try_from(id).ok().and_then(|i| self.enchantments.get(i)).and_then(Option::as_ref)
    }

    /// Enchantments to choose from: `options` in holder set order, or the whole registry.
    pub fn enchantment_candidates(&self, options: Option<&IdSet>) -> Vec<&Enchantment> {
        match options {
            Some(set) => set.ids().iter().filter_map(|&id| self.enchantment(id)).collect(),
            None => self.enchantments.iter().flatten().collect(),
        }
    }
}
