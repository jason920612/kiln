//! From `server.properties` to what the rest of Kiln reads. Kiln's own settings are `KILN_*`
//! environment variables (the test harnesses set them); the properties file provides their
//! defaults, so a variable that is set always wins over the file.

use crate::properties::ServerProperties;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The command line of vanilla's `Main`: `--nogui`, `--initSettings`, `--port <n>`, `--world
/// <name>`, `--universe <dir>`, `--pidFile <file>`, `--serverId <id>`, `--help`, and the flags
/// that only matter to Java's server (`--demo`, `--bonusChest`, `--forceUpgrade`, `--eraseCache`,
/// `--recreateRegionFiles`, `--safeMode`, `--jfrProfile`), which are accepted and ignored.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub init_settings: bool,
    pub help: bool,
    pub port: Option<i32>,
    pub world: Option<String>,
    pub universe: Option<PathBuf>,
    pub pid_file: Option<PathBuf>,
    pub ignored: Vec<String>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = Options::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let name = a.trim_start_matches('-');
            let mut value = |what: &str| it.next().cloned().ok_or_else(|| format!("Option {name} requires an argument ({what})"));
            match name {
                "initSettings" => o.init_settings = true,
                "help" | "h" | "?" => o.help = true,
                "port" => o.port = Some(value("port")?.parse().map_err(|_| "port must be a number".to_owned())?),
                "world" => o.world = Some(value("name")?),
                "universe" => o.universe = Some(PathBuf::from(value("directory")?)),
                "pidFile" => o.pid_file = Some(PathBuf::from(value("file")?)),
                "serverId" => o.ignored.push(value("id")?),
                "nogui" | "demo" | "bonusChest" | "forceUpgrade" | "eraseCache" | "recreateRegionFiles" | "safeMode" | "jfrProfile" => {
                    o.ignored.push(a.clone())
                }
                _ => return Err(format!("{a} is not a recognized option")),
            }
        }
        Ok(o)
    }
}

/// The text `--help` prints (to stderr, like joptsimple's help).
pub const HELP: &str = "Option                       Description
------                       -----------
--bonusChest                 
--demo                       
--eraseCache                 
--forceUpgrade               
--help                       
--initSettings               Initializes 'server.properties' and 'eula.txt', then quits
--jfrProfile                 
--nogui                      
--pidFile <Path>             
--port <Integer>             (default: -1)
--recreateRegionFiles        
--safeMode                   Loads level with vanilla datapack only
--serverId                   
--universe                   (default: .)
--world                      
";

/// Whether the environment configures this run the way the test harnesses do (`KILN_WORLD`,
/// `KILN_PORT` or `KILN_GENERATOR` set). With no `server.properties` such a run uses the
/// settings Kiln had before it read the file, creates neither it nor `eula.txt`, and does not ask
/// for the EULA.
pub fn harness_env(var: impl Fn(&str) -> Option<String>) -> bool {
    ["KILN_WORLD", "KILN_PORT", "KILN_GENERATOR"].iter().any(|k| var(k).is_some())
}

/// What the harnesses got before `server.properties`: offline mode, no whitelist, 100 players.
pub fn harness_defaults() -> crate::javaprops::JavaProps {
    let mut p = crate::javaprops::JavaProps::default();
    p.set("online-mode", "false");
    p.set("white-list", "false");
    p.set("max-players", "100");
    p.set("motd", "A Kiln server");
    p
}

/// The `KILN_*` defaults the properties provide.
pub fn kiln_defaults(p: &ServerProperties, options: &Options, world_dir: Option<&Path>) -> HashMap<&'static str, String> {
    let mut m = HashMap::new();
    let port = options.port.filter(|p| *p >= 0).unwrap_or(p.server_port);
    m.insert("KILN_PORT", port.to_string());
    m.insert("KILN_MAX_PLAYERS", p.max_players.max(0).to_string());
    m.insert("KILN_VIEW_DISTANCE", p.view_distance.to_string());
    m.insert("KILN_SIMULATION_DISTANCE", p.simulation_distance.to_string());
    m.insert("KILN_WHITELIST", p.white_list.to_string());
    m.insert("KILN_ENFORCE_WHITELIST", p.enforce_whitelist.to_string());
    m.insert("KILN_ONLINE_MODE", p.online_mode.to_string());
    m.insert("KILN_PREVENT_PROXY_CONNECTIONS", p.prevent_proxy_connections.to_string());
    m.insert("KILN_DIFFICULTY", ["peaceful", "easy", "normal", "hard"][p.difficulty as usize].to_owned());
    m.insert("KILN_ACCEPTS_TRANSFERS", p.accepts_transfers.to_string());
    if !p.bug_report_link.is_empty() {
        m.insert("KILN_BUG_REPORT_LINK", p.bug_report_link.clone());
    }
    if p.code_of_conduct {
        m.insert("KILN_CODE_OF_CONDUCT", "codeofconduct".to_owned());
    }
    if !p.resource_pack.is_empty() {
        m.insert("KILN_RESOURCE_PACK", p.resource_pack.clone());
        m.insert("KILN_RESOURCE_PACK_SHA1", p.resource_pack_sha1.clone());
        m.insert("KILN_RESOURCE_PACK_ID", p.resource_pack_id.clone());
        m.insert("KILN_REQUIRE_RESOURCE_PACK", p.require_resource_pack.to_string());
        m.insert("KILN_RESOURCE_PACK_PROMPT", p.resource_pack_prompt.clone());
    }
    m.insert("KILN_INITIAL_PACKS", p.initial_enabled_packs.clone());
    m.insert("KILN_INITIAL_DISABLED_PACKS", p.initial_disabled_packs.clone());
    if let Some(dir) = world_dir {
        m.insert("KILN_WORLD", dir.display().to_string());
    }
    m
}

/// `WorldOptions.parseSeed`: a number is the seed, any other text its Java `String.hashCode`,
/// empty text no seed.
pub fn parse_seed(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.parse::<i64>().unwrap_or_else(|_| i64::from(java_hash(text))))
}

fn java_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::javaprops::JavaProps;

    #[test]
    fn command_line_is_vanillas() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = Options::parse(&args(&["--nogui", "--port", "25570", "--world", "lobby", "--initSettings", "--universe", "/srv"])).unwrap();
        assert_eq!((o.port, o.world.as_deref(), o.init_settings), (Some(25570), Some("lobby"), true));
        assert_eq!(o.universe, Some(PathBuf::from("/srv")));
        assert!(Options::parse(&args(&["--bogus"])).is_err());
        assert!(Options::parse(&args(&["--port"])).is_err());
    }

    #[test]
    fn seeds_parse_like_world_options() {
        assert_eq!(parse_seed(""), None);
        assert_eq!(parse_seed("-1234"), Some(-1234));
        // "Kiln".hashCode() in Java.
        assert_eq!(parse_seed("Kiln"), Some(2_338_688));
        assert_eq!(parse_seed("9223372036854775808"), Some(i64::from(java_hash("9223372036854775808"))));
    }

    #[test]
    fn properties_feed_the_kiln_variables() {
        let p = ServerProperties::parse(JavaProps::parse("max-players=7\nwhite-list=false\ndifficulty=hard\nresource-pack=http://x/p.zip\n"), String::new).unwrap();
        let m = kiln_defaults(&p, &Options { port: Some(25570), ..Default::default() }, None);
        assert_eq!(m["KILN_PORT"], "25570");
        assert_eq!(m["KILN_MAX_PLAYERS"], "7");
        assert_eq!(m["KILN_WHITELIST"], "false");
        assert_eq!(m["KILN_DIFFICULTY"], "hard");
        assert_eq!(m["KILN_ONLINE_MODE"], "true");
        assert_eq!(m["KILN_RESOURCE_PACK"], "http://x/p.zip");
        assert!(!harness_env(|_| None));
        assert!(harness_env(|k| (k == "KILN_WORLD").then(|| "w".to_owned())));
    }
}
