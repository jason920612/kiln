//! The boundary between the network runtime and the simulation.
//!
//! The simulation never depends on tokio: connections reach it through `ToSim` messages
//! and it answers through `Sink`s, which the network layer implements.

pub mod access;

use bytes::Bytes;
pub use kiln_proto::packets::{ClientInfo, PlayIn};
use uuid::Uuid;

pub type ConnId = u64;

/// Outbound side of a player connection. Packets are packet id + data; framing,
/// compression and encryption happen on the network side.
pub trait Sink: Send + Sync {
    fn send(&self, packet: Bytes);
    /// Sends a tick's worth of packets in order; one wakeup for the writer.
    fn send_batch(&self, packets: Vec<Bytes>) {
        for p in packets {
            self.send(p);
        }
    }
    /// Sends `packet`, then closes the connection.
    fn disconnect(&self, packet: Bytes);
}

pub enum ToSim {
    Join(JoinInfo),
    Packet(ConnId, PlayIn),
    Leave(ConnId),
    /// A command typed at the server console.
    Console(String),
    /// Save the world and stop; `done` is signalled when finished.
    Shutdown { done: std::sync::mpsc::Sender<()> },
    /// The answer to a profile lookup the simulation started (`request` is its number);
    /// `None` when the profile does not exist or the lookup failed.
    ProfileLookup { request: u64, result: Option<LookedUpProfile> },
    /// Work for the simulation thread from the administration threads (RCON, the management
    /// server): the closure runs between ticks with the simulation's [`AdminHost`] face.
    Admin(Box<dyn FnOnce(&mut dyn AdminHost) + Send>),
}

/// What the simulation lends to remote administration (see [`ToSim::Admin`]).
pub trait AdminHost {
    /// Runs `command` as the RCON source (`RconConsoleSource`: permission level 4, nobody's
    /// position) and returns what it said (`getAndClearBuffer`).
    fn rcon_command(&mut self, command: &str) -> String;
}

/// A player in the player list, as the server list, Query and the management server see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub conn: ConnId,
    pub uuid: Uuid,
    pub name: String,
    /// The client's "Allow Server Listings" option (`ServerPlayer.allowsListing`).
    pub allows_listing: bool,
}

/// Facts about the running server that threads outside the simulation read (the server list,
/// Query, RCON, the management server). The simulation keeps the roster; the settings are the
/// mutable ones of `DedicatedServerProperties`.
#[derive(Debug)]
pub struct ServerInfo {
    pub roster: std::sync::RwLock<Vec<RosterEntry>>,
    pub motd: std::sync::RwLock<String>,
    pub max_players: std::sync::atomic::AtomicI32,
    pub hide_online_players: std::sync::atomic::AtomicBool,
    pub enable_status: std::sync::atomic::AtomicBool,
    /// The world's name (`level-name`), Query's `map`.
    pub level_name: String,
    /// The version Query and the legacy ping report.
    pub version_name: String,
}

impl ServerInfo {
    pub fn new(motd: &str, max_players: i32, level_name: &str) -> Self {
        Self {
            roster: Default::default(),
            motd: std::sync::RwLock::new(motd.to_owned()),
            max_players: std::sync::atomic::AtomicI32::new(max_players),
            hide_online_players: std::sync::atomic::AtomicBool::new(false),
            enable_status: std::sync::atomic::AtomicBool::new(true),
            level_name: level_name.to_owned(),
            version_name: kiln_data::version::NAME.to_owned(),
        }
    }

    pub fn motd(&self) -> String {
        self.motd.read().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn max_players(&self) -> i32 {
        self.max_players.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn players(&self) -> Vec<RosterEntry> {
        self.roster.read().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn player_count(&self) -> usize {
        self.roster.read().map_or(0, |r| r.len())
    }
}

/// A game profile a lookup found, with its properties (skin textures).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookedUpProfile {
    pub id: Uuid,
    pub name: String,
    pub properties: Vec<Property>,
}

/// Looks game profiles up by name or id (the session service). Calls block for as long as the
/// network takes, so the simulation makes them on another thread. A lookup sends only the name
/// or id asked for: nothing about the server's operator or its players.
pub trait ProfileLookup: Send + Sync {
    fn by_name(&self, name: &str) -> Result<Option<LookedUpProfile>, String>;
    fn by_id(&self, id: Uuid) -> Result<Option<LookedUpProfile>, String>;
}

pub struct JoinInfo {
    pub conn: ConnId,
    pub name: String,
    pub uuid: Uuid,
    /// Profile properties from authentication or proxy forwarding (e.g. skin textures).
    pub properties: Vec<Property>,
    /// Settings from the configuration phase (view distance, skin layers, main hand).
    pub client: ClientInfo,
    pub sink: Box<dyn Sink>,
    /// The client's address (`getIpAddress`), when known.
    pub address: Option<std::net::IpAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

/// What the configuration phase sends that the simulation's data packs decide: the world's
/// feature flags (`Update Enabled Features`), the data of the registries the packs define or
/// override (`Registry Data`) and the tags of the enabled packs (`Update Tags`). The simulation
/// writes it when the packs load; logins read it.
#[derive(Debug)]
pub struct DataSync {
    features: std::sync::RwLock<Vec<String>>,
    /// A configuration `Update Tags` packet; `None` sends the built-in vanilla tags.
    config_tags: std::sync::RwLock<Option<Bytes>>,
    /// The `Registry Data` packets of every synchronized registry; `None` sends the vanilla
    /// entries by name only.
    registry_packets: std::sync::RwLock<Option<std::sync::Arc<Vec<Bytes>>>>,
}

impl Default for DataSync {
    fn default() -> Self {
        Self {
            features: std::sync::RwLock::new(vec!["minecraft:vanilla".to_owned()]),
            config_tags: Default::default(),
            registry_packets: Default::default(),
        }
    }
}

impl DataSync {
    pub fn registry_packets(&self) -> Option<std::sync::Arc<Vec<Bytes>>> {
        self.registry_packets.read().ok().and_then(|p| p.clone())
    }

    pub fn set_registry_packets(&self, packets: Option<std::sync::Arc<Vec<Bytes>>>) {
        if let Ok(mut p) = self.registry_packets.write() {
            *p = packets;
        }
    }

    pub fn features(&self) -> Vec<String> {
        self.features.read().map(|f| f.clone()).unwrap_or_default()
    }

    pub fn set_features(&self, features: Vec<String>) {
        if let Ok(mut f) = self.features.write() {
            *f = features;
        }
    }

    pub fn config_tags(&self) -> Option<Bytes> {
        self.config_tags.read().ok().and_then(|t| t.clone())
    }

    pub fn set_config_tags(&self, packet: Option<Bytes>) {
        if let Ok(mut t) = self.config_tags.write() {
            *t = packet;
        }
    }
}
