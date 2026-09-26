//! Block arguments: `block_state` (`BlockStateParser.parseForBlock`) and `block_predicate`
//! (`parseForTesting`, which adds tags and "vague" properties), block tags from kiln-data's
//! tables, and the update flags of `Level.setBlock`.

use crate::error::CommandError;
use crate::reader::StringReader;
use crate::snbt;
use crate::suggestion::SuggestionsBuilder;
use crate::tr;
use crate::types::Identifier;
use kiln_data::blocks_types::{BlockInfo, block_by_name, block_of};
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// `Block.UPDATE_*`: how a block change propagates (`Level.setBlock` flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateFlags(pub u32);

impl UpdateFlags {
    pub const NEIGHBORS: u32 = 1;
    pub const CLIENTS: u32 = 2;
    pub const KNOWN_SHAPE: u32 = 16;
    pub const SUPPRESS_DROPS: u32 = 32;
    pub const SKIP_BLOCK_ENTITY_SIDE_EFFECTS: u32 = 256;
    pub const SKIP_ON_PLACE: u32 = 512;
    /// `Block.UPDATE_ALL`.
    pub const ALL: UpdateFlags = UpdateFlags(Self::NEIGHBORS | Self::CLIENTS);
    /// What `strict` adds: no shape adaption, drops or on-place reactions (816).
    pub const STRICT: u32 = Self::KNOWN_SHAPE | Self::SUPPRESS_DROPS | Self::SKIP_BLOCK_ENTITY_SIDE_EFFECTS | Self::SKIP_ON_PLACE;

    /// `/setblock` and `/fill`: `2 | (strict ? 816 : 256)`.
    pub fn placement(strict: bool) -> Self {
        UpdateFlags(Self::CLIENTS | if strict { Self::STRICT } else { Self::SKIP_BLOCK_ENTITY_SIDE_EFFECTS })
    }

    pub fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    pub fn with(self, flag: u32) -> Self {
        UpdateFlags(self.0 | flag)
    }
}

/// A block state argument (`BlockInput`): the state, the properties the user set (which
/// shape adaption must not override) and block entity data to merge.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInput {
    pub state: u16,
    pub properties: Vec<&'static str>,
    pub nbt: Option<Tag>,
}

impl BlockInput {
    pub fn block(&self) -> &'static BlockInfo {
        block_of(self.state)
    }

    /// `BlockInput.overwriteWithDefinedProperties`: `state` with the explicitly given
    /// properties of this input (when it is the same block).
    pub fn overwrite_defined(&self, state: u16) -> u16 {
        let block = block_of(state);
        if block.name != self.block().name {
            return state;
        }
        self.properties.iter().fold(state, |s, &p| {
            let value = self.block().property(self.state, p).unwrap_or_default();
            block.with_property(s, p, value).unwrap_or(s)
        })
    }
}

/// A `block_predicate` argument (`BlockPredicateArgument.Result`).
#[derive(Debug, Clone, PartialEq)]
pub enum BlockPredicate {
    /// A block with the properties given explicitly.
    Block { state: u16, properties: Vec<&'static str>, nbt: Option<Tag> },
    /// Any block in a tag, with properties matched by name where the block has them.
    Tag { tag: Identifier, blocks: &'static [i32], properties: Vec<(String, String)>, nbt: Option<Tag> },
}

impl BlockPredicate {
    /// Whether the block needs its block entity data to be tested.
    pub fn requires_nbt(&self) -> bool {
        match self {
            BlockPredicate::Block { nbt, .. } | BlockPredicate::Tag { nbt, .. } => nbt.is_some(),
        }
    }

    /// `test(BlockInWorld)`: `block_entity` is the block's entity data, if it has one.
    pub fn test(&self, state: u16, block_entity: Option<&Tag>) -> bool {
        let (matches, nbt) = match self {
            BlockPredicate::Block { state: want, properties, nbt } => {
                let (block, target) = (block_of(state), block_of(*want));
                let same = block.name == target.name
                    && properties.iter().all(|&p| block.property(state, p) == target.property(*want, p));
                (same, nbt)
            }
            BlockPredicate::Tag { blocks, properties, nbt, .. } => {
                let block = block_of(state);
                let in_tag = block_id(block).is_some_and(|id| blocks.contains(&id));
                let props = properties.iter().all(|(k, v)| block.property(state, k).is_some_and(|actual| {
                    block.properties.iter().any(|p| p.name == k && p.values.contains(&v.as_str())) && actual == v
                }));
                (in_tag && props, nbt)
            }
        };
        matches
            && nbt.as_ref().is_none_or(|want| block_entity.is_some_and(|have| compare_nbt(want, have, true)))
    }
}

/// `NbtUtils.compareNbt`: whether `expected` is contained in `actual` (lists match when every
/// expected element is found somewhere, if `partial_lists`).
pub fn compare_nbt(expected: &Tag, actual: &Tag, partial_lists: bool) -> bool {
    match (expected, actual) {
        (Tag::Compound(e), Tag::Compound(a)) => e.iter().all(|(k, v)| {
            a.iter().find(|(ak, _)| ak == k).is_some_and(|(_, av)| compare_nbt(v, av, partial_lists))
        }),
        (Tag::List(e), Tag::List(a)) if partial_lists => {
            if e.is_empty() {
                return a.is_empty();
            }
            e.iter().all(|ev| a.iter().any(|av| compare_nbt(ev, av, partial_lists)))
        }
        _ => expected == actual,
    }
}

/// Registry id of a block.
fn block_id(block: &BlockInfo) -> Option<i32> {
    kiln_data::builtin_id("minecraft:block", block.name)
}

/// Members (block registry ids) of a block tag such as `minecraft:logs`.
pub fn block_tag(tag: &str) -> Option<&'static [i32]> {
    registry_tag("minecraft:block", tag)
}

/// Members of a tag of `registry`, as kiln-data knows them (synchronized tags).
pub fn registry_tag(registry: &str, tag: &str) -> Option<&'static [i32]> {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == registry)
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .map(|(_, ids)| *ids)
}

fn tags_of(registry: &str) -> impl Iterator<Item = &'static str> {
    kiln_data::registries::TAGS.iter().filter(move |(r, _)| *r == registry).flat_map(|(_, tags)| tags.iter().map(|(t, _)| *t))
}

/// `BlockStateParser`: shared by both argument types.
struct Parser<'r, 'a> {
    reader: &'r mut StringReader<'a>,
    for_testing: bool,
}

/// What was parsed: a block or a tag, with properties and NBT.
enum Parsed {
    Block { state: u16, properties: Vec<&'static str>, nbt: Option<Tag> },
    Tag { tag: Identifier, blocks: &'static [i32], properties: Vec<(String, String)>, nbt: Option<Tag> },
}

impl Parser<'_, '_> {
    fn parse(&mut self) -> Result<Parsed> {
        if self.reader.can_read() && self.reader.peek() == '#' {
            let (tag, blocks) = self.read_tag()?;
            let properties = if self.reader.can_read() && self.reader.peek() == '[' {
                self.read_vague_properties(&tag)?
            } else {
                Vec::new()
            };
            let nbt = self.read_nbt()?;
            return Ok(Parsed::Tag { tag, blocks, properties, nbt });
        }
        let (id, block) = self.read_block()?;
        let mut state = block.default;
        let mut properties = Vec::new();
        if self.reader.can_read() && self.reader.peek() == '[' {
            self.read_properties(&id, block, &mut state, &mut properties)?;
        }
        let nbt = self.read_nbt()?;
        Ok(Parsed::Block { state, properties, nbt })
    }

    fn read_block(&mut self) -> Result<(Identifier, &'static BlockInfo)> {
        let start = self.reader.cursor();
        let id = Identifier::read(self.reader)?;
        match block_by_name(id.as_str()) {
            Some(b) => Ok((id, b)),
            None => {
                self.reader.set_cursor(start);
                Err(CommandError::new(tr!("argument.block.id.invalid", id.to_string())).at(self.reader))
            }
        }
    }

    fn read_tag(&mut self) -> Result<(Identifier, &'static [i32])> {
        if !self.for_testing {
            return Err(CommandError::new(tr!("argument.block.tag.disallowed")).at(self.reader));
        }
        let start = self.reader.cursor();
        self.reader.expect('#')?;
        let id = Identifier::read(self.reader)?;
        match block_tag(id.as_str()) {
            Some(blocks) => Ok((id, blocks)),
            None => {
                self.reader.set_cursor(start);
                Err(CommandError::new(tr!("arguments.block.tag.unknown", id.to_string())).at(self.reader))
            }
        }
    }

    fn read_properties(
        &mut self,
        id: &Identifier,
        block: &'static BlockInfo,
        state: &mut u16,
        set: &mut Vec<&'static str>,
    ) -> Result<()> {
        self.reader.skip();
        self.reader.skip_whitespace();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            let start = self.reader.cursor();
            let name = self.reader.read_string()?;
            let Some(prop) = block.properties.iter().find(|p| p.name == name) else {
                self.reader.set_cursor(start);
                return Err(
                    CommandError::new(tr!("argument.block.property.unknown", id.to_string(), name)).at(self.reader)
                );
            };
            if set.contains(&prop.name) {
                self.reader.set_cursor(start);
                return Err(CommandError::new(tr!("argument.block.property.duplicate", name, id.to_string()))
                    .at(self.reader));
            }
            self.reader.skip_whitespace();
            if !self.reader.can_read() || self.reader.peek() != '=' {
                return Err(
                    CommandError::new(tr!("argument.block.property.novalue", name, id.to_string())).at(self.reader)
                );
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            let value_start = self.reader.cursor();
            let value = self.reader.read_string()?;
            match prop.values.iter().find(|v| **v == value) {
                Some(v) => {
                    *state = block.with_property(*state, prop.name, v).unwrap_or(*state);
                    set.push(prop.name);
                }
                None => {
                    self.reader.set_cursor(value_start);
                    return Err(CommandError::new(tr!(
                        "argument.block.property.invalid",
                        id.to_string(),
                        value,
                        prop.name
                    ))
                    .at(self.reader));
                }
            }
            self.reader.skip_whitespace();
            if self.reader.can_read() {
                match self.reader.peek() {
                    ',' => {
                        self.reader.skip();
                        continue;
                    }
                    ']' => break,
                    _ => return Err(unclosed().at(self.reader)),
                }
            }
        }
        if !self.reader.can_read() {
            return Err(unclosed().at(self.reader));
        }
        self.reader.skip();
        Ok(())
    }

    fn read_vague_properties(&mut self, tag: &Identifier) -> Result<Vec<(String, String)>> {
        self.reader.skip();
        let mut out: Vec<(String, String)> = Vec::new();
        let mut value_start: Option<usize> = None;
        self.reader.skip_whitespace();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            let start = self.reader.cursor();
            let name = self.reader.read_string()?;
            if out.iter().any(|(k, _)| *k == name) {
                self.reader.set_cursor(start);
                return Err(CommandError::new(tr!("argument.block.property.duplicate", name, tag.to_string()))
                    .at(self.reader));
            }
            self.reader.skip_whitespace();
            if !self.reader.can_read() || self.reader.peek() != '=' {
                self.reader.set_cursor(start);
                return Err(
                    CommandError::new(tr!("argument.block.property.novalue", name, tag.to_string())).at(self.reader)
                );
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            value_start = Some(self.reader.cursor());
            let value = self.reader.read_string()?;
            out.push((name, value));
            self.reader.skip_whitespace();
            if self.reader.can_read() {
                value_start = None;
                match self.reader.peek() {
                    ',' => {
                        self.reader.skip();
                        continue;
                    }
                    ']' => break,
                    _ => return Err(unclosed().at(self.reader)),
                }
            }
        }
        if !self.reader.can_read() {
            if let Some(v) = value_start {
                self.reader.set_cursor(v);
            }
            return Err(unclosed().at(self.reader));
        }
        self.reader.skip();
        Ok(out)
    }

    fn read_nbt(&mut self) -> Result<Option<Tag>> {
        if self.reader.can_read() && self.reader.peek() == '{' {
            return snbt::parse_compound(self.reader).map(Some);
        }
        Ok(None)
    }
}

fn unclosed() -> CommandError {
    CommandError::new(tr!("argument.block.property.unclosed"))
}

/// `BlockStateArgument.parse`.
pub fn parse_block_state(reader: &mut StringReader) -> Result<BlockInput> {
    let start = reader.cursor();
    let parsed = Parser { reader, for_testing: false }.parse();
    match parsed {
        Ok(Parsed::Block { state, properties, nbt }) => Ok(BlockInput { state, properties, nbt }),
        Ok(Parsed::Tag { .. }) => unreachable!("tags are rejected when not testing"),
        Err(e) => {
            reader.set_cursor(start);
            Err(e)
        }
    }
}

/// `BlockPredicateArgument.parse`.
pub fn parse_block_predicate(reader: &mut StringReader) -> Result<BlockPredicate> {
    let start = reader.cursor();
    match (Parser { reader, for_testing: true }).parse() {
        Ok(Parsed::Block { state, properties, nbt }) => Ok(BlockPredicate::Block { state, properties, nbt }),
        Ok(Parsed::Tag { tag, blocks, properties, nbt }) => Ok(BlockPredicate::Tag { tag, blocks, properties, nbt }),
        Err(e) => {
            reader.set_cursor(start);
            Err(e)
        }
    }
}

/// Suggestions for a block argument: block ids (and `#tags` when testing), then property
/// names and values or the brackets that follow (`BlockStateParser.fillSuggestions`, without
/// NBT).
pub fn suggest(builder: &mut SuggestionsBuilder, for_testing: bool) {
    let remaining = builder.remaining();
    let bracket = remaining.find('[');
    let Some(open) = bracket else {
        let blocks = kiln_data::builtin_entries("minecraft:block").unwrap_or(&[]);
        if for_testing {
            builder.suggest_resources(tags_of("minecraft:block"), "#");
        }
        builder.suggest_resources(blocks.iter().copied(), "");
        // After a complete id: offer the opening bracket.
        if let Some(b) = Identifier::parse(remaining).and_then(|id| block_by_name(id.as_str()))
            && !b.properties.is_empty()
        {
            let mut after = builder.offset(builder.start() + remaining.len());
            after.suggest("[");
            builder.add(after);
        }
        return;
    };
    let id = &remaining[..open];
    let Some(block) = Identifier::parse(id).and_then(|id| block_by_name(id.as_str())) else { return };
    let props = &remaining[open + 1..];
    if props.contains(']') {
        return;
    }
    // The property being written: after the last comma.
    let part_start = props.rfind(',').map_or(0, |i| i + 1);
    let part = &props[part_start..];
    let given: Vec<&str> =
        props[..part_start].split(',').filter_map(|kv| kv.split('=').next()).map(str::trim).collect();
    let base = builder.start() + open + 1 + part_start;
    match part.split_once('=') {
        None => {
            let mut b = builder.offset(base + (part.len() - part.trim_start().len()));
            for p in block.properties.iter().filter(|p| !given.contains(&p.name)) {
                if p.name.starts_with(part.trim()) {
                    b.suggest(format!("{}=", p.name));
                }
            }
            builder.add(b);
        }
        Some((name, value)) => {
            let Some(p) = block.properties.iter().find(|p| p.name == name.trim()) else { return };
            let value_start = base + name.len() + 1;
            let mut b = builder.offset(value_start);
            for v in p.values {
                if v.starts_with(value.trim()) {
                    b.suggest(*v);
                }
            }
            if p.values.contains(&value.trim()) {
                let mut next = builder.offset(value_start + value.len());
                next.suggest(",");
                next.suggest("]");
                b.add(next);
            }
            builder.add(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_data::blocks::default_state as d;

    fn state(s: &str) -> Result<BlockInput> {
        parse_block_state(&mut StringReader::new(s))
    }

    fn err(r: Result<impl std::fmt::Debug>) -> (String, usize) {
        let e = r.unwrap_err();
        (e.key().unwrap().to_owned(), e.cursor().unwrap())
    }

    #[test]
    fn states() {
        assert_eq!(state("stone").unwrap().state, d::STONE);
        let stairs = state("minecraft:oak_stairs[facing=east, half = top]").unwrap();
        let info = block_by_name("minecraft:oak_stairs").unwrap();
        assert_eq!(info.property(stairs.state, "facing"), Some("east"));
        assert_eq!(info.property(stairs.state, "half"), Some("top"));
        assert_eq!(stairs.properties, ["facing", "half"]);
        let chest = state("chest[facing=north]{Items:[],Lock:\"k\"}").unwrap();
        assert!(matches!(chest.nbt, Some(Tag::Compound(ref f)) if f.len() == 2));
        assert_eq!(err(state("nope")), ("argument.block.id.invalid".into(), 0));
        assert_eq!(err(state("stone[x=1]")), ("argument.block.property.unknown".into(), 6));
        assert_eq!(err(state("oak_log[axis=q]")), ("argument.block.property.invalid".into(), 13));
        assert_eq!(err(state("oak_log[axis=x,axis=y]")), ("argument.block.property.duplicate".into(), 15));
        assert_eq!(err(state("oak_log[axis]")), ("argument.block.property.novalue".into(), 12));
        assert_eq!(err(state("oak_log[axis=x")), ("argument.block.property.unclosed".into(), 14));
        assert_eq!(err(state("oak_log[axis=x y")), ("argument.block.property.unclosed".into(), 15));
        assert_eq!(err(state("#minecraft:logs")), ("argument.block.tag.disallowed".into(), 0));
        let e = state("oak_log[axis=q]").unwrap_err();
        assert_eq!(e.args().len(), 3);
    }

    #[test]
    fn predicates() {
        let p = parse_block_predicate(&mut StringReader::new("#minecraft:logs[axis=x]")).unwrap();
        let log = block_by_name("minecraft:oak_log").unwrap();
        let x = log.with_property(log.default, "axis", "x").unwrap();
        assert!(p.test(x, None));
        assert!(!p.test(log.default, None), "axis=y by default");
        assert!(!p.test(d::STONE, None));
        let p = parse_block_predicate(&mut StringReader::new("#minecraft:logs[nonexistent=1]")).unwrap();
        assert!(!p.test(x, None), "properties the block lacks fail the test");
        let p = parse_block_predicate(&mut StringReader::new("oak_log")).unwrap();
        assert!(p.test(x, None), "unspecified properties are ignored");
        let p = parse_block_predicate(&mut StringReader::new("oak_log[axis=z]")).unwrap();
        assert!(!p.test(x, None));
        let p = parse_block_predicate(&mut StringReader::new("chest{Lock:\"k\"}")).unwrap();
        let chest = block_by_name("minecraft:chest").unwrap().default;
        assert!(!p.test(chest, None), "nbt needs a block entity");
        let data = snbt::parse_tag(&mut StringReader::new("{Lock:\"k\",Items:[]}")).unwrap();
        assert!(p.test(chest, Some(&data)));
        assert_eq!(
            err(parse_block_predicate(&mut StringReader::new("#minecraft:nope"))),
            ("arguments.block.tag.unknown".into(), 0)
        );
    }

    #[test]
    fn nbt_comparison() {
        let t = |s: &str| snbt::parse_tag(&mut StringReader::new(s)).unwrap();
        assert!(compare_nbt(&t("{a:1}"), &t("{a:1,b:2}"), true));
        assert!(!compare_nbt(&t("{a:1}"), &t("{a:2}"), true));
        assert!(compare_nbt(&t("{l:[2]}"), &t("{l:[1,2,3]}"), true));
        assert!(!compare_nbt(&t("{l:[]}"), &t("{l:[1]}"), true));
    }

    #[test]
    fn overwrite_keeps_explicit_properties() {
        let input = state("oak_stairs[facing=west]").unwrap();
        let stairs = block_by_name("minecraft:oak_stairs").unwrap();
        let shaped = stairs.with_property(stairs.default, "shape", "inner_left").unwrap();
        let out = input.overwrite_defined(shaped);
        assert_eq!(stairs.property(out, "facing"), Some("west"));
        assert_eq!(stairs.property(out, "shape"), Some("inner_left"));
    }
}
