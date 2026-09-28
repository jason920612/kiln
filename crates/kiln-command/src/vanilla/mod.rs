//! Built-in commands with vanilla 26.3's tree shape (checked against the data generator's
//! `commands.json`) and feedback, plus Kiln's `/kiln`.

mod admin;
mod advancement;
mod blocks;
mod attribute;
mod bossbar;
mod chat;
mod data;
mod effect;
mod entity;
pub mod experience;
mod summon;
mod execute;
mod function;
mod items;
pub use function::{run_as_server, run_function};
pub mod gamerules;
mod players;
mod protocol;
pub mod misc;
mod scoreboard;
pub(crate) mod sound;
mod server;
mod tag;
mod team;
mod title;
mod tracker;

use crate::arguments::GameProfileArg;
use crate::dispatcher::Dispatcher;
use crate::error::CommandError;
use crate::host::{Host, Profile};
use crate::selector::SelectorTarget;

/// Permission levels as vanilla names them.
pub const LEVEL_ALL: u8 = 0;
pub const LEVEL_MODERATORS: u8 = 1;
pub const LEVEL_GAMEMASTERS: u8 = 2;
pub const LEVEL_ADMINS: u8 = 3;
pub const LEVEL_OWNERS: u8 = 4;

/// Names of the commands [`register_all`] adds.
pub const COMMANDS: &[&str] = &[
    "help",
    "teleport",
    "tp",
    "gamemode",
    "say",
    "msg",
    "tell",
    "w",
    "me",
    "list",
    "kill",
    "time",
    "weather",
    "gamerule",
    "give",
    "effect",
    "experience",
    "xp",
    "seed",
    "stop",
    "kick",
    "op",
    "deop",
    "difficulty",
    "spawnpoint",
    "setworldspawn",
    "execute",
    "setblock",
    "fill",
    "clone",
    "tellraw",
    "function",
    "return",
    "schedule",
    "reload",
    "datapack",
    "transfer",
    "dialog",
    "teammsg",
    "tm",
    "scoreboard",
    "trigger",
    "team",
    "bossbar",
    "title",
    "kiln",
    "summon",
    "advancement",
    "recipe",
    "whitelist",
    "ban",
    "ban-ip",
    "banlist",
    "pardon",
    "pardon-ip",
    "save-all",
    "save-on",
    "save-off",
    "defaultgamemode",
    "setidletimeout",
    "version",
    "debug",
    "perf",
    "jfr",
    "particle",
    "playsound",
    "stopsound",
    "stopwatch",
    "posteffect",
    "waypoint",
    // data, tag, item, loot, clear, enchant, attribute, damage, ride, rotate, spectate,
    // swing, fetchprofile
    "data",
    "tag",
    "rotate",
    "spectate",
    "swing",
    "ride",
    "damage",
    "clear",
    "enchant",
    "attribute",
];

/// Registers every built-in command.
pub fn register_all<S: Host + 'static>(d: &mut Dispatcher<S>) {
    server::help(d);
    players::teleport(d);
    players::gamemode(d);
    chat::say(d);
    chat::msg(d);
    chat::me(d);
    server::list(d);
    players::kill(d);
    server::time(d);
    server::weather(d);
    server::gamerule(d);
    players::give(d);
    effect::effect(d);
    experience::experience(d);
    summon::summon(d);
    server::seed(d);
    server::stop(d);
    players::kick(d);
    server::op(d);
    server::deop(d);
    server::difficulty(d);
    players::spawnpoint(d);
    server::setworldspawn(d);
    execute::execute(d);
    blocks::setblock(d);
    blocks::fill(d);
    blocks::clone(d);
    chat::tellraw(d);
    function::function(d);
    function::return_(d);
    function::schedule(d);
    function::reload(d);
    function::datapack(d);
    protocol::transfer(d);
    protocol::dialog(d);
    chat::teammsg(d);
    scoreboard::scoreboard(d);
    scoreboard::trigger(d);
    team::team(d);
    bossbar::bossbar(d);
    title::title(d);
    server::kiln(d);
    advancement::advancement(d);
    advancement::recipe(d);
    admin::whitelist(d);
    admin::ban(d);
    admin::ban_ip(d);
    admin::banlist(d);
    admin::pardon(d);
    admin::pardon_ip(d);
    admin::save(d);
    admin::defaultgamemode(d);
    admin::setidletimeout(d);
    admin::version(d);
    admin::profilers(d);
    sound::particle_command(d);
    sound::playsound(d);
    sound::stopsound(d);
    misc::stopwatch(d);
    misc::posteffect(d);
    misc::waypoint(d);
    // data, tag, item, loot, clear, enchant, attribute, damage, ride, rotate, spectate,
    // swing, fetchprofile
    data::data(d);
    tag::tag(d);
    entity::rotate(d);
    entity::spectate(d);
    entity::swing(d);
    entity::ride(d);
    entity::damage(d);
    items::clear(d);
    items::enchant(d);
    attribute::attribute(d);
}

/// `getEntityOrException`.
fn source_entity<S: Host>(s: &S) -> Result<S::Entity, CommandError> {
    s.source_entity().ok_or_else(CommandError::requires_entity)
}

/// `getPlayerOrException`.
fn source_player<S: Host>(s: &S) -> Result<S::Entity, CommandError> {
    s.source_entity().filter(SelectorTarget::is_player).ok_or_else(CommandError::requires_player)
}

fn profile_of<E: SelectorTarget>(e: &E) -> Profile {
    Profile { uuid: e.uuid(), name: e.name() }
}

/// `GameProfileArgument.getGameProfiles`.
fn resolve_profiles<S: Host>(arg: &GameProfileArg, s: &mut S) -> Result<Vec<Profile>, CommandError> {
    match arg {
        GameProfileArg::Selector(sel) => {
            let players = sel.find_players(s)?;
            if players.is_empty() {
                return Err(CommandError::no_players_found());
            }
            Ok(players.iter().map(profile_of).collect())
        }
        GameProfileArg::Name(name) => s.find_profile(name).map(|p| vec![p]).ok_or_else(CommandError::unknown_player),
    }
}

/// `String.format(Locale.ROOT, "%f", v)`.
fn format_double(v: f64) -> String {
    format!("{v:.6}")
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod world_tests;
