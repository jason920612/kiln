//! `CommandSyntaxException` equivalent, with constructors for the vanilla error types.

use crate::reader::StringReader;
use crate::text::{Arg, ClickEvent, Text};
use crate::tr;
use std::fmt;

/// A command error: a translatable message and, for parse errors, the input and cursor.
/// Boxed so that `Result`s stay small on the parsing paths.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandError(Box<Inner>);

#[derive(Debug, Clone, PartialEq)]
struct Inner {
    message: Text,
    context: Option<(String, usize)>,
}

impl CommandError {
    pub fn new(message: Text) -> Self {
        CommandError(Box::new(Inner { message, context: None }))
    }

    /// Attaches the reader's input and cursor (`createWithContext`).
    pub fn at(mut self, reader: &StringReader) -> Self {
        self.0.context = Some((reader.string().to_owned(), reader.cursor()));
        self
    }

    pub fn with_context(mut self, input: &str, cursor: usize) -> Self {
        self.0.context = Some((input.to_owned(), cursor));
        self
    }

    pub fn without_context(mut self) -> Self {
        self.0.context = None;
        self
    }

    pub fn message(&self) -> &Text {
        &self.0.message
    }

    pub fn key(&self) -> Option<&str> {
        self.0.message.key()
    }

    pub fn args(&self) -> &[Arg] {
        self.0.message.args()
    }

    /// Byte offset into the input where the error was found.
    pub fn cursor(&self) -> Option<usize> {
        self.0.context.as_ref().map(|c| c.1)
    }

    pub fn input(&self) -> Option<&str> {
        self.0.context.as_ref().map(|c| c.0.as_str())
    }

    /// The chat lines vanilla sends for this error when `command` (without `/`) fails: the red
    /// message, then for parse errors the last ten characters before the cursor, the rest
    /// underlined, and `<--[HERE]` (`Commands.finishParsing`).
    pub fn chat_lines(&self, command: &str) -> Vec<Text> {
        let mut lines = vec![Text::empty().append(self.0.message.clone()).color("red")];
        if let Some((input, cursor)) = &self.0.context {
            let cursor = (*cursor).min(input.len());
            let (before, after) = input.split_at(cursor);
            let mut context = Text::empty().color("gray").click(ClickEvent::SuggestCommand(format!("/{command}")));
            let skip = before.chars().count().saturating_sub(10);
            if skip > 0 {
                context = context.append(Text::literal("..."));
            }
            let start = before.char_indices().nth(skip).map_or(before.len(), |(i, _)| i);
            context = context.append(Text::literal(&before[start..]));
            if !after.is_empty() {
                context = context.append(Text::literal(after).color("red").underlined());
            }
            context = context.append(tr!("command.context.here").color("red").italic());
            lines.push(Text::empty().append(context).color("red"));
        }
        lines
    }
}

impl fmt::Display for CommandError {
    /// Brigadier's `getMessage()`: `message at position N: ...context<--[HERE]`.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0.message.to_plain())?;
        if let Some((input, cursor)) = &self.0.context {
            let cursor = (*cursor).min(input.len());
            let before = &input[..cursor];
            let skip = before.chars().count().saturating_sub(10);
            let start = before.char_indices().nth(skip).map_or(before.len(), |(i, _)| i);
            write!(f, " at position {cursor}: {}{}<--[HERE]", if skip > 0 { "..." } else { "" }, &before[start..])?;
        }
        Ok(())
    }
}

impl std::error::Error for CommandError {}

fn err(message: Text) -> CommandError {
    CommandError::new(message)
}

/// Brigadier built-ins with vanilla's translation keys (`BrigadierExceptions`).
impl CommandError {
    pub fn float_too_low(found: f32, min: f32) -> Self {
        err(tr!("argument.float.low", min, found))
    }
    pub fn float_too_high(found: f32, max: f32) -> Self {
        err(tr!("argument.float.big", max, found))
    }
    pub fn double_too_low(found: f64, min: f64) -> Self {
        err(tr!("argument.double.low", min, found))
    }
    pub fn double_too_high(found: f64, max: f64) -> Self {
        err(tr!("argument.double.big", max, found))
    }
    pub fn integer_too_low(found: i32, min: i32) -> Self {
        err(tr!("argument.integer.low", min, found))
    }
    pub fn integer_too_high(found: i32, max: i32) -> Self {
        err(tr!("argument.integer.big", max, found))
    }
    pub fn long_too_low(found: i64, min: i64) -> Self {
        err(tr!("argument.long.low", min, found))
    }
    pub fn long_too_high(found: i64, max: i64) -> Self {
        err(tr!("argument.long.big", max, found))
    }
    pub fn literal_incorrect(literal: &str) -> Self {
        err(tr!("argument.literal.incorrect", literal))
    }
    pub fn expected_start_of_quote() -> Self {
        err(tr!("parsing.quote.expected.start"))
    }
    pub fn expected_end_of_quote() -> Self {
        err(tr!("parsing.quote.expected.end"))
    }
    pub fn invalid_escape(c: char) -> Self {
        err(tr!("parsing.quote.escape", c.to_string()))
    }
    pub fn invalid_bool(value: &str) -> Self {
        err(tr!("parsing.bool.invalid", value))
    }
    pub fn expected_bool() -> Self {
        err(tr!("parsing.bool.expected"))
    }
    pub fn invalid_int(value: &str) -> Self {
        err(tr!("parsing.int.invalid", value))
    }
    pub fn expected_int() -> Self {
        err(tr!("parsing.int.expected"))
    }
    pub fn invalid_long(value: &str) -> Self {
        err(tr!("parsing.long.invalid", value))
    }
    pub fn expected_long() -> Self {
        err(tr!("parsing.long.expected"))
    }
    pub fn invalid_double(value: &str) -> Self {
        err(tr!("parsing.double.invalid", value))
    }
    pub fn expected_double() -> Self {
        err(tr!("parsing.double.expected"))
    }
    pub fn invalid_float(value: &str) -> Self {
        err(tr!("parsing.float.invalid", value))
    }
    pub fn expected_float() -> Self {
        err(tr!("parsing.float.expected"))
    }
    pub fn expected_symbol(c: char) -> Self {
        err(tr!("parsing.expected", c.to_string()))
    }
    pub fn unknown_command() -> Self {
        err(tr!("command.unknown.command"))
    }
    pub fn unknown_argument() -> Self {
        err(tr!("command.unknown.argument"))
    }
    pub fn expected_separator() -> Self {
        err(tr!("command.expected.separator"))
    }
}

/// Errors of the vanilla argument types and commands.
impl CommandError {
    pub fn invalid_name_or_uuid() -> Self {
        err(tr!("argument.entity.invalid"))
    }
    pub fn unknown_selector_type(selector: &str) -> Self {
        err(tr!("argument.entity.selector.unknown", selector))
    }
    pub fn selectors_not_allowed() -> Self {
        err(tr!("argument.entity.selector.not_allowed"))
    }
    pub fn missing_selector_type() -> Self {
        err(tr!("argument.entity.selector.missing"))
    }
    pub fn expected_end_of_options() -> Self {
        err(tr!("argument.entity.options.unterminated"))
    }
    pub fn expected_option_value(option: &str) -> Self {
        err(tr!("argument.entity.options.valueless", option))
    }
    pub fn unknown_option(option: &str) -> Self {
        err(tr!("argument.entity.options.unknown", option))
    }
    pub fn inapplicable_option(option: &str) -> Self {
        err(tr!("argument.entity.options.inapplicable", option))
    }
    pub fn distance_negative() -> Self {
        err(tr!("argument.entity.options.distance.negative"))
    }
    pub fn level_negative() -> Self {
        err(tr!("argument.entity.options.level.negative"))
    }
    pub fn limit_too_small() -> Self {
        err(tr!("argument.entity.options.limit.toosmall"))
    }
    pub fn unknown_sort(sort: &str) -> Self {
        err(tr!("argument.entity.options.sort.irreversible", sort))
    }
    pub fn invalid_selector_game_mode(mode: &str) -> Self {
        err(tr!("argument.entity.options.mode.invalid", mode))
    }
    pub fn invalid_entity_type(id: &str) -> Self {
        err(tr!("argument.entity.options.type.invalid", id))
    }
    pub fn not_single_entity() -> Self {
        err(tr!("argument.entity.toomany"))
    }
    pub fn not_single_player() -> Self {
        err(tr!("argument.player.toomany"))
    }
    pub fn only_players_allowed() -> Self {
        err(tr!("argument.player.entities"))
    }
    pub fn no_entities_found() -> Self {
        err(tr!("argument.entity.notfound.entity"))
    }
    pub fn no_players_found() -> Self {
        err(tr!("argument.entity.notfound.player"))
    }
    pub fn unknown_player() -> Self {
        err(tr!("argument.player.unknown"))
    }
    pub fn range_empty() -> Self {
        err(tr!("argument.range.empty"))
    }
    pub fn range_swapped() -> Self {
        err(tr!("argument.range.swapped"))
    }
    pub fn pos3d_incomplete() -> Self {
        err(tr!("argument.pos3d.incomplete"))
    }
    pub fn pos2d_incomplete() -> Self {
        err(tr!("argument.pos2d.incomplete"))
    }
    pub fn rotation_incomplete() -> Self {
        err(tr!("argument.rotation.incomplete"))
    }
    pub fn pos_mixed() -> Self {
        err(tr!("argument.pos.mixed"))
    }
    pub fn expected_coordinate() -> Self {
        err(tr!("argument.pos.missing.double"))
    }
    pub fn expected_block_position() -> Self {
        err(tr!("argument.pos.missing.int"))
    }
    pub fn pos_unloaded() -> Self {
        err(tr!("argument.pos.unloaded"))
    }
    pub fn pos_out_of_world() -> Self {
        err(tr!("argument.pos.outofworld"))
    }
    pub fn pos_out_of_bounds() -> Self {
        err(tr!("argument.pos.outofbounds"))
    }
    pub fn invalid_id() -> Self {
        err(tr!("argument.id.invalid"))
    }
    pub fn unknown_item(id: &str) -> Self {
        err(tr!("argument.item.id.invalid", id))
    }
    pub fn unknown_resource(id: &str, registry: &str) -> Self {
        err(tr!("argument.resource.not_found", id, registry))
    }
    pub fn unknown_dimension(id: &str) -> Self {
        err(tr!("argument.dimension.invalid", id))
    }
    pub fn invalid_anchor(name: &str) -> Self {
        err(tr!("argument.anchor.invalid", name))
    }
    pub fn invalid_game_mode(name: &str) -> Self {
        err(tr!("argument.gamemode.invalid", name))
    }
    pub fn invalid_time_unit() -> Self {
        err(tr!("argument.time.invalid_unit"))
    }
    pub fn tick_count_too_low(found: i32, min: i32) -> Self {
        err(tr!("argument.time.tick_count_too_low", min, found))
    }
    pub fn message_too_long(length: usize, max: usize) -> Self {
        err(tr!("argument.message.too_long", length as i32, max as i32))
    }
    pub fn requires_player() -> Self {
        err(tr!("permissions.requires.player"))
    }
    pub fn requires_entity() -> Self {
        err(tr!("permissions.requires.entity"))
    }
    /// `command.failed`: an unexpected error while executing.
    pub fn failed() -> Self {
        err(tr!("command.failed"))
    }
    /// `BuildContexts.ERROR_FORK_LIMIT_REACHED`.
    pub fn fork_limit(limit: usize) -> Self {
        err(tr!("command.forkLimit", limit as i32))
    }
    /// Something vanilla supports that needs a Kiln subsystem that does not exist yet
    /// (entity data, loot predicates, ...).
    pub fn unsupported(what: &str) -> Self {
        err(Text::literal(format!("{what} is not supported by this server yet")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_line_matches_vanilla_layout() {
        let e = CommandError::unknown_argument().with_context("gamemode survival Stevee", 18);
        let lines = e.chat_lines("gamemode survival Stevee");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].style.color, Some("red"));
        let ctx = &lines[1].extra[0];
        assert_eq!(ctx.style.click, Some(ClickEvent::SuggestCommand("/gamemode survival Stevee".into())));
        let parts: Vec<String> = ctx.extra.iter().map(Text::to_plain).collect();
        assert_eq!(parts, ["...", " survival ", "Stevee", "command.context.here"]);
        assert_eq!(ctx.extra[2].style.underlined, Some(true));
        assert_eq!(e.to_string(), "command.unknown.argument at position 18: ... survival <--[HERE]");
    }

    #[test]
    fn short_input_has_no_ellipsis() {
        let e = CommandError::unknown_command().with_context("foo", 0);
        let parts: Vec<String> = e.chat_lines("foo")[1].extra[0].extra.iter().map(Text::to_plain).collect();
        assert_eq!(parts, ["", "foo", "command.context.here"]);
        assert_eq!(CommandError::no_players_found().chat_lines("x").len(), 1);
    }
}
