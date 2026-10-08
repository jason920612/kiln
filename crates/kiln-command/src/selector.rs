//! Entity selectors: parsing like `EntitySelectorParser` (selector types, options and their
//! applicability rules) and selection like `EntitySelector.findEntities`.

use crate::coords::wrap_degrees;
use crate::error::CommandError;
use crate::host::Source;
use crate::range::{DoubleRange, FloatRange, IntRange};
use crate::reader::StringReader;
use crate::scoreboard::Scoreboard;
use crate::snbt;
use crate::suggestion::SuggestionsBuilder;
use crate::text::Text;
use crate::tr;
use crate::types::{self, GameMode, Identifier};
use uuid::Uuid;

type Result<T> = std::result::Result<T, CommandError>;

/// Selectors are allowed from permission level 2 (`commands/entity_selectors`).
pub const SELECTOR_PERMISSION: u8 = 2;

/// Axis-aligned box, min inclusive / max exclusive as in `AABB.intersects`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    pub fn intersects(&self, o: &Aabb) -> bool {
        (0..3).all(|a| self.min[a] < o.max[a] && self.max[a] > o.min[a])
    }

    fn offset(&self, d: [f64; 3]) -> Aabb {
        Aabb { min: std::array::from_fn(|a| self.min[a] + d[a]), max: std::array::from_fn(|a| self.max[a] + d[a]) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Arbitrary,
    Nearest,
    Furthest,
    Random,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdvancementCheck {
    Done(bool),
    Criteria(Vec<(String, bool)>),
}

/// One `key=value` condition. `invert` is the `!` prefix.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    Alive,
    Name { name: String, invert: bool },
    Type { id: String, invert: bool },
    TypeTag { tag: String, invert: bool },
    Tag { tag: String, invert: bool },
    Team { team: String, invert: bool },
    GameMode { mode: GameMode, invert: bool },
    Nbt { snbt: String, invert: bool },
    Scores(Vec<(String, IntRange)>),
    Advancements(Vec<(String, AdvancementCheck)>),
    Predicate { id: String, invert: bool },
}

/// A parsed selector, player name or UUID.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySelector {
    pub max_results: usize,
    pub includes_entities: bool,
    pub world_limited: bool,
    pub current_entity: bool,
    pub uses_selector: bool,
    pub player_name: Option<String>,
    pub uuid: Option<Uuid>,
    pub position: [Option<f64>; 3],
    pub delta: [Option<f64>; 3],
    pub distance: Option<DoubleRange>,
    pub level: Option<IntRange>,
    pub x_rotation: Option<FloatRange>,
    pub y_rotation: Option<FloatRange>,
    pub order: Order,
    pub filters: Vec<Filter>,
    /// `limitToType`: `@p`/`@a`/`@r` and positive `type=` options.
    pub entity_type: Option<String>,
}

/// What selection needs to know about an entity. Implement it on an entity snapshot or handle.
pub trait SelectorTarget {
    fn uuid(&self) -> Uuid;
    /// Plain-text name (`getPlainTextName`), matched by `name=` and player names.
    fn name(&self) -> String;
    fn display_name(&self) -> Text {
        Text::literal(self.name())
    }
    /// Entity type id, e.g. `minecraft:player`.
    fn entity_type(&self) -> &str;
    fn is_player(&self) -> bool {
        self.entity_type() == "minecraft:player"
    }
    fn position(&self) -> [f64; 3];
    /// `[yaw, pitch]`.
    fn rotation(&self) -> [f32; 2];
    fn dimension(&self) -> &str;
    fn bounding_box(&self) -> Aabb;
    fn eye_height(&self) -> f64 {
        0.0
    }
    fn is_alive(&self) -> bool {
        true
    }
    /// Players only.
    fn game_mode(&self) -> Option<GameMode> {
        None
    }
    /// Players only.
    fn experience_level(&self) -> Option<i32> {
        None
    }
    fn tags(&self) -> &[String] {
        &[]
    }
    fn team(&self) -> Option<&str> {
        None
    }
    /// `None` when the objective does not exist or the entity has no score.
    fn score(&self, _objective: &str) -> Option<i32> {
        None
    }
    /// `None` when the advancement does not exist.
    fn advancement_done(&self, _id: &str) -> Option<bool> {
        None
    }
    /// `None` when the advancement or criterion does not exist.
    fn criterion_done(&self, _id: &str, _criterion: &str) -> Option<bool> {
        None
    }
    /// `nbt=`: whether the entity's data contains `snbt`.
    fn matches_nbt(&self, _snbt: &str) -> bool {
        false
    }
    /// `predicate=`: `None` when the predicate does not exist.
    fn test_predicate(&self, _id: &str) -> Option<bool> {
        None
    }
    /// `Entity.getScoreboardName`: the player name, or the UUID for other entities.
    fn scoreboard_name(&self) -> String {
        if self.is_player() { self.name() } else { self.uuid().to_string() }
    }
}

/// The entity type of sources that never have entities (it has no values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoEntity {}

impl SelectorTarget for NoEntity {
    fn uuid(&self) -> Uuid {
        match *self {}
    }
    fn name(&self) -> String {
        match *self {}
    }
    fn entity_type(&self) -> &str {
        match *self {}
    }
    fn position(&self) -> [f64; 3] {
        match *self {}
    }
    fn rotation(&self) -> [f32; 2] {
        match *self {}
    }
    fn dimension(&self) -> &str {
        match *self {}
    }
    fn bounding_box(&self) -> Aabb {
        match *self {}
    }
}

/// The world as seen by selectors, implemented by the command source. The executing entity,
/// position and dimension come from the [`SourceStack`](crate::SourceStack).
pub trait SelectorWorld: Source {
    /// Online players in player-list order.
    fn players(&self) -> Vec<Self::Entity>;
    /// Candidate entities in `dimension` (all dimensions if `None`); may be pre-filtered to
    /// those intersecting `area`.
    fn entities(&self, dimension: Option<&str>, area: Option<&Aabb>) -> Vec<Self::Entity>;
    fn entity_by_uuid(&self, uuid: Uuid) -> Option<Self::Entity> {
        self.entities(None, None).into_iter().find(|e| e.uuid() == uuid)
    }
    /// Shuffles for `sort=random` and `@r`.
    fn shuffle(&mut self, entities: &mut [Self::Entity]);
    /// The server scoreboard, if the host keeps one.
    fn scoreboard(&self) -> Option<&crate::scoreboard::Scoreboard> {
        None
    }
    /// `nbt=`: whether the entity's saved data contains the compound `snbt`
    /// (`NbtPredicate.matches`).
    fn entity_nbt_matches(&self, entity: &Self::Entity, snbt: &str) -> bool {
        entity.matches_nbt(snbt)
    }
    /// `advancements=`: whether a player has completed advancement `id` (`None` when it does
    /// not exist).
    fn entity_advancement_done(&self, entity: &Self::Entity, id: &str) -> Option<bool> {
        entity.advancement_done(id)
    }
    /// `advancements={id={criterion=..}}`: whether the player has the criterion (`None` when the
    /// advancement or criterion does not exist).
    fn entity_criterion_done(&self, entity: &Self::Entity, id: &str, criterion: &str) -> Option<bool> {
        entity.criterion_done(id, criterion)
    }
    /// `predicate=`: whether the loot predicate `id` holds with `entity` as `this_entity` at its
    /// position (`None` when the predicate does not exist).
    fn entity_predicate(&self, entity: &Self::Entity, id: &str) -> Option<bool> {
        entity.test_predicate(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limitation {
    None,
    Single,
    Multiple,
}

/// `InvertableSetOptionState`: one positive value, or any number of negated values/tags.
#[derive(Debug, Clone)]
struct InvertableSet {
    state: Limitation,
    tags: Vec<String>,
}

impl InvertableSet {
    fn new() -> Self {
        InvertableSet { state: Limitation::None, tags: Vec::new() }
    }
    fn can_parse_any(&self) -> bool {
        self.state != Limitation::Single
    }
    fn can_parse_element(&self, invert: bool) -> bool {
        if invert { self.state != Limitation::Single } else { self.state == Limitation::None }
    }
    fn can_parse_tag(&self, tag: &str) -> bool {
        self.state != Limitation::Single && !self.tags.iter().any(|t| t == tag)
    }
    fn mark_element(&mut self, invert: bool) {
        self.state = if invert { Limitation::Multiple } else { Limitation::Single };
    }
    fn mark_tag(&mut self, tag: String) {
        self.state = Limitation::Multiple;
        self.tags.push(tag);
    }
}

/// Option names, for suggestions.
pub const OPTIONS: &[&str] = &[
    "name",
    "distance",
    "level",
    "x",
    "y",
    "z",
    "dx",
    "dy",
    "dz",
    "x_rotation",
    "y_rotation",
    "limit",
    "sort",
    "gamemode",
    "team",
    "type",
    "tag",
    "nbt",
    "scores",
    "advancements",
    "predicate",
];

/// Where vanilla's parser would suggest from (`EntitySelectorParser.suggestions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hint {
    NameOrSelector,
    Name,
    Selector,
    OpenOptions,
    Key,
    Nothing,
    NextOrClose,
    Sort,
    GameMode,
    Type,
}

struct Parser<'r, 'a> {
    reader: &'r mut StringReader<'a>,
    allow_selectors: bool,
    hint: Hint,
    start: usize,
    sel: EntitySelector,
    name_opt: InvertableSet,
    gamemode_opt: InvertableSet,
    team_opt: InvertableSet,
    type_opt: InvertableSet,
    limited: bool,
    sorted: bool,
    scores: bool,
    advancements: bool,
}

impl EntitySelector {
    /// Parses a selector, name or UUID. `allow_selectors` is false below permission level 2.
    pub fn parse(reader: &mut StringReader, allow_selectors: bool) -> Result<Self> {
        let mut p = Parser::new(reader, allow_selectors);
        p.parse()?;
        Ok(p.sel)
    }

    /// Whether this can match only the executing entity (`isSelfSelector`).
    pub fn is_self_selector(&self) -> bool {
        self.current_entity
    }
}

/// Suggestions for a selector argument starting at `builder.start()`, following vanilla's
/// parser states; `names` are online player names.
pub fn suggest(builder: &mut SuggestionsBuilder, allow_selectors: bool, names: &[String]) {
    let mut reader = StringReader::new(builder.input());
    reader.set_cursor(builder.start());
    let mut p = Parser::new(&mut reader, allow_selectors);
    let _ = p.parse();
    let mut b = builder.offset(p.reader.cursor());
    let selectors = |b: &mut SuggestionsBuilder| {
        for (s, key) in [
            ("@p", "nearestPlayer"),
            ("@a", "allPlayers"),
            ("@r", "randomPlayer"),
            ("@s", "self"),
            ("@e", "allEntities"),
            ("@n", "nearestEntity"),
        ] {
            b.suggest_with_tooltip(s, tr!(format!("argument.entity.selector.{key}")));
        }
    };
    match p.hint {
        Hint::NameOrSelector => {
            b.suggest_matching(names.iter().map(String::as_str));
            if allow_selectors {
                selectors(&mut b);
            }
        }
        Hint::Name => {
            let mut n = b.offset(p.start);
            n.suggest_matching(names.iter().map(String::as_str));
            b.add(n);
        }
        Hint::Selector => {
            let mut n = b.offset(b.start() - 1);
            selectors(&mut n);
            b.add(n);
        }
        Hint::OpenOptions => b.suggest("["),
        Hint::Key => {
            let prefix = b.remaining_lowercase().to_owned();
            let at = p.reader.cursor();
            for &name in OPTIONS {
                if name.starts_with(&prefix) && p.check_option(name, at).is_ok() {
                    b.suggest_with_tooltip(
                        format!("{name}="),
                        tr!(format!("argument.entity.options.{name}.description")),
                    );
                }
            }
        }
        Hint::Nothing => {}
        Hint::NextOrClose => {
            b.suggest(",");
            b.suggest("]");
        }
        Hint::Sort => b.suggest_matching(["nearest", "furthest", "random", "arbitrary"]),
        Hint::GameMode => {
            let negated = b.remaining_lowercase().starts_with('!');
            let typed = b.remaining_lowercase().trim_start_matches('!').to_owned();
            for mode in GameMode::ALL {
                if mode.name().starts_with(&typed) {
                    if negated && p.gamemode_opt.can_parse_element(true) {
                        b.suggest(format!("!{}", mode.name()));
                    } else if !negated && p.gamemode_opt.can_parse_element(false) {
                        b.suggest(mode.name());
                    }
                }
            }
        }
        Hint::Type => {
            let types = kiln_data::builtin_entries("minecraft:entity_type").unwrap_or(&[]);
            if p.type_opt.can_parse_element(true) {
                b.suggest_resources(types.iter().copied(), "!");
            }
            if p.type_opt.can_parse_element(false) {
                b.suggest_resources(types.iter().copied(), "");
            }
            if p.type_opt.can_parse_any() {
                b.suggest_resources(types::entity_type_tags(), "#");
                b.suggest_resources(types::entity_type_tags(), "!#");
            }
        }
    }
    builder.add(b);
}

impl<'r, 'a> Parser<'r, 'a> {
    fn new(reader: &'r mut StringReader<'a>, allow_selectors: bool) -> Self {
        Parser {
            reader,
            allow_selectors,
            hint: Hint::NameOrSelector,
            start: 0,
            sel: EntitySelector {
                max_results: 0,
                includes_entities: false,
                world_limited: false,
                current_entity: false,
                uses_selector: false,
                player_name: None,
                uuid: None,
                position: [None; 3],
                delta: [None; 3],
                distance: None,
                level: None,
                x_rotation: None,
                y_rotation: None,
                order: Order::Arbitrary,
                filters: Vec::new(),
                entity_type: None,
            },
            name_opt: InvertableSet::new(),
            gamemode_opt: InvertableSet::new(),
            team_opt: InvertableSet::new(),
            type_opt: InvertableSet::new(),
            limited: false,
            sorted: false,
            scores: false,
            advancements: false,
        }
    }

    fn parse(&mut self) -> Result<()> {
        self.start = self.reader.cursor();
        self.hint = Hint::NameOrSelector;
        if self.reader.can_read() && self.reader.peek() == '@' {
            if !self.allow_selectors {
                return Err(CommandError::selectors_not_allowed().at(self.reader));
            }
            self.reader.skip();
            self.parse_selector()
        } else {
            self.parse_name_or_uuid()
        }
    }

    fn rollback(&mut self, cursor: usize, e: CommandError) -> CommandError {
        self.reader.set_cursor(cursor);
        e.at(self.reader)
    }

    fn parse_selector(&mut self) -> Result<()> {
        self.sel.uses_selector = true;
        self.hint = Hint::Selector;
        if !self.reader.can_read() {
            return Err(CommandError::missing_selector_type().at(self.reader));
        }
        let start = self.reader.cursor();
        let s = &mut self.sel;
        match self.reader.read() {
            'p' => {
                (s.max_results, s.includes_entities, s.order) = (1, false, Order::Nearest);
                s.entity_type = Some("minecraft:player".into());
            }
            'a' => {
                (s.max_results, s.includes_entities, s.order) = (usize::MAX, false, Order::Arbitrary);
                s.entity_type = Some("minecraft:player".into());
            }
            'r' => {
                (s.max_results, s.includes_entities, s.order) = (1, false, Order::Random);
                s.entity_type = Some("minecraft:player".into());
            }
            's' => (s.max_results, s.includes_entities, s.current_entity) = (1, true, true),
            'e' => {
                (s.max_results, s.includes_entities, s.order) = (usize::MAX, true, Order::Arbitrary);
                s.filters.push(Filter::Alive);
            }
            'n' => {
                (s.max_results, s.includes_entities, s.order) = (1, true, Order::Nearest);
                s.filters.push(Filter::Alive);
            }
            c => return Err(self.rollback(start, CommandError::unknown_selector_type(&format!("@{c}")))),
        }
        self.hint = Hint::OpenOptions;
        if self.reader.can_read() && self.reader.peek() == '[' {
            self.reader.skip();
            self.parse_options()?;
        }
        Ok(())
    }

    fn parse_name_or_uuid(&mut self) -> Result<()> {
        if self.reader.can_read() {
            self.hint = Hint::Name;
        }
        let start = self.reader.cursor();
        let s = self.reader.read_string()?;
        if let Some(uuid) = java_uuid_from_string(&s) {
            self.sel.uuid = Some(uuid);
            self.sel.includes_entities = true;
        } else if s.is_empty() || s.encode_utf16().count() > 16 {
            return Err(self.rollback(start, CommandError::invalid_name_or_uuid()));
        } else {
            self.sel.includes_entities = false;
            self.sel.player_name = Some(s);
        }
        self.sel.max_results = 1;
        Ok(())
    }

    fn parse_options(&mut self) -> Result<()> {
        self.hint = Hint::Key;
        self.reader.skip_whitespace();
        while self.reader.can_read() && self.reader.peek() != ']' {
            self.reader.skip_whitespace();
            let start = self.reader.cursor();
            let name = self.reader.read_string()?;
            self.check_option(&name, start)?;
            self.reader.skip_whitespace();
            if !self.reader.can_read() || self.reader.peek() != '=' {
                return Err(self.rollback(start, CommandError::expected_option_value(&name)));
            }
            self.reader.skip();
            self.reader.skip_whitespace();
            self.hint = Hint::Nothing;
            self.handle_option(&name)?;
            self.reader.skip_whitespace();
            self.hint = Hint::NextOrClose;
            if self.reader.can_read() {
                match self.reader.peek() {
                    ',' => {
                        self.reader.skip();
                        self.hint = Hint::Key;
                    }
                    ']' => break,
                    _ => return Err(CommandError::expected_end_of_options().at(self.reader)),
                }
            }
        }
        if !self.reader.can_read() {
            return Err(CommandError::expected_end_of_options().at(self.reader));
        }
        self.reader.skip();
        Ok(())
    }

    /// `EntitySelectorOptions.get`: unknown and inapplicable options.
    fn check_option(&mut self, name: &str, start: usize) -> Result<()> {
        let s = &self.sel;
        let applicable = match name {
            "name" => self.name_opt.can_parse_any(),
            "distance" => s.distance.is_none(),
            "level" => s.level.is_none(),
            "x" => s.position[0].is_none(),
            "y" => s.position[1].is_none(),
            "z" => s.position[2].is_none(),
            "dx" => s.delta[0].is_none(),
            "dy" => s.delta[1].is_none(),
            "dz" => s.delta[2].is_none(),
            "x_rotation" => s.x_rotation.is_none(),
            "y_rotation" => s.y_rotation.is_none(),
            "limit" => !s.current_entity && !self.limited,
            "sort" => !s.current_entity && !self.sorted,
            "gamemode" => self.gamemode_opt.can_parse_any(),
            "team" => self.team_opt.can_parse_any(),
            "type" => self.type_opt.can_parse_any(),
            "tag" | "nbt" | "predicate" => true,
            "scores" => !self.scores,
            "advancements" => !self.advancements,
            _ => return Err(self.rollback(start, CommandError::unknown_option(name))),
        };
        if applicable { Ok(()) } else { Err(self.rollback(start, CommandError::inapplicable_option(name))) }
    }

    fn should_invert(&mut self) -> bool {
        self.prefix('!')
    }

    fn is_tag(&mut self) -> bool {
        self.prefix('#')
    }

    fn prefix(&mut self, c: char) -> bool {
        self.reader.skip_whitespace();
        let found = self.reader.can_read() && self.reader.peek() == c;
        if found {
            self.reader.skip();
            self.reader.skip_whitespace();
        }
        found
    }

    fn handle_option(&mut self, name: &str) -> Result<()> {
        let start = self.reader.cursor();
        match name {
            "name" => {
                let invert = self.should_invert();
                let name = self.reader.read_string()?;
                if !self.name_opt.can_parse_element(invert) {
                    return Err(self.rollback(start, CommandError::inapplicable_option("name")));
                }
                self.name_opt.mark_element(invert);
                self.sel.filters.push(Filter::Name { name, invert });
            }
            "distance" => {
                let range = DoubleRange::parse(self.reader)?;
                if range.min.is_some_and(|v| v < 0.0) || range.max.is_some_and(|v| v < 0.0) {
                    return Err(self.rollback(start, CommandError::distance_negative()));
                }
                self.sel.distance = Some(range);
                self.sel.world_limited = true;
            }
            "level" => {
                let range = IntRange::parse(self.reader)?;
                if range.min.is_some_and(|v| v < 0) || range.max.is_some_and(|v| v < 0) {
                    return Err(self.rollback(start, CommandError::level_negative()));
                }
                self.sel.level = Some(range);
                self.sel.includes_entities = false;
            }
            "x" | "y" | "z" | "dx" | "dy" | "dz" => {
                self.sel.world_limited = true;
                let v = self.reader.read_double()?;
                let axis = (name.as_bytes()[name.len() - 1] - b'x') as usize;
                let slot = if name.len() == 1 { &mut self.sel.position } else { &mut self.sel.delta };
                slot[axis] = Some(v);
            }
            "x_rotation" => self.sel.x_rotation = Some(FloatRange::parse(self.reader)?),
            "y_rotation" => self.sel.y_rotation = Some(FloatRange::parse(self.reader)?),
            "limit" => {
                let n = self.reader.read_int()?;
                if n < 1 {
                    return Err(self.rollback(start, CommandError::limit_too_small()));
                }
                self.sel.max_results = n as usize;
                self.limited = true;
            }
            "sort" => {
                let s = self.reader.read_unquoted_string();
                self.hint = Hint::Sort;
                self.sel.order = match s {
                    "nearest" => Order::Nearest,
                    "furthest" => Order::Furthest,
                    "random" => Order::Random,
                    "arbitrary" => Order::Arbitrary,
                    _ => return Err(self.rollback(start, CommandError::unknown_sort(s))),
                };
                self.sorted = true;
            }
            "gamemode" => {
                self.hint = Hint::GameMode;
                let invert = self.should_invert();
                if !self.gamemode_opt.can_parse_element(invert) {
                    return Err(self.rollback(start, CommandError::inapplicable_option("gamemode")));
                }
                let s = self.reader.read_unquoted_string();
                let Some(mode) = GameMode::by_name(s) else {
                    return Err(self.rollback(start, CommandError::invalid_selector_game_mode(s)));
                };
                self.sel.includes_entities = false;
                self.sel.filters.push(Filter::GameMode { mode, invert });
                self.gamemode_opt.mark_element(invert);
            }
            "team" => {
                let invert = self.should_invert();
                let team = self.reader.read_unquoted_string().to_owned();
                if !self.team_opt.can_parse_element(invert) {
                    return Err(self.rollback(start, CommandError::inapplicable_option("team")));
                }
                self.sel.filters.push(Filter::Team { team, invert });
                self.team_opt.mark_element(invert);
            }
            "type" => {
                self.hint = Hint::Type;
                let invert = self.should_invert();
                if self.is_tag() {
                    if !self.type_opt.can_parse_any() {
                        return Err(self.rollback(start, CommandError::inapplicable_option("type")));
                    }
                    let tag = Identifier::read(self.reader)?.to_string();
                    if !self.type_opt.can_parse_tag(&tag) {
                        return Err(self.rollback(start, CommandError::inapplicable_option("type")));
                    }
                    self.sel.filters.push(Filter::TypeTag { tag: tag.clone(), invert });
                    self.type_opt.mark_tag(tag);
                } else {
                    if !self.type_opt.can_parse_element(invert) {
                        return Err(self.rollback(start, CommandError::inapplicable_option("type")));
                    }
                    let id = Identifier::read(self.reader)?.to_string();
                    if !types::entity_type_exists(&id) {
                        return Err(self.rollback(start, CommandError::invalid_entity_type(&id)));
                    }
                    if id == "minecraft:player" && !invert {
                        self.sel.includes_entities = false;
                    }
                    if !invert {
                        self.sel.entity_type = Some(id.clone());
                    }
                    self.sel.filters.push(Filter::Type { id, invert });
                    self.type_opt.mark_element(invert);
                }
            }
            "tag" => {
                let invert = self.should_invert();
                let tag = self.reader.read_unquoted_string().to_owned();
                self.sel.filters.push(Filter::Tag { tag, invert });
            }
            "nbt" => {
                let invert = self.should_invert();
                let snbt = snbt::read_compound(self.reader)?.trim_start().to_owned();
                self.sel.filters.push(Filter::Nbt { snbt, invert });
            }
            "scores" => {
                let mut scores = Vec::new();
                self.reader.expect('{')?;
                self.for_each_entry(|p| {
                    let objective = p.reader.read_unquoted_string().to_owned();
                    p.reader.skip_whitespace();
                    p.reader.expect('=')?;
                    p.reader.skip_whitespace();
                    scores.push((objective, IntRange::parse(p.reader)?));
                    Ok(())
                })?;
                if !scores.is_empty() {
                    self.sel.filters.push(Filter::Scores(scores));
                }
                self.scores = true;
            }
            "advancements" => {
                let mut checks = Vec::new();
                self.reader.expect('{')?;
                self.for_each_entry(|p| {
                    let id = Identifier::read(p.reader)?.to_string();
                    p.reader.skip_whitespace();
                    p.reader.expect('=')?;
                    p.reader.skip_whitespace();
                    if p.reader.can_read() && p.reader.peek() == '{' {
                        let mut criteria = Vec::new();
                        p.reader.skip_whitespace();
                        p.reader.expect('{')?;
                        p.for_each_entry(|p| {
                            let name = p.reader.read_unquoted_string().to_owned();
                            p.reader.skip_whitespace();
                            p.reader.expect('=')?;
                            p.reader.skip_whitespace();
                            criteria.push((name, p.reader.read_boolean()?));
                            Ok(())
                        })?;
                        p.reader.skip_whitespace();
                        checks.push((id, AdvancementCheck::Criteria(criteria)));
                    } else {
                        checks.push((id, AdvancementCheck::Done(p.reader.read_boolean()?)));
                    }
                    Ok(())
                })?;
                if !checks.is_empty() {
                    self.sel.filters.push(Filter::Advancements(checks));
                }
                self.sel.includes_entities = false;
                self.advancements = true;
            }
            "predicate" => {
                let invert = self.should_invert();
                let id = Identifier::read(self.reader)?.to_string();
                self.sel.filters.push(Filter::Predicate { id, invert });
            }
            _ => unreachable!("checked by check_option"),
        }
        Ok(())
    }

    /// `{entry, entry, ...}` after the opening brace, through the closing brace.
    fn for_each_entry(&mut self, mut entry: impl FnMut(&mut Self) -> Result<()>) -> Result<()> {
        loop {
            self.reader.skip_whitespace();
            if !self.reader.can_read() || self.reader.peek() == '}' {
                break;
            }
            entry(self)?;
            self.reader.skip_whitespace();
            if self.reader.can_read() && self.reader.peek() == ',' {
                self.reader.skip();
            }
        }
        self.reader.expect('}')
    }
}

/// `UUID.fromString`: five dash-separated hex groups, at most 36 characters.
pub fn java_uuid_from_string(s: &str) -> Option<Uuid> {
    if s.len() > 36 {
        return None;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let hex = |p: &str| -> Option<i64> {
        let digits = p.strip_prefix('+').unwrap_or(p);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        i64::from_str_radix(digits, 16).ok()
    };
    let (a, b, c, d, e) = (hex(parts[0])?, hex(parts[1])?, hex(parts[2])?, hex(parts[3])?, hex(parts[4])?);
    let most = ((a & 0xffff_ffff) << 32) | ((b & 0xffff) << 16) | (c & 0xffff);
    let least = ((d & 0xffff) << 48) | (e & 0xffff_ffff_ffff);
    Some(Uuid::from_u64_pair(most as u64, least as u64))
}

fn dist_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

fn rotation_matches(range: &FloatRange, value: f32) -> bool {
    let min = wrap_degrees(range.min.unwrap_or(0.0));
    let max = wrap_degrees(range.max.unwrap_or(359.0));
    let f = wrap_degrees(value);
    if min > max { f >= min || f <= max } else { f >= min && f <= max }
}

impl Filter {
    fn test<E: SelectorTarget>(&self, e: &E, scoreboard: Option<&Scoreboard>) -> bool {
        match self {
            Filter::Alive => e.is_alive(),
            Filter::Name { name, invert } => (e.name() == *name) != *invert,
            Filter::Type { id, invert } => (e.entity_type() == id) != *invert,
            Filter::TypeTag { tag, invert } => types::entity_type_in_tag(e.entity_type(), tag) != *invert,
            Filter::Tag { tag, invert } if tag.is_empty() => e.tags().is_empty() != *invert,
            Filter::Tag { tag, invert } => e.tags().contains(tag) != *invert,
            Filter::Team { team, invert } => (e.team().unwrap_or("") == team) != *invert,
            Filter::GameMode { mode, invert } => e.game_mode().is_some_and(|m| (m == *mode) != *invert),
            // (`nbt=` reads the entity's saved data, which the world has: [`EntitySelector::nbt_ok`].)
            Filter::Nbt { .. } => true,
            Filter::Scores(scores) => scores.iter().all(|(obj, range)| {
                let score = match scoreboard {
                    Some(sb) => sb.objective(obj).and(sb.score(&e.scoreboard_name(), obj)),
                    None => e.score(obj),
                };
                score.is_some_and(|v| range.matches(v))
            }),
            // (`advancements=` and `predicate=` read the player's progress and the loot predicates,
            // which the world has: [`EntitySelector::world_ok`].)
            Filter::Advancements(_) | Filter::Predicate { .. } => true,
        }
    }
}

impl EntitySelector {
    fn check_permissions<W: Source>(&self, world: &W) -> Result<()> {
        if self.uses_selector && world.permission() < SELECTOR_PERMISSION {
            return Err(CommandError::selectors_not_allowed());
        }
        Ok(())
    }

    fn resolve_position(&self, origin: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|a| self.position[a].unwrap_or(origin[a]))
    }

    /// The search box, relative to the position (`getSelector`'s `aabb`).
    fn relative_aabb(&self) -> Option<Aabb> {
        if self.delta.iter().any(Option::is_some) {
            let d = self.delta.map(|v| v.unwrap_or(0.0));
            Some(Aabb {
                min: d.map(|v| if v < 0.0 { v } else { 0.0 }),
                max: d.map(|v| if v < 0.0 { 0.0 } else { v } + 1.0),
            })
        } else {
            let max = self.distance.and_then(|d| d.max)?;
            Some(Aabb { min: [-max; 3], max: [max + 1.0; 3] })
        }
    }

    fn matches<E: SelectorTarget>(&self, e: &E, pos: [f64; 3], aabb: Option<&Aabb>, sb: Option<&Scoreboard>) -> bool {
        self.filters.iter().all(|f| f.test(e, sb))
            && self.x_rotation.is_none_or(|r| rotation_matches(&r, e.rotation()[1]))
            && self.y_rotation.is_none_or(|r| rotation_matches(&r, e.rotation()[0]))
            && self.level.is_none_or(|r| e.is_player() && e.experience_level().is_some_and(|l| r.matches(l)))
            && aabb.is_none_or(|b| b.intersects(&e.bounding_box()))
            && self.distance.is_none_or(|r| r.matches_sqr(dist_sqr(e.position(), pos)))
    }

    /// The filters the world answers: `nbt=` (`NbtPredicate.matches(entity)`), `advancements=`
    /// (`PlayerAdvancements` progress of a player) and `predicate=` (a loot predicate with the
    /// entity as `this_entity`).
    fn nbt_ok<W: SelectorWorld>(&self, world: &W, e: &W::Entity) -> bool {
        self.filters.iter().all(|f| match f {
            Filter::Nbt { snbt, invert } => world.entity_nbt_matches(e, snbt) != *invert,
            Filter::Advancements(checks) => {
                e.is_player()
                    && checks.iter().all(|(id, check)| match check {
                        AdvancementCheck::Done(want) => world.entity_advancement_done(e, id) == Some(*want),
                        AdvancementCheck::Criteria(cs) => {
                            world.entity_advancement_done(e, id).is_some()
                                && cs.iter().all(|(c, want)| world.entity_criterion_done(e, id, c) == Some(*want))
                        }
                    })
            }
            Filter::Predicate { id, invert } => world.entity_predicate(e, id).is_some_and(|v| v != *invert),
            _ => true,
        })
    }

    fn result_limit(&self) -> usize {
        if self.order == Order::Arbitrary { self.max_results } else { usize::MAX }
    }

    fn sort_and_limit<W: SelectorWorld>(
        &self,
        world: &mut W,
        pos: [f64; 3],
        mut list: Vec<W::Entity>,
    ) -> Vec<W::Entity> {
        if list.len() > 1 {
            match self.order {
                Order::Arbitrary => {}
                Order::Nearest => {
                    list.sort_by(|a, b| dist_sqr(a.position(), pos).total_cmp(&dist_sqr(b.position(), pos)))
                }
                Order::Furthest => {
                    list.sort_by(|a, b| dist_sqr(b.position(), pos).total_cmp(&dist_sqr(a.position(), pos)))
                }
                Order::Random => world.shuffle(&mut list),
            }
        }
        list.truncate(self.max_results);
        list
    }

    /// `findEntities`: every match, possibly empty.
    pub fn find_entities<W: SelectorWorld>(&self, world: &mut W) -> Result<Vec<W::Entity>> {
        self.check_permissions(world)?;
        if !self.includes_entities {
            return self.find_players(world);
        }
        if let Some(name) = &self.player_name {
            return Ok(world.players().into_iter().filter(|p| p.name().eq_ignore_ascii_case(name)).take(1).collect());
        }
        if let Some(uuid) = self.uuid {
            return Ok(world.entity_by_uuid(uuid).into_iter().collect());
        }
        let pos = self.resolve_position(world.origin());
        let aabb = self.relative_aabb().map(|b| b.offset(pos));
        let sb = world.scoreboard();
        if self.current_entity {
            return Ok(world.source_entity().filter(|e| self.matches(e, pos, aabb.as_ref(), sb) && self.nbt_ok(&*world, e)).into_iter().collect());
        }
        let dimension = self.world_limited.then(|| world.dimension().to_owned());
        let limit = self.result_limit();
        let mut list = Vec::new();
        for e in world.entities(dimension.as_deref(), aabb.as_ref()) {
            if self.entity_type.as_ref().is_some_and(|t| t != e.entity_type())
                || !self.matches(&e, pos, aabb.as_ref(), world.scoreboard())
                || !self.nbt_ok(&*world, &e)
            {
                continue;
            }
            list.push(e);
            if list.len() >= limit {
                break;
            }
        }
        Ok(self.sort_and_limit(world, pos, list))
    }

    /// `findPlayers`: every matching player, possibly empty.
    pub fn find_players<W: SelectorWorld>(&self, world: &mut W) -> Result<Vec<W::Entity>> {
        self.check_permissions(world)?;
        if let Some(name) = &self.player_name {
            return Ok(world.players().into_iter().filter(|p| p.name().eq_ignore_ascii_case(name)).take(1).collect());
        }
        if let Some(uuid) = self.uuid {
            return Ok(world.players().into_iter().filter(|p| p.uuid() == uuid).take(1).collect());
        }
        let pos = self.resolve_position(world.origin());
        let aabb = self.relative_aabb().map(|b| b.offset(pos));
        if self.current_entity {
            let sb = world.scoreboard();
            return Ok(world
                .source_entity()
                .filter(|e| e.is_player() && self.matches(e, pos, aabb.as_ref(), sb) && self.nbt_ok(&*world, e))
                .into_iter()
                .collect());
        }
        let limit = self.result_limit();
        let dimension = world.dimension().to_owned();
        let mut list = Vec::new();
        for p in world.players() {
            if (self.world_limited && p.dimension() != dimension)
                || !self.matches(&p, pos, aabb.as_ref(), world.scoreboard())
                || !self.nbt_ok(&*world, &p)
            {
                continue;
            }
            list.push(p);
            if list.len() >= limit {
                break;
            }
        }
        Ok(self.sort_and_limit(world, pos, list))
    }

    /// `EntityArgument.getEntities`: at least one entity.
    pub fn entities<W: SelectorWorld>(&self, world: &mut W) -> Result<Vec<W::Entity>> {
        let list = self.find_entities(world)?;
        if list.is_empty() { Err(CommandError::no_entities_found()) } else { Ok(list) }
    }

    /// `EntityArgument.getPlayers`: at least one player.
    pub fn players<W: SelectorWorld>(&self, world: &mut W) -> Result<Vec<W::Entity>> {
        let list = self.find_players(world)?;
        if list.is_empty() { Err(CommandError::no_players_found()) } else { Ok(list) }
    }

    /// `EntityArgument.getEntity` (`findSingleEntity`).
    pub fn entity<W: SelectorWorld>(&self, world: &mut W) -> Result<W::Entity> {
        let mut list = self.find_entities(world)?;
        match list.len() {
            0 => Err(CommandError::no_entities_found()),
            1 => Ok(list.pop().unwrap()),
            _ => Err(CommandError::not_single_entity()),
        }
    }

    /// `EntityArgument.getPlayer` (`findSinglePlayer`).
    pub fn player<W: SelectorWorld>(&self, world: &mut W) -> Result<W::Entity> {
        let mut list = self.find_players(world)?;
        if list.len() != 1 {
            return Err(CommandError::no_players_found());
        }
        Ok(list.pop().unwrap())
    }
}

/// `EntitySelector.joinNames`.
pub fn join_names<E: SelectorTarget>(entities: &[E]) -> Text {
    Text::join(entities.iter().map(SelectorTarget::display_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<EntitySelector> {
        EntitySelector::parse(&mut StringReader::new(s), true)
    }

    fn err(s: &str) -> (String, usize) {
        let e = parse(s).unwrap_err();
        (e.key().unwrap().to_owned(), e.cursor().unwrap())
    }

    #[test]
    fn selector_types() {
        let p = parse("@p").unwrap();
        assert_eq!((p.max_results, p.includes_entities, p.order), (1, false, Order::Nearest));
        let a = parse("@a").unwrap();
        assert_eq!((a.max_results, a.includes_entities), (usize::MAX, false));
        assert_eq!(parse("@r").unwrap().order, Order::Random);
        let s = parse("@s").unwrap();
        assert!(s.current_entity && s.includes_entities && s.is_self_selector());
        let e = parse("@e").unwrap();
        assert_eq!((e.max_results, e.filters.as_slice()), (usize::MAX, &[Filter::Alive][..]));
        let n = parse("@n").unwrap();
        assert_eq!((n.max_results, n.order, n.includes_entities), (1, Order::Nearest, true));
        assert_eq!(err("@x"), ("argument.entity.selector.unknown".into(), 1));
        assert_eq!(err("@"), ("argument.entity.selector.missing".into(), 1));
        let e = EntitySelector::parse(&mut StringReader::new("@a"), false).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("argument.entity.selector.not_allowed"), Some(0)));
    }

    #[test]
    fn names_and_uuids() {
        let n = parse("Notch").unwrap();
        assert_eq!((n.player_name.as_deref(), n.includes_entities, n.max_results), (Some("Notch"), false, 1));
        let u = parse("f81d4fae-7dec-11d0-a765-00a0c91e6bf6").unwrap();
        assert_eq!(u.uuid, Some(Uuid::parse_str("f81d4fae-7dec-11d0-a765-00a0c91e6bf6").unwrap()));
        assert!(u.includes_entities);
        assert_eq!(java_uuid_from_string("0-0-0-0-1"), Some(Uuid::from_u64_pair(0, 1)));
        assert_eq!(java_uuid_from_string("00000000000000000000000000000001"), None);
        assert_eq!(err("abcdefghijklmnopq"), ("argument.entity.invalid".into(), 0));
        assert_eq!(err(""), ("argument.entity.invalid".into(), 0));
    }

    #[test]
    fn options() {
        let s = parse("@e[type=!minecraft:cow, x = 1, y=2,z=3.5,dx=-2,distance=..5.5,limit=3,sort=furthest,tag=a,tag=!b,name=\"A b\",team=!red,gamemode=creative,level=1..,x_rotation=-90..90,y_rotation=170..-170,scores={o=1..,p=..-1},advancements={minecraft:story/root=true,a/b={c=false}},nbt={Tags:[\"x\"]},predicate=!kiln:p]").unwrap();
        assert_eq!(s.position, [Some(1.0), Some(2.0), Some(3.5)]);
        assert_eq!(s.delta, [Some(-2.0), None, None]);
        assert_eq!((s.max_results, s.order, s.world_limited, s.includes_entities), (3, Order::Furthest, true, false));
        assert_eq!(s.distance.unwrap().max, Some(5.5));
        assert_eq!(s.y_rotation.unwrap(), FloatRange { min: Some(170.0), max: Some(-170.0) });
        assert!(s.filters.contains(&Filter::Type { id: "minecraft:cow".into(), invert: true }));
        assert!(s.filters.contains(&Filter::Name { name: "A b".into(), invert: false }));
        assert!(s.filters.contains(&Filter::GameMode { mode: GameMode::Creative, invert: false }));
        assert!(s.filters.contains(&Filter::Nbt { snbt: "{Tags:[\"x\"]}".into(), invert: false }));
        assert!(s.filters.contains(&Filter::Predicate { id: "kiln:p".into(), invert: true }));
        assert!(s.filters.contains(&Filter::Advancements(vec![
            ("minecraft:story/root".into(), AdvancementCheck::Done(true)),
            ("minecraft:a/b".into(), AdvancementCheck::Criteria(vec![("c".into(), false)])),
        ])));
        assert!(s.entity_type.is_none());
        let t = parse("@e[type=#minecraft:skeletons,type=!#minecraft:raiders]").unwrap();
        assert_eq!(t.filters.len(), 3);
        assert!(!parse("@a[type=minecraft:player]").unwrap().includes_entities);
        assert!(parse("@e[type=minecraft:zombie]").unwrap().includes_entities);
    }

    #[test]
    fn option_errors() {
        assert_eq!(err("@e[foo=1]"), ("argument.entity.options.unknown".into(), 3));
        assert_eq!(err("@e[limit=1,limit=2]"), ("argument.entity.options.inapplicable".into(), 11));
        assert_eq!(err("@s[limit=1]"), ("argument.entity.options.inapplicable".into(), 3));
        assert_eq!(err("@s[sort=nearest]"), ("argument.entity.options.inapplicable".into(), 3));
        assert_eq!(err("@e[name=a,name=b]"), ("argument.entity.options.inapplicable".into(), 10));
        assert_eq!(err("@e[name=!a,name=b]"), ("argument.entity.options.inapplicable".into(), 16));
        assert!(parse("@e[name=!a,name=!b]").is_ok());
        assert_eq!(err("@e[type=cow,type=pig]"), ("argument.entity.options.inapplicable".into(), 12));
        assert_eq!(err("@e[type=#skeletons,type=#skeletons]"), ("argument.entity.options.inapplicable".into(), 24));
        assert_eq!(err("@e[type=minecraft:stone]"), ("argument.entity.options.type.invalid".into(), 8));
        assert_eq!(err("@e[limit=0]"), ("argument.entity.options.limit.toosmall".into(), 9));
        assert_eq!(err("@e[sort=up]"), ("argument.entity.options.sort.irreversible".into(), 8));
        assert_eq!(err("@e[gamemode=god]"), ("argument.entity.options.mode.invalid".into(), 12));
        assert_eq!(err("@e[distance=-1]"), ("argument.entity.options.distance.negative".into(), 12));
        assert_eq!(err("@e[level=..-1]"), ("argument.entity.options.level.negative".into(), 9));
        assert_eq!(err("@e[distance=5..1]"), ("argument.range.swapped".into(), 12));
        assert_eq!(err("@e[limit]"), ("argument.entity.options.valueless".into(), 3));
        assert_eq!(err("@e[limit=1 x=1]"), ("argument.entity.options.unterminated".into(), 11));
        assert_eq!(err("@e[limit=1"), ("argument.entity.options.unterminated".into(), 10));
        assert_eq!(err("@e[x=a]"), ("parsing.double.expected".into(), 5));
        let e = parse("@e[scores={o=1").unwrap_err();
        assert_eq!(e.key(), Some("parsing.expected"));
    }

    #[derive(Clone, Debug)]
    struct Ent {
        name: &'static str,
        kind: &'static str,
        pos: [f64; 3],
        dim: &'static str,
        mode: Option<GameMode>,
        tags: Vec<String>,
    }

    impl SelectorTarget for Ent {
        fn uuid(&self) -> Uuid {
            Uuid::from_u64_pair(0, self.name.len() as u64)
        }
        fn name(&self) -> String {
            self.name.into()
        }
        fn entity_type(&self) -> &str {
            self.kind
        }
        fn position(&self) -> [f64; 3] {
            self.pos
        }
        fn rotation(&self) -> [f32; 2] {
            [0.0, 0.0]
        }
        fn dimension(&self) -> &str {
            self.dim
        }
        fn bounding_box(&self) -> Aabb {
            let [x, y, z] = self.pos;
            Aabb { min: [x - 0.3, y, z - 0.3], max: [x + 0.3, y + 1.8, z + 0.3] }
        }
        fn game_mode(&self) -> Option<GameMode> {
            self.mode
        }
        fn tags(&self) -> &[String] {
            &self.tags
        }
    }

    struct World {
        ents: Vec<Ent>,
        level: u8,
        stack: crate::host::SourceStack<World>,
    }

    impl Source for World {
        type Entity = Ent;
        fn permission_level(&self) -> u8 {
            self.level
        }
        fn stack(&self) -> &crate::host::SourceStack<World> {
            &self.stack
        }
        fn stack_mut(&mut self) -> &mut crate::host::SourceStack<World> {
            &mut self.stack
        }
    }

    impl SelectorWorld for World {
        fn players(&self) -> Vec<Ent> {
            self.ents.iter().filter(|e| e.is_player()).cloned().collect()
        }
        fn entities(&self, dim: Option<&str>, _: Option<&Aabb>) -> Vec<Ent> {
            self.ents.iter().filter(|e| dim.is_none_or(|d| d == e.dim)).cloned().collect()
        }
        fn shuffle(&mut self, list: &mut [Ent]) {
            list.reverse();
        }
    }

    fn world() -> World {
        let p = |name, pos, dim, mode| Ent { name, kind: "minecraft:player", pos, dim, mode: Some(mode), tags: vec![] };
        let alice = p("Alice", [1.0, 0.0, 0.0], "minecraft:overworld", GameMode::Creative);
        let stack = crate::host::SourceStack::new(Text::literal("Alice"), "minecraft:overworld", [0.0; 3]).with_entity(alice);
        World {
            stack,
            ents: vec![
                p("Alice", [1.0, 0.0, 0.0], "minecraft:overworld", GameMode::Creative),
                p("Bob", [10.0, 0.0, 0.0], "minecraft:overworld", GameMode::Survival),
                p("Carol", [3.0, 0.0, 0.0], "minecraft:the_nether", GameMode::Survival),
                Ent {
                    name: "Zed",
                    kind: "minecraft:zombie",
                    pos: [2.0, 0.0, 0.0],
                    dim: "minecraft:overworld",
                    mode: None,
                    tags: vec!["t".into()],
                },
                Ent {
                    name: "Sk",
                    kind: "minecraft:skeleton",
                    pos: [20.0, 0.0, 0.0],
                    dim: "minecraft:overworld",
                    mode: None,
                    tags: vec![],
                },
            ],
            level: 4,
        }
    }

    fn names(sel: &str, w: &mut World) -> Vec<&'static str> {
        parse(sel).unwrap().find_entities(w).unwrap().iter().map(|e| e.name).collect()
    }

    #[test]
    fn selection() {
        let w = &mut world();
        assert_eq!(names("@p", w), ["Alice"]);
        assert_eq!(names("@a", w), ["Alice", "Bob", "Carol"]);
        assert_eq!(names("@a[sort=furthest,limit=2]", w), ["Bob", "Carol"]);
        assert_eq!(names("@a[distance=..5]", w), ["Alice"]);
        assert_eq!(names("@e[type=!minecraft:player]", w), ["Zed", "Sk"]);
        assert_eq!(names("@e[type=#minecraft:skeletons]", w), ["Sk"]);
        assert_eq!(names("@n[type=!minecraft:player]", w), ["Zed"]);
        assert_eq!(names("@e[tag=t]", w), ["Zed"]);
        assert_eq!(names("@e[tag=]", w).len(), 4);
        assert_eq!(names("@a[gamemode=!creative]", w), ["Bob", "Carol"]);
        assert_eq!(names("@e[gamemode=!creative]", w), ["Bob", "Carol"]);
        assert_eq!(names("@e[x=9,dx=2,dy=1,dz=0]", w), ["Bob"]);
        assert_eq!(names("@s", w), ["Alice"]);
        assert_eq!(names("@r", w), ["Carol"]);
        assert_eq!(names("bob", w), ["Bob"]);
        assert_eq!(names("@e[name=Zed]", w), ["Zed"]);
        let sel = parse("@e[type=minecraft:cow]").unwrap();
        assert_eq!(sel.entities(w).unwrap_err().key(), Some("argument.entity.notfound.entity"));
        assert_eq!(parse("@a").unwrap().entity(w).unwrap_err().key(), Some("argument.entity.toomany"));
        assert_eq!(parse("Nobody").unwrap().player(w).unwrap_err().key(), Some("argument.entity.notfound.player"));
        w.level = 0;
        assert_eq!(
            parse("@a").unwrap().find_entities(w).unwrap_err().key(),
            Some("argument.entity.selector.not_allowed")
        );
    }
}
