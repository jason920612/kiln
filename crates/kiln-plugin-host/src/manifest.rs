//! `plugin.toml`: identity, capabilities, event subscriptions with their failure policies,
//! and the plugin's configuration (design §11.4, §11.5).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

/// What a plugin may link against. `state` and `log` are always linked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// `kiln:api/chat`: send chat to players.
    PlayerMessage,
    /// Commands returned by `global-hooks.init` are registered.
    CommandRegister,
    /// WASI preopens the plugin's own data directory (`/data`).
    FsData,
}

impl Capability {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "player.message" => Capability::PlayerMessage,
            "command.register" => Capability::CommandRegister,
            "fs.data" => Capability::FsData,
            // Declared in the design but not in this slice's WIT: refuse rather than ignore.
            other => bail!("unknown or unsupported capability `{other}`"),
        })
    }
}

/// Events a plugin subscribes to. Only subscribed events cross the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    BlockBreak,
    BlockPlace,
    Chat,
    Command,
    Observe,
    Join,
    Leave,
}

impl EventKind {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "block-break" => EventKind::BlockBreak,
            "block-place" => EventKind::BlockPlace,
            "chat" => EventKind::Chat,
            "command" => EventKind::Command,
            "observe" => EventKind::Observe,
            "join" => EventKind::Join,
            "leave" => EventKind::Leave,
            other => bail!("unknown event `{other}`"),
        })
    }

    /// Handled by region instances (the rest by the global instance).
    pub fn is_region(self) -> bool {
        !matches!(self, EventKind::Join | EventKind::Leave)
    }
}

/// What a cancellable event becomes when its handler traps, times out, or the plugin was
/// demoted to observe-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailPolicy {
    /// Deny (protection plugins): never silently allow.
    Closed,
    /// Carry on as if the plugin allowed it.
    Open,
}

#[derive(Clone, Debug)]
pub struct Subscription {
    pub event: EventKind,
    pub policy: FailPolicy,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub capabilities: Vec<Capability>,
    pub subscriptions: Vec<Subscription>,
    /// The `[config]` table, values as strings.
    pub config: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    subscribe: Vec<RawSub>,
    #[serde(default)]
    config: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSub {
    event: String,
    #[serde(default)]
    policy: Option<String>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest> {
        let raw: Raw = toml::from_str(text).context("plugin.toml")?;
        if raw.id.is_empty() || !raw.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
            bail!("plugin id `{}` must be lowercase letters, digits, `-` or `_`", raw.id);
        }
        let capabilities = raw.capabilities.iter().map(|c| Capability::parse(c)).collect::<Result<Vec<_>>>()?;
        let mut subscriptions = Vec::new();
        for s in raw.subscribe {
            let event = EventKind::parse(&s.event)?;
            let policy = match s.policy.as_deref() {
                None | Some("fail-open") => FailPolicy::Open,
                Some("fail-closed") => FailPolicy::Closed,
                Some(other) => bail!("unknown failure policy `{other}`"),
            };
            if subscriptions.iter().any(|x: &Subscription| x.event == event) {
                bail!("event `{}` subscribed twice", s.event);
            }
            subscriptions.push(Subscription { event, policy });
        }
        let config = raw
            .config
            .into_iter()
            .map(|(k, v)| {
                let v = match v {
                    toml::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, v)
            })
            .collect();
        Ok(Manifest { id: raw.id, version: raw.version, capabilities, subscriptions, config })
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    pub fn subscription(&self, event: EventKind) -> Option<&Subscription> {
        self.subscriptions.iter().find(|s| s.event == event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subscriptions_and_config() {
        let m = Manifest::parse(
            r#"
            id = "spawn-protection"
            version = "0.1.0"
            capabilities = ["player.message"]
            [[subscribe]]
            event = "block-break"
            policy = "fail-closed"
            [[subscribe]]
            event = "chat"
            [config]
            radius = 16
            chaos = "trap"
            "#,
        )
        .unwrap();
        assert_eq!(m.subscription(EventKind::BlockBreak).unwrap().policy, FailPolicy::Closed);
        assert_eq!(m.subscription(EventKind::Chat).unwrap().policy, FailPolicy::Open);
        assert!(m.subscription(EventKind::BlockPlace).is_none());
        assert_eq!(m.config["radius"], "16");
        assert_eq!(m.config["chaos"], "trap");
        assert!(m.has(Capability::PlayerMessage));
    }

    #[test]
    fn rejects_unknown_capabilities_and_policies() {
        assert!(Manifest::parse("id = \"x\"\ncapabilities = [\"packet.observe\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\npolicy = \"maybe\"").is_err());
        assert!(Manifest::parse("id = \"Bad Id\"").is_err());
    }
}
