//! The methods and notifications the management server has, with their parameter and result schemas:
//! what `rpc.discover` answers (an OpenRPC document) and what the dispatcher checks parameters against.

use super::json::J;

/// A schema (`Schema`): a primitive or a reference to one of the components.
#[derive(Clone, Copy)]
pub enum S {
    Bool,
    Int,
    Num,
    Str,
    Ref(&'static str),
    ArrRef(&'static str),
    ArrStr,
}

impl S {
    fn json(self) -> J {
        let r = |name: &str| J::obj(vec![("$ref", J::str(format!("#/components/schemas/{name}")))]);
        match self {
            S::Bool => J::obj(vec![("type", J::str("boolean"))]),
            S::Int => J::obj(vec![("type", J::str("integer"))]),
            S::Num => J::obj(vec![("type", J::str("number"))]),
            S::Str => J::obj(vec![("type", J::str("string"))]),
            S::Ref(n) => r(n),
            S::ArrRef(n) => J::obj(vec![("type", J::str("array")), ("items", r(n))]),
            S::ArrStr => J::obj(vec![("type", J::str("array")), ("items", J::obj(vec![("type", J::str("string"))]))]),
        }
    }
}

pub struct Def {
    /// Without the `minecraft:` namespace (and for notifications, without `notification/`).
    pub name: &'static str,
    pub desc: &'static str,
    /// The one parameter's name and schema.
    pub param: Option<(&'static str, S)>,
    /// The result's name and schema (notifications have none).
    pub result: Option<(&'static str, S)>,
}

const fn m(name: &'static str, desc: &'static str, param: Option<(&'static str, S)>, result: (&'static str, S)) -> Def {
    Def { name, desc, param, result: Some(result) }
}

const fn n(name: &'static str, desc: &'static str, param: Option<(&'static str, S)>) -> Def {
    Def { name, desc, param, result: None }
}

use S::{ArrRef, ArrStr, Bool, Int, Num, Ref, Str};

pub static INCOMING: &[Def] = &[
    m("allowlist", "Get the allowlist", None, ("allowlist", ArrRef("player"))),
    m("allowlist/set", "Set the allowlist", Some(("players", ArrRef("player"))), ("allowlist", ArrRef("player"))),
    m("allowlist/add", "Add players to allowlist", Some(("add", ArrRef("player"))), ("allowlist", ArrRef("player"))),
    m("allowlist/remove", "Remove players from allowlist", Some(("remove", ArrRef("player"))), ("allowlist", ArrRef("player"))),
    m("allowlist/clear", "Clear all players in allowlist", None, ("allowlist", ArrRef("player"))),
    m("bans", "Get the ban list", None, ("banlist", ArrRef("user_ban"))),
    m("bans/set", "Set the banlist", Some(("bans", ArrRef("user_ban"))), ("banlist", ArrRef("user_ban"))),
    m("bans/add", "Add players to ban list", Some(("add", ArrRef("user_ban"))), ("banlist", ArrRef("user_ban"))),
    m("bans/remove", "Remove players from ban list", Some(("remove", ArrRef("player"))), ("banlist", ArrRef("user_ban"))),
    m("bans/clear", "Clear all players in ban list", None, ("banlist", ArrRef("user_ban"))),
    m("ip_bans", "Get the ip ban list", None, ("banlist", ArrRef("ip_ban"))),
    m("ip_bans/set", "Set the ip banlist", Some(("banlist", ArrRef("ip_ban"))), ("banlist", ArrRef("ip_ban"))),
    m("ip_bans/add", "Add ip to ban list", Some(("add", ArrRef("incoming_ip_ban"))), ("banlist", ArrRef("ip_ban"))),
    m("ip_bans/remove", "Remove ip from ban list", Some(("ip", ArrStr)), ("banlist", ArrRef("ip_ban"))),
    m("ip_bans/clear", "Clear all ips in ban list", None, ("banlist", ArrRef("ip_ban"))),
    m("players", "Get all connected players", None, ("players", ArrRef("player"))),
    m("players/kick", "Kick players", Some(("kick", ArrRef("kick_player"))), ("kicked", ArrRef("player"))),
    m("operators", "Get all oped players", None, ("operators", ArrRef("operator"))),
    m("operators/set", "Set all oped players", Some(("operators", ArrRef("operator"))), ("operators", ArrRef("operator"))),
    m("operators/add", "Op players", Some(("add", ArrRef("operator"))), ("operators", ArrRef("operator"))),
    m("operators/remove", "Deop players", Some(("remove", ArrRef("player"))), ("operators", ArrRef("operator"))),
    m("operators/clear", "Deop all players", None, ("operators", ArrRef("operator"))),
    m("server/status", "Get server status", None, ("status", Ref("server_state"))),
    m("server/save", "Save server state", Some(("flush", Bool)), ("saving", Bool)),
    m("server/stop", "Stop server", None, ("stopping", Bool)),
    m("server/system_message", "Send a system message", Some(("message", Ref("system_message"))), ("sent", Bool)),
    m("serversettings/autosave", "Get whether automatic world saving is enabled on the server", None, ("enabled", Bool)),
    m("serversettings/autosave/set", "Enable or disable automatic world saving on the server", Some(("enable", Bool)), ("enabled", Bool)),
    m("serversettings/difficulty", "Get the current difficulty level of the server", None, ("difficulty", Ref("difficulty"))),
    m("serversettings/difficulty/set", "Set the difficulty level of the server", Some(("difficulty", Ref("difficulty"))), ("difficulty", Ref("difficulty"))),
    m("serversettings/enforce_allowlist", "Get whether allowlist enforcement is enabled (kicks players immediately when removed from allowlist)", None, ("enforced", Bool)),
    m("serversettings/enforce_allowlist/set", "Enable or disable allowlist enforcement (when enabled, players are kicked immediately upon removal from allowlist)", Some(("enforce", Bool)), ("enforced", Bool)),
    m("serversettings/use_allowlist", "Get whether the allowlist is enabled on the server", None, ("used", Bool)),
    m("serversettings/use_allowlist/set", "Enable or disable the allowlist on the server (controls whether only allowlisted players can join)", Some(("use", Bool)), ("used", Bool)),
    m("serversettings/max_players", "Get the maximum number of players allowed to connect to the server", None, ("max", Int)),
    m("serversettings/max_players/set", "Set the maximum number of players allowed to connect to the server", Some(("max", Int)), ("max", Int)),
    m("serversettings/pause_when_empty_seconds", "Get the number of seconds before the game is automatically paused when no players are online", None, ("seconds", Int)),
    m("serversettings/pause_when_empty_seconds/set", "Set the number of seconds before the game is automatically paused when no players are online", Some(("seconds", Int)), ("seconds", Int)),
    m("serversettings/player_idle_timeout", "Get the number of seconds before idle players are automatically kicked from the server", None, ("seconds", Int)),
    m("serversettings/player_idle_timeout/set", "Set the number of seconds before idle players are automatically kicked from the server", Some(("seconds", Int)), ("seconds", Int)),
    m("serversettings/allow_flight", "Get whether flight is allowed for players in Survival mode", None, ("allowed", Bool)),
    m("serversettings/allow_flight/set", "Allow or disallow flight for players in Survival mode", Some(("allow", Bool)), ("allowed", Bool)),
    m("serversettings/motd", "Get the server's message of the day displayed to players", None, ("message", Str)),
    m("serversettings/motd/set", "Set the server's message of the day displayed to players", Some(("message", Str)), ("message", Str)),
    m("serversettings/spawn_protection_radius", "Get the spawn protection radius in blocks (only operators can edit within this area)", None, ("radius", Int)),
    m("serversettings/spawn_protection_radius/set", "Set the spawn protection radius in blocks (only operators can edit within this area)", Some(("radius", Int)), ("radius", Int)),
    m("serversettings/force_game_mode", "Get whether players are forced to use the server's default game mode", None, ("forced", Bool)),
    m("serversettings/force_game_mode/set", "Enable or disable forcing players to use the server's default game mode", Some(("force", Bool)), ("forced", Bool)),
    m("serversettings/game_mode", "Get the server's default game mode", None, ("mode", Ref("game_type"))),
    m("serversettings/game_mode/set", "Set the server's default game mode", Some(("mode", Ref("game_type"))), ("mode", Ref("game_type"))),
    m("serversettings/view_distance", "Get the server's view distance in chunks", None, ("distance", Int)),
    m("serversettings/view_distance/set", "Set the server's view distance in chunks", Some(("distance", Int)), ("distance", Int)),
    m("serversettings/simulation_distance", "Get the server's simulation distance in chunks", None, ("distance", Int)),
    m("serversettings/simulation_distance/set", "Set the server's simulation distance in chunks", Some(("distance", Int)), ("distance", Int)),
    m("serversettings/accept_transfers", "Get whether the server accepts player transfers from other servers", None, ("accepted", Bool)),
    m("serversettings/accept_transfers/set", "Enable or disable accepting player transfers from other servers", Some(("accept", Bool)), ("accepted", Bool)),
    m("serversettings/status_heartbeat_interval", "Get the interval in seconds between server status heartbeats", None, ("seconds", Int)),
    m("serversettings/status_heartbeat_interval/set", "Set the interval in seconds between server status heartbeats", Some(("seconds", Int)), ("seconds", Int)),
    m("serversettings/operator_user_permission_level", "Get default operator permission level", None, ("level", Int)),
    m("serversettings/operator_user_permission_level/set", "Set default operator permission level", Some(("level", Int)), ("level", Int)),
    m("serversettings/hide_online_players", "Get whether the server hides online player information from status queries", None, ("hidden", Bool)),
    m("serversettings/hide_online_players/set", "Enable or disable hiding online player information from status queries", Some(("hide", Bool)), ("hidden", Bool)),
    m("serversettings/status_replies", "Get whether the server responds to connection status requests", None, ("enabled", Bool)),
    m("serversettings/status_replies/set", "Enable or disable the server responding to connection status requests", Some(("enable", Bool)), ("enabled", Bool)),
    m("serversettings/entity_broadcast_range", "Get the entity broadcast range as a percentage", None, ("percentage_points", Int)),
    m("serversettings/entity_broadcast_range/set", "Set the entity broadcast range as a percentage", Some(("percentage_points", Int)), ("percentage_points", Int)),
    m("gamerules", "Get the available game rule keys and their current values", None, ("gamerules", ArrRef("typed_game_rule"))),
    m("gamerules/update", "Update game rule value", Some(("gamerule", Ref("untyped_game_rule"))), ("gamerule", Ref("typed_game_rule"))),
];

pub static OUTGOING: &[Def] = &[
    n("server/started", "Server started", None),
    n("server/stopping", "Server shutting down", None),
    n("server/saving", "Server save started", None),
    n("server/saved", "Server save completed", None),
    n("server/activity", "Server activity occurred. Rate limited to 1 notification per 30 seconds", None),
    n("world/upgrade_started", "World upgrade started", None),
    n("world/upgrade_progress", "World upgrade progress. Rate limited to 1 notification per second", Some(("progress", Num))),
    n("world/upgrade_finished", "World upgrade finished", None),
    n("world/upgrade_failed", "World upgrade failed", Some(("reason", Str))),
    n("players/joined", "Player joined", Some(("player", Ref("player")))),
    n("players/left", "Player left", Some(("player", Ref("player")))),
    n("operators/added", "Player was oped", Some(("player", Ref("operator")))),
    n("operators/removed", "Player was deoped", Some(("player", Ref("operator")))),
    n("allowlist/added", "Player was added to allowlist", Some(("player", Ref("player")))),
    n("allowlist/removed", "Player was removed from allowlist", Some(("player", Ref("player")))),
    n("ip_bans/added", "Ip was added to ip ban list", Some(("player", Ref("ip_ban")))),
    n("ip_bans/removed", "Ip was removed from ip ban list", Some(("player", Str))),
    n("bans/added", "Player was added to ban list", Some(("player", Ref("user_ban")))),
    n("bans/removed", "Player was removed from ban list", Some(("player", Ref("player")))),
    n("gamerules/updated", "Gamerule was changed", Some(("gamerule", Ref("typed_game_rule")))),
    n("server/status", "Server status heartbeat, including before the server has spun up", Some(("status", Ref("server_state")))),
];

/// The component schemas, in the order the vanilla server lists them (the order of its hash maps).
const COMPONENTS: &str = r##"{"schemas":{"untyped_game_rule":{"type":"object","properties":{"value":{"type":["boolean","integer"]},"key":{"type":"string"}}},"incoming_ip_ban":{"type":"object","properties":{"reason":{"type":"string"},"expires":{"type":"string"},"ip":{"type":"string"},"source":{"type":"string"},"player":{"$ref":"#/components/schemas/player"}}},"system_message":{"type":"object","properties":{"receivingPlayers":{"type":"array","items":{"$ref":"#/components/schemas/player"}},"overlay":{"type":"boolean"},"message":{"$ref":"#/components/schemas/message"}}},"kick_player":{"type":"object","properties":{"message":{"$ref":"#/components/schemas/message"},"player":{"$ref":"#/components/schemas/player"}}},"game_type":{"type":"string","enum":["survival","creative","adventure","spectator"]},"message":{"type":"object","properties":{"translatable":{"type":"string"},"translatableParams":{"type":"array","items":{"type":"string"}},"literal":{"type":"string"}}},"version":{"type":"object","properties":{"protocol":{"type":"integer"},"name":{"type":"string"}}},"operator":{"type":"object","properties":{"permissionLevel":{"type":"integer"},"bypassesPlayerLimit":{"type":"boolean"},"player":{"$ref":"#/components/schemas/player"}}},"difficulty":{"type":"string","enum":["peaceful","easy","normal","hard"]},"ip_ban":{"type":"object","properties":{"reason":{"type":"string"},"expires":{"type":"string"},"ip":{"type":"string"},"source":{"type":"string"}}},"typed_game_rule":{"type":"object","properties":{"type":{"type":"string","enum":["integer","boolean"]},"value":{"type":["boolean","integer"]},"key":{"type":"string"}}},"user_ban":{"type":"object","properties":{"reason":{"type":"string"},"expires":{"type":"string"},"source":{"type":"string"},"player":{"$ref":"#/components/schemas/player"}}},"server_state":{"type":"object","properties":{"players":{"type":"array","items":{"$ref":"#/components/schemas/player"}},"started":{"type":"boolean"},"version":{"$ref":"#/components/schemas/version"}}},"player":{"type":"object","properties":{"name":{"type":"string"},"id":{"type":"string"}}}}}"##;

pub fn find(name: &str) -> Option<&'static Def> {
    INCOMING.iter().find(|d| d.name == name)
}

/// The OpenRPC document `rpc.discover` returns (`DiscoveryService`).
pub fn discover() -> J {
    let entry = |d: &Def, prefix: &str| {
        let params: Vec<J> = d
            .param
            .iter()
            .map(|(name, s)| J::obj(vec![("name", J::str(*name)), ("schema", s.json()), ("required", J::Bool(true))]))
            .collect();
        let mut members = vec![("name", J::str(format!("minecraft:{prefix}{}", d.name))), ("description", J::str(d.desc)), ("params", J::Arr(params))];
        if let Some((rname, s)) = &d.result {
            members.push(("result", J::obj(vec![("name", J::str(*rname)), ("schema", s.json())])));
        }
        J::obj(members)
    };
    let methods: Vec<J> = INCOMING.iter().map(|d| entry(d, "")).chain(OUTGOING.iter().map(|d| entry(d, "notification/"))).collect();
    J::obj(vec![
        ("openrpc", J::str("1.3.2")),
        ("info", J::obj(vec![("title", J::str("Minecraft Server JSON-RPC")), ("version", J::str("3.1.0"))])),
        ("methods", J::Arr(methods)),
        ("components", super::json::parse(COMPONENTS).expect("components")),
    ])
}
