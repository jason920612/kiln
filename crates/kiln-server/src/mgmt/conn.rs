//! One client connection: the HTTP upgrade with its authentication (`AuthenticationHandler`), then
//! WebSocket text frames of JSON-RPC (`Connection`), with the notifications written between requests.

use super::hub::notification_json;
use super::json;
use super::ws::{self, Decoder, Message, WsError};
use super::{Ctx, rpc};
use kiln_link::mgmt::Notification;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use tracing::debug;

/// A connection's byte stream: a socket, or a socket under TLS.
pub trait Wire: Read + Write + Send {
    fn set_timeout(&self, d: Duration);
    fn close(&self);
}

impl Wire for TcpStream {
    fn set_timeout(&self, d: Duration) {
        let _ = self.set_read_timeout(Some(d));
    }

    fn close(&self) {
        let _ = self.shutdown(std::net::Shutdown::Both);
    }
}

pub struct TlsWire(pub rustls::StreamOwned<rustls::ServerConnection, TcpStream>);

impl Read for TlsWire {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for TlsWire {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Wire for TlsWire {
    fn set_timeout(&self, d: Duration) {
        let _ = self.0.sock.set_read_timeout(Some(d));
    }

    fn close(&self) {
        let _ = self.0.sock.shutdown(std::net::Shutdown::Both);
    }
}

/// What every connection shares.
pub struct Shared {
    pub ctx: Arc<Ctx>,
    pub secret: String,
    /// `management-server-allowed-origins` split at commas (`AuthenticationHandler.allowedOrigins`).
    pub origins: HashSet<String>,
    pub stopping: AtomicBool,
}

/// `AuthenticationHandler.SecurityCheckResult`.
enum Check {
    Allowed { token_in_protocol: bool },
    Denied(&'static str),
}

fn valid_key(shared: &Shared, token: &str) -> bool {
    use subtle::ConstantTimeEq;
    !token.is_empty() && token.as_bytes().ct_eq(shared.secret.as_bytes()).into()
}

fn security_check(shared: &Shared, req: &ws::HttpRequest) -> Check {
    // `Authorization: Bearer <secret>` (the first such header, its value trimmed).
    if let Some(auth) = req.header("Authorization")
        && let Some(rest) = auth.strip_prefix("Bearer ")
    {
        return if valid_key(shared, rest.trim()) { Check::Allowed { token_in_protocol: false } } else { Check::Denied("Invalid API key") };
    }
    // `Sec-WebSocket-Protocol: minecraft-v1, <secret>` (for browsers, which cannot set headers).
    if let Some(proto) = req.header("Sec-WebSocket-Protocol")
        && let Some(rest) = proto.strip_prefix("minecraft-v1,")
    {
        let origin_ok = req.header("Origin").is_some_and(|o| !o.is_empty() && shared.origins.contains(o));
        if !origin_ok {
            return Check::Denied("Origin Not Allowed");
        }
        return if valid_key(shared, rest.trim()) { Check::Allowed { token_in_protocol: true } } else { Check::Denied("Invalid API key") };
    }
    Check::Denied("Missing API key")
}

/// Reads until the end of the request head. `None` if the peer goes away, or the server stops.
fn read_head(wire: &mut dyn Wire, buf: &mut Vec<u8>, shared: &Shared) -> Option<usize> {
    let mut chunk = [0u8; 4096];
    wire.set_timeout(Duration::from_millis(200));
    loop {
        if let Some(end) = ws::head_end(buf) {
            return Some(end);
        }
        if buf.len() > 4 * ws::MAX_PAYLOAD || shared.stopping.load(Ordering::SeqCst) {
            return None;
        }
        match wire.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return None,
        }
    }
}

fn send(wire: &mut dyn Wire, bytes: &[u8]) -> bool {
    wire.write_all(bytes).and_then(|()| wire.flush()).is_ok()
}

fn send_text(wire: &mut dyn Wire, text: &str) -> bool {
    send(wire, &ws::encode_frame(ws::TEXT, text.as_bytes()))
}

fn peer_text(peer: SocketAddr) -> String {
    match peer {
        SocketAddr::V4(a) => format!("/{a}"),
        SocketAddr::V6(a) => format!("/[{}]:{}", a.ip(), a.port()),
    }
}

/// Serves one connection until it closes.
pub fn serve(mut wire: Box<dyn Wire>, peer: SocketAddr, id: u32, shared: Arc<Shared>) {
    let shown = peer_text(peer);
    shared.ctx.log(id, &format!("Management connection opened for {shown}"));
    let upgraded = upgrade(&mut *wire, peer, &shared);
    if let Some((token_in_protocol, leftover)) = upgraded {
        let _ = token_in_protocol;
        let notes = shared.ctx.hub.listen(id);
        talk(&mut *wire, id, &shared, &notes, leftover);
        shared.ctx.hub.forget(id);
    }
    wire.close();
    shared.ctx.log(id, &format!("Management connection closed for {shown}"));
}

/// The HTTP request: authentication, then the WebSocket handshake. `Some` with what followed the request.
fn upgrade(wire: &mut dyn Wire, peer: SocketAddr, shared: &Shared) -> Option<(bool, Vec<u8>)> {
    let mut buf = Vec::new();
    loop {
        let end = read_head(wire, &mut buf, shared)?;
        let sep = if buf[..end].ends_with(b"\r\n\r\n") { 4 } else { 2 };
        let head = String::from_utf8_lossy(&buf[..end - sep]).into_owned();
        let req = ws::parse_request(&head);
        // `HttpObjectAggregator`: a client that waits for permission to send a body gets it.
        if req.header("Expect").is_some_and(|e| e.eq_ignore_ascii_case("100-continue")) {
            send(wire, b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        // What the request carries as a body is read and dropped.
        let body = req.header("Content-Length").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(ws::MAX_PAYLOAD);
        while buf.len() < end + body {
            let mut chunk = [0u8; 4096];
            match wire.read(&mut chunk) {
                Ok(0) => return None,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if shared.stopping.load(Ordering::SeqCst) {
                        return None;
                    }
                }
                Err(_) => return None,
            }
        }
        let leftover = buf[end + body..].to_vec();
        buf.clear();
        let token_in_protocol = match security_check(shared, &req) {
            Check::Allowed { token_in_protocol } => token_in_protocol,
            Check::Denied(reason) => {
                debug!("Authentication rejected for connection with ip {}: {reason}", peer.ip());
                let body = format!("{{\"error\":\"Unauthorized\",\"message\":\"{reason}\"}}");
                send(wire, &ws::status_response(401, "Unauthorized", "application/json", &body, &[]));
                return None;
            }
        };
        // Only "/" is the WebSocket path; any other request is passed on to nobody (and never answered).
        if req.target != "/" {
            buf = leftover;
            continue;
        }
        match ws::handshake(&req) {
            ws::Handshake::Accept(key) => {
                let protocol = token_in_protocol.then_some("minecraft-v1");
                return send(wire, &ws::switching_protocols(&key, protocol)).then_some((token_in_protocol, leftover));
            }
            ws::Handshake::Draft(answer) => {
                // (The draft's own framing is not served: the connection stays open and says nothing.)
                send(wire, &answer);
                let mut chunk = [0u8; 1024];
                while !shared.stopping.load(Ordering::SeqCst) {
                    match wire.read(&mut chunk) {
                        Ok(0) => break,
                        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                        Err(_) => break,
                        Ok(_) => {}
                    }
                }
                return None;
            }
            ws::Handshake::Refuse(answer, close) => {
                send(wire, &answer);
                if close {
                    return None;
                }
                buf = leftover;
            }
        }
    }
}

/// Writes what happened since the last look, in order.
fn flush_notifications(wire: &mut dyn Wire, shared: &Shared, notes: &Receiver<Notification>) -> bool {
    loop {
        match notes.try_recv() {
            Ok(n) => {
                let text = notification_json(&n, &shared.ctx.info).to_string();
                if !send_text(wire, &text) {
                    return false;
                }
            }
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

fn talk(wire: &mut dyn Wire, id: u32, shared: &Shared, notes: &Receiver<Notification>, leftover: Vec<u8>) {
    let mut decoder = Decoder::default();
    let mut buf = leftover;
    let mut chunk = vec![0u8; 16 * 1024];
    wire.set_timeout(Duration::from_millis(25));
    loop {
        // Everything that is complete in the buffer.
        loop {
            match decoder.next(&mut buf) {
                Ok(Some(Message::Text(bytes))) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let reply = match json::parse(&text) {
                        Ok(message) => match rpc::handle_message(&shared.ctx, id, &message) {
                            Ok(reply) => reply,
                            Err(()) => return,
                        },
                        Err(e) => Some(rpc::parse_error(&e.0)),
                    };
                    if let Some(reply) = reply
                        && !send_text(wire, &reply.to_string())
                    {
                        return;
                    }
                    // (A notification raised by the request follows its reply.)
                    if !flush_notifications(wire, shared, notes) {
                        return;
                    }
                }
                Ok(Some(Message::Ping(p))) => {
                    if !send(wire, &ws::encode_frame(ws::PONG, &p)) {
                        return;
                    }
                }
                Ok(Some(Message::Pong | Message::Binary)) => {}
                Ok(Some(Message::Close(p))) => {
                    let _ = send(wire, &ws::encode_frame(ws::CLOSE, &p));
                    return;
                }
                Ok(None) => break,
                Err(WsError::TooBig) => {
                    let _ = send(wire, &ws::close_frame(1009, "Max frame length of 65536 has been exceeded."));
                    return;
                }
                Err(WsError::Protocol(why)) => {
                    let _ = send(wire, &ws::close_frame(1002, why));
                    return;
                }
            }
        }
        if !flush_notifications(wire, shared, notes) {
            return;
        }
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        match wire.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
    }
}
