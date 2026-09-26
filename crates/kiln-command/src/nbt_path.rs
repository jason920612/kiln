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
        let mut first = true;
        while reader.can_read() && reader.peek() != ' ' {
            nodes.push(Self::parse_node(reader, first)?);
            first = false;
            if reader.can_read() {
                let c = reader.peek();
                if c != ' ' && c != '[' && c != '{' {
                    reader.expect('.')?;
                }
            }
        }
        Ok(NbtPath { text: reader.string()[start..reader.cursor()].to_owned(), nodes })
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
            let mut next = Vec::new();
            for t in current {
                match (node, t) {
                    (Node::MatchRoot(p), _) => {
                        if compare_nbt(p, t, true) {
                            next.push(t);
                        }
                    }
                    (Node::Child(name), Tag::Compound(f)) => next.extend(f.iter().filter(|(k, _)| k == name).map(|(_, v)| v)),
                    (Node::MatchObject(name, p), Tag::Compound(f)) => next.extend(
                        f.iter().filter(|(k, v)| k == name && compare_nbt(p, v, true)).map(|(_, v)| v),
                    ),
                    (Node::AllElements, Tag::List(items)) => next.extend(items.iter()),
                    (Node::Index(i), Tag::List(items)) => {
                        let idx = if *i < 0 { items.len() as i64 + *i as i64 } else { *i as i64 };
                        if (0..items.len() as i64).contains(&idx) {
                            next.push(&items[idx as usize]);
                        }
                    }
                    (Node::MatchElement(p), Tag::List(items)) => {
                        next.extend(items.iter().filter(|e| compare_nbt(p, e, true)))
                    }
                    _ => {}
                }
            }
            current = next;
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
        let mut parents = 0;
        let changed = set_in(&self.nodes, root, value, &mut parents);
        if parents == 0 {
            return Err(CommandError::new(tr!("arguments.nbtpath.nothing_found", self.text.as_str())));
        }
        Ok(changed)
    }
}

impl Node {
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

/// Walks the parents of the last node (creating them), then sets under each.
fn set_in(nodes: &[Node], tag: &mut Tag, value: &Tag, parents: &mut usize) -> i32 {
    let Some((node, rest)) = nodes.split_first() else { return 0 };
    let Some(next) = rest.first() else {
        *parents += 1;
        return node.set_tag(tag, value);
    };
    node.get_or_create(tag, || next.preferred_parent()).into_iter().map(|child| set_in(rest, child, value, parents)).sum()
}

/// `CommandStorage`: compound tags by id for `execute store ... storage` and
/// `execute if data storage`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandStorage(std::collections::BTreeMap<String, Tag>);

impl CommandStorage {
    /// `get`: an empty compound for ids never written.
    pub fn get(&self, id: &str) -> Tag {
        self.0.get(id).cloned().unwrap_or(Tag::Compound(Vec::new()))
    }

    /// `ExecuteCommand.storeData` through `StorageDataAccessor`: `get` hands out the stored
    /// compound itself (so parents created by a failed set stay), or a fresh one that is
    /// only kept when the set succeeds.
    pub fn store(&mut self, id: &str, path: &NbtPath, value: &Tag) -> Result<i32> {
        match self.0.get_mut(id) {
            Some(tag) => path.set(tag, value),
            None => {
                let mut tag = Tag::Compound(Vec::new());
                let changed = path.set(&mut tag, value)?;
                self.0.insert(id.to_owned(), tag);
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
