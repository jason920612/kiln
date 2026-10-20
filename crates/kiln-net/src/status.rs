//! The server list: the status response (`MinecraftServer.buildServerStatus`, the JSON of
//! `ServerStatus.CODEC`), its player sample, and the pre-1.7 pings that
//! `LegacyQueryHandler` still answers.

use kiln_link::{RosterEntry, ServerInfo};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// `MinecraftServer.STATUS_EXPIRE_TIME_NANOS`: the status is rebuilt this often.
pub const REBUILD_EVERY: Duration = Duration::from_secs(5);

/// `ServerStatus.Players.sample` entries show at most this many players.
pub const SAMPLE_SIZE: usize = 12;

/// `MinecraftServer.ANONYMOUS_PLAYER_PROFILE`: who a player with "Allow Server Listings" off is.
pub const ANONYMOUS: (Uuid, &str) = (Uuid::nil(), "Anonymous Player");

/// Gson without HTML escaping, as vanilla's `ServerStatus` JSON is written: `"` and `\`, the
/// control characters (`\b \t \n \f \r` short, the rest `\u00xx`), and U+2028/U+2029.
pub fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What goes into one status response.
pub struct Status<'a> {
    pub motd: &'a str,
    pub max_players: i32,
    pub online: usize,
    /// `None`: the `players` object has no sample (`hide-online-players`).
    pub sample: Option<&'a [(Uuid, String)]>,
    /// The `data:image/png;base64,...` favicon.
    pub favicon: Option<&'a str>,
    pub version_name: &'a str,
    pub protocol: i32,
    pub enforces_secure_chat: bool,
}

impl Status<'_> {
    /// `ServerStatus.CODEC` through `JsonOps`: the fields in the codec's order; the ones that
    /// equal their default (an empty description or sample, `enforcesSecureChat: false`) left out.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        if !self.motd.is_empty() {
            out.push_str("\"description\":");
            out.push_str(&json_string(self.motd));
            out.push(',');
        }
        out.push_str(&format!("\"players\":{{\"max\":{},\"online\":{}", self.max_players, self.online));
        if let Some(sample) = self.sample.filter(|s| !s.is_empty()) {
            out.push_str(",\"sample\":[");
            for (i, (id, name)) in sample.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&format!("{{\"id\":\"{id}\",\"name\":{}}}", json_string(name)));
            }
            out.push(']');
        }
        out.push_str(&format!(
            "}},\"version\":{{\"name\":{},\"protocol\":{}}}",
            json_string(self.version_name),
            self.protocol
        ));
        if let Some(icon) = self.favicon {
            out.push_str(",\"favicon\":");
            out.push_str(&json_string(icon));
        }
        if self.enforces_secure_chat {
            out.push_str(",\"enforcesSecureChat\":true");
        }
        out.push('}');
        out
    }
}

/// `java.util.Random`-like source for the sample (`MinecraftServer.random` is unseeded).
pub struct SampleRng(u64);

impl SampleRng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5DEE_CE66_D)
    }

    /// `RandomSource.nextInt(bound)`.
    pub fn next_int(&mut self, bound: usize) -> usize {
        // xorshift64*; only the spread matters, vanilla's is not reproducible either.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % bound.max(1)
    }
}

/// `MinecraftServer.buildPlayerStatus`: up to 12 consecutive players from a random start, the
/// ones who do not allow listing as the anonymous profile, in shuffled order.
pub fn pick_sample(players: &[RosterEntry], rng: &mut SampleRng) -> Vec<(Uuid, String)> {
    let take = players.len().min(SAMPLE_SIZE);
    // `Mth.nextInt(random, 0, size - take)` is inclusive on both ends.
    let start = rng.next_int(players.len() - take + 1);
    let mut list: Vec<(Uuid, String)> = players[start..start + take]
        .iter()
        .map(|p| if p.allows_listing { (p.uuid, p.name.clone()) } else { (ANONYMOUS.0, ANONYMOUS.1.to_owned()) })
        .collect();
    // `Util.shuffle`: Fisher-Yates from the end.
    for i in (2..=list.len()).rev() {
        list.swap(i - 1, rng.next_int(i));
    }
    list
}

/// The status JSON kept for 5 seconds, as `MinecraftServer.status` is.
#[derive(Default)]
pub struct StatusCache {
    built: Option<(Instant, String)>,
}

impl StatusCache {
    pub fn get(&mut self, now: Instant, build: impl FnOnce() -> String) -> &str {
        if self.built.as_ref().is_none_or(|(at, _)| now.duration_since(*at) >= REBUILD_EVERY) {
            self.built = Some((now, build()));
        }
        &self.built.as_ref().expect("just built").1
    }
}

/// The current status of the server, built from `info`.
pub fn build(info: &ServerInfo, favicon: Option<&str>, enforces_secure_chat: bool, rng: &mut SampleRng) -> String {
    let players = info.players();
    let hidden = info.hide_online_players.load(std::sync::atomic::Ordering::Relaxed);
    let sample = (!hidden).then(|| pick_sample(&players, rng));
    Status {
        motd: &info.motd(),
        max_players: info.max_players(),
        online: players.len(),
        sample: sample.as_deref(),
        favicon,
        version_name: &info.version_name,
        protocol: kiln_data::version::PROTOCOL,
        enforces_secure_chat,
    }
    .to_json()
}

/// `ServerStatus.Favicon.CODEC` encoding of an icon file: `server-icon.png` must be a 64x64
/// PNG (`PngInfo.fromBytes`), else vanilla logs "Couldn't load server icon" and sends none.
pub fn favicon_from_png(bytes: &[u8]) -> Result<String, String> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err("Invalid PNG file".to_owned());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("4 bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("4 bytes"));
    if width != 64 || height != 64 {
        return Err(format!("Must be 64 pixels wide and high, but was {width}x{height}"));
    }
    Ok(format!("data:image/png;base64,{}", base64(bytes)))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// ---- the pre-1.7 pings ------------------------------------------------------------------

/// `LegacyProtocolUtils.writeLegacyString` after the `0xFF` kick byte: the length in UTF-16
/// units, then the text in UTF-16BE.
fn legacy_kick(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = vec![0xff];
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

/// A client's first bytes (starting with `0xFE`) answered the way `LegacyQueryHandler` does:
/// `FE` alone (before 1.3): `motd§players§max`; `FE 01` (1.4 to 1.5) and the 1.6 `FE 01 FA
/// MC|PingHost` ping: `§1 NUL protocol NUL version NUL motd NUL players NUL max`. `None` when
/// the bytes are not such a ping (the connection is then handled as an ordinary one).
pub fn legacy_ping_reply(bytes: &[u8], motd: &str, version: &str, players: usize, max: i32) -> Option<Vec<u8>> {
    if bytes.first() != Some(&0xfe) {
        return None;
    }
    let v1 = || legacy_kick(&format!("\u{a7}1\0{}\0{version}\0{motd}\0{players}\0{max}", 127));
    match bytes.len() {
        1 => Some(legacy_kick(&format!("{motd}\u{a7}{players}\u{a7}{max}"))),
        _ if bytes[1] != 1 => None,
        2 => Some(v1()),
        _ => legacy_ping_host(&bytes[2..]).then(v1),
    }
}

/// `LegacyQueryHandler.readCustomPayloadPacket`: `FA`, the string `MC|PingHost`, the length of
/// the rest, a protocol byte of at least 73, the host and a port.
fn legacy_ping_host(b: &[u8]) -> bool {
    fn string(b: &[u8]) -> Option<(String, &[u8])> {
        let n = u16::from_be_bytes(b.get(..2)?.try_into().ok()?) as usize;
        let raw = b.get(2..2 + 2 * n)?;
        let units: Vec<u16> = raw.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Some((String::from_utf16_lossy(&units), &b[2 + 2 * n..]))
    }
    let Some((0xfa, rest)) = b.split_first().map(|(f, r)| (*f, r)) else { return false };
    let Some((channel, rest)) = string(rest) else { return false };
    if channel != "MC|PingHost" {
        return false;
    }
    let Some(len) = rest.get(..2).map(|l| u16::from_be_bytes([l[0], l[1]]) as usize) else { return false };
    let rest = &rest[2..];
    if rest.len() != len {
        return false;
    }
    let Some((&protocol, rest)) = rest.split_first() else { return false };
    if protocol < 73 {
        return false;
    }
    let Some((_host, rest)) = string(rest) else { return false };
    let Some(port) = rest.get(..4).map(|p| i32::from_be_bytes([p[0], p[1], p[2], p[3]])) else { return false };
    port <= 65535
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u8, listing: bool) -> RosterEntry {
        RosterEntry { conn: u64::from(n), uuid: Uuid::from_u128(u128::from(n)), name: format!("P{n}"), allows_listing: listing }
    }

    #[test]
    fn json_matches_vanillas_codec() {
        let s = Status {
            motd: "<b>&amp; 'q' = \"x\" \u{e9} \u{2028} \t end",
            max_players: 20,
            online: 0,
            sample: Some(&[]),
            favicon: None,
            version_name: "26.3",
            protocol: 777,
            enforces_secure_chat: false,
        };
        // Taken from the vanilla 26.3 server list reply for the same motd.
        assert_eq!(
            s.to_json(),
            "{\"description\":\"<b>&amp; 'q' = \\\"x\\\" \u{e9} \\u2028 \\t end\",\"players\":{\"max\":20,\"online\":0},\"version\":{\"name\":\"26.3\",\"protocol\":777}}"
        );
        let id = Uuid::from_u128(5);
        let one = [(id, "Ann".to_owned())];
        let with = Status { sample: Some(&one), online: 1, favicon: Some("data:image/png;base64,AA=="), enforces_secure_chat: true, ..s };
        assert_eq!(
            with.to_json(),
            format!(
                "{{\"description\":{},\"players\":{{\"max\":20,\"online\":1,\"sample\":[{{\"id\":\"{id}\",\"name\":\"Ann\"}}]}},\"version\":{{\"name\":\"26.3\",\"protocol\":777}},\"favicon\":\"data:image/png;base64,AA==\",\"enforcesSecureChat\":true}}",
                json_string(s.motd)
            )
        );
    }

    #[test]
    fn sample_is_twelve_consecutive_shuffled_with_anonymous_for_unlisted() {
        let players: Vec<_> = (1..=30).map(|n| entry(n, n % 5 != 0)).collect();
        let mut rng = SampleRng::new(7);
        for _ in 0..50 {
            let sample = pick_sample(&players, &mut rng);
            assert_eq!(sample.len(), 12);
            let mut ids: Vec<u128> = sample.iter().filter(|(id, _)| !id.is_nil()).map(|(id, _)| id.as_u128()).collect();
            ids.sort_unstable();
            // Consecutive roster slots: the listed ones among them, in a window of 12.
            let (first, last) = (ids[0], *ids.last().unwrap());
            assert!(last - first < 12, "{ids:?}");
            assert!(sample.iter().filter(|(id, _)| id.is_nil()).all(|(_, n)| n == "Anonymous Player"));
        }
        assert_eq!(pick_sample(&players[..3], &mut rng).len(), 3);
        assert!(pick_sample(&[], &mut rng).is_empty());
    }

    #[test]
    fn legacy_pings() {
        // 1.4/1.5: FE 01.
        let r = legacy_ping_reply(&[0xfe, 0x01], "A Minecraft Server", "26.3", 2, 20).unwrap();
        assert_eq!(r[0], 0xff);
        let units: Vec<u16> = r[3..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        assert_eq!(u16::from_be_bytes([r[1], r[2]]) as usize, units.len());
        assert_eq!(String::from_utf16(&units).unwrap(), "\u{a7}1\0127\026.3\0A Minecraft Server\02\020");
        // before 1.3: FE alone.
        let r = legacy_ping_reply(&[0xfe], "m", "26.3", 2, 20).unwrap();
        let units: Vec<u16> = r[3..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "m\u{a7}2\u{a7}20");
        // 1.6 with MC|PingHost.
        let mut ping = vec![0xfe, 0x01, 0xfa, 0x00, 0x0b];
        for u in "MC|PingHost".encode_utf16() {
            ping.extend_from_slice(&u.to_be_bytes());
        }
        let host: Vec<u8> = "localhost".encode_utf16().flat_map(u16::to_be_bytes).collect();
        ping.extend_from_slice(&((7 + host.len()) as u16).to_be_bytes());
        ping.push(78);
        ping.extend_from_slice(&9u16.to_be_bytes());
        ping.extend_from_slice(&host);
        ping.extend_from_slice(&25565i32.to_be_bytes());
        assert!(legacy_ping_reply(&ping, "m", "26.3", 0, 20).is_some());
        *ping.last_mut().unwrap() ^= 1;
        let n = ping.len();
        ping[n - 4] = 1; // port 0x01000000 + ...: over 65535
        assert!(legacy_ping_reply(&ping, "m", "26.3", 0, 20).is_none());
        assert!(legacy_ping_reply(&[0xfe, 0x02], "m", "26.3", 0, 20).is_none());
        assert!(legacy_ping_reply(&[0x10], "m", "26.3", 0, 20).is_none());
    }

    #[test]
    fn favicon_must_be_a_64x64_png() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        let icon = favicon_from_png(&png).unwrap();
        assert!(icon.starts_with("data:image/png;base64,iVBORw0KGgo"));
        png[19] = 32;
        assert!(favicon_from_png(&png).unwrap_err().contains("64"));
        assert!(favicon_from_png(b"nope").is_err());
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"abc"), "YWJj");
    }
}
