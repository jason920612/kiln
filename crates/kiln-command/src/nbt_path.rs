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
}
