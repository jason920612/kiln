//! Connection handling on the tokio runtime: handshake, status, login and configuration run
//! here; once a player reaches play state, packets are relayed to and from the simulation.

use kiln_link::{ConnId, JoinInfo, Sink, ToSim};
use kiln_proto::packets;
use anyhow::{Context, Result, anyhow, bail};
use bytes::{Bytes, BytesMut};
use kiln_data::packets as ids;
use kiln_data::version;
use kiln_proto::frame::FrameCodec;
use kiln_proto::Reader;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct Config {
    pub bind: SocketAddr,
    pub motd: String,
    pub max_players: usize,
    pub view_distance: u8,
    pub simulation_distance: u8,
    /// `None` disables compression (recommended behind a proxy).
    pub compression_threshold: Option<usize>,
}

pub struct Shared {
    pub config: Config,
    pub online: AtomicUsize,
    pub to_sim: crossbeam_channel::Sender<ToSim>,
    next_conn: AtomicU64,
    registry_packets: Vec<Bytes>,
    tags_packet: Bytes,
}

impl Shared {
    pub fn new(config: Config, to_sim: crossbeam_channel::Sender<ToSim>) -> Self {
        // Encoded once and reused for every login.
        let registry_packets = kiln_data::registries::SYNCHRONIZED
            .iter()
            .map(|(reg, entries)| packets::registry_data(reg, entries))
            .collect();
        let tags_packet =
            packets::update_tags(ids::configuration::clientbound::UPDATE_TAGS, kiln_data::registries::TAGS);
        Self {
            config,
            online: AtomicUsize::new(0),
            to_sim,
            next_conn: AtomicU64::new(1),
            registry_packets,
            tags_packet,
        }
    }
}

/// Messages from the simulation to a connection's writer.
enum Outbound {
    Packet(Bytes),
    /// Send this packet, then close the connection.
    Disconnect(Bytes),
}

struct ChannelSink(mpsc::UnboundedSender<Outbound>);

impl Sink for ChannelSink {
    fn send(&self, packet: Bytes) {
        let _ = self.0.send(Outbound::Packet(packet));
    }
    fn disconnect(&self, packet: Bytes) {
        let _ = self.0.send(Outbound::Disconnect(packet));
    }
}

const PRE_PLAY_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn listen(shared: Arc<Shared>) -> Result<()> {
    let listener = TcpListener::bind(shared.config.bind).await?;
    info!("listening on {}", shared.config.bind);
    loop {
        let (stream, addr) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let shared = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, addr, shared).await {
                debug!("{addr}: {e:#}");
            }
        });
    }
}

struct Conn {
    stream: TcpStream,
    rbuf: BytesMut,
    rx: FrameCodec,
    tx: FrameCodec,
    wbuf: BytesMut,
}

impl Conn {
    async fn read(&mut self) -> Result<BytesMut> {
        loop {
            if let Some(p) = self.rx.decode(&mut self.rbuf)? {
                return Ok(p);
            }
            let n = tokio::time::timeout(PRE_PLAY_TIMEOUT, self.stream.read_buf(&mut self.rbuf))
                .await
                .context("timed out")??;
            if n == 0 {
                bail!("connection closed");
            }
        }
    }

    fn queue(&mut self, pkt: &[u8]) -> Result<()> {
        self.tx.encode(pkt, &mut self.wbuf)?;
        Ok(())
    }

    async fn flush(&mut self) -> Result<()> {
        self.stream.write_all(&self.wbuf).await?;
        self.wbuf.clear();
        Ok(())
    }

    async fn send(&mut self, pkt: &[u8]) -> Result<()> {
        self.queue(pkt)?;
        self.flush().await
    }
}

fn split_id(pkt: &[u8]) -> Result<(i32, Reader<'_>)> {
    let mut r = Reader::new(pkt);
    let id = r.varint()?;
    Ok((id, r))
}

async fn handle(stream: TcpStream, addr: SocketAddr, shared: Arc<Shared>) -> Result<()> {
    let mut conn = Conn {
        stream,
        rbuf: BytesMut::with_capacity(4096),
        rx: FrameCodec::new(),
        tx: FrameCodec::new(),
        wbuf: BytesMut::with_capacity(4096),
    };

    let pkt = conn.read().await?;
    let (id, mut r) = split_id(&pkt)?;
    if id != ids::handshake::serverbound::INTENTION {
        bail!("expected handshake, got packet {id}");
    }
    let protocol = r.varint()?;
    let _host = r.string(255)?;
    let _port = r.u16()?;
    let intent = r.varint()?;
    r.finish()?;

    match intent {
        1 => status(&mut conn, &shared, protocol).await,
        2 | 3 => login(conn, addr, &shared, protocol).await,
        n => bail!("unknown handshake intent {n}"),
    }
}

async fn status(conn: &mut Conn, shared: &Shared, _client_protocol: i32) -> Result<()> {
    loop {
        let pkt = conn.read().await?;
        let (id, mut r) = split_id(&pkt)?;
        match id {
            ids::status::serverbound::STATUS_REQUEST => {
                let json = serde_json::json!({
                    "version": { "name": version::NAME, "protocol": version::PROTOCOL },
                    "players": {
                        "max": shared.config.max_players,
                        "online": shared.online.load(Ordering::Relaxed),
                        "sample": [],
                    },
                    "description": { "text": shared.config.motd },
                    "enforcesSecureChat": false,
                });
                conn.send(&packets::status_response(&json.to_string())).await?;
            }
            ids::status::serverbound::PING_REQUEST => {
                let ts = r.i64()?;
                conn.send(&packets::status_pong(ts)).await?;
                return Ok(());
            }
            other => bail!("unexpected status packet {other}"),
        }
    }
}

/// UUID the vanilla server assigns in offline mode: v3 of "OfflinePlayer:<name>" without a namespace.
fn offline_uuid(name: &str) -> Uuid {
    use md5::{Digest, Md5};
    let mut h: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

async fn login(mut conn: Conn, addr: SocketAddr, shared: &Shared, protocol: i32) -> Result<()> {
    let pkt = conn.read().await?;
    let (id, mut r) = split_id(&pkt)?;
    if id != ids::login::serverbound::HELLO {
        bail!("expected login start, got packet {id}");
    }
    let name = r.string(16)?.to_owned();
    let _client_uuid = r.uuid()?;
    r.finish()?;

    if protocol != version::PROTOCOL {
        let msg = format!("Kiln runs Minecraft {} (protocol {}).", version::NAME, version::PROTOCOL);
        conn.send(&packets::login_disconnect(&msg)).await?;
        bail!("{name}: unsupported protocol {protocol}");
    }
    if !valid_name(&name) {
        conn.send(&packets::login_disconnect("Invalid username.")).await?;
        bail!("invalid username {name:?}");
    }
    if shared.online.load(Ordering::Relaxed) >= shared.config.max_players {
        conn.send(&packets::login_disconnect("The server is full.")).await?;
        bail!("{name}: server full");
    }

    if let Some(t) = shared.config.compression_threshold {
        conn.send(&packets::login_compression(t as i32)).await?;
        conn.rx.set_threshold(Some(t));
        conn.tx.set_threshold(Some(t));
    }
    let uuid = offline_uuid(&name);
    conn.send(&packets::login_finished(uuid, &name, &[], Uuid::new_v4())).await?;

    let pkt = conn.read().await?;
    let (id, _) = split_id(&pkt)?;
    if id != ids::login::serverbound::LOGIN_ACKNOWLEDGED {
        bail!("expected login acknowledged, got packet {id}");
    }

    let view_distance = configure(&mut conn, shared).await?;
    info!("{name} ({uuid}) joined from {addr}");
    play(conn, shared, name, uuid, view_distance).await
}

/// Configuration phase; returns the client's view distance.
async fn configure(conn: &mut Conn, shared: &Shared) -> Result<u8> {
    use ids::configuration::serverbound as sb;
    let core = ("minecraft", "core", version::NAME);
    conn.queue(&packets::config_brand("kiln"))?;
    conn.queue(&packets::update_enabled_features(&["minecraft:vanilla"]))?;
    conn.queue(&packets::select_known_packs(&[core]))?;
    conn.flush().await?;

    let mut view_distance = shared.config.view_distance;
    let mut registries_sent = false;
    loop {
        let pkt = conn.read().await?;
        let (id, mut r) = split_id(&pkt)?;
        match id {
            sb::CLIENT_INFORMATION => view_distance = packets::read_client_information(&mut r)?,
            sb::SELECT_KNOWN_PACKS if !registries_sent => {
                let n = r.len()?;
                let mut knows_core = false;
                for _ in 0..n.min(64) {
                    let pack = (r.string(32767)?, r.string(32767)?, r.string(32767)?);
                    knows_core |= pack == core;
                }
                if !knows_core {
                    let msg = format!("Kiln requires the Minecraft {} client.", version::NAME);
                    conn.send(&packets::config_disconnect(&msg)).await?;
                    bail!("client does not know {core:?}");
                }
                for p in &shared.registry_packets {
                    conn.queue(p)?;
                }
                conn.queue(&shared.tags_packet)?;
                conn.queue(&packets::finish_configuration())?;
                conn.flush().await?;
                registries_sent = true;
            }
            sb::FINISH_CONFIGURATION if registries_sent => return Ok(view_distance),
            sb::CUSTOM_PAYLOAD | sb::KEEP_ALIVE | sb::PONG | sb::RESOURCE_PACK => {}
            other => debug!("ignoring configuration packet {other}"),
        }
    }
}

async fn play(conn: Conn, shared: &Shared, name: String, uuid: Uuid, view_distance: u8) -> Result<()> {
    let conn_id = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    let Conn { stream, mut rbuf, mut rx, tx, wbuf } = conn;
    let (reader, writer) = stream.into_split();
    let (out_tx, out_rx) = mpsc::unbounded_channel();

    shared.online.fetch_add(1, Ordering::Relaxed);
    let joined = shared
        .to_sim
        .send(ToSim::Join(JoinInfo {
            conn: conn_id,
            name: name.clone(),
            uuid,
            properties: Vec::new(),
            view_distance,
            sink: Box::new(ChannelSink(out_tx)),
        }))
        .is_ok();

    let writer_task = tokio::spawn(write_loop(writer, tx, wbuf, out_rx));
    let result = if joined {
        read_loop(reader, &mut rbuf, &mut rx, conn_id, &shared.to_sim).await
    } else {
        Err(anyhow!("simulation is not running"))
    };

    let _ = shared.to_sim.send(ToSim::Leave(conn_id));
    shared.online.fetch_sub(1, Ordering::Relaxed);
    writer_task.abort();
    info!("{name} left: {}", result.as_ref().err().map_or("disconnected".to_string(), |e| format!("{e:#}")));
    Ok(())
}

async fn read_loop(
    mut reader: OwnedReadHalf,
    rbuf: &mut BytesMut,
    rx: &mut FrameCodec,
    conn: ConnId,
    to_sim: &crossbeam_channel::Sender<ToSim>,
) -> Result<()> {
    loop {
        while let Some(pkt) = rx.decode(rbuf)? {
            let (id, mut r) = split_id(&pkt)?;
            match packets::decode_play(id, &mut r) {
                Ok(Some(p)) => {
                    if to_sim.send(ToSim::Packet(conn, p)).is_err() {
                        bail!("simulation stopped");
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("bad play packet {id}: {e}");
                    bail!("malformed packet");
                }
            }
        }
        if reader.read_buf(rbuf).await? == 0 {
            bail!("connection closed");
        }
    }
}

async fn write_loop(
    mut writer: OwnedWriteHalf,
    mut tx: FrameCodec,
    mut wbuf: BytesMut,
    mut out: mpsc::UnboundedReceiver<Outbound>,
) -> Result<()> {
    while let Some(msg) = out.recv().await {
        let mut close = false;
        let mut next = Some(msg);
        // Coalesce everything queued so far into one write.
        while let Some(msg) = next {
            match msg {
                Outbound::Packet(p) => tx.encode(&p, &mut wbuf)?,
                Outbound::Disconnect(p) => {
                    tx.encode(&p, &mut wbuf)?;
                    close = true;
                    break;
                }
            }
            next = out.try_recv().ok();
        }
        writer.write_all(&wbuf).await?;
        wbuf.clear();
        if close {
            writer.shutdown().await?;
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_vanilla() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch").to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }
}
