//! HTTP upgrade and WebSocket framing (RFC 6455), as netty's `HttpServerCodec`,
//! `WebSocketServerProtocolHandler` ("/") and `WebSocketFrameAggregator` serve the management server.

use base64::Engine as _;
use sha1::{Digest, Sha1};

/// `HttpObjectAggregator`, `WebSocketServerProtocolHandler` and `WebSocketFrameAggregator`: 65536 bytes.
pub const MAX_PAYLOAD: usize = 65536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
}

impl HttpRequest {
    /// The first header called `name` (any case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Parses the head of a request (everything up to the blank line); `None` if it is no HTTP.
pub fn parse_request(head: &str) -> Option<HttpRequest> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    if !first.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let headers = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())))
        .collect();
    Some(HttpRequest { method, target, headers })
}

/// The index just past the blank line that ends a request head.
pub fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// `Sec-WebSocket-Accept` for a key.
pub fn accept_key(key: &str) -> String {
    let mut h = Sha1::new();
    h.update(key.as_bytes());
    h.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(h.finalize())
}

/// A WebSocket handshake request the way netty checks one: GET, `Upgrade: websocket`, `Connection: Upgrade`,
/// a key and version 13. Err is the status line and reason of the refusal.
pub fn check_handshake(req: &HttpRequest) -> Result<String, (u16, &'static str)> {
    if req.method != "GET" {
        return Err((403, "Forbidden"));
    }
    let has = |name: &str, token: &str| req.header(name).is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)));
    if !has("Connection", "upgrade") || !has("Upgrade", "websocket") {
        return Err((400, "Bad Request"));
    }
    match req.header("Sec-WebSocket-Version") {
        Some("13") => {}
        _ => return Err((426, "Upgrade Required")),
    }
    match req.header("Sec-WebSocket-Key") {
        Some(k) if !k.is_empty() => Ok(k.to_owned()),
        _ => Err((400, "Bad Request")),
    }
}

pub fn switching_protocols(key: &str, protocol: Option<&str>) -> Vec<u8> {
    let mut s = format!("HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: upgrade\r\nsec-websocket-accept: {}\r\n", accept_key(key));
    if let Some(p) = protocol {
        s.push_str(&format!("sec-websocket-protocol: {p}\r\n"));
    }
    s.push_str("\r\n");
    s.into_bytes()
}

pub fn status_response(code: u16, reason: &str, content_type: &str, body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut s = format!("HTTP/1.1 {code} {reason}\r\n");
    for (k, v) in extra {
        s.push_str(&format!("{k}: {v}\r\n"));
    }
    if !content_type.is_empty() {
        s.push_str(&format!("content-type: {content_type}\r\n"));
    }
    s.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()));
    s.into_bytes()
}

pub const TEXT: u8 = 1;
pub const BINARY: u8 = 2;
pub const CLOSE: u8 = 8;
pub const PING: u8 = 9;
pub const PONG: u8 = 10;

/// A server frame (never masked).
pub fn encode_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode);
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n < 65536 => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

pub fn close_frame(code: u16, reason: &str) -> Vec<u8> {
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend_from_slice(reason.as_bytes());
    encode_frame(CLOSE, &payload)
}

#[derive(Debug, PartialEq, Eq)]
pub enum WsError {
    /// 1002: an unmasked client frame, a reserved bit or opcode, a fragmented control frame.
    Protocol(&'static str),
    /// 1009.
    TooBig,
}

/// What the client sent: complete messages (fragments joined) and control frames.
#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    Text(Vec<u8>),
    Binary,
    Ping(Vec<u8>),
    Pong,
    Close(Vec<u8>),
}

/// Reads frames out of what has arrived; keeps a started fragmented message between calls.
#[derive(Default)]
pub struct Decoder {
    fragment: Option<(u8, Vec<u8>)>,
}

impl Decoder {
    /// The next message in `buf` (consumed from it), if all of it has arrived.
    pub fn next(&mut self, buf: &mut Vec<u8>) -> Result<Option<Message>, WsError> {
        loop {
            if buf.len() < 2 {
                return Ok(None);
            }
            let (b0, b1) = (buf[0], buf[1]);
            let fin = b0 & 0x80 != 0;
            if b0 & 0x70 != 0 {
                return Err(WsError::Protocol("RSV bits"));
            }
            let opcode = b0 & 0x0f;
            if b1 & 0x80 == 0 {
                return Err(WsError::Protocol("unmasked client frame"));
            }
            let mut len = u64::from(b1 & 0x7f);
            let mut at = 2;
            if len == 126 {
                if buf.len() < 4 {
                    return Ok(None);
                }
                len = u64::from(u16::from_be_bytes([buf[2], buf[3]]));
                at = 4;
            } else if len == 127 {
                if buf.len() < 10 {
                    return Ok(None);
                }
                len = u64::from_be_bytes(buf[2..10].try_into().expect("8 bytes"));
                at = 10;
            }
            if len > MAX_PAYLOAD as u64 {
                return Err(WsError::TooBig);
            }
            let len = len as usize;
            if buf.len() < at + 4 + len {
                return Ok(None);
            }
            let mask = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
            let payload: Vec<u8> = buf[at + 4..at + 4 + len].iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect();
            buf.drain(..at + 4 + len);
            match opcode {
                8..=10 => {
                    if !fin || len > 125 {
                        return Err(WsError::Protocol("control frame"));
                    }
                    return Ok(Some(match opcode {
                        CLOSE => Message::Close(payload),
                        PING => Message::Ping(payload),
                        _ => Message::Pong,
                    }));
                }
                0 => {
                    let Some((first, mut data)) = self.fragment.take() else { return Err(WsError::Protocol("stray continuation")) };
                    data.extend_from_slice(&payload);
                    if data.len() > MAX_PAYLOAD {
                        return Err(WsError::TooBig);
                    }
                    if fin {
                        return Ok(Some(if first == TEXT { Message::Text(data) } else { Message::Binary }));
                    }
                    self.fragment = Some((first, data));
                }
                TEXT | BINARY => {
                    if self.fragment.is_some() {
                        return Err(WsError::Protocol("new message inside a fragmented one"));
                    }
                    if fin {
                        return Ok(Some(if opcode == TEXT { Message::Text(payload) } else { Message::Binary }));
                    }
                    self.fragment = Some((opcode, payload));
                }
                _ => return Err(WsError::Protocol("opcode")),
            }
        }
    }
}
