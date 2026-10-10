//! `plugin.toml`: identity, capabilities, event subscriptions with their failure policies and
//! host-side filters, and the plugin's configuration (design §11.4, §11.5).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

/// The API major version this host links (`kiln:api@1`).
pub const API_MAJOR: u32 = 1;

/// What a plugin may link against. `state`, `event`, `env`, `registry` and `log` are always
/// linked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    /// `kiln:api/chat`: send chat to players.
    PlayerMessage,
    /// Commands returned by `global-hooks.init` are registered.
    CommandRegister,
    /// `kiln:api/scheduler`: delayed tasks.
    Scheduler,
    /// WASI preopens the plugin's own data directory (`/data`).
    FsData,
    /// `kiln:api/hud`: titles, action bar, sidebar, boss bars.
    PlayerHud,
    /// `kiln:api/players`: teleport, game mode, heal, kick.
    PlayerControl,
    /// `kiln:api/inventory`: give, take, clear, menus.
    Inventory,
    /// `kiln:api/entities`: spawn and remove the plugin's own entities.
    EntityControl,
    /// `kiln:api/blocks`: set blocks in the cell of the call.
    WorldWrite,
    /// `kiln:api/world-read` (1.1): read the blocks around a block event.
    WorldRead,
    /// `kiln:api/events`: raise events to other plugins.
    EventsRaise,
    /// `async-tasks` world: HTTP requests to this host (`http:example.com`).
    Http(String),
    /// `async-tasks` world: timers.
    Timers,
    /// `async-tasks` world: the plugin's simple key-value storage.
    Storage,
}

impl Capability {
    fn parse(s: &str) -> Result<Self> {
        if let Some(host) = s.strip_prefix("http:") {
            if host.is_empty() || host.contains('/') {
                bail!("capability `{s}`: expected `http:<host>`");
            }
            return Ok(Capability::Http(host.to_ascii_lowercase()));
        }
        Ok(match s {
            "player.message" => Capability::PlayerMessage,
            "command.register" => Capability::CommandRegister,
            "scheduler" => Capability::Scheduler,
            "fs.data" => Capability::FsData,
            "player.hud" => Capability::PlayerHud,
            "player.control" => Capability::PlayerControl,
            "inventory" => Capability::Inventory,
            "entity.control" => Capability::EntityControl,
            "world.write" => Capability::WorldWrite,
            "world.read" => Capability::WorldRead,
            "events.raise" => Capability::EventsRaise,
            "timers" => Capability::Timers,
            "storage" => Capability::Storage,
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
    /// A player hits an entity.
    EntityAttack,
    /// A player is about to take damage.
    PlayerDamage,
    /// The held item is used (only tagged items, unless the filter's `items` say otherwise).
    ItemUse,
    /// A click in a container screen (the plugin's own menus, and vanilla containers with
    /// the filter's `vanilla`).
    ContainerClick,
    Chat,
    Command,
    /// Events other plugins raise (`events.raise`).
    Custom,
    Observe,
    Join,
    Leave,
    /// Results of the plugin's atomic operations and effects, the tick after they applied.
    OpResults,
}

impl EventKind {
    pub const ALL: [EventKind; 14] = [
        EventKind::BlockBreak,
        EventKind::BlockPlace,
        EventKind::EntityInteract,
        EventKind::EntityAttack,
        EventKind::PlayerDamage,
        EventKind::ItemUse,
        EventKind::ContainerClick,
        EventKind::Chat,
        EventKind::Command,
        EventKind::Custom,
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
            "entity-attack" => EventKind::EntityAttack,
            "player-damage" => EventKind::PlayerDamage,
            "item-use" => EventKind::ItemUse,
            "container-click" => EventKind::ContainerClick,
            "chat" => EventKind::Chat,
            "command" => EventKind::Command,
            "custom" => EventKind::Custom,
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
        !matches!(self, EventKind::Custom | EventKind::Observe | EventKind::Join | EventKind::Leave | EventKind::OpResults)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// What an `observe` subscription receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserveKinds(pub u8);

impl ObserveKinds {
    pub const BLOCK_BROKEN: u8 = 1;
    pub const BLOCK_PLACED: u8 = 2;
    pub const PLAYER_DIED: u8 = 4;
    pub const PLAYER_SPAWNED: u8 = 8;
    /// 1.1: a player's block position changed.
    pub const PLAYER_MOVED: u8 = 16;
    /// Without `kinds`: block changes only (what the first versions of the API sent).
    pub const DEFAULT: ObserveKinds = ObserveKinds(Self::BLOCK_BROKEN | Self::BLOCK_PLACED);

    pub fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    fn parse(names: &[String]) -> Result<ObserveKinds> {
        let mut bits = 0;
        for n in names {
            bits |= match n.as_str() {
                "block-broken" => Self::BLOCK_BROKEN,
                "block-placed" => Self::BLOCK_PLACED,
                "player-died" => Self::PLAYER_DIED,
                "player-spawned" => Self::PLAYER_SPAWNED,
                "player-moved" => Self::PLAYER_MOVED,
                other => bail!("unknown observed kind `{other}`"),
            };
        }
        Ok(ObserveKinds(bits))
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
    /// Entity type keys or `#tags` (entity-interact, entity-attack).
    pub entities: Vec<String>,
    /// Item keys or `#tags` (item-use): items to deliver besides the plugin's tagged ones;
    /// `*` delivers every item.
    pub items: Vec<String>,
    /// Event names (custom): `<plugin>:<name>`.
    pub names: Vec<String>,
    /// container-click: also deliver clicks in vanilla containers (the plugin's own menus
    /// always arrive).
    pub vanilla: bool,
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
    /// For `observe`: which observed kinds.
    pub observe: ObserveKinds,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    /// The API major version the plugin was written for.
    pub api: u32,
    /// The file of the plugin's `async-tasks` component next to `plugin.wasm`, if it has one.
    pub tasks: Option<String>,
    /// The bytes of that component (filled in by whoever loads the plugin; not part of the toml).
    pub tasks_wasm: Option<Vec<u8>>,
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
    api: Option<String>,
    #[serde(default)]
    tasks: Option<String>,
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
    items: Vec<String>,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    vanilla: bool,
    #[serde(default)]
    kinds: Vec<String>,
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
        let api = match raw.api.as_deref() {
            None => API_MAJOR,
            Some(v) => v.split('.').next().and_then(|m| m.parse().ok()).with_context(|| format!("api `{v}`: expected a version like `1`"))?,
        };
        if api != API_MAJOR {
            bail!("plugin {} is written for API {api}, this host links API {API_MAJOR}", raw.id);
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
            let filter =
                Filter { blocks: s.blocks, entities: s.entities, items: s.items, names: s.names, vanilla: s.vanilla, area, bypass_permission: s.bypass_permission };
            if !filter.entities.is_empty() && !matches!(event, EventKind::EntityInteract | EventKind::EntityAttack) {
                bail!("`entities` filters only apply to entity-interact and entity-attack");
            }
            if !filter.blocks.is_empty() && !matches!(event, EventKind::BlockBreak | EventKind::Observe) {
                bail!("`blocks` filters only apply to block-break and observe");
            }
            if !filter.items.is_empty() && event != EventKind::ItemUse {
                bail!("`items` filters only apply to item-use");
            }
            if !filter.names.is_empty() && event != EventKind::Custom {
                bail!("`names` filters only apply to custom");
            }
            if filter.vanilla && event != EventKind::ContainerClick {
                bail!("`vanilla` only applies to container-click");
            }
            if !s.kinds.is_empty() && event != EventKind::Observe {
                bail!("`kinds` only applies to observe");
            }
            let observe = if s.kinds.is_empty() { ObserveKinds::DEFAULT } else { ObserveKinds::parse(&s.kinds)? };
            subscriptions.push(Subscription { event, policy, filter, observe });
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
        let needs_tasks = capabilities.iter().any(|c| matches!(c, Capability::Http(_) | Capability::Timers | Capability::Storage));
        if raw.tasks.is_none() && needs_tasks {
            bail!("the `http:<host>`, `timers` and `storage` capabilities belong to the async-tasks component: name it with `tasks = \"tasks.wasm\"`");
        }
        if raw.tasks.as_ref().is_some_and(|t| t.is_empty() || t.contains(['/', '\\']) || t.starts_with('.')) {
            bail!("`tasks` is the name of a file next to plugin.toml");
        }
        Ok(Manifest { id: raw.id, version: raw.version, api, tasks: raw.tasks, tasks_wasm: None, capabilities, subscriptions, config })
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// The hosts the plugin may fetch from (`http:<host>`).
    pub fn http_hosts(&self) -> impl Iterator<Item = &str> {
        self.capabilities.iter().filter_map(|c| match c {
            Capability::Http(h) => Some(h.as_str()),
            _ => None,
        })
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
            [[subscribe]]
            event = "observe"
            kinds = ["player-died", "player-spawned"]
            [[subscribe]]
            event = "container-click"
            vanilla = true
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
        let obs = m.subscription(EventKind::Observe).unwrap().observe;
        assert!(obs.has(ObserveKinds::PLAYER_DIED) && obs.has(ObserveKinds::PLAYER_SPAWNED) && !obs.has(ObserveKinds::BLOCK_BROKEN));
        assert!(m.subscription(EventKind::ContainerClick).unwrap().filter.vanilla);
    }

    #[test]
    fn rejects_unknown_capabilities_policies_and_misplaced_filters() {
        assert!(Manifest::parse("id = \"x\"\ncapabilities = [\"packet.observe\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\npolicy = \"maybe\"").is_err());
        assert!(Manifest::parse("id = \"Bad Id\"").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\nblocks = [\"minecraft:stone\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"block-break\"\nentities = [\"minecraft:cow\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"block-break\"\narea = { x = 1, radius = 3 }").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\nvanilla = true").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"chat\"\nkinds = [\"player-died\"]").is_err());
        assert!(Manifest::parse("id = \"x\"\n[[subscribe]]\nevent = \"observe\"\nkinds = [\"nothing\"]").is_err());
    }

    #[test]
    fn api_major_must_match_and_http_hosts_parse() {
        assert!(Manifest::parse("id = \"x\"\napi = \"1.4\"").is_ok());
        assert!(Manifest::parse("id = \"x\"\napi = \"2\"").is_err());
        assert!(Manifest::parse("id = \"x\"\napi = \"soon\"").is_err());
        let m = Manifest::parse("id = \"x\"\ntasks = \"tasks.wasm\"\ncapabilities = [\"http:API.example.com\", \"timers\"]").unwrap();
        assert_eq!(m.tasks.as_deref(), Some("tasks.wasm"));
        assert!(Manifest::parse("id = \"x\"\ncapabilities = [\"timers\"]").is_err(), "the capabilities belong to the component");
        assert!(Manifest::parse("id = \"x\"\ntasks = \"../x.wasm\"").is_err());
        assert_eq!(m.http_hosts().collect::<Vec<_>>(), ["api.example.com"]);
        assert!(Manifest::parse("id = \"x\"\ncapabilities = [\"http:\"]").is_err());
    }
}
