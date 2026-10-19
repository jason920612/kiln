//! Registry tags loaded from a datapack's `data/<namespace>/tags/<registry>/**.json`, resolved in
//! vanilla's order (`TagLoader.tryBuildTag`: entries in file order, nested tags expanded in place,
//! first occurrence kept), which loot depends on wherever it picks by index.
//!
//! A tag with a missing required entry or a reference cycle is not loaded (vanilla logs it and
//! goes on); the problems are listed in [`Tags::errors`]. The same loader feeds the tags the
//! game logic reads and sends (`kiln_data::live`).

use crate::json::Json;
use crate::parse::IdSet;
use kiln_item::Identifier;
use kiln_item::registry::Registry;
use std::collections::HashMap;
use std::path::Path;

/// Every tag of every registry found in the datapack, as ordered entry names.
#[derive(Debug, Default, Clone)]
pub struct Tags {
    /// (registry, tag) → entry names (`namespace:path`) in vanilla order.
    resolved: HashMap<String, HashMap<Identifier, Vec<Identifier>>>,
    /// The same tags as network ids, for registries kiln has id tables for.
    ids: HashMap<String, HashMap<Identifier, IdSet>>,
    /// Tags that did not load: `registry/tag: why`.
    pub errors: Vec<String>,
}

/// Where a pack's tags come from.
#[derive(Debug, Clone, Copy)]
pub enum TagSource<'a> {
    /// A pack directory holding `data/`.
    Dir(&'a Path),
    /// The vanilla pack without its files: the built-in copy (`kiln_data::registries::TAGS`).
    Builtin,
}

#[derive(Debug, Clone)]
struct RawEntry {
    id: String,
    required: bool,
}

type Raw = HashMap<(String, Identifier), Vec<RawEntry>>;

impl Tags {
    /// Loads every tag directory under `data/*/tags/`. `known` answers whether a registry
    /// entry exists (optional entries naming missing elements are skipped, as vanilla does).
    pub fn load(datapack: &Path, known: impl Fn(&str, &Identifier) -> bool) -> Result<Tags, String> {
        Tags::load_packs(&[datapack], known)
    }

    /// Tags of several packs in order (`TagLoader`): entries add up, `replace` drops the
    /// earlier packs' entries.
    pub fn load_packs(packs: &[&Path], known: impl Fn(&str, &Identifier) -> bool) -> Result<Tags, String> {
        let sources: Vec<TagSource> = packs.iter().map(|p| TagSource::Dir(p)).collect();
        Tags::load_sources(&sources, known)
    }

    /// [`load_packs`](Self::load_packs) with the built-in vanilla tags as a source too.
    pub fn load_sources(sources: &[TagSource], known: impl Fn(&str, &Identifier) -> bool) -> Result<Tags, String> {
        let mut raw: Raw = HashMap::new();
        for source in sources {
            match source {
                TagSource::Dir(datapack) => Tags::read_pack(datapack, &mut raw)?,
                TagSource::Builtin => Tags::read_builtin(&mut raw),
            }
        }
        Ok(Tags::resolve_all(raw, known))
    }

    /// The vanilla tags the build knows (already resolved, so every entry is plain).
    fn read_builtin(raw: &mut Raw) {
        for (registry, tags) in kiln_data::registries::TAGS {
            let names = kiln_data::registries::SYNCHRONIZED
                .iter()
                .chain(kiln_data::registries::BUILTIN)
                .find(|(r, _)| r == registry)
                .map_or(&[][..], |(_, n)| *n);
            for (tag, members) in *tags {
                let slot = raw.entry(((*registry).to_owned(), Identifier::new_unchecked((*tag).to_owned()))).or_default();
                slot.extend(members.iter().filter_map(|&i| names.get(i as usize)).map(|n| RawEntry { id: (*n).to_owned(), required: true }));
            }
        }
    }

    fn read_pack(datapack: &Path, raw: &mut Raw) -> Result<(), String> {
        let data = datapack.join("data");
        let Ok(namespaces) = std::fs::read_dir(&data) else { return Ok(()) };
        let mut ns_dirs: Vec<_> = namespaces.flatten().collect();
        ns_dirs.sort_by_key(|e| e.file_name());
        for ns in ns_dirs {
            let Some(namespace) = ns.file_name().to_str().map(str::to_owned) else { continue };
            let tags = ns.path().join("tags");
            if !tags.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            collect(&tags, "", &mut files).map_err(|e| format!("{}: {e}", tags.display()))?;
            for rel in files {
                // rel = "<registry path...>/<tag path>.json"; registries may be nested
                // (worldgen/biome), so split on the registry directories that hold files.
                let text = std::fs::read_to_string(tags.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
                let json = Json::parse(&text).map_err(|e| format!("tags/{rel}: {e}"))?;
                let (registry, tag) = split_registry(&rel);
                let tag_id = Identifier::new_unchecked(format!("{namespace}:{tag}"));
                let entries = parse_tag_file(&json).map_err(|e| format!("tags/{rel}: {e}"))?;
                let slot = raw.entry((format!("minecraft:{registry}"), tag_id)).or_default();
                if json.get("replace").and_then(Json::as_bool).unwrap_or(false) {
                    slot.clear();
                }
                slot.extend(entries);
            }
        }
        Ok(())
    }

    fn resolve_all(raw: Raw, known: impl Fn(&str, &Identifier) -> bool) -> Tags {
        let mut keys: Vec<(String, Identifier)> = raw.keys().cloned().collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_str().cmp(b.1.as_str())));
        let mut memo: HashMap<(String, Identifier), Option<Vec<Identifier>>> = HashMap::new();
        let mut errors = Vec::new();
        for key in &keys {
            resolve(key, &raw, &known, &mut memo, &mut Vec::new(), &mut errors);
        }
        let mut resolved: HashMap<String, HashMap<Identifier, Vec<Identifier>>> = HashMap::new();
        for ((registry, tag), names) in memo {
            if let Some(names) = names {
                resolved.entry(registry).or_default().insert(tag, names);
            }
        }
        let mut ids: HashMap<String, HashMap<Identifier, IdSet>> = HashMap::new();
        for (registry, tags) in &resolved {
            if !has_id_table(registry) {
                continue;
            }
            let reg = Registry(static_name(registry));
            let slot = ids.entry(registry.clone()).or_default();
            for (tag, names) in tags {
                let list: Vec<i32> = names.iter().filter_map(|n| reg.id(n.as_str())).collect();
                slot.insert(tag.clone(), IdSet::new(Some(tag.clone()), list));
            }
        }
        Tags { resolved, ids, errors }
    }

    /// The tag as network ids of `registry` (a registry kiln-data has an id table for).
    pub fn ids(&self, registry: Registry, tag: &Identifier) -> Option<&IdSet> {
        self.ids.get(registry.0).and_then(|m| m.get(tag))
    }

    /// Every tag of `registry` as network ids (a registry kiln-data has an id table for), in no particular order.
    pub fn all_ids(&self, registry: &str) -> impl Iterator<Item = (&Identifier, &IdSet)> {
        self.ids.get(registry).into_iter().flatten()
    }

    /// The registries that have tags.
    pub fn registries(&self) -> impl Iterator<Item = &str> {
        self.resolved.keys().map(String::as_str)
    }

    /// Entries of `tag` in `registry` (such as `minecraft:item`), in vanilla order.
    pub fn get(&self, registry: &str, tag: &Identifier) -> Option<&[Identifier]> {
        self.resolved.get(registry).and_then(|m| m.get(tag)).map(Vec::as_slice)
    }

    pub fn contains(&self, registry: &str, tag: &Identifier, entry: &Identifier) -> bool {
        self.get(registry, tag).is_some_and(|e| e.contains(entry))
    }

    pub fn len(&self) -> usize {
        self.resolved.values().map(HashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.resolved.values().all(HashMap::is_empty)
    }
}

/// Whether kiln-data has network ids for `registry`.
pub fn has_id_table(registry: &str) -> bool {
    kiln_data::builtin_entries(registry).is_some()
        || kiln_data::live::synced().iter().any(|(r, _)| *r == registry)
}

/// The `'static` registry name kiln-data uses (so a [`Registry`] can be built at runtime).
fn static_name(registry: &str) -> &'static str {
    kiln_data::registries::BUILTIN
        .iter()
        .chain(kiln_data::live::synced())
        .find(|(r, _)| *r == registry)
        .map(|(r, _)| *r)
        .expect("registry with an id table")
}

/// Tag registries whose directories are nested (`tags/worldgen/biome/...`).
const NESTED: &[&str] = &["worldgen"];

fn split_registry(rel: &str) -> (String, String) {
    let rel = rel.strip_suffix(".json").unwrap_or(rel);
    let mut parts = rel.splitn(3, '/');
    let first = parts.next().unwrap_or("");
    if NESTED.contains(&first) {
        let second = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        (format!("{first}/{second}"), rest.to_owned())
    } else {
        let rest = rel[first.len()..].trim_start_matches('/');
        (first.to_owned(), rest.to_owned())
    }
}

fn parse_tag_file(json: &Json) -> Result<Vec<RawEntry>, String> {
    let values = json.get("values").and_then(Json::as_array).ok_or("missing values")?;
    values
        .iter()
        .map(|v| match v {
            Json::Str(s) => Ok(RawEntry { id: s.clone(), required: true }),
            Json::Obj(_) => {
                let id = v.get("id").and_then(Json::as_str).ok_or("tag entry without id")?;
                let required = v.get("required").and_then(Json::as_bool).unwrap_or(true);
                Ok(RawEntry { id: id.to_owned(), required })
            }
            _ => Err("invalid tag entry".to_owned()),
        })
        .collect()
}

/// `TagLoader.tryBuildTag` for one tag: `None` (and an error noted) when a required entry or tag
/// is missing, or the tag is part of a cycle. Memoized, so a tag is built once.
fn resolve(
    key: &(String, Identifier),
    raw: &Raw,
    known: &impl Fn(&str, &Identifier) -> bool,
    memo: &mut HashMap<(String, Identifier), Option<Vec<Identifier>>>,
    stack: &mut Vec<Identifier>,
    errors: &mut Vec<String>,
) -> Option<Vec<Identifier>> {
    if let Some(done) = memo.get(key) {
        return done.clone();
    }
    if stack.contains(&key.1) {
        errors.push(format!("{}/{}: tag cycle", key.0, key.1));
        return None;
    }
    stack.push(key.1.clone());
    let mut out: Vec<Identifier> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for entry in raw.get(key).map(Vec::as_slice).unwrap_or(&[]) {
        if let Some(tag) = entry.id.strip_prefix('#') {
            let found = Identifier::parse(tag).and_then(|tag| {
                let nested = (key.0.clone(), tag);
                if raw.contains_key(&nested) { resolve(&nested, raw, known, memo, stack, errors) } else { None }
            });
            match found {
                Some(members) => {
                    for id in members {
                        if !out.contains(&id) {
                            out.push(id);
                        }
                    }
                }
                None if entry.required => missing.push(entry.id.clone()),
                None => {}
            }
        } else {
            match Identifier::parse(&entry.id).filter(|id| known(&key.0, id)) {
                Some(id) => {
                    if !out.contains(&id) {
                        out.push(id);
                    }
                }
                None if entry.required => missing.push(entry.id.clone()),
                None => {}
            }
        }
    }
    stack.pop();
    let result = if missing.is_empty() {
        Some(out)
    } else {
        errors.push(format!("{}/{}: missing references {}", key.0, key.1, missing.join(",")));
        None
    };
    memo.insert(key.clone(), result.clone());
    result
}

pub(crate) fn collect(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Some(name) = e.file_name().to_str().map(str::to_owned) else { continue };
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if e.path().is_dir() {
            collect(&e.path(), &rel, out)?;
        } else if name.ends_with(".json") {
            out.push(rel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_split() {
        assert_eq!(split_registry("item/planks.json"), ("item".into(), "planks".into()));
        assert_eq!(split_registry("worldgen/biome/is_ocean.json"), ("worldgen/biome".into(), "is_ocean".into()));
        assert_eq!(split_registry("block/mineable/axe.json"), ("block".into(), "mineable/axe".into()));
    }
}
