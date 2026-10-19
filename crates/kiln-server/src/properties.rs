//! `server.properties` as vanilla 26.3's `DedicatedServerProperties` reads and writes it: every
//! key with its default and parser, written back normalized (a bad number becomes the default,
//! a clamped one the clamped value), sorted by key, unknown keys kept.

use crate::javaprops::JavaProps;
use std::path::Path;
use rand_core::RngCore;
use std::time::SystemTime;
use tracing::{error, info, warn};

/// `DedicatedServerProperties` and its `Settings` base, field by field.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerProperties {
    raw: JavaProps,
    pub online_mode: bool,
    pub prevent_proxy_connections: bool,
    pub server_ip: String,
    pub allow_flight: bool,
    pub motd: String,
    pub code_of_conduct: bool,
    pub bug_report_link: String,
    pub force_gamemode: bool,
    pub enforce_whitelist: bool,
    /// `Difficulty` id: 0 peaceful .. 3 hard.
    pub difficulty: u8,
    /// `GameType` id: 0 survival .. 3 spectator.
    pub gamemode: u8,
    pub level_name: String,
    pub server_port: i32,
    pub management_server_enabled: bool,
    pub management_server_host: String,
    pub management_server_port: i32,
    pub management_server_secret: String,
    pub management_server_tls_enabled: bool,
    pub management_server_tls_keystore: String,
    pub management_server_tls_keystore_password: String,
    pub management_server_allowed_origins: String,
    /// The legacy `announce-player-achievements`: read once, then removed from the file.
    pub announce_player_achievements: Option<bool>,
    pub enable_query: bool,
    pub query_port: i32,
    pub enable_rcon: bool,
    pub rcon_port: i32,
    pub rcon_password: String,
    pub hardcore: bool,
    pub use_native_transport: bool,
    pub spawn_protection: i32,
    /// `op-permission-level`: 0..=4.
    pub op_permission_level: u8,
    pub function_permission_level: u8,
    /// Milliseconds; zero or less turns the watchdog off.
    pub max_tick_time: i64,
    pub max_chained_neighbor_updates: i32,
    pub rate_limit: i32,
    pub command_spam_threshold_seconds: i32,
    pub chat_spam_threshold_seconds: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub max_players: i32,
    pub network_compression_threshold: i32,
    pub broadcast_rcon_to_ops: bool,
    pub broadcast_console_to_ops: bool,
    pub max_world_size: i32,
    pub sync_chunk_writes: bool,
    pub region_file_compression: String,
    pub enable_jmx_monitoring: bool,
    pub enable_status: bool,
    pub hide_online_players: bool,
    pub entity_broadcast_range_percentage: i32,
    pub text_filtering_config: String,
    pub text_filtering_version: i32,
    pub player_idle_timeout: i32,
    pub status_heartbeat_interval: i32,
    pub white_list: bool,
    pub enforce_secure_profile: bool,
    pub log_ips: bool,
    pub pause_when_empty_seconds: i32,
    pub accepts_transfers: bool,
    pub level_seed: String,
    pub generate_structures: bool,
    pub generator_settings: String,
    /// Lowercased, as vanilla stores it.
    pub level_type: String,
    pub resource_pack_id: String,
    pub resource_pack: String,
    pub resource_pack_sha1: String,
    pub require_resource_pack: bool,
    pub resource_pack_prompt: String,
    pub initial_enabled_packs: String,
    pub initial_disabled_packs: String,
}

/// A value vanilla would not survive: `entity-broadcast-range-percentage=abc` throws in its
/// settings (only the plain numbers fall back to the default).
#[derive(Debug, thiserror::Error)]
#[error("server.properties: {key}: {value:?} is not a number")]
pub struct PropertyError {
    pub key: &'static str,
    pub value: String,
}

/// The reader behind `Settings.get`/`getMutable`: the raw value (or the default when it is
/// missing or does not parse) is written back as its canonical text.
struct Reader {
    raw: JavaProps,
}

impl Reader {
    fn put(&mut self, key: &str, text: String) {
        self.raw.set(key, text);
    }

    fn string(&mut self, key: &str, default: &str) -> String {
        let v = self.raw.get(key).map_or_else(|| default.to_owned(), str::to_owned);
        self.put(key, v.clone());
        v
    }

    fn boolean(&mut self, key: &str, default: bool) -> bool {
        // `Boolean.valueOf`: anything but "true" (any case) is false.
        let v = self.raw.get(key).map_or(default, |s| s.eq_ignore_ascii_case("true"));
        self.put(key, v.to_string());
        v
    }

    fn int(&mut self, key: &str, default: i32) -> i32 {
        let v = self.raw.get(key).and_then(java_parse_int).unwrap_or(default);
        self.put(key, v.to_string());
        v
    }

    fn long(&mut self, key: &str, default: i64) -> i64 {
        let v = self.raw.get(key).and_then(|s| s.strip_prefix('+').unwrap_or(s).parse::<i64>().ok()).unwrap_or(default);
        self.put(key, v.to_string());
        v
    }

    /// `get(key, UnaryOperator, default)`: the parsed number, clamped.
    fn clamped_int(&mut self, key: &str, lo: i32, hi: i32, default: i32) -> i32 {
        let v = self.raw.get(key).and_then(java_parse_int).map_or(default, |v| v.clamp(lo, hi));
        self.put(key, v.to_string());
        v
    }

    /// `getLegacy*`: read once and removed from the file.
    fn legacy(&mut self, key: &str) -> Option<String> {
        self.raw.remove(key)
    }
}

/// `Integer.parseInt`: an optional sign and decimal digits, nothing else.
fn java_parse_int(s: &str) -> Option<i32> {
    s.parse::<i32>().ok()
}

const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];
const GAME_TYPES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];

impl ServerProperties {
    /// `DedicatedServerProperties(Properties)`. `secret` makes the management server secret that a
    /// missing key gets (vanilla draws 40 random letters and digits).
    pub fn parse(raw: JavaProps, secret: impl FnOnce() -> String) -> Result<Self, PropertyError> {
        let mut r = Reader { raw };
        let online_mode = r.boolean("online-mode", true);
        let prevent_proxy_connections = r.boolean("prevent-proxy-connections", false);
        let server_ip = r.string("server-ip", "");
        let allow_flight = r.boolean("allow-flight", false);
        let motd = r.string("motd", "A Minecraft Server");
        let code_of_conduct = r.boolean("enable-code-of-conduct", false);
        let bug_report_link = r.string("bug-report-link", "");
        let force_gamemode = r.boolean("force-gamemode", false);
        let enforce_whitelist = r.boolean("enforce-whitelist", false);
        // dispatchNumberOrString: a number is an id (difficulty wraps around, game type falls back
        // to survival), anything else a name (unknown difficulties are easy, unknown modes survival).
        let difficulty = match r.raw.get("difficulty").map(str::to_owned) {
            Some(s) => match java_parse_int(&s) {
                Some(id) => id.rem_euclid(4) as u8,
                None => DIFFICULTIES.iter().position(|d| *d == s).unwrap_or(1) as u8,
            },
            None => 1,
        };
        r.put("difficulty", DIFFICULTIES[difficulty as usize].to_owned());
        let gamemode = match r.raw.get("gamemode").map(str::to_owned) {
            Some(s) => match java_parse_int(&s) {
                Some(id) if (0..4).contains(&id) => id as u8,
                Some(_) => 0,
                None => GAME_TYPES.iter().position(|g| *g == s).unwrap_or(0) as u8,
            },
            None => 0,
        };
        r.put("gamemode", GAME_TYPES[gamemode as usize].to_owned());
        let level_name = r.string("level-name", "world");
        let server_port = r.int("server-port", 25565);
        let management_server_enabled = r.boolean("management-server-enabled", false);
        let management_server_host = r.string("management-server-host", "localhost");
        let management_server_port = r.int("management-server-port", 0);
        let management_server_secret = if r.raw.get("management-server-secret").is_some() {
            r.string("management-server-secret", "")
        } else {
            r.string("management-server-secret", &secret())
        };
        let management_server_tls_enabled = r.boolean("management-server-tls-enabled", true);
        let management_server_tls_keystore = r.string("management-server-tls-keystore", "");
        let management_server_tls_keystore_password = r.string("management-server-tls-keystore-password", "");
        let management_server_allowed_origins = r.string("management-server-allowed-origins", "");
        let announce_player_achievements = r.legacy("announce-player-achievements").map(|s| s.eq_ignore_ascii_case("true"));
        let enable_query = r.boolean("enable-query", false);
        let query_port = r.int("query.port", 25565);
        let enable_rcon = r.boolean("enable-rcon", false);
        let rcon_port = r.int("rcon.port", 25575);
        let rcon_password = r.string("rcon.password", "");
        let hardcore = r.boolean("hardcore", false);
        let use_native_transport = r.boolean("use-native-transport", true);
        let spawn_protection = r.int("spawn-protection", 16);
        // deserializePermission: a number clamped to a level; text that is no number is the default.
        let permission = |r: &mut Reader, key: &str, default: u8| {
            let v = r.raw.get(key).and_then(java_parse_int).map_or(default, |v| v.clamp(0, 4) as u8);
            r.put(key, v.to_string());
            v
        };
        let op_permission_level = permission(&mut r, "op-permission-level", 4);
        let function_permission_level = permission(&mut r, "function-permission-level", 2);
        let max_tick_time = r.long("max-tick-time", 60_000);
        let max_chained_neighbor_updates = r.int("max-chained-neighbor-updates", 1_000_000);
        let rate_limit = r.int("rate-limit", 0);
        let command_spam_threshold_seconds = r.int("command-spam-threshold-seconds", 10);
        let chat_spam_threshold_seconds = r.int("chat-spam-threshold-seconds", 10);
        let view_distance = r.int("view-distance", 10);
        let simulation_distance = r.int("simulation-distance", 10);
        let max_players = r.int("max-players", 20);
        let network_compression_threshold = r.int("network-compression-threshold", 256);
        let broadcast_rcon_to_ops = r.boolean("broadcast-rcon-to-ops", true);
        let broadcast_console_to_ops = r.boolean("broadcast-console-to-ops", true);
        let max_world_size = r.clamped_int("max-world-size", 1, 29_999_984, 29_999_984);
        let sync_chunk_writes = r.boolean("sync-chunk-writes", true);
        let region_file_compression = r.string("region-file-compression", "deflate");
        let enable_jmx_monitoring = r.boolean("enable-jmx-monitoring", false);
        let enable_status = r.boolean("enable-status", true);
        let hide_online_players = r.boolean("hide-online-players", false);
        // The one number that is not forgiven: its parser has no NumberFormatException guard.
        let entity_broadcast_range_percentage = match r.raw.get("entity-broadcast-range-percentage").map(str::to_owned) {
            Some(s) => java_parse_int(&s)
                .map(|v| v.clamp(10, 1000))
                .ok_or(PropertyError { key: "entity-broadcast-range-percentage", value: s })?,
            None => 100,
        };
        r.put("entity-broadcast-range-percentage", entity_broadcast_range_percentage.to_string());
        let text_filtering_config = r.string("text-filtering-config", "");
        let text_filtering_version = r.int("text-filtering-version", 0);
        let player_idle_timeout = r.int("player-idle-timeout", 0);
        let status_heartbeat_interval = r.int("status-heartbeat-interval", 0);
        let white_list = r.boolean("white-list", true);
        let enforce_secure_profile = r.boolean("enforce-secure-profile", true);
        let log_ips = r.boolean("log-ips", true);
        let pause_when_empty_seconds = r.int("pause-when-empty-seconds", 60);
        let accepts_transfers = r.boolean("accepts-transfers", false);
        let level_seed = r.string("level-seed", "");
        let generate_structures = r.boolean("generate-structures", true);
        let generator_settings = r.string("generator-settings", "{}");
        let level_type = r.string("level-type", "minecraft:normal").to_lowercase();
        r.put("level-type", level_type.clone());
        let resource_pack_id = r.string("resource-pack-id", "");
        let resource_pack = r.string("resource-pack", "");
        let mut resource_pack_sha1 = r.string("resource-pack-sha1", "");
        // The deprecated hash: read and dropped from the file; it counts when the sha1 is empty.
        let legacy_hash = r.legacy("resource-pack-hash");
        if !resource_pack.is_empty() {
            if !resource_pack_sha1.is_empty() {
                if legacy_hash.as_deref().is_some_and(|h| !h.is_empty()) {
                    warn!("resource-pack-hash is deprecated and found along side resource-pack-sha1. resource-pack-hash will be ignored.");
                }
            } else if let Some(h) = legacy_hash.as_deref().filter(|h| !h.is_empty()) {
                warn!("resource-pack-hash is deprecated. Please use resource-pack-sha1 instead.");
                resource_pack_sha1 = h.to_owned();
            }
        }
        let require_resource_pack = r.boolean("require-resource-pack", false);
        let resource_pack_prompt = r.string("resource-pack-prompt", "");
        let initial_enabled_packs = r.string("initial-enabled-packs", "vanilla");
        let initial_disabled_packs = r.string("initial-disabled-packs", "");
        Ok(Self {
            raw: r.raw,
            online_mode,
            prevent_proxy_connections,
            server_ip,
            allow_flight,
            motd,
            code_of_conduct,
            bug_report_link,
            force_gamemode,
            enforce_whitelist,
            difficulty,
            gamemode,
            level_name,
            server_port,
            management_server_enabled,
            management_server_host,
            management_server_port,
            management_server_secret,
            management_server_tls_enabled,
            management_server_tls_keystore,
            management_server_tls_keystore_password,
            management_server_allowed_origins,
            announce_player_achievements,
            enable_query,
            query_port,
            enable_rcon,
            rcon_port,
            rcon_password,
            hardcore,
            use_native_transport,
            spawn_protection,
            op_permission_level,
            function_permission_level,
            max_tick_time,
            max_chained_neighbor_updates,
            rate_limit,
            command_spam_threshold_seconds,
            chat_spam_threshold_seconds,
            view_distance,
            simulation_distance,
            max_players,
            network_compression_threshold,
            broadcast_rcon_to_ops,
            broadcast_console_to_ops,
            max_world_size,
            sync_chunk_writes,
            region_file_compression,
            enable_jmx_monitoring,
            enable_status,
            hide_online_players,
            entity_broadcast_range_percentage,
            text_filtering_config,
            text_filtering_version,
            player_idle_timeout,
            status_heartbeat_interval,
            white_list,
            enforce_secure_profile,
            log_ips,
            pause_when_empty_seconds,
            accepts_transfers,
            level_seed,
            generate_structures,
            generator_settings,
            level_type,
            resource_pack_id,
            resource_pack,
            resource_pack_sha1,
            require_resource_pack,
            resource_pack_prompt,
            initial_enabled_packs,
            initial_disabled_packs,
        })
    }

    /// `Settings.loadFromFile` + `DedicatedServerProperties`: UTF-8, else ISO-8859-1; a file that
    /// cannot be read is logged and read as empty (all defaults).
    pub fn load(path: &Path) -> Result<Self, PropertyError> {
        let raw = match std::fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => JavaProps::parse(&text),
                Err(e) => {
                    info!("Failed to load properties as UTF-8 from file {}, trying ISO_8859_1", path.display());
                    let text: String = e.into_bytes().into_iter().map(char::from).collect();
                    JavaProps::parse(&text)
                }
            },
            Err(e) => {
                error!("Failed to load properties from file: {}: {e}", path.display());
                JavaProps::default()
            }
        };
        Self::parse(raw, generate_secret_key)
    }

    /// The entries as `store` writes them (normalized values, unknown keys kept).
    pub fn raw(&self) -> &JavaProps {
        &self.raw
    }

    /// Replaces the entry of `key` (a typed field must be updated by the caller as well).
    pub fn set_raw(&mut self, key: &str, value: impl Into<String>) {
        self.raw.set(key, value);
    }

    /// `Settings.store`: writes `Minecraft server properties`, the date, the sorted entries.
    pub fn store(&self, path: &Path) {
        let text = self.raw.store("Minecraft server properties", SystemTime::now());
        if let Err(e) = std::fs::write(path, text) {
            error!("Failed to store properties to file: {}: {e}", path.display());
        }
    }
}

/// `SecurityConfig.generateSecretKey`: 40 random letters and digits.
pub fn generate_secret_key() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 40];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ServerProperties {
        ServerProperties::parse(JavaProps::parse(text), || "S".repeat(40)).unwrap()
    }

    #[test]
    fn defaults_are_vanillas() {
        let p = parse("");
        assert!(p.online_mode && p.white_list && p.enforce_secure_profile && p.enable_status && p.log_ips);
        assert_eq!((p.server_port, p.max_players, p.view_distance, p.simulation_distance), (25565, 20, 10, 10));
        assert_eq!((p.difficulty, p.gamemode, p.op_permission_level, p.function_permission_level), (1, 0, 4, 2));
        assert_eq!((p.max_tick_time, p.pause_when_empty_seconds, p.spawn_protection), (60_000, 60, 16));
        assert_eq!(p.motd, "A Minecraft Server");
        assert_eq!(p.level_type, "minecraft:normal");
        assert_eq!(p.raw().get("management-server-secret"), Some("S".repeat(40).as_str()));
    }

    #[test]
    fn bad_values_fall_back_and_are_normalized() {
        let p = parse(
            "server-port=abc\nmax-players= 5\nview-distance=+12\ndifficulty=HARD\ngamemode=7\nop-permission-level=9\n\
             max-world-size=0\nonline-mode=yes\nlevel-type=MineCraft:FLAT\nannounce-player-achievements=true\nfoo=bar\n",
        );
        assert_eq!(p.server_port, 25565, "unparsable number: default");
        assert_eq!(p.max_players, 20, "trailing space: not a number");
        assert_eq!(p.view_distance, 12);
        assert_eq!(p.difficulty, 1, "names are case sensitive, unknown is easy");
        assert_eq!(p.gamemode, 0, "an id out of range is survival");
        assert_eq!(p.op_permission_level, 4, "clamped");
        assert_eq!(p.max_world_size, 1);
        assert!(!p.online_mode, "Boolean.valueOf(\"yes\") is false");
        assert_eq!(p.level_type, "minecraft:flat");
        assert_eq!(p.announce_player_achievements, Some(true));
        let raw = p.raw();
        assert_eq!(raw.get("server-port"), Some("25565"));
        assert_eq!(raw.get("difficulty"), Some("easy"));
        assert_eq!(raw.get("announce-player-achievements"), None, "the legacy key is dropped");
        assert_eq!(raw.get("foo"), Some("bar"), "unknown keys stay");
        assert!(ServerProperties::parse(JavaProps::parse("entity-broadcast-range-percentage=abc"), String::new).is_err());
        assert_eq!(parse("entity-broadcast-range-percentage=5000").entity_broadcast_range_percentage, 1000);
    }

    #[test]
    fn numbers_name_the_enums() {
        assert_eq!(parse("difficulty=5").difficulty, 1, "ids wrap around");
        assert_eq!(parse("difficulty=0").difficulty, 0);
        assert_eq!(parse("gamemode=creative").gamemode, 1);
        assert_eq!(parse("gamemode=3").gamemode, 3);
    }
}
