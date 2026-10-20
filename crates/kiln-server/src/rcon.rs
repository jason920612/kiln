//! RCON, as vanilla's `RconThread` and `RconClient` serve it (the Source RCON protocol):
//! packets are `length`, `request id`, `type` (little-endian ints), a NUL-terminated body and one
//! more NUL; a login (type 3) with the password is answered with an auth response (type 2) with the
//! same id, or id -1 when the password is wrong; a command (type 2) with one or more
//! response packets (type 0) of at most 4096 characters each.
//!
//! Like vanilla's, this reads one chunk of up to 1460 bytes from the socket as one packet.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{info, warn};

/// What runs a command and returns what it printed (`ServerInterface.runCommand`).
pub type Runner = Arc<dyn Fn(&str) -> String + Send + Sync>;

const MAX_PACKET: usize = 1460;
const MAX_RESPONSE: usize = 4096;

pub const TYPE_RESPONSE: i32 = 0;
pub const TYPE_COMMAND: i32 = 2;
pub const TYPE_AUTH_RESPONSE: i32 = 2;
pub const TYPE_LOGIN: i32 = 3;

pub struct RconServer {
    running: Arc<AtomicBool>,
    clients: Arc<std::sync::Mutex<Vec<(Arc<AtomicBool>, TcpStream)>>>,
    addr: SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RconServer {
    /// `RconThread.create`: `None` (with the log lines vanilla writes) if the port or the password is unusable or the port is taken.
    pub fn start(ip: &str, port: i32, password: &str, runner: Runner) -> Option<RconServer> {
        let ip = if ip.is_empty() { "0.0.0.0" } else { ip };
        if !(1..=65535).contains(&port) {
            warn!("Invalid rcon port {port} found in server.properties, rcon disabled!");
            return None;
        }
        if password.is_empty() {
            warn!("No rcon password set in server.properties, rcon disabled!");
            return None;
        }
        let bound = ip.parse::<IpAddr>().ok().map(|ip| TcpListener::bind((ip, port as u16)));
        let listener = match bound {
            Some(Ok(l)) => l,
            Some(Err(e)) => {
                warn!("Unable to initialise RCON on {ip}:{port}: {e}");
                return None;
            }
            None => {
                warn!("Unable to initialise RCON on {ip}:{port}: unknown host");
                return None;
            }
        };
        info!("RCON running on {ip}:{port}");
        Self::from_listener(listener, password, runner)
    }

    /// Serves RCON on a bound listener.
    pub fn from_listener(listener: TcpListener, password: &str, runner: Runner) -> Option<RconServer> {
        let addr = listener.local_addr().ok()?;
        listener.set_nonblocking(true).ok()?;
        let running = Arc::new(AtomicBool::new(true));
        let clients: Arc<std::sync::Mutex<Vec<(Arc<AtomicBool>, TcpStream)>>> = Arc::default();
        info!("Thread RCON Listener started");
        let (run, cl, pw) = (running.clone(), clients.clone(), password.to_owned());
        let thread = std::thread::Builder::new()
            .name("RCON Listener".into())
            .spawn(move || accept_loop(listener, run, cl, pw, runner))
            .ok()?;
        Some(RconServer { running, clients, addr, thread: Some(thread) })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// `RconThread.stop`: closes the listener and every client.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let mut clients = self.clients.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (alive, stream) in clients.drain(..) {
            alive.store(false, Ordering::SeqCst);
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        info!("Thread RCON Listener stopped");
    }
}

impl Drop for RconServer {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}

fn accept_loop(
    listener: TcpListener,
    running: Arc<AtomicBool>,
    clients: Arc<std::sync::Mutex<Vec<(Arc<AtomicBool>, TcpStream)>>>,
    password: String,
    runner: Runner,
) {
    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                let alive = Arc::new(AtomicBool::new(true));
                if let Ok(copy) = stream.try_clone() {
                    let mut list = clients.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    list.retain(|(a, _)| a.load(Ordering::SeqCst));
                    list.push((alive.clone(), copy));
                }
                let (pw, run, flag) = (password.clone(), runner.clone(), alive);
                let name = format!("RCON Client /{}", peer.ip());
                info!("Thread {name} started");
                let n = name.clone();
                let _ = std::thread::Builder::new().name(name).spawn(move || {
                    client_loop(&stream, &pw, &run, &flag);
                    // (The listener keeps a handle too; the connection closes now, not when that one goes.)
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    flag.store(false, Ordering::SeqCst);
                    info!("Thread {n} shutting down");
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    info!("IO exception: {e}");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// `PktUtils.intFromByteArray(buf, start, end)`: a little-endian int, 0 if fewer than 4 bytes remain.
fn int_le(buf: &[u8], start: usize, end: usize) -> i32 {
    if end < start + 4 || buf.len() < start + 4 {
        return 0;
    }
    i32::from_le_bytes([buf[start], buf[start + 1], buf[start + 2], buf[start + 3]])
}

/// `PktUtils.stringFromByteArray(buf, start, end)`: up to the first NUL, but not past `end - 1`.
fn string_at(buf: &[u8], start: usize, end: usize) -> String {
    let limit = end.saturating_sub(1);
    let mut j = start.min(limit);
    while j < limit && buf[j] != 0 {
        j += 1;
    }
    String::from_utf8_lossy(&buf[start.min(j)..j]).into_owned()
}

/// A packet: `length`, `id`, `type`, the body, two NULs.
pub fn packet(id: i32, ty: i32, body: &str) -> Vec<u8> {
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 14);
    out.extend_from_slice(&(bytes.len() as i32 + 10).to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(&[0, 0]);
    out
}

/// `sendCmdResponse`: the response in pieces of at most 4096 characters (UTF-16 units), at least one packet.
pub fn response_packets(id: i32, response: &str) -> Vec<Vec<u8>> {
    let units: Vec<u16> = response.encode_utf16().collect();
    let mut out = Vec::new();
    let mut rest = &units[..];
    loop {
        let n = rest.len().min(MAX_RESPONSE);
        out.push(packet(id, TYPE_RESPONSE, &String::from_utf16_lossy(&rest[..n])));
        rest = &rest[n..];
        if rest.is_empty() {
            return out;
        }
    }
}

fn client_loop(mut stream: &TcpStream, password: &str, runner: &Runner, alive: &AtomicBool) {
    let mut buf = [0u8; MAX_PACKET];
    let mut authed = false;
    while alive.load(Ordering::SeqCst) {
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if n < 10 {
            return;
        }
        let length = int_le(&buf, 0, n);
        if length != n as i32 - 4 {
            return;
        }
        let id = int_le(&buf, 4, n);
        let ty = int_le(&buf, 8, n);
        let sent = match ty {
            TYPE_LOGIN => {
                let given = string_at(&buf, 12, n);
                if !given.is_empty() && given == password {
                    authed = true;
                    stream.write_all(&packet(id, TYPE_AUTH_RESPONSE, ""))
                } else {
                    authed = false;
                    stream.write_all(&packet(-1, TYPE_AUTH_RESPONSE, ""))
                }
            }
            TYPE_COMMAND if authed => {
                let command = string_at(&buf, 12, n);
                let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner(&command)))
                    .unwrap_or_else(|_| format!("Error executing: {command} (command failed)"));
                response_packets(id, &reply).iter().try_for_each(|p| stream.write_all(p))
            }
            TYPE_COMMAND => stream.write_all(&packet(-1, TYPE_AUTH_RESPONSE, "")),
            other => response_packets(id, &format!("Unknown request {other:x}")).iter().try_for_each(|p| stream.write_all(p)),
        };
        if sent.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
