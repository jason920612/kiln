//! Brigadier-compatible commands for Kiln: the command tree and its builders, Brigadier's
//! parse/execute/suggest algorithms (redirect modifiers and forks included), vanilla argument
//! types (entity selectors, coordinates, block states, SNBT, text components, ...), the
//! clientbound commands and suggestion packets, and the built-in commands.
//!
//! The simulation owns a [`Dispatcher`] over its command source type, which implements
//! [`Source`] (with the current [`SourceStack`]), [`SelectorWorld`] and [`Host`]:
//!
//! ```text
//! let mut commands = Dispatcher::<MySource>::new();
//! kiln_command::vanilla::register_all(&mut commands);
//! sink.send(commands.commands_packet(permission_level));   // after login and on op changes
//! *source.stack_mut() = SourceStack::of_entity(player);    // who runs the next command
//! match commands.execute(&command, &mut source) {          // chat_command / chat_command_signed
//!     Ok(_) => {}
//!     Err(e) => e.chat_lines(&command).into_iter().for_each(|t| send_system(t.to_nbt())),
//! }
//! sink.send(commands.suggestions_packet(req.id, &req.command, &source)); // command_suggestion
//! ```

pub mod arguments;
pub mod blocks;
pub mod bossbar;
pub mod component;
pub mod coords;
pub mod dispatcher;
pub mod error;
pub mod functions;
pub mod host;
pub mod nbt_path;
pub mod nbt_text;
pub mod range;
pub mod reader;
pub mod scoreboard;
pub mod selector;
pub mod slots;
pub mod snbt;
pub mod suggestion;
pub mod text;
pub mod types;
pub mod vanilla;

pub use arguments::{ArgumentType, ArgumentValue, GameProfileArg, MessageArg, ScoreHolderArg};
pub use blocks::{BlockInput, BlockPredicate, UpdateFlags};
pub use component::Component;
pub use coords::{Coordinates, WorldCoordinate};
pub use dispatcher::{
    Builder, CommandContext, Dispatcher, Modifier, NodeId, NodeKind, ParseResults, SuggestionProvider, argument,
    literal,
};
pub use error::CommandError;
pub use host::{
    ChatKind, ChatMessage, GameRuleValue, Host, Profile, ResultCallback, Source, SourceStack, SpawnPoint, Teleport,
    TimeAction, Weather,
};
pub use reader::StringReader;
pub use nbt_path::CommandStorage;
pub use bossbar::{BossBar, BossBars};
pub use scoreboard::{NumberFormat, Objective, ScoreAccess, Scoreboard, Team};
pub use selector::{Aabb, EntitySelector, NoEntity, SelectorTarget, SelectorWorld};
pub use suggestion::{Suggestion, Suggestions, SuggestionsBuilder};
pub use text::{Arg, Language, Text};
pub use types::{Anchor, Difficulty, GameMode, Heightmap, Identifier, ItemInput};
