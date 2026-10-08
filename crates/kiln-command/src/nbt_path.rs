//! NBT paths (`NbtPathArgument.NbtPath`): `a.b[0]."c d"{x:1}[]` and friends, as parsed by
//! the `nbt_path` argument, with the queries `execute if data` needs.

use crate::blocks::compare_nbt;
use crate::error::CommandError;
use crate::reader::StringReader;
use crate::snbt;
use crate::tr;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `{...}` at the start: the root matches the compound.
    MatchRoot(Tag),
    /// `name`.
    Child(String),
    /// `name{...}`: a child compound matching the pattern.
    MatchObject(String, Tag),
    /// `[]`.
    AllElements,
    /// `[i]` (negative counts from the end).
    Index(i32),
    /// `[{...}]`: list elements matching the pattern.
    MatchElement(Tag),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NbtPath {
    pub text: String,
    pub nodes: Vec<Node>,
    /// Where each node ends in `text` (`nodeToOriginalPosition`; `[]` nodes share one entry
    /// in vanilla, the last one's).
    pub ends: Vec<usize>,
}

fn invalid_node() -> CommandError {
    CommandError::new(tr!("arguments.nbtpath.node.invalid"))
}

/// `NbtPathArgument.isAllowedInUnquotedName`.
fn allowed_in_name(c: char) -> bool {
    !matches!(c, ' ' | '"' | '\'' | '[' | ']' | '.' | '{' | '}')
}

impl NbtPath {
    /// `NbtPathArgument.parse`.
    pub fn parse(reader: &mut StringReader) -> Result<Self> {
        let start = reader.cursor();
        let mut nodes = Vec::new();
        let mut ends = Vec::new();
        let mut first = true;
        while reader.can_read() && reader.peek() != ' ' {
            nodes.push(Self::parse_node(reader, first)?);
            ends.push(reader.cursor() - start);
            first = false;
            if reader.can_read() {
                let c = reader.peek();
                if c != ' ' && c != '[' && c != '{' {
                    reader.expect('.')?;
                }
            }
        }
        // `[]` is one shared node object in vanilla: every one reports the last position.
        if let Some(last) = nodes.iter().zip(&ends).filter(|(n, _)| **n == Node::AllElements).map(|(_, e)| *e).last() {
            for (n, e) in nodes.iter().zip(ends.iter_mut()) {
                if *n == Node::AllElements {
                    *e = last;
                }
            }
        }
        Ok(NbtPath { text: reader.string()[start..reader.cursor()].to_owned(), nodes, ends })
    }

    /// `createNotFoundException`: the path up to the node that found nothing.
    fn not_found(&self, node: usize) -> CommandError {
        let end = self.ends.get(node).copied().unwrap_or(self.text.len()).min(self.text.len());
        CommandError::new(tr!("arguments.nbtpath.nothing_found", &self.text[..end]))
    }

    /// `NbtPath.get`: like [`get`](Self::get), but finding nothing is an error.
    pub fn get_checked<'t>(&self, root: &'t Tag) -> Result<Vec<&'t Tag>> {
        let mut current = vec![root];
        for (i, node) in self.nodes.iter().enumerate() {
            current = current.into_iter().flat_map(|t| node.get_one(t)).collect();
            if current.is_empty() {
                return Err(self.not_found(i));
            }
        }
        Ok(current)
    }

    /// `getOrCreateParents`: the tags the last node applies to, missing parents created.
    fn get_or_create_parents<'t>(&self, root: &'t mut Tag) -> Result<Vec<&'t mut Tag>> {
        let mut current = vec![root];
        for i in 0..self.nodes.len().saturating_sub(1) {
            let (node, next_node) = (&self.nodes[i], &self.nodes[i + 1]);
            let mut next = Vec::new();
            for t in current {
                next.extend(node.get_or_create(t, || next_node.preferred_parent()));
            }
            if next.is_empty() {
                return Err(self.not_found(i));
            }
            current = next;
        }
        Ok(current)
    }

    /// `NbtPath.getOrCreate`: the selected tags, missing ones created from `make`.
    pub fn get_or_create<'t>(&self, root: &'t mut Tag, make: impl Fn() -> Tag) -> Result<Vec<&'t mut Tag>> {
        let parents = self.get_or_create_parents(root)?;
        let last = self.nodes.last().expect("paths have a node");
        Ok(parents.into_iter().flat_map(|p| last.get_or_create(p, &make)).collect())
    }

    /// `NbtPath.remove`: removes every tag the path selects; how many went.
    pub fn remove(&self, root: &mut Tag) -> i32 {
        let Some((last, parents)) = self.nodes.split_last() else { return 0 };
        fn walk(nodes: &[Node], tag: &mut Tag, last: &Node) -> i32 {
            let Some((node, rest)) = nodes.split_first() else { return last.remove_tag(tag) };
            node.get_mut(tag).into_iter().map(|child| walk(rest, child, last)).sum()
        }
        walk(parents, root, last)
    }

    /// `NbtPath.insert`: inserts copies of `values` at `index` (negative from the end, -1
    /// appends) into every list the path selects (created if missing); how many lists changed.
    pub fn insert(&self, index: i32, root: &mut Tag, values: &[Tag]) -> Result<i32> {
        for v in values {
            if is_too_deep(v, self.nodes.len()) {
                return Err(CommandError::new(tr!("arguments.nbtpath.too_deep")));
            }
        }
        let targets = self.get_or_create(root, || Tag::List(Vec::new()))?;
        let mut changed = 0;
        for target in targets {
            let size = match &*target {
                Tag::List(l) => l.len(),
                Tag::ByteArray(a) => a.len(),
                Tag::IntArray(a) => a.len(),
                Tag::LongArray(a) => a.len(),
                other => {
                    return Err(CommandError::new(tr!("commands.data.modify.expected_list", crate::nbt_text::snbt(other))));
                }
            } as i64;
            let mut at = if index < 0 { size + index as i64 + 1 } else { index as i64 };
            let mut modified = false;
            for v in values {
                let len = match &*target {
                    Tag::List(l) => l.len(),
                    Tag::ByteArray(a) => a.len(),
                    Tag::IntArray(a) => a.len(),
                    Tag::LongArray(a) => a.len(),
                    _ => 0,
                } as i64;
                if at < 0 || at > len {
                    return Err(CommandError::new(tr!("commands.data.modify.invalid_index", at as i32)));
                }
                if add_tag(target, at as usize, v) {
                    at += 1;
                    modified = true;
                }
            }
            changed += modified as i32;
        }
        Ok(changed)
    }

    fn parse_node(reader: &mut StringReader, first: bool) -> Result<Node> {
        match reader.peek() {
            '{' => {
                if !first {
                    return Err(invalid_node().at(reader));
                }
                Ok(Node::MatchRoot(snbt::parse_compound(reader)?))
            }
            '[' => {
                reader.skip();
                match reader.peek() {
                    '{' => {
                        let pattern = snbt::parse_compound(reader)?;
                        reader.expect(']')?;
                        Ok(Node::MatchElement(pattern))
                    }
                    ']' => {
                        reader.skip();
                        Ok(Node::AllElements)
                    }
                    _ => {
                        let i = reader.read_int()?;
                        reader.expect(']')?;
                        Ok(Node::Index(i))
                    }
                }
            }
            '"' | '\'' => {
                let name = reader.read_string()?;
                Self::object_node(reader, name)
            }
            _ => {
                let start = reader.cursor();
                while reader.can_read() && allowed_in_name(reader.peek()) {
                    reader.skip();
                }
                if reader.cursor() == start {
                    return Err(invalid_node().at(reader));
                }
                let name = reader.string()[start..reader.cursor()].to_owned();
                Self::object_node(reader, name)
            }
        }
    }

    fn object_node(reader: &mut StringReader, name: String) -> Result<Node> {
        if name.is_empty() {
            return Err(invalid_node().at(reader));
        }
        if reader.can_read() && reader.peek() == '{' {
            let pattern = snbt::parse_compound(reader)?;
            return Ok(Node::MatchObject(name, pattern));
        }
        Ok(Node::Child(name))
    }

    /// `NbtPath.get`: every tag the path selects in `root`.
    pub fn get<'t>(&self, root: &'t Tag) -> Vec<&'t Tag> {
        let mut current = vec![root];
        for node in &self.nodes {
            current = current.into_iter().flat_map(|t| node.get_one(t)).collect();
        }
        current
    }

    /// `NbtPath.countMatching`.
    pub fn count_matching(&self, root: &Tag) -> usize {
        self.get(root).len()
    }

    /// `NbtPath.set`: sets every tag the path selects to `value`, first creating missing
    /// parents like `getOrCreateParents`; returns how many tags changed. List elements are
    /// stored as given (no element type checks).
    pub fn set(&self, root: &mut Tag, value: &Tag) -> Result<i32> {
        if is_too_deep(value, self.nodes.len()) {
            return Err(CommandError::new(tr!("arguments.nbtpath.too_deep")));
        }
        let parents = self.get_or_create_parents(root)?;
        let last = self.nodes.last().expect("paths have a node");
        Ok(parents.into_iter().map(|p| last.set_tag(p, value)).sum())
    }
}

/// `NbtPath.isTooDeep`: nesting of 512 levels (counting `depth` already used).
pub fn is_too_deep(tag: &Tag, depth: usize) -> bool {
    if depth >= 512 {
        return true;
    }
    match tag {
        Tag::Compound(f) => f.iter().any(|(_, v)| is_too_deep(v, depth + 1)),
        Tag::List(l) => l.iter().any(|v| is_too_deep(v, depth + 1)),
        _ => false,
    }
}

/// `Tag.equals`: compounds compare as maps (key order does not matter).
pub fn nbt_eq(a: &Tag, b: &Tag) -> bool {
    match (a, b) {
        (Tag::Compound(x), Tag::Compound(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.iter().find(|(yk, _)| yk == k).is_some_and(|(_, yv)| nbt_eq(v, yv)))
        }
        (Tag::List(x), Tag::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| nbt_eq(a, b)),
        _ => a == b,
    }
}

/// `CompoundTag.merge`: `other`'s fields into `into`, compounds merged recursively.
pub fn merge_compound(into: &mut Vec<(String, Tag)>, other: &[(String, Tag)]) {
    for (k, v) in other {
        match (into.iter_mut().find(|(ik, _)| ik == k), v) {
            (Some((_, Tag::Compound(existing))), Tag::Compound(sub)) => merge_compound(existing, sub),
            (Some((_, slot)), _) => *slot = v.clone(),
            (None, _) => into.push((k.clone(), v.clone())),
        }
    }
}

/// `CollectionTag.addTag`: lists take anything; arrays take numbers of any kind (cast).
fn add_tag(target: &mut Tag, at: usize, v: &Tag) -> bool {
    let num = |t: &Tag| -> Option<i64> {
        Some(match t {
            Tag::Byte(b) => *b as i64,
            Tag::Short(s) => *s as i64,
            Tag::Int(i) => *i as i64,
            Tag::Long(l) => *l,
            Tag::Float(f) => *f as i64,
            Tag::Double(d) => *d as i64,
            _ => return None,
        })
    };
    match target {
        Tag::List(l) => {
            l.insert(at, v.clone());
            true
        }
        Tag::ByteArray(a) => num(v).map(|n| a.insert(at, n as i8)).is_some(),
        Tag::IntArray(a) => num(v).map(|n| a.insert(at, n as i32)).is_some(),
        Tag::LongArray(a) => num(v).map(|n| a.insert(at, n)).is_some(),
        _ => false,
    }
}

impl Node {
    /// `Node.getTag` on one tag.
    fn get_one<'t>(&self, t: &'t Tag) -> Vec<&'t Tag> {
        match (self, t) {
            (Node::MatchRoot(p), _) => {
                if compare_nbt(p, t, true) {
                    vec![t]
                } else {
                    Vec::new()
                }
            }
            (Node::Child(name), Tag::Compound(f)) => f.iter().filter(|(k, _)| k == name).map(|(_, v)| v).collect(),
            (Node::MatchObject(name, p), Tag::Compound(f)) => {
                f.iter().filter(|(k, v)| k == name && compare_nbt(p, v, true)).map(|(_, v)| v).collect()
            }
            (Node::AllElements, Tag::List(items)) => items.iter().collect(),
            (Node::Index(i), Tag::List(items)) => {
                let idx = if *i < 0 { items.len() as i64 + *i as i64 } else { *i as i64 };
                if (0..items.len() as i64).contains(&idx) { vec![&items[idx as usize]] } else { Vec::new() }
            }
            (Node::MatchElement(p), Tag::List(items)) => items.iter().filter(|e| compare_nbt(p, e, true)).collect(),
            _ => Vec::new(),
        }
    }

    /// [`get_one`](Self::get_one), mutably.
    fn get_mut<'t>(&self, t: &'t mut Tag) -> Vec<&'t mut Tag> {
        match (self, t) {
            (Node::MatchRoot(p), t) => {
                if compare_nbt(p, t, true) {
                    vec![t]
                } else {
                    Vec::new()
                }
            }
            (Node::Child(name), Tag::Compound(f)) => f.iter_mut().filter(|(k, _)| k == name).map(|(_, v)| v).collect(),
            (Node::MatchObject(name, p), Tag::Compound(f)) => {
                f.iter_mut().filter(|(k, v)| k == name && compare_nbt(p, v, true)).map(|(_, v)| v).collect()
            }
            (Node::AllElements, Tag::List(items)) => items.iter_mut().collect(),
            (Node::Index(i), Tag::List(items)) => element(items, *i).into_iter().collect(),
            (Node::MatchElement(p), Tag::List(items)) => items.iter_mut().filter(|e| compare_nbt(p, e, true)).collect(),
            _ => Vec::new(),
        }
    }

    /// `Node.removeTag`: how many tags went.
    fn remove_tag(&self, t: &mut Tag) -> i32 {
        match (self, t) {
            (Node::Child(name), Tag::Compound(f)) => {
                let before = f.len();
                f.retain(|(k, _)| k != name);
                (f.len() != before) as i32
            }
            (Node::MatchObject(name, p), Tag::Compound(f)) => {
                match f.iter().position(|(k, v)| k == name && compare_nbt(p, v, true)) {
                    Some(i) => {
                        f.remove(i);
                        1
                    }
                    None => 0,
                }
            }
            (Node::AllElements, t) => {
                let n = match t {
                    Tag::List(l) => std::mem::take(l).len(),
                    Tag::ByteArray(a) => std::mem::take(a).len(),
                    Tag::IntArray(a) => std::mem::take(a).len(),
                    Tag::LongArray(a) => std::mem::take(a).len(),
                    _ => 0,
                };
                n as i32
            }
            (Node::Index(i), t) => {
                let len = match t {
                    Tag::List(l) => l.len(),
                    Tag::ByteArray(a) => a.len(),
                    Tag::IntArray(a) => a.len(),
                    Tag::LongArray(a) => a.len(),
                    _ => return 0,
                } as i64;
                let idx = if *i < 0 { len + *i as i64 } else { *i as i64 };
                if !(0..len).contains(&idx) {
                    return 0;
                }
                let idx = idx as usize;
                match t {
                    Tag::List(l) => drop(l.remove(idx)),
                    Tag::ByteArray(a) => drop(a.remove(idx)),
                    Tag::IntArray(a) => drop(a.remove(idx)),
                    Tag::LongArray(a) => drop(a.remove(idx)),
                    _ => {}
                }
                1
            }
            (Node::MatchElement(p), Tag::List(items)) => {
                let before = items.len();
                items.retain(|e| !compare_nbt(p, e, true));
                (before - items.len()) as i32
            }
            _ => 0,
        }
    }

    /// `createPreferredParentTag`: what a missing parent of this node is created as.
    fn preferred_parent(&self) -> Tag {
        match self {
            Node::MatchRoot(_) | Node::Child(_) | Node::MatchObject(..) => Tag::Compound(Vec::new()),
            Node::AllElements | Node::Index(_) | Node::MatchElement(_) => Tag::List(Vec::new()),
        }
    }

    /// `getOrCreate` on one tag: the selected children, missing ones created from `make`.
    fn get_or_create<'t>(&self, tag: &'t mut Tag, make: impl Fn() -> Tag) -> Vec<&'t mut Tag> {
        match (self, tag) {
            (Node::MatchRoot(p), t) => {
                if compare_nbt(p, t, true) {
                    vec![t]
                } else {
                    Vec::new()
                }
            }
            (Node::Child(name), Tag::Compound(f)) => vec![child_or_insert(f, name, make)],
            (Node::MatchObject(name, p), Tag::Compound(f)) => {
                let exists = f.iter().any(|(k, _)| k == name);
                let child = child_or_insert(f, name, || p.clone());
                if !exists || compare_nbt(p, child, true) { vec![child] } else { Vec::new() }
            }
            (Node::AllElements, Tag::List(items)) => {
                if items.is_empty() {
                    items.push(make());
                }
                items.iter_mut().collect()
            }
            (Node::Index(i), Tag::List(items)) => element(items, *i).into_iter().collect(),
            (Node::MatchElement(p), Tag::List(items)) => {
                if !items.iter().any(|e| compare_nbt(p, e, true)) {
                    items.push(p.clone());
                }
                items.iter_mut().filter(|e| compare_nbt(p, e, true)).collect()
            }
            _ => Vec::new(),
        }
    }

    /// `setTag` on one parent: the number of tags changed.
    fn set_tag(&self, tag: &mut Tag, value: &Tag) -> i32 {
        let assign = |slot: &mut Tag| {
            let changed = slot != value;
            *slot = value.clone();
            changed as i32
        };
        match (self, tag) {
            (Node::Child(name), Tag::Compound(f)) => match f.iter_mut().find(|(k, _)| k == name) {
                Some((_, slot)) => assign(slot),
                None => {
                    f.push((name.clone(), value.clone()));
                    1
                }
            },
            (Node::MatchObject(name, p), Tag::Compound(f)) => match f.iter_mut().find(|(k, _)| k == name) {
                Some((_, slot)) if compare_nbt(p, slot, true) => assign(slot),
                _ => 0,
            },
            (Node::AllElements, Tag::List(items)) => {
                if items.is_empty() {
                    items.push(value.clone());
                    return 1;
                }
                items.iter_mut().map(assign).sum()
            }
            (Node::Index(i), Tag::List(items)) => element(items, *i).map_or(0, assign),
            (Node::MatchElement(p), Tag::List(items)) => {
                items.iter_mut().filter(|e| compare_nbt(p, e, true)).map(assign).sum()
            }
            _ => 0,
        }
    }
}

fn child_or_insert<'t>(fields: &'t mut Vec<(String, Tag)>, name: &str, make: impl FnOnce() -> Tag) -> &'t mut Tag {
    let i = match fields.iter().position(|(k, _)| k == name) {
        Some(i) => i,
        None => {
            fields.push((name.to_owned(), make()));
            fields.len() - 1
        }
    };
    &mut fields[i].1
}

/// A list element by index, negative from the end.
fn element(items: &mut [Tag], i: i32) -> Option<&mut Tag> {
    let idx = if i < 0 { items.len() as i64 + i as i64 } else { i as i64 };
    usize::try_from(idx).ok().and_then(|idx| items.get_mut(idx))
}

/// `CommandStorage`: compound tags by id for `execute store ... storage` and
/// `execute if data storage`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandStorage {
    tags: std::collections::BTreeMap<String, Tag>,
    /// Changed since [`CommandStorage::take_dirty`] (`SavedData.setDirty`).
    dirty: bool,
}

impl CommandStorage {
    /// `get`: an empty compound for ids never written.
    pub fn get(&self, id: &str) -> Tag {
        self.tags.get(id).cloned().unwrap_or(Tag::Compound(Vec::new()))
    }

    /// `CommandStorage.set` (`/data` on storage): an empty compound removes the id
    /// (`CommandStorage.Container.put`).
    pub fn set(&mut self, id: &str, data: Tag) {
        if matches!(&data, Tag::Compound(f) if f.is_empty()) {
            self.tags.remove(id);
        } else {
            self.tags.insert(id.to_owned(), data);
        }
        self.dirty = true;
    }

    /// Ids with stored data, sorted (suggestions).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.tags.keys().map(String::as_str)
    }

    /// The stored tags with their ids, sorted by id.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Tag)> {
        self.tags.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Whether anything changed since the last call (what needs saving).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Loading a save: sets `id` without marking the storage changed.
    pub fn load(&mut self, id: &str, data: Tag) {
        let dirty = self.dirty;
        self.set(id, data);
        self.dirty = dirty;
    }

    /// `ExecuteCommand.storeData` through `StorageDataAccessor`: `get` hands out the stored
    /// compound itself (so parents created by a failed set stay), or a fresh one that is
    /// only kept when the set succeeds.
    pub fn store(&mut self, id: &str, path: &NbtPath, value: &Tag) -> Result<i32> {
        self.dirty = true;
        match self.tags.get_mut(id) {
            Some(tag) => path.set(tag, value),
            None => {
                let mut tag = Tag::Compound(Vec::new());
                let changed = path.set(&mut tag, value)?;
                self.tags.insert(id.to_owned(), tag);
                Ok(changed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> Result<NbtPath> {
        NbtPath::parse(&mut StringReader::new(s))
    }

    #[test]
    fn parsing() {
        let p = path("a.b[0].\"c d\"[]{x:1b}").unwrap_err();
        assert_eq!(p.key(), Some("arguments.nbtpath.node.invalid"));
        let p = path("a.b[0].\"c d\"[] rest").unwrap();
        assert_eq!(p.text, "a.b[0].\"c d\"[]");
        assert_eq!(p.nodes, [Node::Child("a".into()), Node::Child("b".into()), Node::Index(0), Node::Child("c d".into()), Node::AllElements]);
        assert!(matches!(path("{a:1}").unwrap().nodes[0], Node::MatchRoot(_)));
        assert!(matches!(path("Items[{Slot:0b}]").unwrap().nodes[1], Node::MatchElement(_)));
        assert!(matches!(path("x{a:1}").unwrap().nodes[0], Node::MatchObject(..)));
        assert_eq!(path("a..b").unwrap_err().key(), Some("arguments.nbtpath.node.invalid"));
        assert_eq!(path("a[x]").unwrap_err().key(), Some("parsing.int.expected"));
    }

    #[test]
    fn queries() {
        let data = snbt::parse_tag(&mut StringReader::new("{a:{b:[1,2,3]},l:[{id:x},{id:y}]}")).unwrap();
        assert_eq!(path("a.b[]").unwrap().count_matching(&data), 3);
        assert_eq!(path("a.b[-1]").unwrap().get(&data), [&Tag::Int(3)]);
        assert_eq!(path("l[{id:y}]").unwrap().count_matching(&data), 1);
        assert_eq!(path("nope").unwrap().count_matching(&data), 0);
        assert_eq!(path("{a:{}}").unwrap().count_matching(&data), 1);
    }

    #[test]
    fn set_creates_parents() {
        let snbt = |s: &str| snbt::parse_tag(&mut StringReader::new(s)).unwrap();
        let mut data = Tag::Compound(Vec::new());
        assert_eq!(path("a.b.c").unwrap().set(&mut data, &Tag::Int(3)).unwrap(), 1);
        assert_eq!(data, snbt("{a:{b:{c:3}}}"));
        assert_eq!(path("a.b.c").unwrap().set(&mut data, &Tag::Int(3)).unwrap(), 0);
        assert_eq!(path("l[].x").unwrap().set(&mut data, &Tag::Byte(1)).unwrap(), 1);
        assert_eq!(path("l[0].x").unwrap().get(&data), [&Tag::Byte(1)]);
        assert_eq!(path("l[{x:1b}].y").unwrap().set(&mut data, &Tag::Byte(2)).unwrap(), 1);
        assert_eq!(path("l[{x:5b}].y").unwrap().set(&mut data, &Tag::Byte(2)).unwrap(), 1);
        assert_eq!(path("l[]").unwrap().count_matching(&data), 2);
        let e = path("a.b.c[0].d").unwrap().set(&mut data, &Tag::Int(1)).unwrap_err();
        assert_eq!(e.key(), Some("arguments.nbtpath.nothing_found"));
        let e = path("n[3].d").unwrap().set(&mut data, &Tag::Int(1)).unwrap_err();
        assert_eq!(e.key(), Some("arguments.nbtpath.nothing_found"));
    }
}
