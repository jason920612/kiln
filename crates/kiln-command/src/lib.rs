//! Brigadier-compatible commands for Kiln: the command tree and its builders, Brigadier's
//! parse/execute/suggest algorithms, vanilla argument types (entity selectors, coordinates,
//! messages, ...), the clientbound commands and suggestion packets, and the built-in commands.
//!
//! The simulation owns a [`Dispatcher`] over its command source type, which implements
//! [`Source`], [`SelectorWorld`] and [`Host`]:
//!
//! ```text
//! let mut commands = Dispatcher::<MySource>::new();
//! kiln_command::vanilla::register_all(&mut commands);
//! sink.send(commands.commands_packet(permission_level));   // after login and on op changes
//! match commands.execute(&command, &mut source) {          // chat_command / chat_command_signed
//!     Ok(_) => {}
//!     Err(e) => e.chat_lines(&command).into_iter().for_each(|t| send_system(t.to_nbt())),
//! }
//! sink.send(commands.suggestions_packet(req.id, &req.command, &source)); // command_suggestion
//! ```

pub mod arguments;
pub mod blocks;
pub mod coords;
pub mod dispatcher;
pub mod error;
pub mod host;
pub mod nbt_path;
pub mod range;
pub mod reader;
pub mod scoreboard;
pub mod selector;
pub mod snbt;
pub mod suggestion;
pub mod text;
pub mod types;
pub mod vanilla;

pub use arguments::{ArgumentType, ArgumentValue, GameProfileArg, MessageArg};
pub use coords::{Coordinates, WorldCoordinate};
pub use dispatcher::{
    Builder, CommandContext, Dispatcher, NodeId, NodeKind, ParseResults, SuggestionProvider, argument, literal,
};
pub use error::CommandError;
pub use host::{
    ChatKind, ChatMessage, GameRuleValue, Host, Profile, Source, SpawnPoint, Teleport, TimeAction, Weather,
};
pub use reader::StringReader;
pub use selector::{Aabb, EntitySelector, SelectorTarget, SelectorWorld};
pub use suggestion::{Suggestion, Suggestions, SuggestionsBuilder};
pub use text::{Arg, Text};
pub use types::{Anchor, Difficulty, GameMode, Identifier, ItemInput};
