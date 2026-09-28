//! Argument types: parsing with vanilla semantics, server-side suggestions and the parser
//! id and properties sent in the commands packet.

use crate::blocks::{self, BlockInput, BlockPredicate};
use crate::component::{self, Component};
use crate::coords::Coordinates;
use crate::error::CommandError;
use crate::host::Source;
use crate::nbt_path::NbtPath;
use crate::range::{DoubleRange, IntRange};
use crate::reader::StringReader;
use crate::selector::{self, EntitySelector, SELECTOR_PERMISSION, SelectorTarget, SelectorWorld};
use crate::snbt;
use crate::suggestion::SuggestionsBuilder;
use crate::text::Text;
use crate::tr;
use crate::types::{self, Anchor, GameMode, Heightmap, Identifier, ItemInput};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::commands::{Parser, StringKind};

type Result<T> = std::result::Result<T, CommandError>;

#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentType {
    /// `brigadier:bool`
    Bool,
    /// `brigadier:float`
    Float { min: f32, max: f32 },
    /// `brigadier:double`
    Double { min: f64, max: f64 },
    /// `brigadier:integer`
    Integer { min: i32, max: i32 },
    /// `brigadier:long`
    Long { min: i64, max: i64 },
    /// `brigadier:string`
    String(StringKind),
    /// `minecraft:entity`
    Entity { single: bool, players_only: bool },
    /// `minecraft:game_profile`
    GameProfile,
    /// `minecraft:block_pos`
    BlockPos,
    /// `minecraft:column_pos`
    ColumnPos,
    /// `minecraft:vec3`; `center_correct` adds 0.5 to integer x and z.
    Vec3 { center_correct: bool },
    /// `minecraft:vec2`
    Vec2 { center_correct: bool },
    /// `minecraft:rotation`
    Rotation,
    /// `minecraft:item_stack`
    ItemStack,
    /// `minecraft:message`: the rest of the input, with selectors resolved at execution.
    Message,
    /// `minecraft:resource_location`
    ResourceLocation,
    /// `minecraft:entity_anchor`
    EntityAnchor,
    /// `minecraft:dimension`
    Dimension,
    /// `minecraft:gamemode`
    GameMode,
    /// `minecraft:time`: a duration in ticks, with unit suffix `d`, `s` or `t`.
    Time { min: i32 },
    /// `minecraft:resource`: an entry of `registry`.
    Resource { registry: &'static str },
    /// `minecraft:resource_key`: an id of `registry`, looked up when the command runs (data
    /// pack registries: advancements, recipes).
    ResourceKey { registry: &'static str },
    /// `minecraft:block_state`
    BlockState,
    /// `minecraft:block_predicate`
    BlockPredicate,
    /// `minecraft:swizzle`: a set of axes such as `xz`.
    Swizzle,
    /// `minecraft:heightmap`
    Heightmap,
    /// `minecraft:score_holder`
    ScoreHolder { multiple: bool },
    /// `minecraft:objective`: an objective name (looked up when used).
    Objective,
    /// `minecraft:objective_criteria`
    ObjectiveCriteria,
    /// `minecraft:scoreboard_slot`
    ScoreboardSlot,
    /// `minecraft:operation`: `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `<`, `>`, `><`.
    Operation,
    /// `minecraft:int_range`
    IntRange,
    /// `minecraft:float_range`
    FloatRange,
    /// `minecraft:nbt_path`
    NbtPath,
    /// `minecraft:resource_or_tag`: an entry or `#tag` of `registry`.
    ResourceOrTag { registry: &'static str },
    /// `minecraft:resource_or_tag_key`: an id or `#tag` of `registry`, looked up when used.
    ResourceOrTagKey { registry: &'static str },
    /// `minecraft:feature` (`ResourceOrIdArgument`): a configured feature id or an inline one.
    Feature,
    /// `minecraft:template_rotation`: `none`, `clockwise_90`, `180`, `counterclockwise_90`.
    TemplateRotation,
    /// `minecraft:template_mirror`: `none`, `left_right`, `front_back`.
    TemplateMirror,
    /// `minecraft:function`: a function id or `#tag`.
    Function,
    /// `minecraft:item_predicate`
    ItemPredicate,
    /// `minecraft:slot_source`
    SlotSource,
    /// `minecraft:loot_predicate`: an id or an inline predicate.
    LootPredicate,
    /// `minecraft:component`: a text component.
    Component,
    /// `minecraft:style`: text style fields.
    Style,
    /// `minecraft:team`: a team name (looked up when used).
    Team,
    /// `minecraft:nbt_compound_tag`: an SNBT compound.
    NbtCompound,
    /// `minecraft:team_color`: one of the sixteen [`TEAM_COLORS`].
    TeamColor,
    /// `minecraft:dialog`: a `minecraft:dialog` registry id ([`ArgumentValue::Identifier`]) or
    /// an inline definition ([`ArgumentValue::Nbt`]).
    Dialog,
    /// `minecraft:particle`: a particle type id and its options as SNBT (checked when the
    /// command runs).
    Particle,
    /// `minecraft:hex_color`: `RGB` or `RRGGBB`, as an [`ArgumentValue::Integer`] `0xRRGGBB`.
    HexColor,
    /// `minecraft:nbt_tag`: any SNBT value.
    NbtTag,
    /// `minecraft:context_int_provider` / `minecraft:context_float_provider`: an inline number
    /// provider ([`ArgumentValue::Nbt`]); ids name data-driven providers, which Kiln has none of.
    ContextProvider { float: bool },
    /// `minecraft:item_slot`: a slot name such as `container.5` or `weapon.mainhand`
    /// ([`ArgumentValue::Integer`]).
    ItemSlot,
    /// `minecraft:item_slots`: a slot range such as `container.*` ([`ArgumentValue::String`]
    /// with the name; see [`item_slots`]).
    ItemSlots,
    /// `minecraft:loot_table`, `minecraft:loot_modifier`: an id or an inline definition
    /// ([`ArgumentValue::Identifier`] or [`ArgumentValue::Nbt`]).
    LootResource { registry: &'static str },
    /// `minecraft:uuid`.
    Uuid,
    /// `minecraft:swing_animation`: `whack` or `stab`... ([`ArgumentValue::String`]).
    SwingAnimation,
}

impl ArgumentType {
    pub fn integer() -> Self {
        ArgumentType::Integer { min: i32::MIN, max: i32::MAX }
    }
    pub fn integer_min(min: i32) -> Self {
        ArgumentType::Integer { min, max: i32::MAX }
    }
    pub fn integer_range(min: i32, max: i32) -> Self {
        ArgumentType::Integer { min, max }
    }
    pub fn long() -> Self {
        ArgumentType::Long { min: i64::MIN, max: i64::MAX }
    }
    pub fn float() -> Self {
        ArgumentType::Float { min: f32::MIN, max: f32::MAX }
    }
    pub fn float_range(min: f32, max: f32) -> Self {
        ArgumentType::Float { min, max }
    }
    pub fn double() -> Self {
        ArgumentType::Double { min: f64::MIN, max: f64::MAX }
    }
    pub fn word() -> Self {
        ArgumentType::String(StringKind::Word)
    }
    pub fn string() -> Self {
        ArgumentType::String(StringKind::Phrase)
    }
    pub fn greedy_string() -> Self {
        ArgumentType::String(StringKind::Greedy)
    }
    pub fn entity() -> Self {
        ArgumentType::Entity { single: true, players_only: false }
    }
    pub fn entities() -> Self {
        ArgumentType::Entity { single: false, players_only: false }
    }
    pub fn player() -> Self {
        ArgumentType::Entity { single: true, players_only: true }
    }
    pub fn players() -> Self {
        ArgumentType::Entity { single: false, players_only: true }
    }
    pub fn vec3() -> Self {
        ArgumentType::Vec3 { center_correct: true }
    }
    pub fn vec2() -> Self {
        ArgumentType::Vec2 { center_correct: true }
    }
    /// `TimeArgument.time()`: non-negative.
    pub fn time() -> Self {
        ArgumentType::Time { min: 0 }
    }
    pub fn time_min(min: i32) -> Self {
        ArgumentType::Time { min }
    }
    pub fn resource(registry: &'static str) -> Self {
        ArgumentType::Resource { registry }
    }

    /// Parser id and properties for the commands packet.
    pub fn wire(&self) -> Parser<'static> {
        match *self {
            ArgumentType::Bool => Parser::Plain("brigadier:bool"),
            ArgumentType::Float { min, max } => {
                Parser::Float { min: (min != f32::MIN).then_some(min), max: (max != f32::MAX).then_some(max) }
            }
            ArgumentType::Double { min, max } => {
                Parser::Double { min: (min != f64::MIN).then_some(min), max: (max != f64::MAX).then_some(max) }
            }
            ArgumentType::Integer { min, max } => {
                Parser::Integer { min: (min != i32::MIN).then_some(min), max: (max != i32::MAX).then_some(max) }
            }
            ArgumentType::Long { min, max } => {
                Parser::Long { min: (min != i64::MIN).then_some(min), max: (max != i64::MAX).then_some(max) }
            }
            ArgumentType::String(kind) => Parser::String(kind),
            ArgumentType::Entity { single, players_only } => Parser::Entity { single, players_only },
            ArgumentType::GameProfile => Parser::Plain("minecraft:game_profile"),
            ArgumentType::BlockPos => Parser::Plain("minecraft:block_pos"),
            ArgumentType::ColumnPos => Parser::Plain("minecraft:column_pos"),
            ArgumentType::Vec3 { .. } => Parser::Plain("minecraft:vec3"),
            ArgumentType::Vec2 { .. } => Parser::Plain("minecraft:vec2"),
            ArgumentType::Rotation => Parser::Plain("minecraft:rotation"),
            ArgumentType::ItemStack => Parser::Plain("minecraft:item_stack"),
            ArgumentType::Message => Parser::Plain("minecraft:message"),
            ArgumentType::ResourceLocation => Parser::Plain("minecraft:resource_location"),
            ArgumentType::EntityAnchor => Parser::Plain("minecraft:entity_anchor"),
            ArgumentType::Dimension => Parser::Plain("minecraft:dimension"),
            ArgumentType::GameMode => Parser::Plain("minecraft:gamemode"),
            ArgumentType::Time { min } => Parser::Time { min },
            ArgumentType::Resource { registry } => Parser::Registry { id: "minecraft:resource", registry },
            ArgumentType::ResourceKey { registry } => Parser::Registry { id: "minecraft:resource_key", registry },
            ArgumentType::BlockState => Parser::Plain("minecraft:block_state"),
            ArgumentType::BlockPredicate => Parser::Plain("minecraft:block_predicate"),
            ArgumentType::Swizzle => Parser::Plain("minecraft:swizzle"),
            ArgumentType::Heightmap => Parser::Plain("minecraft:heightmap"),
            ArgumentType::ScoreHolder { multiple } => Parser::ScoreHolder { multiple },
            ArgumentType::Objective => Parser::Plain("minecraft:objective"),
            ArgumentType::ObjectiveCriteria => Parser::Plain("minecraft:objective_criteria"),
            ArgumentType::ScoreboardSlot => Parser::Plain("minecraft:scoreboard_slot"),
            ArgumentType::Operation => Parser::Plain("minecraft:operation"),
            ArgumentType::IntRange => Parser::Plain("minecraft:int_range"),
            ArgumentType::FloatRange => Parser::Plain("minecraft:float_range"),
            ArgumentType::NbtPath => Parser::Plain("minecraft:nbt_path"),
            ArgumentType::ResourceOrTag { registry } => Parser::Registry { id: "minecraft:resource_or_tag", registry },
            ArgumentType::ResourceOrTagKey { registry } => {
                Parser::Registry { id: "minecraft:resource_or_tag_key", registry }
            }
            ArgumentType::Feature => Parser::Plain("minecraft:feature"),
            ArgumentType::TemplateRotation => Parser::Plain("minecraft:template_rotation"),
            ArgumentType::TemplateMirror => Parser::Plain("minecraft:template_mirror"),
            ArgumentType::Function => Parser::Plain("minecraft:function"),
            ArgumentType::ItemPredicate => Parser::Plain("minecraft:item_predicate"),
            ArgumentType::SlotSource => Parser::Plain("minecraft:slot_source"),
            ArgumentType::LootPredicate => Parser::Plain("minecraft:loot_predicate"),
            ArgumentType::Component => Parser::Plain("minecraft:component"),
            ArgumentType::Style => Parser::Plain("minecraft:style"),
            ArgumentType::Team => Parser::Plain("minecraft:team"),
            ArgumentType::NbtCompound => Parser::Plain("minecraft:nbt_compound_tag"),
            ArgumentType::TeamColor => Parser::Plain("minecraft:team_color"),
            ArgumentType::Dialog => Parser::Plain("minecraft:dialog"),
            ArgumentType::Particle => Parser::Plain("minecraft:particle"),
            ArgumentType::HexColor => Parser::Plain("minecraft:hex_color"),
            ArgumentType::NbtTag => Parser::Plain("minecraft:nbt_tag"),
            ArgumentType::ContextProvider { float: false } => Parser::Plain("minecraft:context_int_provider"),
            ArgumentType::ContextProvider { float: true } => Parser::Plain("minecraft:context_float_provider"),
            ArgumentType::ItemSlot => Parser::Plain("minecraft:item_slot"),
            ArgumentType::ItemSlots => Parser::Plain("minecraft:item_slots"),
            ArgumentType::LootResource { registry } => Parser::Plain(match registry {
                "minecraft:loot_table" => "minecraft:loot_table",
                _ => "minecraft:loot_modifier",
            }),
            ArgumentType::Uuid => Parser::Plain("minecraft:uuid"),
            ArgumentType::SwingAnimation => Parser::Plain("minecraft:swing_animation"),
        }
    }

    pub fn parse(&self, reader: &mut StringReader, allow_selectors: bool) -> Result<ArgumentValue> {
        let start = reader.cursor();
        let too = |reader: &mut StringReader, e: CommandError| {
            reader.set_cursor(start);
            Err(e.at(reader))
        };
        Ok(match *self {
            ArgumentType::Bool => ArgumentValue::Bool(reader.read_boolean()?),
            ArgumentType::Integer { min, max } => {
                let v = reader.read_int()?;
                if v < min {
                    return too(reader, CommandError::integer_too_low(v, min));
                }
                if v > max {
                    return too(reader, CommandError::integer_too_high(v, max));
                }
                ArgumentValue::Integer(v)
            }
            ArgumentType::Long { min, max } => {
                let v = reader.read_long()?;
                if v < min {
                    return too(reader, CommandError::long_too_low(v, min));
                }
                if v > max {
                    return too(reader, CommandError::long_too_high(v, max));
                }
                ArgumentValue::Long(v)
            }
            ArgumentType::Float { min, max } => {
                let v = reader.read_float()?;
                if v < min {
                    return too(reader, CommandError::float_too_low(v, min));
                }
                if v > max {
                    return too(reader, CommandError::float_too_high(v, max));
                }
                ArgumentValue::Float(v)
            }
            ArgumentType::Double { min, max } => {
                let v = reader.read_double()?;
                if v < min {
                    return too(reader, CommandError::double_too_low(v, min));
                }
                if v > max {
                    return too(reader, CommandError::double_too_high(v, max));
                }
                ArgumentValue::Double(v)
            }
            ArgumentType::String(StringKind::Word) => ArgumentValue::String(reader.read_unquoted_string().to_owned()),
            ArgumentType::String(StringKind::Phrase) => ArgumentValue::String(reader.read_string()?),
            ArgumentType::String(StringKind::Greedy) => {
                let rest = reader.remaining().to_owned();
                reader.set_cursor(reader.total_len());
                ArgumentValue::String(rest)
            }
            ArgumentType::Entity { single, players_only } => {
                let sel = EntitySelector::parse(reader, allow_selectors)?;
                if sel.max_results > 1 && single {
                    reader.set_cursor(0);
                    let e = if players_only {
                        CommandError::not_single_player()
                    } else {
                        CommandError::not_single_entity()
                    };
                    return Err(e.at(reader));
                }
                if sel.includes_entities && players_only && !sel.is_self_selector() {
                    reader.set_cursor(0);
                    return Err(CommandError::only_players_allowed().at(reader));
                }
                ArgumentValue::Entity(Box::new(sel))
            }
            ArgumentType::GameProfile => {
                if reader.can_read() && reader.peek() == '@' {
                    let sel = EntitySelector::parse(reader, allow_selectors)?;
                    if sel.includes_entities {
                        return Err(CommandError::only_players_allowed().at(reader));
                    }
                    ArgumentValue::GameProfile(GameProfileArg::Selector(Box::new(sel)))
                } else {
                    let s = reader.remaining().split(' ').next().unwrap_or("");
                    reader.set_cursor(start + s.len());
                    ArgumentValue::GameProfile(GameProfileArg::Name(s.to_owned()))
                }
            }
            ArgumentType::BlockPos => ArgumentValue::Coordinates(Coordinates::parse_block_pos(reader)?),
            ArgumentType::ColumnPos => ArgumentValue::Coordinates(Coordinates::parse_column_pos(reader)?),
            ArgumentType::Vec3 { center_correct } => {
                ArgumentValue::Coordinates(Coordinates::parse_vec3(reader, center_correct)?)
            }
            ArgumentType::Vec2 { center_correct } => {
                ArgumentValue::Coordinates(Coordinates::parse_vec2(reader, center_correct)?)
            }
            ArgumentType::Rotation => ArgumentValue::Coordinates(Coordinates::parse_rotation(reader)?),
            ArgumentType::ItemStack => ArgumentValue::Item(parse_item(reader)?),
            ArgumentType::Message => ArgumentValue::Message(MessageArg::parse(reader, allow_selectors)?),
            ArgumentType::ResourceLocation | ArgumentType::Dimension | ArgumentType::ResourceKey { .. } => {
                ArgumentValue::Identifier(Identifier::read(reader)?)
            }
            ArgumentType::Resource { registry } => {
                let id = Identifier::read(reader)?;
                if types::registry_entries(registry).is_some_and(|e| !e.contains(&id.as_str())) {
                    return Err(CommandError::unknown_resource(id.as_str(), registry).at(reader));
                }
                ArgumentValue::Identifier(id)
            }
            ArgumentType::EntityAnchor => {
                let s = reader.read_unquoted_string();
                match Anchor::by_name(s) {
                    Some(a) => ArgumentValue::Anchor(a),
                    None => return too(reader, CommandError::invalid_anchor(s)),
                }
            }
            ArgumentType::GameMode => {
                let s = reader.read_unquoted_string();
                match GameMode::by_name(s) {
                    Some(m) => ArgumentValue::GameMode(m),
                    // `GameModeArgument`: the error points after the word.
                    None => return Err(CommandError::invalid_game_mode(s).at(reader)),
                }
            }
            ArgumentType::Time { min } => {
                let v = reader.read_float()?;
                let unit = match reader.read_unquoted_string() {
                    "d" => 24000.0,
                    "s" => 20.0,
                    "t" | "" => 1.0,
                    _ => return Err(CommandError::invalid_time_unit().at(reader)),
                };
                let ticks = java_round(v * unit);
                if ticks < min {
                    return Err(CommandError::tick_count_too_low(ticks, min).at(reader));
                }
                ArgumentValue::Time(ticks)
            }
            ArgumentType::BlockState => ArgumentValue::BlockState(blocks::parse_block_state(reader)?),
            ArgumentType::BlockPredicate => ArgumentValue::BlockPredicate(blocks::parse_block_predicate(reader)?),
            ArgumentType::Swizzle => ArgumentValue::Swizzle(parse_swizzle(reader)?),
            ArgumentType::Heightmap => {
                let s = reader.read_unquoted_string();
                match Heightmap::by_name(s) {
                    Some(h) => ArgumentValue::Heightmap(h),
                    None => return Err(CommandError::new(tr!("argument.enum.invalid", s)).at(reader)),
                }
            }
            ArgumentType::ScoreHolder { multiple } => {
                ArgumentValue::ScoreHolder(ScoreHolderArg::parse(reader, allow_selectors, multiple)?)
            }
            ArgumentType::Objective | ArgumentType::Team => {
                ArgumentValue::String(reader.read_unquoted_string().to_owned())
            }
            ArgumentType::NbtCompound => {
                let tag = snbt::parse_tag(reader)?;
                if !matches!(tag, Tag::Compound(_)) {
                    return Err(CommandError::new(tr!("argument.nbt.expected.compound")).at(reader));
                }
                ArgumentValue::Nbt(tag)
            }
            ArgumentType::TeamColor => {
                let s = reader.read_unquoted_string();
                if !TEAM_COLORS.contains(&s) {
                    return Err(CommandError::new(tr!("argument.color.invalid", s)).at(reader));
                }
                ArgumentValue::String(s.to_owned())
            }
            ArgumentType::ObjectiveCriteria => {
                let s = read_until_space(reader);
                if !criterion_exists(s) {
                    reader.set_cursor(start);
                    return Err(CommandError::new(tr!("argument.criteria.invalid", s)).at(reader));
                }
                ArgumentValue::String(s.to_owned())
            }
            ArgumentType::ScoreboardSlot => {
                let s = reader.read_unquoted_string();
                if !display_slot_exists(s) {
                    return Err(CommandError::new(tr!("argument.scoreboardDisplaySlot.invalid", s)).at(reader));
                }
                ArgumentValue::String(s.to_owned())
            }
            ArgumentType::Operation => {
                if !reader.can_read() {
                    return Err(CommandError::new(tr!("arguments.operation.invalid")).at(reader));
                }
                let s = read_until_space(reader);
                match Operation::by_symbol(s) {
                    Some(op) => ArgumentValue::Operation(op),
                    None => return Err(CommandError::new(tr!("arguments.operation.invalid"))),
                }
            }
            ArgumentType::IntRange => ArgumentValue::IntRange(IntRange::parse(reader)?),
            ArgumentType::FloatRange => ArgumentValue::DoubleRange(DoubleRange::parse(reader)?),
            ArgumentType::NbtPath => ArgumentValue::NbtPath(NbtPath::parse(reader)?),
            ArgumentType::ResourceOrTag { registry } => {
                if reader.can_read() && reader.peek() == '#' {
                    reader.skip();
                    let id = Identifier::read(reader)?;
                    if blocks::registry_tag(registry, id.as_str()).is_none() {
                        let e = CommandError::new(tr!("argument.resource_tag.not_found", id.to_string(), registry));
                        let e = e.at(reader);
                        reader.set_cursor(start);
                        return Err(e);
                    }
                    ArgumentValue::ResourceOrTag(ResourceOrTag::Tag(id))
                } else {
                    let id = Identifier::read(reader)?;
                    if types::registry_entries(registry).is_some_and(|e| !e.contains(&id.as_str())) {
                        return Err(CommandError::unknown_resource(id.as_str(), registry).at(reader));
                    }
                    ArgumentValue::ResourceOrTag(ResourceOrTag::Resource(id))
                }
            }
            ArgumentType::ResourceOrTagKey { .. } => {
                if reader.can_read() && reader.peek() == '#' {
                    reader.skip();
                    ArgumentValue::ResourceOrTag(ResourceOrTag::Tag(Identifier::read(reader)?))
                } else {
                    ArgumentValue::ResourceOrTag(ResourceOrTag::Resource(Identifier::read(reader)?))
                }
            }
            ArgumentType::Feature => {
                if reader.can_read() && matches!(reader.peek(), '{' | '[' | '"' | '\'') {
                    ArgumentValue::Nbt(snbt::parse_tag(reader)?)
                } else {
                    let id = Identifier::read(reader)?;
                    if configured_features().is_some_and(|ids| !ids.iter().any(|i| i == id.as_str())) {
                        let e = tr!(
                            "argument.resource_or_id.no_such_element",
                            id.to_string(),
                            "minecraft:worldgen/feature"
                        );
                        return Err(CommandError::new(e).at(reader));
                    }
                    ArgumentValue::Identifier(id)
                }
            }
            ArgumentType::TemplateRotation | ArgumentType::TemplateMirror => {
                let s = reader.read_unquoted_string();
                let names: &[&str] = if *self == ArgumentType::TemplateRotation {
                    &["none", "clockwise_90", "180", "counterclockwise_90"]
                } else {
                    &["none", "left_right", "front_back"]
                };
                if !names.contains(&s) {
                    return Err(CommandError::new(tr!("argument.enum.invalid", s)).at(reader));
                }
                ArgumentValue::String(s.to_owned())
            }
            ArgumentType::Function => {
                let tag = reader.can_read() && reader.peek() == '#';
                if tag {
                    reader.skip();
                }
                ArgumentValue::Function { tag, id: Identifier::read(reader)? }
            }
            ArgumentType::ItemPredicate => ArgumentValue::String(parse_item_predicate(reader)?),
            ArgumentType::SlotSource => ArgumentValue::String(parse_slot_source(reader)?),
            ArgumentType::LootPredicate => {
                if reader.can_read() && matches!(reader.peek(), '{' | '[' | '"' | '\'') {
                    ArgumentValue::Nbt(snbt::parse_tag(reader)?)
                } else {
                    let id = Identifier::read(reader)?;
                    // No data-driven predicates are loaded.
                    return Err(CommandError::new(tr!(
                        "argument.resource_or_id.no_such_element",
                        id.to_string(),
                        "minecraft:predicate"
                    ))
                    .at(reader));
                }
            }
            ArgumentType::Dialog => {
                if reader.can_read() && matches!(reader.peek(), '{' | '[' | '"' | '\'') {
                    let tag = snbt::parse_tag(reader)?;
                    if let Err(message) = check_dialog(&tag) {
                        return Err(CommandError::new(tr!("argument.resource_or_id.failed_to_parse", message)).at(reader));
                    }
                    ArgumentValue::Nbt(tag)
                } else {
                    let id = Identifier::read(reader)?;
                    if kiln_data::synced_id("minecraft:dialog", id.as_str()).is_none() {
                        let e = tr!("argument.resource_or_id.no_such_element", id.to_string(), "minecraft:dialog");
                        return Err(CommandError::new(e).at(reader));
                    }
                    ArgumentValue::Identifier(id)
                }
            }
            ArgumentType::Particle => {
                // `ParticleArgument.readParticle`: the type, then its options through the codec
                // (whose errors carry no position).
                let id = Identifier::read(reader)?;
                if kiln_data::builtin_id("minecraft:particle_type", id.as_str()).is_none() {
                    return Err(CommandError::new(tr!("particle.notFound", id.to_string())).at(reader));
                }
                let options = if reader.can_read() && reader.peek() == '{' {
                    snbt::parse_tag(reader)?
                } else {
                    Tag::Compound(Vec::new())
                };
                let arg = ParticleArg { id, options };
                if let Err(message) = crate::vanilla::sound::decode_particle(&arg) {
                    return Err(CommandError::new(tr!("particle.invalidOptions", message)));
                }
                ArgumentValue::Particle(arg)
            }
            ArgumentType::HexColor => {
                // `HexColorArgument`: `Integer.parseInt` of each digit (doubled) or pair; its
                // `NumberFormatException` reaches brigadier, which reports it as a parse failure.
                let s = reader.read_unquoted_string();
                let width = match s.len() {
                    3 => 1,
                    6 => 2,
                    _ => return Err(CommandError::new(tr!("argument.hexcolor.invalid", s)).at(reader)),
                };
                let mut channels = [0i32; 3];
                for (i, part) in s.as_bytes().chunks(width).enumerate() {
                    match java_parse_hex(part) {
                        Ok(v) => channels[i] = if width == 1 { v * 17 } else { v },
                        Err(at) => {
                            let text = std::str::from_utf8(part).unwrap_or("");
                            let message = format!("Error at index {at} in: \"{text}\"");
                            return Err(CommandError::new(tr!("command.exception", message)).at(reader));
                        }
                    }
                }
                // `ARGB.color(r, g, b)`: opaque.
                let [r, g, b] = channels;
                ArgumentValue::Integer(0xFF << 24 | (r & 0xFF) << 16 | (g & 0xFF) << 8 | (b & 0xFF))
            }
            ArgumentType::NbtTag => ArgumentValue::Nbt(snbt::parse_tag(reader)?),
            ArgumentType::ContextProvider { float } => {
                let tag = snbt::parse_tag(reader)?;
                if let Tag::String(id) = &tag {
                    // An id of a data-driven provider (none are loaded).
                    let registry = if float { "minecraft:context_float_provider" } else { "minecraft:context_int_provider" };
                    let id = Identifier::parse(id).map_or_else(|| id.clone(), |i| i.to_string());
                    return Err(CommandError::new(tr!("argument.resource_or_id.no_such_element", id, registry)).at(reader));
                }
                ArgumentValue::Nbt(tag)
            }
            ArgumentType::ItemSlot | ArgumentType::ItemSlots => {
                let name = read_until_space(reader).to_owned();
                let Some(slots) = crate::slots::by_name(&name) else {
                    return Err(CommandError::new(tr!("slot.unknown", name.as_str())).at(reader));
                };
                if matches!(self, ArgumentType::ItemSlot) {
                    if slots.len() != 1 {
                        return Err(CommandError::new(tr!("slot.only_single_allowed", name.as_str())).at(reader));
                    }
                    ArgumentValue::Integer(slots[0])
                } else {
                    ArgumentValue::String(name)
                }
            }
            ArgumentType::LootResource { .. } => {
                // Ids are looked up when the command runs (the host has the loot data).
                if reader.can_read() && matches!(reader.peek(), '{' | '[' | '"' | '\'') {
                    ArgumentValue::Nbt(snbt::parse_tag(reader)?)
                } else {
                    ArgumentValue::Identifier(Identifier::read(reader)?)
                }
            }
            ArgumentType::Uuid => {
                let rest = reader.remaining();
                let len = rest.find(|c: char| !(c == '-' || c.is_ascii_hexdigit())).unwrap_or(rest.len());
                match crate::selector::java_uuid_from_string(&rest[..len]) {
                    Some(u) if len > 0 => {
                        reader.set_cursor(reader.cursor() + len);
                        ArgumentValue::String(u.to_string())
                    }
                    _ => return Err(CommandError::new(tr!("argument.uuid.invalid")).at(reader)),
                }
            }
            ArgumentType::SwingAnimation => {
                let s = reader.read_unquoted_string();
                if !["none", "whack", "stab"].contains(&s) {
                    let s = s.to_owned();
                    return Err(CommandError::new(tr!("argument.swing_animation.invalid", s)).at(reader));
                }
                ArgumentValue::String(s.to_owned())
            }
            ArgumentType::Component => ArgumentValue::Component(Box::new(component::parse(reader)?)),
            ArgumentType::Style => {
                let tag = snbt::parse_tag(reader)?;
                let valid = match &tag {
                    Tag::Compound(f) => component::decode(&Tag::Compound(
                        std::iter::once(("text".to_owned(), Tag::String(String::new()))).chain(f.iter().cloned()).collect(),
                    ))
                    .map(|_| ()),
                    other => Err(format!("Not a map: {}", snbt::to_snbt(other))),
                };
                if let Err(message) = valid {
                    reader.set_cursor(start);
                    return Err(CommandError::new(tr!("argument.style.invalid", message)).at(reader));
                }
                ArgumentValue::Nbt(tag)
            }
        })
    }

    /// Server-side suggestions (the client computes most of these itself).
    pub fn suggest<S: Source>(&self, builder: &mut SuggestionsBuilder, source: &S) {
        let allow = source.permission() >= SELECTOR_PERMISSION;
        match self {
            ArgumentType::Bool => {
                for v in ["true", "false"] {
                    if v.starts_with(builder.remaining_lowercase()) {
                        builder.suggest(v);
                    }
                }
            }
            ArgumentType::Entity { .. } | ArgumentType::GameProfile => {
                selector::suggest(builder, allow, &source.player_names());
            }
            ArgumentType::BlockPos => suggest_coordinates(builder, self, 3, false),
            ArgumentType::Vec3 { .. } => suggest_coordinates(builder, self, 3, true),
            ArgumentType::ColumnPos => suggest_coordinates(builder, self, 2, false),
            ArgumentType::Vec2 { .. } => suggest_coordinates(builder, self, 2, true),
            ArgumentType::ItemStack => {
                let items = kiln_data::builtin_entries("minecraft:item").unwrap_or(&[]);
                builder.suggest_resources(items.iter().copied(), "");
            }
            ArgumentType::Dimension => {
                let dims = source.dimensions();
                builder.suggest_resources(dims.iter().map(String::as_str), "");
            }
            ArgumentType::Resource { registry } => {
                if let Some(entries) = types::registry_entries(registry) {
                    builder.suggest_resources(entries.iter().copied(), "");
                }
            }
            ArgumentType::ResourceKey { registry } => {
                let ids = source.registry_ids(registry);
                builder.suggest_resources(ids.iter().map(String::as_str), "");
            }
            ArgumentType::EntityAnchor => builder.suggest_matching(["feet", "eyes"]),
            ArgumentType::GameMode => builder.suggest_matching(GameMode::ALL.map(GameMode::name)),
            ArgumentType::BlockState => blocks::suggest(builder, false),
            ArgumentType::BlockPredicate => blocks::suggest(builder, true),
            ArgumentType::Heightmap => builder.suggest_matching(Heightmap::ALL.map(Heightmap::name)),
            ArgumentType::Swizzle => builder.suggest_matching(["x", "xy", "xz", "xyz", "y", "yz", "z"]),
            ArgumentType::ScoreHolder { .. } => {
                let mut names = source.player_names();
                names.push("*".to_owned());
                selector::suggest(builder, allow, &names);
            }
            ArgumentType::Operation => builder.suggest_matching(Operation::ALL.map(Operation::symbol)),
            ArgumentType::TeamColor => builder.suggest_matching(TEAM_COLORS),
            ArgumentType::ScoreboardSlot => {
                let mut slots = vec!["list".to_owned(), "sidebar".to_owned(), "below_name".to_owned()];
                slots.extend(TEAM_COLORS.iter().map(|c| format!("sidebar.team.{c}")));
                builder.suggest_matching(slots.iter().map(String::as_str));
            }
            ArgumentType::ObjectiveCriteria => {
                let mut all: Vec<String> = CUSTOM_CRITERIA.iter().map(|c| (*c).to_owned()).collect();
                for c in TEAM_COLORS {
                    all.push(format!("teamkill.{c}"));
                    all.push(format!("killedByTeam.{c}"));
                }
                builder.suggest_matching(all.iter().map(String::as_str));
            }
            ArgumentType::ResourceOrTag { registry } => {
                if let Some(entries) = types::registry_entries(registry) {
                    builder.suggest_resources(entries.iter().copied(), "");
                }
                let tags: Vec<&str> = kiln_data::registries::TAGS
                    .iter()
                    .filter(|(r, _)| r == registry)
                    .flat_map(|(_, t)| t.iter().map(|(n, _)| *n))
                    .collect();
                builder.suggest_resources(tags, "#");
            }
            ArgumentType::Time { .. } => {
                let mut r = StringReader::new(builder.remaining());
                if r.read_float().is_ok() {
                    let mut units = builder.offset(builder.start() + r.cursor());
                    units.suggest_matching(["d", "s", "t", ""]);
                    builder.add(units);
                }
            }
            _ => {}
        }
    }
}

/// `Math.round(float)`.
fn java_round(v: f32) -> i32 {
    (v as f64 + 0.5).floor() as i32
}

/// `SharedSuggestionProvider.suggestCoordinates` / `suggest2DCoordinates` for a source whose
/// relevant coordinates are `~ ~ ~` (or `^ ^ ^` once the input starts with `^`).
fn suggest_coordinates(builder: &mut SuggestionsBuilder, ty: &ArgumentType, dims: usize, allow_local: bool) {
    let remaining = builder.remaining();
    let fill = if allow_local && remaining.starts_with('^') { "^" } else { "~" };
    let valid = |s: &str| ty.parse(&mut StringReader::new(s), false).is_ok();
    let mut out: Vec<String> = Vec::new();
    // Java's `split(" ")`: trailing empty parts are dropped.
    let mut parts: Vec<&str> = remaining.split(' ').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    let full = |given: &[&str]| {
        let mut all: Vec<&str> = given.to_vec();
        all.resize(dims, fill);
        all.join(" ")
    };
    match parts.len() {
        0 if valid(&full(&[])) => {
            for n in 1..=dims {
                out.push(vec![fill; n].join(" "));
            }
        }
        n if n > 0 && n < dims && valid(&full(&parts)) => {
            for k in n + 1..=dims {
                let mut v = parts.clone();
                v.resize(k, fill);
                out.push(v.join(" "));
            }
        }
        _ => {}
    }
    builder.suggest_matching(out.iter().map(String::as_str));
}

fn parse_item(reader: &mut StringReader) -> Result<ItemInput> {
    let start = reader.cursor();
    let item = Identifier::read(reader)?;
    if kiln_data::builtin_id("minecraft:item", item.as_str()).is_none() {
        reader.set_cursor(start);
        return Err(CommandError::unknown_item(item.as_str()).at(reader));
    }
    let mut components: Vec<(Identifier, Option<String>)> = Vec::new();
    if reader.can_read() && reader.peek() == '[' {
        reader.skip();
        while reader.can_read() && reader.peek() != ']' {
            reader.skip_whitespace();
            let remove = reader.can_read() && reader.peek() == '!';
            if remove {
                reader.skip();
            }
            let id = read_component_type(reader)?;
            if components.iter().any(|(c, _)| *c == id) {
                return Err(CommandError::new(tr!("arguments.item.component.repeated", id.to_string())));
            }
            let value = if remove {
                None
            } else {
                reader.skip_whitespace();
                reader.expect('=')?;
                reader.skip_whitespace();
                Some(snbt::read_value(reader)?.to_owned())
            };
            reader.skip_whitespace();
            components.push((id, value));
            if !reader.can_read() || reader.peek() != ',' {
                break;
            }
            reader.skip();
            reader.skip_whitespace();
            if !reader.can_read() {
                return Err(CommandError::new(tr!("arguments.item.component.expected")).at(reader));
            }
        }
        reader.expect(']')?;
    }
    Ok(ItemInput { item, components })
}

fn read_component_type(reader: &mut StringReader) -> Result<Identifier> {
    if !reader.can_read() {
        return Err(CommandError::new(tr!("arguments.item.component.expected")).at(reader));
    }
    let start = reader.cursor();
    let id = Identifier::read(reader)?;
    if kiln_data::builtin_id("minecraft:data_component_type", id.as_str()).is_none() {
        reader.set_cursor(start);
        return Err(CommandError::new(tr!("arguments.item.component.unknown", id.to_string())).at(reader));
    }
    Ok(id)
}

/// `SwizzleArgument.parse`: each of `x`, `y`, `z` at most once, up to a space.
fn parse_swizzle(reader: &mut StringReader) -> Result<[bool; 3]> {
    let mut axes = [false; 3];
    while reader.can_read() && reader.peek() != ' ' {
        let axis = match reader.read() {
            'x' => 0,
            'y' => 1,
            'z' => 2,
            _ => return Err(CommandError::new(tr!("arguments.swizzle.invalid")).at(reader)),
        };
        if axes[axis] {
            return Err(CommandError::new(tr!("arguments.swizzle.invalid")).at(reader));
        }
        axes[axis] = true;
    }
    Ok(axes)
}

fn read_until_space<'a>(reader: &mut StringReader<'a>) -> &'a str {
    let start = reader.cursor();
    while reader.can_read() && reader.peek() != ' ' {
        reader.skip();
    }
    &reader.string()[start..reader.cursor()]
}

/// `ChatFormatting` colors usable for teams (`sidebar.team.<color>`, `teamkill.<color>`).
pub const TEAM_COLORS: [&str; 16] = [
    "black",
    "dark_blue",
    "dark_green",
    "dark_aqua",
    "dark_red",
    "dark_purple",
    "gold",
    "gray",
    "dark_gray",
    "blue",
    "green",
    "aqua",
    "red",
    "light_purple",
    "yellow",
    "white",
];

/// A light check of an inline dialog (`Dialog.DIRECT_CODEC`): a compound whose `type` is a
/// `minecraft:dialog_type` and which has a `title`. The client decodes the rest.
fn check_dialog(tag: &Tag) -> std::result::Result<(), String> {
    let Tag::Compound(_) = tag else { return Err(format!("Not a map: {}", snbt::to_snbt(tag))) };
    let Some(ty) = tag.get("type").and_then(Tag::as_str) else { return Err("No key type in MapLike".into()) };
    let ty = Identifier::parse(ty).map(|i| i.to_string()).unwrap_or_default();
    if types::registry_entries("minecraft:dialog_type").is_some_and(|e| !e.contains(&ty.as_str())) {
        return Err(format!("Unknown registry key in ResourceKey[minecraft:root / minecraft:dialog_type]: {ty}"));
    }
    if tag.get("title").is_none() {
        return Err("No key title in MapLike".into());
    }
    Ok(())
}

/// `ObjectiveCriteria.CUSTOM_CRITERIA` apart from the team ones.
pub const CUSTOM_CRITERIA: [&str; 11] =
    ["dummy", "trigger", "deathCount", "playerKillCount", "totalKillCount", "health", "xp", "level", "food", "air", "armor"];

/// `ObjectiveCriteria.byName`: custom criteria, team criteria and `<stat type>:<stat>`
/// statistics with `.` for `:` in the ids (`minecraft.mined:minecraft.stone`).
pub fn criterion_exists(name: &str) -> bool {
    if CUSTOM_CRITERIA.contains(&name) {
        return true;
    }
    for prefix in ["teamkill.", "killedByTeam."] {
        if let Some(color) = name.strip_prefix(prefix) {
            return TEAM_COLORS.contains(&color);
        }
    }
    let Some((kind, value)) = name.split_once(':') else { return false };
    let registry = match kind.replacen('.', ":", 1).as_str() {
        "minecraft:mined" => "minecraft:block",
        "minecraft:crafted" | "minecraft:used" | "minecraft:broken" | "minecraft:picked_up" | "minecraft:dropped" => {
            "minecraft:item"
        }
        "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
        "minecraft:custom" => "minecraft:custom_stat",
        _ => return false,
    };
    let id = value.replacen('.', ":", 1);
    kiln_data::builtin_id(registry, &id).is_some()
}

/// `DisplaySlot` names.
pub fn display_slot_exists(name: &str) -> bool {
    matches!(name, "list" | "sidebar" | "below_name")
        || name.strip_prefix("sidebar.team.").is_some_and(|c| TEAM_COLORS.contains(&c))
}

/// `OperationArgument.Operation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Assign,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Min,
    Max,
    Swap,
}

impl Operation {
    pub const ALL: [Operation; 9] = [
        Operation::Assign,
        Operation::Add,
        Operation::Subtract,
        Operation::Multiply,
        Operation::Divide,
        Operation::Modulo,
        Operation::Min,
        Operation::Max,
        Operation::Swap,
    ];

    pub fn symbol(self) -> &'static str {
        ["=", "+=", "-=", "*=", "/=", "%=", "<", ">", "><"][self as usize]
    }

    pub fn by_symbol(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.symbol() == s)
    }
}

/// `ResourceOrTagArgument.Result`.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceOrTag {
    Resource(Identifier),
    Tag(Identifier),
}

impl ResourceOrTag {
    /// Whether registry entry `id` of `registry` is this resource or in this tag.
    pub fn test(&self, registry: &str, id: &str) -> bool {
        match self {
            ResourceOrTag::Resource(r) => r.as_str() == id,
            ResourceOrTag::Tag(t) => {
                let index = types::registry_entries(registry).and_then(|e| e.iter().position(|x| *x == id));
                index.zip(blocks::registry_tag(registry, t.as_str())).is_some_and(|(i, tag)| tag.contains(&(i as i32)))
            }
        }
    }
}

/// Configured feature ids of the vanilla datapack (`KILN_DATAPACK` or `work/generated`), if
/// it is there.
pub(crate) fn configured_features() -> Option<&'static Vec<String>> {
    static IDS: std::sync::OnceLock<Option<Vec<String>>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        let root = std::env::var_os("KILN_DATAPACK").map(std::path::PathBuf::from).unwrap_or_else(|| "work/generated".into());
        let dir = root.join("data/minecraft/worldgen/feature");
        let mut ids: Vec<String> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(|n| format!("minecraft:{n}")))
            .collect();
        ids.sort();
        Some(ids)
    })
    .as_ref()
}

/// `ScoreHolderArgument.Result`: a selector, `*` or a name.
#[derive(Debug, Clone, PartialEq)]
pub enum ScoreHolderArg {
    Selector(Box<EntitySelector>),
    Wildcard,
    Name(String),
}

impl ScoreHolderArg {
    /// `ScoreHolderArgument.parse`.
    pub fn parse(reader: &mut StringReader, allow_selectors: bool, multiple: bool) -> Result<Self> {
        if reader.can_read() && reader.peek() == '@' {
            let sel = EntitySelector::parse(reader, allow_selectors)?;
            if !multiple && sel.max_results > 1 {
                return Err(CommandError::not_single_entity().at(reader));
            }
            return Ok(ScoreHolderArg::Selector(Box::new(sel)));
        }
        let name = read_until_space(reader);
        Ok(if name == "*" { ScoreHolderArg::Wildcard } else { ScoreHolderArg::Name(name.to_owned()) })
    }

    /// `ScoreHolderArgument.getNames`: holder names; `*` stands for `wildcard` (the
    /// scoreboard's tracked holders for `getNamesWithDefaultWildcard`, nothing otherwise).
    pub fn names<W: SelectorWorld>(&self, world: &mut W, wildcard: Option<Vec<String>>) -> Result<Vec<String>> {
        let names = match self {
            ScoreHolderArg::Selector(sel) => {
                let found = sel.find_entities(world)?;
                if found.is_empty() {
                    return Err(CommandError::no_entities_found());
                }
                found.iter().map(SelectorTarget::scoreboard_name).collect()
            }
            ScoreHolderArg::Wildcard => {
                let all = wildcard.unwrap_or_default();
                if all.is_empty() {
                    return Err(CommandError::new(tr!("argument.scoreHolder.empty")));
                }
                all
            }
            ScoreHolderArg::Name(name) if name.starts_with('#') => vec![name.clone()],
            ScoreHolderArg::Name(name) => match selector::java_uuid_from_string(name) {
                Some(uuid) => {
                    vec![world.entity_by_uuid(uuid).map_or_else(|| name.clone(), |e| e.scoreboard_name())]
                }
                None => vec![
                    world
                        .players()
                        .into_iter()
                        .find(|p| p.name() == *name)
                        .map_or_else(|| name.clone(), |p| p.scoreboard_name()),
                ],
            },
        };
        if names.is_empty() {
            return Err(CommandError::no_entities_found());
        }
        Ok(names)
    }

    /// `ScoreHolderArgument.getName`: the first holder.
    pub fn name<W: SelectorWorld>(&self, world: &mut W) -> Result<String> {
        Ok(self.names(world, None)?.swap_remove(0))
    }
}

/// `ItemPredicateArgument`: `*`, an item id or `#tag`, then optional `[...]` tests. Kept as
/// text: Kiln cannot evaluate item predicates yet.
fn parse_item_predicate(reader: &mut StringReader) -> Result<String> {
    let start = reader.cursor();
    if reader.can_read() && reader.peek() == '*' {
        reader.skip();
    } else if reader.can_read() && reader.peek() == '#' {
        reader.skip();
        let id = Identifier::read(reader)?;
        if blocks::registry_tag("minecraft:item", id.as_str()).is_none() {
            return Err(CommandError::new(tr!("arguments.item.tag.unknown", id.to_string())).at(reader));
        }
    } else {
        let id = Identifier::read(reader)?;
        if kiln_data::builtin_id("minecraft:item", id.as_str()).is_none() {
            return Err(CommandError::unknown_item(id.as_str()).at(reader));
        }
    }
    if reader.can_read() && reader.peek() == '[' {
        reader.skip();
        loop {
            reader.skip_whitespace();
            if !reader.can_read() {
                return Err(CommandError::expected_symbol(']').at(reader));
            }
            if reader.peek() == ']' {
                reader.skip();
                break;
            }
            while reader.can_read() && matches!(reader.peek(), '!' | '~') {
                reader.skip();
            }
            Identifier::read(reader)?;
            reader.skip_whitespace();
            if reader.can_read() && reader.peek() == '=' {
                reader.skip();
                snbt::parse_tag(reader)?;
            }
            reader.skip_whitespace();
            if reader.can_read() && matches!(reader.peek(), ',' | '|') {
                reader.skip();
            }
        }
    }
    Ok(reader.string()[start..reader.cursor()].to_owned())
}

/// `SlotSourceArgument`: a slot range such as `container.*` or a slot source id or inline
/// definition. Kept as text.
fn parse_slot_source(reader: &mut StringReader) -> Result<String> {
    let start = reader.cursor();
    if reader.can_read() && matches!(reader.peek(), '{' | '[') {
        snbt::parse_tag(reader)?;
    } else {
        while reader.can_read() && (types::is_allowed_in_identifier(reader.peek()) || reader.peek() == '*') {
            reader.skip();
        }
        if reader.cursor() == start {
            return Err(CommandError::invalid_id().at(reader));
        }
    }
    Ok(reader.string()[start..reader.cursor()].to_owned())
}

/// A parsed argument value.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentValue {
    Bool(bool),
    Integer(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    Entity(Box<EntitySelector>),
    GameProfile(GameProfileArg),
    /// `block_pos`, `column_pos`, `vec2`, `vec3` and `rotation`.
    Coordinates(Coordinates),
    Item(ItemInput),
    Message(MessageArg),
    /// `resource_location`, `dimension` and `resource`.
    Identifier(Identifier),
    Anchor(Anchor),
    GameMode(GameMode),
    Time(i32),
    BlockState(BlockInput),
    BlockPredicate(BlockPredicate),
    /// Axes `[x, y, z]`.
    Swizzle([bool; 3]),
    Heightmap(Heightmap),
    ScoreHolder(ScoreHolderArg),
    Operation(Operation),
    IntRange(IntRange),
    DoubleRange(DoubleRange),
    NbtPath(NbtPath),
    ResourceOrTag(ResourceOrTag),
    Function { tag: bool, id: Identifier },
    Component(Box<Component>),
    /// Inline NBT (`loot_predicate` definitions, `style`).
    Nbt(Tag),
    Particle(ParticleArg),
}

/// `Integer.parseInt(s, 16)` of a short run: the value, or the index `NumberFormatException`
/// reports (a sign is allowed first, as in Java).
fn java_parse_hex(s: &[u8]) -> std::result::Result<i32, usize> {
    let (negative, start) = match s.first() {
        Some(b'-') => (true, 1),
        Some(b'+') => (false, 1),
        _ => (false, 0),
    };
    if start == s.len() {
        return Err(start);
    }
    let mut v = 0i32;
    for (i, &c) in s.iter().enumerate().skip(start) {
        match (c as char).to_digit(16) {
            Some(d) => v = v * 16 + d as i32,
            None => return Err(i),
        }
    }
    Ok(if negative { -v } else { v })
}

/// `ParticleArgument`: the type and its options compound (empty when none was given).
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleArg {
    pub id: Identifier,
    pub options: Tag,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GameProfileArg {
    Selector(Box<EntitySelector>),
    /// A player name, resolved through the host's profile cache.
    Name(String),
}

/// `MessageArgument.Message`: the text and the selectors found in it (byte ranges into
/// `text`), resolved to names when the source may use selectors.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageArg {
    pub text: String,
    pub parts: Vec<(usize, usize, EntitySelector)>,
}

pub const MAX_MESSAGE_LENGTH: usize = 256;

impl MessageArg {
    pub fn parse(reader: &mut StringReader, allow_selectors: bool) -> Result<Self> {
        let len = reader.remaining().encode_utf16().count();
        if len > MAX_MESSAGE_LENGTH {
            return Err(CommandError::message_too_long(len, MAX_MESSAGE_LENGTH));
        }
        let text = reader.remaining().to_owned();
        let base = reader.cursor();
        if !allow_selectors {
            reader.set_cursor(reader.total_len());
            return Ok(MessageArg { text, parts: Vec::new() });
        }
        let mut parts = Vec::new();
        while reader.can_read() {
            if reader.peek() != '@' {
                reader.skip();
                continue;
            }
            let at = reader.cursor();
            match EntitySelector::parse(reader, true) {
                Ok(sel) => parts.push((at - base, reader.cursor() - base, sel)),
                Err(e)
                    if matches!(
                        e.key(),
                        Some("argument.entity.selector.missing" | "argument.entity.selector.unknown")
                    ) =>
                {
                    reader.set_cursor(at + 1);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(MessageArg { text, parts })
    }

    /// `Message.toComponent`: selectors become the names of the entities they find.
    pub fn resolve<W: SelectorWorld>(&self, world: &mut W) -> Result<Text> {
        if self.parts.is_empty() || world.permission() < SELECTOR_PERMISSION {
            return Ok(Text::literal(&self.text));
        }
        let mut out = Text::literal(&self.text[..self.parts[0].0]);
        let mut i = self.parts[0].0;
        for (start, end, sel) in &self.parts {
            let names = selector::join_names(&sel.find_entities(world)?);
            if i < *start {
                out = out.append(Text::literal(&self.text[i..*start]));
            }
            out = out.append(names);
            i = *end;
        }
        if i < self.text.len() {
            out = out.append(Text::literal(&self.text[i..]));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(ty: ArgumentType, s: &str) -> Result<ArgumentValue> {
        ty.parse(&mut StringReader::new(s), true)
    }

    fn err(ty: ArgumentType, s: &str) -> (String, Option<usize>) {
        let e = parse(ty, s).unwrap_err();
        (e.key().unwrap().to_owned(), e.cursor())
    }

    #[test]
    fn numbers_with_bounds() {
        assert_eq!(parse(ArgumentType::integer_min(1), "5").unwrap(), ArgumentValue::Integer(5));
        assert_eq!(err(ArgumentType::integer_min(1), "0"), ("argument.integer.low".into(), Some(0)));
        assert_eq!(err(ArgumentType::integer_range(0, 8), "9"), ("argument.integer.big".into(), Some(0)));
        let e = parse(ArgumentType::float_range(1e-5, 1000.0), "2000").unwrap_err();
        assert_eq!(e.args(), &[crate::text::Arg::Float(1000.0), crate::text::Arg::Float(2000.0)]);
        assert_eq!(parse(ArgumentType::long(), "-9000000000").unwrap(), ArgumentValue::Long(-9_000_000_000));
        assert_eq!(parse(ArgumentType::double(), "1.25").unwrap(), ArgumentValue::Double(1.25));
        assert_eq!(parse(ArgumentType::Bool, "false").unwrap(), ArgumentValue::Bool(false));
    }

    #[test]
    fn strings() {
        let mut r = StringReader::new("word \"quoted phrase\" greedy rest");
        assert_eq!(ArgumentType::word().parse(&mut r, true).unwrap(), ArgumentValue::String("word".into()));
        r.skip();
        assert_eq!(ArgumentType::string().parse(&mut r, true).unwrap(), ArgumentValue::String("quoted phrase".into()));
        r.skip();
        assert_eq!(
            ArgumentType::greedy_string().parse(&mut r, true).unwrap(),
            ArgumentValue::String("greedy rest".into())
        );
        assert!(!r.can_read());
    }

    #[test]
    fn entity_argument_restrictions() {
        assert_eq!(err(ArgumentType::entity(), "@e"), ("argument.entity.toomany".into(), Some(0)));
        assert_eq!(err(ArgumentType::player(), "@a"), ("argument.player.toomany".into(), Some(0)));
        assert_eq!(err(ArgumentType::players(), "@e"), ("argument.player.entities".into(), Some(0)));
        assert!(parse(ArgumentType::players(), "@s").is_ok());
        assert!(parse(ArgumentType::players(), "@e[type=player]").is_ok());
        assert!(parse(ArgumentType::entity(), "@e[limit=1]").is_ok());
        let e = ArgumentType::entities().parse(&mut StringReader::new("@e"), false).unwrap_err();
        assert_eq!(e.key(), Some("argument.entity.selector.not_allowed"));
        assert!(ArgumentType::entities().parse(&mut StringReader::new("Steve"), false).is_ok());
    }

    #[test]
    fn game_profile() {
        let mut r = StringReader::new("Notch rest");
        assert_eq!(
            ArgumentType::GameProfile.parse(&mut r, true).unwrap(),
            ArgumentValue::GameProfile(GameProfileArg::Name("Notch".into()))
        );
        assert_eq!(r.cursor(), 5);
        assert!(matches!(
            parse(ArgumentType::GameProfile, "@a").unwrap(),
            ArgumentValue::GameProfile(GameProfileArg::Selector(_))
        ));
        assert_eq!(err(ArgumentType::GameProfile, "@e").0, "argument.player.entities");
    }

    #[test]
    fn simple_enums_and_ids() {
        assert_eq!(parse(ArgumentType::GameMode, "creative").unwrap(), ArgumentValue::GameMode(GameMode::Creative));
        assert_eq!(err(ArgumentType::GameMode, "god"), ("argument.gamemode.invalid".into(), Some(3)));
        assert_eq!(parse(ArgumentType::EntityAnchor, "eyes").unwrap(), ArgumentValue::Anchor(Anchor::Eyes));
        assert_eq!(err(ArgumentType::EntityAnchor, "head"), ("argument.anchor.invalid".into(), Some(0)));
        assert_eq!(
            parse(ArgumentType::Dimension, "the_nether").unwrap(),
            ArgumentValue::Identifier(Identifier::parse("minecraft:the_nether").unwrap())
        );
        assert!(parse(ArgumentType::resource("minecraft:timeline"), "day").is_ok());
        assert_eq!(
            err(ArgumentType::resource("minecraft:timeline"), "night"),
            ("argument.resource.not_found".into(), Some(5))
        );
        assert_eq!(err(ArgumentType::ResourceLocation, "a:b:c"), ("argument.id.invalid".into(), Some(0)));
    }

    #[test]
    fn time_units() {
        assert_eq!(parse(ArgumentType::time(), "5").unwrap(), ArgumentValue::Time(5));
        assert_eq!(parse(ArgumentType::time(), "1.5s").unwrap(), ArgumentValue::Time(30));
        assert_eq!(parse(ArgumentType::time(), "0.5d").unwrap(), ArgumentValue::Time(12000));
        assert_eq!(parse(ArgumentType::time(), "3t").unwrap(), ArgumentValue::Time(3));
        assert_eq!(err(ArgumentType::time(), "3x"), ("argument.time.invalid_unit".into(), Some(2)));
        let e = parse(ArgumentType::time_min(1), "0").unwrap_err();
        assert_eq!(e.args(), &[crate::text::Arg::Int(1), crate::text::Arg::Int(0)]);
        assert_eq!(parse(ArgumentType::time_min(i32::MIN), "-2d").unwrap(), ArgumentValue::Time(-48000));
    }

    #[test]
    fn items() {
        let ArgumentValue::Item(i) =
            parse(ArgumentType::ItemStack, "diamond_sword[damage=5,!minecraft:unbreakable, custom_name='x']").unwrap()
        else {
            panic!()
        };
        assert_eq!(i.item.as_str(), "minecraft:diamond_sword");
        assert_eq!(i.components.len(), 3);
        assert_eq!(i.components[1], (Identifier::parse("unbreakable").unwrap(), None));
        assert_eq!(i.components[2].1.as_deref(), Some("'x'"));
        assert_eq!(err(ArgumentType::ItemStack, "minecraft:nope"), ("argument.item.id.invalid".into(), Some(0)));
        assert_eq!(
            err(ArgumentType::ItemStack, "stone[bogus=1]"),
            ("arguments.item.component.unknown".into(), Some(6))
        );
        assert_eq!(err(ArgumentType::ItemStack, "stone[damage=1,damage=2]").0, "arguments.item.component.repeated");
    }

    #[test]
    fn message_selectors() {
        let mut r = StringReader::new("hi @a and @x @ and @e[type=zombie]!");
        let ArgumentValue::Message(m) = ArgumentType::Message.parse(&mut r, true).unwrap() else { panic!() };
        assert_eq!(m.parts.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(), [(3, 5), (19, 34)]);
        let ArgumentValue::Message(m) = ArgumentType::Message.parse(&mut StringReader::new("hi @a"), false).unwrap()
        else {
            panic!()
        };
        assert!(m.parts.is_empty());
        let long = "x".repeat(257);
        let e = ArgumentType::Message.parse(&mut StringReader::new(&long), true).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("argument.message.too_long"), None));
        assert_eq!(err(ArgumentType::Message, "hi @e[foo=1]").0, "argument.entity.options.unknown");
    }

    #[test]
    fn wire_properties() {
        assert_eq!(ArgumentType::integer_min(1).wire(), Parser::Integer { min: Some(1), max: None });
        assert_eq!(ArgumentType::float().wire(), Parser::Float { min: None, max: None });
        assert_eq!(ArgumentType::players().wire(), Parser::Entity { single: false, players_only: true });
        assert_eq!(ArgumentType::time_min(i32::MIN).wire(), Parser::Time { min: i32::MIN });
        assert_eq!(ArgumentType::greedy_string().wire(), Parser::String(StringKind::Greedy));
    }

    struct Src(u8, crate::host::SourceStack<Src>);
    impl Source for Src {
        type Entity = crate::selector::NoEntity;
        fn permission_level(&self) -> u8 {
            self.0
        }
        fn stack(&self) -> &crate::host::SourceStack<Src> {
            &self.1
        }
        fn stack_mut(&mut self) -> &mut crate::host::SourceStack<Src> {
            &mut self.1
        }
        fn player_names(&self) -> Vec<String> {
            vec!["Alice".into(), "Bob".into()]
        }
    }

    fn suggest(ty: ArgumentType, input: &str, start: usize, level: u8) -> Vec<String> {
        let mut b = SuggestionsBuilder::new(input, start);
        let stack = crate::host::SourceStack::new(Text::literal("Server"), "minecraft:overworld", [0.0; 3]);
        ty.suggest(&mut b, &Src(level, stack));
        b.build().list.into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn suggestions() {
        assert_eq!(
            suggest(ArgumentType::entities(), "kill ", 5, 2),
            ["@a", "@e", "@n", "@p", "@r", "@s", "Alice", "Bob"]
        );
        assert_eq!(suggest(ArgumentType::entities(), "kill ", 5, 0), ["Alice", "Bob"]);
        assert_eq!(suggest(ArgumentType::entities(), "kill a", 5, 2), ["Alice"]);
        assert_eq!(suggest(ArgumentType::entities(), "kill @", 5, 2), ["@a", "@e", "@n", "@p", "@r", "@s"]);
        assert_eq!(suggest(ArgumentType::entities(), "kill @e", 5, 2), ["["]);
        assert_eq!(suggest(ArgumentType::entities(), "kill @e[li", 5, 2), ["limit="]);
        let keys = suggest(ArgumentType::entities(), "kill @s[", 5, 2);
        assert!(keys.contains(&"name=".to_owned()) && !keys.iter().any(|k| k.contains("limit")));
        assert_eq!(suggest(ArgumentType::entities(), "kill @e[sort=f", 5, 2), ["furthest"]);
        assert_eq!(suggest(ArgumentType::entities(), "kill @e[limit=1", 5, 2), [",", "]"]);
        assert_eq!(suggest(ArgumentType::entities(), "kill @e[gamemode=!sp", 5, 2), ["!spectator"]);
        assert!(suggest(ArgumentType::entities(), "kill @e[type=zomb", 5, 2).contains(&"minecraft:zombie".to_owned()));
        assert_eq!(suggest(ArgumentType::GameMode, "gamemode s", 9, 0), ["spectator", "survival"]);
        assert_eq!(suggest(ArgumentType::Bool, "x t", 2, 0), ["true"]);
        assert_eq!(suggest(ArgumentType::time(), "t 10", 2, 0), ["d", "s", "t"]);
        assert_eq!(suggest(ArgumentType::vec3(), "tp ", 3, 0), ["~", "~ ~", "~ ~ ~"]);
        assert_eq!(suggest(ArgumentType::vec3(), "tp 1", 3, 0), ["1 ~", "1 ~ ~"]);
        assert_eq!(suggest(ArgumentType::vec3(), "tp 1 ", 3, 0), ["1 ~", "1 ~ ~"]);
        assert_eq!(suggest(ArgumentType::vec3(), "tp 1 2", 3, 0), ["1 2 ~"]);
        assert_eq!(suggest(ArgumentType::vec3(), "tp ^", 3, 0), ["^ ^", "^ ^ ^"]);
        assert_eq!(suggest(ArgumentType::ColumnPos, "c ", 2, 0), ["~", "~ ~"]);
        assert!(
            suggest(ArgumentType::ItemStack, "give @s diamond_s", 8, 0).contains(&"minecraft:diamond_sword".to_owned())
        );
    }
}
