//! Decoding helpers shared by the loot types: field access with vanilla's strictness, errors
//! that carry the path to the offending value, holders and holder sets.

use crate::data::{Kind, Names};
use crate::json::Json;
use crate::tags::Tags;
use kiln_item::registry::Registry;
use kiln_item::{Identifier, Value};
use std::fmt;
use std::sync::Arc;

/// A decoding failure with the path of the value (`pools[0].entries[2].modifier`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub path: String,
    pub message: String,
}

impl ParseError {
    pub fn new(message: impl Into<String>) -> Self {
        ParseError { path: String::new(), message: message.into() }
    }

    /// Prefixes the path with a field name or index.
    pub fn at(mut self, segment: impl fmt::Display) -> Self {
        let seg = segment.to_string();
        self.path = if self.path.is_empty() {
            seg
        } else if self.path.starts_with('[') {
            format!("{seg}{}", self.path)
        } else {
            format!("{seg}.{}", self.path)
        };
        self
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() { f.write_str(&self.message) } else { write!(f, "{}: {}", self.path, self.message) }
    }
}

impl std::error::Error for ParseError {}

pub type PResult<T> = Result<T, ParseError>;

pub fn fail<T>(msg: impl Into<String>) -> PResult<T> {
    Err(ParseError::new(msg))
}

/// The fields of a JSON object.
pub fn obj(j: &Json) -> PResult<&[(String, Json)]> {
    j.as_object().ok_or_else(|| ParseError::new(format!("expected an object, found {}", kind(j))))
}

pub fn kind(j: &Json) -> &'static str {
    match j {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Num(_) => "a number",
        Json::Str(_) => "a string",
        Json::Arr(_) => "a list",
        Json::Obj(_) => "an object",
    }
}

/// A required field decoded with `f` (`fieldOf`).
pub fn req<'a, T>(j: &'a Json, key: &str, f: impl FnOnce(&'a Json) -> PResult<T>) -> PResult<T> {
    obj(j)?;
    match j.get(key) {
        Some(v) => f(v).map_err(|e| e.at(key)),
        None => Err(ParseError::new(format!("missing field {key}"))),
    }
}

/// An optional field (`optionalFieldOf`): absent is `None`, present but invalid is an error.
pub fn opt<'a, T>(j: &'a Json, key: &str, f: impl FnOnce(&'a Json) -> PResult<T>) -> PResult<Option<T>> {
    obj(j)?;
    match j.get(key) {
        Some(v) => f(v).map(Some).map_err(|e| e.at(key)),
        None => Ok(None),
    }
}

pub fn opt_or<'a, T>(j: &'a Json, key: &str, default: T, f: impl FnOnce(&'a Json) -> PResult<T>) -> PResult<T> {
    Ok(opt(j, key, f)?.unwrap_or(default))
}

/// A list decoded element by element, with the index in errors.
pub fn list<'a, T>(j: &'a Json, mut f: impl FnMut(&'a Json) -> PResult<T>) -> PResult<Vec<T>> {
    let items = j.as_array().ok_or_else(|| ParseError::new(format!("expected a list, found {}", kind(j))))?;
    items.iter().enumerate().map(|(i, v)| f(v).map_err(|e| e.at(format!("[{i}]")))).collect()
}

pub fn int(j: &Json) -> PResult<i32> {
    j.as_i32().ok_or_else(|| ParseError::new(format!("expected a number, found {}", kind(j))))
}

pub fn long(j: &Json) -> PResult<i64> {
    j.as_i64().ok_or_else(|| ParseError::new(format!("expected a number, found {}", kind(j))))
}

pub fn float(j: &Json) -> PResult<f32> {
    j.as_f32().ok_or_else(|| ParseError::new(format!("expected a number, found {}", kind(j))))
}

/// `Codec.BOOL` through `JsonOps`: a boolean, or a number whose byte value is non-zero.
pub fn boolean(j: &Json) -> PResult<bool> {
    match j {
        Json::Bool(b) => Ok(*b),
        Json::Num(_) => Ok(j.as_i32().unwrap_or(0) as i8 != 0),
        _ => fail(format!("expected a boolean, found {}", kind(j))),
    }
}

pub fn string(j: &Json) -> PResult<String> {
    j.as_str().map(str::to_owned).ok_or_else(|| ParseError::new(format!("expected a string, found {}", kind(j))))
}

pub fn ident(j: &Json) -> PResult<Identifier> {
    let s = j.as_str().ok_or_else(|| ParseError::new(format!("expected an identifier, found {}", kind(j))))?;
    Identifier::parse(s).ok_or_else(|| ParseError::new(format!("invalid identifier {s:?}")))
}

/// A value decoded by one of kiln-item's persistent codecs.
pub fn value<T>(j: &Json, f: impl FnOnce(&Value) -> Result<T, kiln_item::DataError>) -> PResult<T> {
    f(&j.to_value()).map_err(|e| ParseError::new(e.0))
}

/// A reference to a registry entry or an inline value (`RegistryFileCodec`): strings name an
/// entry of the loot data's own registries.
#[derive(Debug, Clone)]
pub enum Ref<T> {
    Direct(Arc<T>),
    /// Index into the registry of this kind in [`crate::LootData`].
    Named(usize),
}

impl<T> Ref<T> {
    pub fn direct(v: T) -> Self {
        Ref::Direct(Arc::new(v))
    }
}

/// An ordered set of registry entries by network id (`HolderSet`): a tag or explicit list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdSet {
    pub tag: Option<Identifier>,
    ids: Vec<i32>,
    sorted: Vec<i32>,
}

impl IdSet {
    pub fn new(tag: Option<Identifier>, ids: Vec<i32>) -> Self {
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        IdSet { tag, ids, sorted }
    }

    pub fn contains(&self, id: i32) -> bool {
        self.sorted.binary_search(&id).is_ok()
    }

    /// Entries in holder set order (tag order for tags).
    pub fn ids(&self) -> &[i32] {
        &self.ids
    }

    pub fn is_bound_tag(&self) -> bool {
        self.tag.is_some()
    }
}

/// An ordered set of entries of a registry kiln has no id table for (structures, biomes...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSet {
    pub tag: Option<Identifier>,
    pub names: Vec<Identifier>,
}

impl NameSet {
    pub fn contains(&self, name: &Identifier) -> bool {
        self.names.contains(name)
    }
}

/// State shared while decoding one datapack: the loot registries' names and the tags.
pub struct Parser<'a> {
    pub names: &'a Names,
    pub tags: &'a Tags,
}

impl Parser<'_> {
    /// `RegistryFileCodec`: a string references an entry of `kind`, anything else is inline.
    pub fn holder<T>(&self, j: &Json, kind: Kind, direct: impl FnOnce(&Self, &Json) -> PResult<T>) -> PResult<Ref<T>> {
        match j {
            Json::Str(s) => {
                let id = Identifier::parse(s).ok_or_else(|| ParseError::new(format!("invalid identifier {s:?}")))?;
                self.names
                    .index(kind, &id)
                    .map(Ref::Named)
                    .ok_or_else(|| ParseError::new(format!("unknown {} {id}", kind.dir())))
            }
            _ => direct(self, j).map(Ref::direct),
        }
    }

    /// `HolderSetCodec` over a loot registry: `"#tag"` is unsupported (no loot tags exist),
    /// a list holds references or inline values, a single holder is a one-element set.
    pub fn holder_list<T>(
        &self,
        j: &Json,
        kind: Kind,
        direct: impl Fn(&Self, &Json) -> PResult<T>,
    ) -> PResult<Vec<Ref<T>>> {
        match j {
            Json::Str(s) if s.starts_with('#') => fail(format!("unknown {} tag {s}", kind.dir())),
            Json::Arr(_) => list(j, |e| self.holder(e, kind, &direct)),
            _ => Ok(vec![self.holder(j, kind, &direct)?]),
        }
    }

    /// `RegistryCodecs.homogeneousList` / `HolderSetCodec` over a registry kiln has ids for.
    pub fn id_set(&self, j: &Json, reg: Registry) -> PResult<IdSet> {
        match j {
            Json::Str(s) if s.starts_with('#') => {
                let tag = Identifier::parse(&s[1..]).ok_or_else(|| ParseError::new(format!("invalid tag {s:?}")))?;
                let names = self.tags.get(reg.0, &tag).ok_or_else(|| ParseError::new(format!("missing tag #{tag}")))?;
                let ids = names.iter().map(|n| registry_id(reg, n)).collect::<PResult<Vec<_>>>()?;
                Ok(IdSet::new(Some(tag), ids))
            }
            Json::Arr(_) => Ok(IdSet::new(None, list(j, |e| self.id(e, reg))?)),
            _ => Ok(IdSet::new(None, vec![self.id(j, reg)?])),
        }
    }

    /// A registry entry by name (`Registry.byNameCodec` / reference holder).
    pub fn id(&self, j: &Json, reg: Registry) -> PResult<i32> {
        registry_id(reg, &ident(j)?)
    }

    /// A holder set over a registry kiln keeps no id table for.
    pub fn name_set(&self, j: &Json, registry: &str) -> PResult<NameSet> {
        match j {
            Json::Str(s) if s.starts_with('#') => {
                let tag = Identifier::parse(&s[1..]).ok_or_else(|| ParseError::new(format!("invalid tag {s:?}")))?;
                let names = self.tags.get(registry, &tag).ok_or_else(|| ParseError::new(format!("missing tag #{tag}")))?;
                Ok(NameSet { tag: Some(tag), names: names.to_vec() })
            }
            Json::Arr(_) => Ok(NameSet { tag: None, names: list(j, ident)? }),
            _ => Ok(NameSet { tag: None, names: vec![ident(j)?] }),
        }
    }
}

pub fn registry_id(reg: Registry, name: &Identifier) -> PResult<i32> {
    reg.id(name.as_str()).ok_or_else(|| ParseError::new(format!("unknown {} entry {name}", reg.0)))
}
