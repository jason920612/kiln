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
    /// The client's address (`getIpAddress`), when known.
    pub address: Option<std::net::IpAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}
