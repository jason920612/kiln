//! Server settings for the configuration phase and transfers, as vanilla's
//! `server.properties` has them: the server resource pack (`resource-pack`,
//! `resource-pack-sha1`, `resource-pack-id`, `require-resource-pack`, `resource-pack-prompt`),
//! `accepts-transfers`, the code of conduct (`enable-code-of-conduct` with
//! `codeofconduct/<language>.txt`) and server links (`bug-report-link`).

use anyhow::{Context, Result, bail};
use kiln_proto::nbt::Tag;
use std::collections::BTreeMap;
use std::path::Path;
use tracing::warn;
use uuid::Uuid;

/// `MinecraftServer.ServerResourcePackInfo`.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerResourcePack {
    pub id: Uuid,
    pub url: String,
    /// Lowercase hex SHA-1, or empty.
    pub hash: String,
    pub required: bool,
    pub prompt: Option<Tag>,
}

/// A link in the pause menu (`ServerLinks.Entry`).
#[derive(Debug, Clone, PartialEq)]
pub enum LinkKind {
    /// `ServerLinks.KnownLinkType` ordinal (0 is the bug report link).
    Known(i32),
    Custom(Tag),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LobbyConfig {
    pub resource_pack: Option<ServerResourcePack>,
    pub accepts_transfers: bool,
    /// Code of conduct text by lowercase language code (`en_us`, ...).
    pub code_of_conduct: BTreeMap<String, String>,
    pub links: Vec<(LinkKind, String)>,
}

/// `KnownLinkType` names, by ordinal.
const KNOWN_LINKS: [&str; 10] = [
    "bug_report",
    "community_guidelines",
    "support",
    "status",
    "feedback",
    "community",
    "website",
    "forums",
    "news",
    "announcements",
];

impl LobbyConfig {
    /// From the environment until the server has a config file: `KILN_RESOURCE_PACK` (URL),
    /// `KILN_RESOURCE_PACK_SHA1`, `KILN_RESOURCE_PACK_ID` (UUID), `KILN_REQUIRE_RESOURCE_PACK`,
    /// `KILN_RESOURCE_PACK_PROMPT` (a JSON text component), `KILN_ACCEPTS_TRANSFERS`,
    /// `KILN_CODE_OF_CONDUCT` (a directory of `<language>.txt` files), `KILN_BUG_REPORT_LINK`
    /// and `KILN_SERVER_LINKS` (`name=url` pairs separated by `;`, where `name` is a known
    /// link type such as `website`, or any other label).
    pub fn from_env() -> Result<Self> {
        Self::from_vars(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
    }

    pub(crate) fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let flag = |k: &str| -> Result<bool> {
            match var(k).as_deref().map(str::trim) {
                None => Ok(false),
                Some("true" | "1" | "yes") => Ok(true),
                Some("false" | "0" | "no") => Ok(false),
                Some(v) => bail!("{k}: expected true or false, got {v:?}"),
            }
        };
        let resource_pack = match var("KILN_RESOURCE_PACK") {
            None => None,
            Some(url) => server_pack(
                url.trim(),
                var("KILN_RESOURCE_PACK_SHA1").unwrap_or_default().trim(),
                var("KILN_RESOURCE_PACK_ID").unwrap_or_default().trim(),
                flag("KILN_REQUIRE_RESOURCE_PACK")?,
                var("KILN_RESOURCE_PACK_PROMPT").unwrap_or_default().trim(),
            ),
        };
        let code_of_conduct = match var("KILN_CODE_OF_CONDUCT") {
            None => BTreeMap::new(),
            Some(dir) => read_code_of_conduct(Path::new(dir.trim()))?,
        };
        let mut links = Vec::new();
        if let Some(url) = var("KILN_BUG_REPORT_LINK") {
            links.push((LinkKind::Known(0), url.trim().to_owned()));
        }
        for pair in var("KILN_SERVER_LINKS").unwrap_or_default().split(';').filter(|p| !p.trim().is_empty()) {
            let Some((name, url)) = pair.split_once('=') else { bail!("KILN_SERVER_LINKS: expected name=url, got {pair:?}") };
            let (name, url) = (name.trim(), url.trim());
            let kind = match KNOWN_LINKS.iter().position(|k| *k == name) {
                Some(i) => LinkKind::Known(i as i32),
                None => LinkKind::Custom(Tag::String(name.to_owned())),
            };
            links.push((kind, url.to_owned()));
        }
        Ok(Self { resource_pack, accepts_transfers: flag("KILN_ACCEPTS_TRANSFERS")?, code_of_conduct, links })
    }

    /// `ServerConfigurationPacketListenerImpl.addOptionalTasks`: the text for the client's
    /// language, else `en_us`, else any.
    pub fn code_of_conduct_for(&self, language: &str) -> Option<&str> {
        let c = &self.code_of_conduct;
        c.get(&language.to_lowercase()).or_else(|| c.get("en_us")).or_else(|| c.values().next()).map(String::as_str)
    }
}

/// `DedicatedServerProperties.getServerPackInfo`.
fn server_pack(url: &str, sha1: &str, id: &str, required: bool, prompt: &str) -> Option<ServerResourcePack> {
    if url.is_empty() {
        return None;
    }
    if sha1.is_empty() {
        warn!(
            "You specified a resource pack without providing a sha1 hash. Pack will be updated on the client only if you change the name of the pack."
        );
    } else if !(sha1.len() == 40 && sha1.bytes().all(|c| c.is_ascii_hexdigit())) {
        warn!("Invalid sha1 for resource-pack-sha1");
    }
    let prompt = if prompt.is_empty() {
        None
    } else {
        match serde_json::from_str::<serde_json::Value>(prompt) {
            Ok(v) => Some(json_to_nbt(&v)),
            Err(_) => {
                warn!("Failed to parse resource pack prompt '{prompt}'");
                None
            }
        }
    };
    let id = if id.is_empty() {
        let id = name_uuid(url.as_bytes());
        warn!("resource-pack-id missing, using default of {id}");
        id
    } else {
        match Uuid::parse_str(id) {
            Ok(id) => id,
            Err(_) => {
                warn!("Failed to parse '{id}' into UUID");
                return None;
            }
        }
    };
    Some(ServerResourcePack { id, url: url.to_owned(), hash: sha1.to_owned(), required, prompt })
}

/// `UUID.nameUUIDFromBytes`: MD5 with version 3.
fn name_uuid(bytes: &[u8]) -> Uuid {
    use md5::{Digest, Md5};
    let mut h: [u8; 16] = Md5::digest(bytes).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

/// A JSON text component as network NBT.
fn json_to_nbt(v: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match v {
        Value::Null => Tag::String(String::new()),
        Value::Bool(b) => Tag::Byte(*b as i8),
        Value::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => Tag::Int(i as i32),
            Some(i) => Tag::Long(i),
            None => Tag::Double(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Tag::String(s.clone()),
        Value::Array(a) => Tag::List(a.iter().map(json_to_nbt).collect()),
        Value::Object(o) => Tag::Compound(o.iter().map(|(k, v)| (k.clone(), json_to_nbt(v))).collect()),
    }
}

/// `DedicatedServer.readCodeOfConducts`: every `<language>.txt` in the folder, lines joined
/// with `\n`, formatting codes stripped.
fn read_code_of_conduct(dir: &Path) -> Result<BTreeMap<String, String>> {
    if !dir.is_dir() {
        bail!("Code of Conduct folder does not exist: {}", dir.display());
    }
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).context("Failed to read Code of Conduct folder")? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(language) = name.strip_suffix(".txt") else { continue };
        let text = std::fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let text = text.lines().collect::<Vec<_>>().join("\n");
        out.insert(language.to_lowercase(), strip_color(&text));
    }
    Ok(out)
}

/// `StringUtil.stripColor`: removes `§` and the character after it.
fn strip_color(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(vars: &[(&str, &str)]) -> LobbyConfig {
        LobbyConfig::from_vars(|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())).unwrap()
    }

    #[test]
    fn resource_pack_settings() {
        assert_eq!(cfg(&[]), LobbyConfig::default());
        let c = cfg(&[
            ("KILN_RESOURCE_PACK", "http://example.com/pack.zip"),
            ("KILN_REQUIRE_RESOURCE_PACK", "true"),
            ("KILN_RESOURCE_PACK_PROMPT", r#"{"text":"Please","color":"gold"}"#),
            ("KILN_ACCEPTS_TRANSFERS", "true"),
        ]);
        let pack = c.resource_pack.unwrap();
        assert!(pack.required && c.accepts_transfers);
        // `UUID.nameUUIDFromBytes("http://example.com/pack.zip".getBytes(UTF_8))` in jshell.
        assert_eq!(pack.id.to_string(), "f813f402-b8ab-31f9-9a36-10f9e06ca4e4");
        assert_eq!(pack.prompt.unwrap().get("color").and_then(Tag::as_str), Some("gold"));
        let bad = cfg(&[("KILN_RESOURCE_PACK", "u"), ("KILN_RESOURCE_PACK_ID", "nope")]);
        assert_eq!(bad.resource_pack, None, "an unparsable id drops the pack, as vanilla does");
    }

    #[test]
    fn links_and_code_of_conduct() {
        let c = cfg(&[("KILN_BUG_REPORT_LINK", "https://bugs"), ("KILN_SERVER_LINKS", "website=https://w; Rules = https://r")]);
        assert_eq!(c.links[0], (LinkKind::Known(0), "https://bugs".into()));
        assert_eq!(c.links[1], (LinkKind::Known(6), "https://w".into()));
        assert_eq!(c.links[2], (LinkKind::Custom(Tag::String("Rules".into())), "https://r".into()));
        let dir = std::env::temp_dir().join(format!("kiln-coc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("en_us.txt"), "Be \u{a7}cnice\r\nline 2\n").unwrap();
        std::fs::write(dir.join("de_DE.txt"), "Sei nett").unwrap();
        let c = cfg(&[("KILN_CODE_OF_CONDUCT", dir.to_str().unwrap())]);
        assert_eq!(c.code_of_conduct_for("EN_US"), Some("Be nice\nline 2"));
        assert_eq!(c.code_of_conduct_for("de_de"), Some("Sei nett"));
        assert_eq!(c.code_of_conduct_for("fr_fr"), Some("Be nice\nline 2"), "falls back to en_us");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
