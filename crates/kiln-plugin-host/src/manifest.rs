//! `plugin.toml`: identity, capabilities, event subscriptions with their failure policies and
//! host-side filters, and the plugin's configuration (design §11.4, §11.5).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

/// What a plugin may link against. `state`, `env`, `registry` and `log` are always linked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// `kiln:api/chat`: send chat to players.
    PlayerMessage,
    /// Commands returned by `global-hooks.init` are registered.
    CommandRegister,
    /// `kiln:api/scheduler`: delayed tasks.
    Scheduler,
    /// WASI preopens the plugin's own data directory (`/data`).
    FsData,
}

impl Capability {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "player.message" => Capability::PlayerMessage,
            "command.register" => Capability::CommandRegister,
            "scheduler" => Capability::Scheduler,
            "fs.data" => Capability::FsData,
            // Declared in the design but not in this WIT: refuse rather than ignore.
            other => bail!("unknown or unsupported capability `{other}`"),
        })
    }
}

/// Events a plugin subscribes to. Only subscribed events cross the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    BlockBreak,
    BlockPlace,
    EntityInteract,
    Chat,
    Command,
    Observe,
    Join,
    Leave,
    /// Results of the plugin's atomic operations, the tick after they applied.
    OpResults,
}

impl EventKind {
    pub const ALL: [EventKind; 9] = [
        EventKind::BlockBreak,
        EventKind::BlockPlace,
        EventKind::EntityInteract,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Observe,
        EventKind::Join,
        EventKind::Leave,
        EventKind::OpResults,
    ];

    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "block-break" => EventKind::BlockBreak,
            "block-place" => EventKind::BlockPlace,
            "entity-interact" => EventKind::EntityInteract,
            "chat" => EventKind::Chat,
            "command" => EventKind::Command,
            "observe" => EventKind::Observe,
            "join" => EventKind::Join,
            "leave" => EventKind::Leave,
            "op-results" => EventKind::OpResults,
            other => bail!("unknown event `{other}`"),
        })
    }

    /// Handled by region instances (the rest by the global instance).
    pub fn is_region(self) -> bool {
        !matches!(self, EventKind::Join | EventKind::Leave | EventKind::OpResults)
    }

    /// Cancellable events: rate-limited per player and subject to the failure policy.
    pub fn is_cancellable(self) -> bool {
        matches!(self, EventKind::BlockBreak | EventKind::BlockPlace | EventKind::EntityInteract | EventKind::Chat | EventKind::Command)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// What a cancellable event becomes when its handler traps, times out, the plugin was
/// demoted to observe-only, or the acting player or the instance ran out of budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailPolicy {
    /// Deny (protection plugins): never silently allow.
    Closed,
    /// Carry on as if the plugin allowed it.
    Open,
}

/// A square area of a level (the shape of vanilla's spawn protection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Area {
    /// Level key; any level when `None`.
    pub level: Option<String>,
    /// Centre; the world spawn when `None`.
    pub center: Option<(i32, i32)>,
    pub radius: i32,
}

/// Host-side filters of a subscription (design §11.4): events outside them never cross the
/// boundary, and count as allowed for a cancellable event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Block keys or `#tags` the event's block must match (block-break, and observe, where
    /// the batch keeps only matching block changes).
    pub blocks: Vec<String>,
    /// Entity type keys or `#tags` (entity-interact).
    pub entities: Vec<String>,
    /// The event's position must be inside.
    pub area: Option<Area>,
    /// Actors with at least this permission level are not subject to the subscription (an
    /// operator has level 4).
    pub bypass_permission: Option<u8>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        *self == Filter::default()
    }
}

#[derive(Clone, Debug)]
pub struct Subscription {
    pub event: EventKind,
    pub policy: FailPolicy,
    pub filter: Filter,
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
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawSub {
    event: String,
    #[serde(default)]
    policy: Option<String>,
    #[serde(default)]
    blocks: Vec<String>,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    area: Option<RawArea>,
    #[serde(default)]
    bypass_permission: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArea {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    z: Option<i32>,
    radius: i32,
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
            let area = match s.area {
                Some(a) => {
                    let center = match (a.x, a.z) {
                        (Some(x), Some(z)) => Some((x, z)),
                        (None, None) => None,
                        _ => bail!("an area needs both `x` and `z`, or neither (the spawn)"),
                    };
                    if a.radius < 0 {
                        bail!("an area's radius cannot be negative");
                    }
                    Some(Area { level: a.level, center, radius: a.radius })
                }
                None => None,
            };
            let filter = Filter { blocks: s.blocks, entities: s.entities, area, bypass_permission: s.bypass_permission };
            if !filter.entities.is_empty() && event != EventKind::EntityInteract {
                bail!("`entities` filters only apply to entity-interact");
            }
            if !filter.blocks.is_empty() && !matches!(event, EventKind::BlockBreak | EventKind::Observe) {
                bail!("`blocks` filters only apply to block-break and observe");
            }
            subscriptions.push(Subscription { event, policy, filter });
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
    fn parses_subscriptions_filters_and_config() {
        let m = Manifest::parse(
            r##"
            id = "spawn-protection"
            version = "0.1.0"
            capabilities = ["player.message", "scheduler"]
            [[subscribe]]
            event = "block-break"
            policy = "fail-closed"
            blocks = ["#minecraft:logs", "minecraft:stone"]
            area = { level = "minecraft:overworld", radius = 16 }
            bypass-permission = 2
            [[subscribe]]
            event = "chat"
            [[subscribe]]
            event = "entity-interact"
            entities = ["minecraft:cow"]
            area = { x = 100, z = -3, radius = 5 }
            [config]
            radius = 16
            chaos = "trap"
            "##,
        )
        .unwrap();
        let brk = m.subscription(EventKind::BlockBreak).unwrap();
        assert_eq!(brk.policy, FailPolicy::Closed);
        assert_eq!(brk.filter.blocks, ["#minecraft:logs", "minecraft:stone"]);
        assert_eq!(brk.filter.area, Some(Area { level: Some("minecraft:overworld".into()), center: None, radius: 16 }));
        assert_eq!(brk.filter.bypass_permission, Some(2));
        assert_eq!(m.subscription(EventKind::Chat).unwrap().policy, FailPolicy::Open);
        assert!(m.subscription(EventKind::Chat).unwrap().filter.is_empty());
        let ent = m.subscription(EventKind::EntityInteract).unwrap();
        assert_eq!(ent.filter.area.as_ref().unwrap().center, Some((100, -3)));
        assert!(m.subscription(EventKind::BlockPlace).is_none());
        assert_eq!(m.config["radius"], "16");
        assert_eq!(m.config["chaos"], "trap");
        assert!(m.has(Capability::PlayerMessage) && m.has(Capability::Scheduler));
    }

    #[test]
    fn rejects_unknown_capabilities_policies_and_misplaced_filters() {
        assert!(Manifest::parse("id = \"x\"\ncapabilities = [\"packet.observe\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\npolicy = \"maybe\"").is_err());
        assert!(Manifest::parse("id = \"Bad Id\"").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\nblocks = [\"minecraft:stone\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"block-break\"\nentities = [\"minecraft:cow\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"block-break\"\narea = { x = 1, radius = 3 }").is_err());
    }
}
