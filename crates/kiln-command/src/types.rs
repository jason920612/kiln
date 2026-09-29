//! Small value types used by arguments and commands.

use crate::error::CommandError;
use crate::reader::StringReader;
use crate::text::Text;
use crate::tr;
use std::fmt;

/// A resource location, stored as `namespace:path`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier(String);

impl Identifier {
    /// `Identifier.parse`: the namespace defaults to `minecraft`.
    pub fn parse(s: &str) -> Option<Self> {
        let (ns, path) = match s.find(':') {
            Some(0) => ("minecraft", &s[1..]),
            Some(i) => (&s[..i], &s[i + 1..]),
            None => ("minecraft", s),
        };
        let ns_ok = ns.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-'));
        let path_ok = path.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-' | '/'));
        (ns_ok && path_ok).then(|| Identifier(format!("{ns}:{path}")))
    }

    /// `Identifier.read`: the longest run of identifier characters, then [`parse`](Self::parse).
    pub fn read(reader: &mut StringReader) -> Result<Self, CommandError> {
        let start = reader.cursor();
        while reader.can_read() && is_allowed_in_identifier(reader.peek()) {
            reader.skip();
        }
        Self::parse(&reader.string()[start..reader.cursor()]).ok_or_else(|| {
            reader.set_cursor(start);
            CommandError::invalid_id().at(reader)
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        &self.0[..self.0.find(':').unwrap()]
    }

    pub fn path(&self) -> &str {
        &self.0[self.0.find(':').unwrap() + 1..]
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn is_allowed_in_identifier(c: char) -> bool {
    matches!(c, '0'..='9' | 'a'..='z' | '_' | ':' | '/' | '.' | '-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameMode {
    Survival = 0,
    Creative = 1,
    Adventure = 2,
    Spectator = 3,
}

impl GameMode {
    pub const ALL: [GameMode; 4] = [GameMode::Survival, GameMode::Creative, GameMode::Adventure, GameMode::Spectator];

    pub fn name(self) -> &'static str {
        ["survival", "creative", "adventure", "spectator"][self as usize]
    }

    /// `GameType.byName`.
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    pub fn id(self) -> i32 {
        self as i32
    }

    /// `GameType.getLongDisplayName`: `gameMode.creative` etc.
    pub fn display_name(self) -> Text {
        Text::translate(format!("gameMode.{}", self.name()), vec![])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Difficulty {
    Peaceful = 0,
    Easy = 1,
    Normal = 2,
    Hard = 3,
}

impl Difficulty {
    pub const ALL: [Difficulty; 4] = [Difficulty::Peaceful, Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn name(self) -> &'static str {
        ["peaceful", "easy", "normal", "hard"][self as usize]
    }

    /// `Difficulty.byName`.
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.name() == name)
    }

    pub fn id(self) -> i32 {
        self as i32
    }

    /// `Difficulty.getDisplayName`: `options.difficulty.easy` etc.
    pub fn display_name(self) -> Text {
        Text::translate(format!("options.difficulty.{}", self.name()), vec![])
    }
}

/// `EntityAnchorArgument.Anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    Feet,
    Eyes,
}

impl Anchor {
    pub fn name(self) -> &'static str {
        match self {
            Anchor::Feet => "feet",
            Anchor::Eyes => "eyes",
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        [Anchor::Feet, Anchor::Eyes].into_iter().find(|a| a.name() == name)
    }
}

/// `Heightmap.Types` kept after world generation (the `heightmap` argument's values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Heightmap {
    /// Highest non-air block.
    WorldSurface,
    /// Highest block in `#blocks_motion_in_heightmap`.
    OceanFloor,
    /// Highest block in `#blocks_motion_in_heightmap` or with a fluid.
    MotionBlocking,
    /// As [`MotionBlocking`](Self::MotionBlocking) with the `_no_leaves` tag.
    MotionBlockingNoLeaves,
}

impl Heightmap {
    pub const ALL: [Heightmap; 4] =
        [Heightmap::WorldSurface, Heightmap::OceanFloor, Heightmap::MotionBlocking, Heightmap::MotionBlockingNoLeaves];

    pub fn name(self) -> &'static str {
        match self {
            Heightmap::WorldSurface => "world_surface",
            Heightmap::OceanFloor => "ocean_floor",
            Heightmap::MotionBlocking => "motion_blocking",
            Heightmap::MotionBlockingNoLeaves => "motion_blocking_no_leaves",
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.name() == name)
    }

    /// Whether `state` counts toward this heightmap.
    pub fn counts(self, state: u16) -> bool {
        match self {
            Heightmap::WorldSurface => !kiln_data::blocks_types::is_air(state),
            Heightmap::OceanFloor => crate::blocks::block_tag("minecraft:blocks_motion_in_heightmap")
                .zip(kiln_data::builtin_id("minecraft:block", kiln_data::blocks_types::block_of(state).name))
                .is_some_and(|(tag, id)| tag.contains(&id)),
            Heightmap::MotionBlocking => kiln_data::block_props::motion_blocking(state),
            Heightmap::MotionBlockingNoLeaves => kiln_data::block_props::motion_blocking_no_leaves(state),
        }
    }
}

/// An item argument: a registered item id plus its component list as written (`[k=v,!k]`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInput {
    pub item: Identifier,
    /// Component changes in order: `(id, Some(snbt))` sets, `(id, None)` removes.
    pub components: Vec<(Identifier, Option<String>)>,
}

impl ItemInput {
    /// Vanilla's item display name: `[` + the translated name + `]`.
    pub fn display_name(&self) -> Text {
        let kind =
            if kiln_data::builtin_id("minecraft:block", self.item.as_str()).is_some() { "block" } else { "item" };
        tr!(format!("{kind}.{}.{}", self.item.namespace(), self.item.path().replace('/', "."))).bracketed()
    }
}

pub(crate) fn entity_type_exists(id: &str) -> bool {
    kiln_data::builtin_id("minecraft:entity_type", id).is_some()
}

/// Whether `entity_type` is in the entity type tag `tag` (without `#`).
pub fn entity_type_in_tag(entity_type: &str, tag: &str) -> bool {
    let Some(id) = kiln_data::builtin_id("minecraft:entity_type", entity_type) else { return false };
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:entity_type")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

pub(crate) fn entity_type_tags() -> impl Iterator<Item = &'static str> {
    kiln_data::registries::TAGS
        .iter()
        .filter(|(r, _)| *r == "minecraft:entity_type")
        .flat_map(|(_, tags)| tags.iter().map(|(t, _)| *t))
}

/// `FilenameUtils.wildcardMatch(name, pattern)` (case sensitive): `*` is any run of characters,
/// `?` exactly one.
pub fn wildcard_match(name: &str, pattern: &str) -> bool {
    let (n, p): (Vec<char>, Vec<char>) = (name.chars().collect(), pattern.chars().collect());
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            star = Some((j, i));
            j += 1;
        } else if let Some((sj, si)) = star {
            j = sj + 1;
            i = si + 1;
            star = Some((sj, si + 1));
        } else {
            return false;
        }
    }
    p[j..].iter().all(|&c| c == '*')
}

/// Entries of a synchronized or built-in registry, if kiln-data knows it.
pub(crate) fn registry_entries(registry: &str) -> Option<&'static [&'static str]> {
    kiln_data::registries::SYNCHRONIZED
        .iter()
        .find(|(r, _)| *r == registry)
        .map(|(_, e)| *e)
        .or_else(|| kiln_data::builtin_entries(registry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert_eq!(Identifier::parse("stone").unwrap().as_str(), "minecraft:stone");
        assert_eq!(Identifier::parse(":stone").unwrap().as_str(), "minecraft:stone");
        assert_eq!(Identifier::parse("kiln:a/b").unwrap().path(), "a/b");
        assert!(Identifier::parse("a:b:c").is_none());
        assert!(Identifier::parse("A").is_none());
        let mut r = StringReader::new("foo:bar/baz qux");
        assert_eq!(Identifier::read(&mut r).unwrap().as_str(), "foo:bar/baz");
        let mut r = StringReader::new("a:b:c");
        let e = Identifier::read(&mut r).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("argument.id.invalid"), Some(0)));
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("minecraft:always_pass", "minecraft:*"));
        assert!(wildcard_match("minecraft:always_pass", "*:always_pass"));
        assert!(wildcard_match("minecraft:always_pass", "minecraft:always_p?ss"));
        assert!(wildcard_match("a:b", "*"));
        assert!(wildcard_match("a:b", "a:b*"));
        assert!(wildcard_match("aXbXc", "a*b*c"));
        assert!(!wildcard_match("minecraft:always_pass", "minecraft:always_pas"));
        assert!(!wildcard_match("minecraft:always_pass", "kiln:*"));
        assert!(!wildcard_match("ab", "a?b"));
        assert!(!wildcard_match("Minecraft:x", "minecraft:x"));
    }

    #[test]
    fn data_lookups() {
        assert!(entity_type_exists("minecraft:zombie"));
        assert!(!entity_type_exists("minecraft:stone"));
        assert!(entity_type_in_tag("minecraft:skeleton", "minecraft:skeletons"));
        assert!(!entity_type_in_tag("minecraft:zombie", "minecraft:skeletons"));
        let stone = ItemInput { item: Identifier::parse("stone").unwrap(), components: vec![] };
        assert_eq!(stone.display_name().to_plain(), "[block.minecraft.stone]");
        assert!(registry_entries("minecraft:timeline").unwrap().contains(&"minecraft:day"));
    }
}
