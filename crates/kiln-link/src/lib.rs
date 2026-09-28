//! The boundary between the network runtime and the simulation.
//!
//! The simulation never depends on tokio: connections reach it through `ToSim` messages
//! and it answers through `Sink`s, which the network layer implements.

use bytes::Bytes;
pub use kiln_proto::packets::{ClientInfo, PlayIn};
use uuid::Uuid;

pub type ConnId = u64;

/// Outbound side of a player connection. Packets are packet id + data; framing,
/// compression and encryption happen on the network side.
pub trait Sink: Send {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

/// What the configuration phase sends that the simulation's data packs decide: the world's
/// feature flags (`Update Enabled Features`) and the tags of the enabled packs (`Update
/// Tags`). The simulation writes it when the packs load; logins read it.
#[derive(Debug)]
pub struct DataSync {
    features: std::sync::RwLock<Vec<String>>,
    /// A configuration `Update Tags` packet; `None` sends the built-in vanilla tags.
    config_tags: std::sync::RwLock<Option<Bytes>>,
}

impl Default for DataSync {
    fn default() -> Self {
        Self { features: std::sync::RwLock::new(vec!["minecraft:vanilla".to_owned()]), config_tags: Default::default() }
    }
}

impl DataSync {
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
