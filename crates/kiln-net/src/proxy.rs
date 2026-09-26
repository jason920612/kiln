//! Player info forwarding from a proxy in front of Kiln: Velocity modern forwarding (an
//! HMAC-signed login plugin answer) and BungeeCord legacy forwarding (extra fields in the
//! handshake host), optionally hardened with BungeeGuard tokens.

use crate::profile::{GameProfile, PropertyJson, check_properties, read_properties};
use hmac::{Hmac, Mac};
use kiln_link::Property;
use kiln_proto::{DecodeError, Reader};
use sha2::Sha256;
use std::net::IpAddr;
use subtle::ConstantTimeEq;
use uuid::Uuid;

pub enum ProxyMode {
    None,
    /// Velocity modern forwarding; `secret` is the proxy's `forwarding.secret`.
    Velocity {
        secret: Vec<u8>,
    },
    /// BungeeCord legacy forwarding. With tokens set, every login must carry one of them in a
    /// `bungeeguard-token` property (BungeeGuard); without, anyone who can reach the port can
    /// claim any identity, so the port must be firewalled to the proxy.
    BungeeCord {
        tokens: Vec<String>,
    },
}

/// Handshake host length vanilla accepts, and the length allowed under legacy forwarding.
pub const MAX_HOST: usize = 255;
pub const MAX_FORWARDED_HOST: usize = 32767;

pub const VELOCITY_CHANNEL: &str = "velocity:player_info";
/// Highest forwarding version we understand: 1 plain, 2 and 3 add a chat signing key
/// (3 also its holder), 4 is 1 again for proxies that fetch chat sessions lazily.
pub const VELOCITY_MAX_VERSION: u8 = 4;

const BUNGEEGUARD_PROPERTY: &str = "bungeeguard-token";

/// Identity and address of a player as vouched for by the proxy.
#[derive(Debug, PartialEq, Eq)]
pub struct Forwarded {
    pub address: IpAddr,
    pub profile: GameProfile,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ForwardError {
    #[error("no forwarding data (is forwarding enabled on the proxy?)")]
    Missing,
    #[error("forwarding signature does not match the secret")]
    BadSignature,
    #[error("unsupported forwarding version {0}")]
    UnsupportedVersion(i32),
    #[error("invalid forwarded address {0:?}")]
    BadAddress(String),
    #[error("invalid forwarded UUID")]
    BadUuid,
    #[error("invalid forwarded name {0:?}")]
    BadName(String),
    #[error("malformed forwarding data: {0}")]
    Malformed(String),
    #[error("missing or wrong BungeeGuard token")]
    BadToken,
}

impl From<DecodeError> for ForwardError {
    fn from(e: DecodeError) -> Self {
        ForwardError::Malformed(e.to_string())
    }
}

/// The request payload: the highest version we accept, as a single byte.
pub fn velocity_request() -> [u8; 1] {
    [VELOCITY_MAX_VERSION]
}

/// Verifies and parses Velocity's answer: a 32-byte HMAC-SHA256 over the rest, then
/// version, address, UUID, name, properties and, for versions 2 and 3, the chat key.
pub fn velocity_verify(secret: &[u8], payload: &[u8]) -> Result<Forwarded, ForwardError> {
    if payload.len() < 32 {
        return Err(ForwardError::Malformed("shorter than the signature".into()));
    }
    let (signature, data) = payload.split_at(32);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(data);
    // Constant-time comparison.
    mac.verify_slice(signature).map_err(|_| ForwardError::BadSignature)?;

    let mut r = Reader::new(data);
    let version = r.varint()?;
    if !(1..=VELOCITY_MAX_VERSION as i32).contains(&version) {
        return Err(ForwardError::UnsupportedVersion(version));
    }
    let address = parse_address(r.string(MAX_HOST)?)?;
    let uuid = r.uuid()?;
    let name = r.string(16)?.to_owned();
    let properties = read_properties(&mut r)?;
    if version == 2 || version == 3 {
        let _expires_at = r.i64()?;
        let key_len = r.len()?;
        r.bytes(key_len)?;
        let signature_len = r.len()?;
        r.bytes(signature_len)?;
        if version == 3 && r.bool()? {
            let _holder = r.uuid()?;
        }
    }
    r.finish()?;
    Ok(Forwarded { address, profile: checked_profile(uuid, name, properties)? })
}

/// Parses a legacy-forwarded handshake host, `host\0address\0uuid[\0propertiesJson]`, and
/// checks BungeeGuard tokens when any are configured. The name comes from Login Start.
pub fn bungee_parse(host: &str, name: &str, tokens: &[String]) -> Result<Forwarded, ForwardError> {
    let mut parts = host.split('\0');
    // The host the client connected to is unused, including any 26.4-style `?key=value` suffix.
    let _host = parts.next();
    let (Some(address), Some(uuid)) = (parts.next(), parts.next()) else {
        return Err(ForwardError::Missing);
    };
    let json = parts.next().filter(|p| !p.is_empty());
    if parts.next().is_some() {
        return Err(ForwardError::Malformed("too many host fields".into()));
    }
    let address = parse_address(address)?;
    let uuid = Uuid::try_parse(uuid).map_err(|_| ForwardError::BadUuid)?;
    let mut properties: Vec<Property> = match json {
        Some(json) => serde_json::from_str::<Vec<PropertyJson>>(json)
            .map_err(|e| ForwardError::Malformed(e.to_string()))?
            .into_iter()
            .map(Property::from)
            .collect(),
        None => Vec::new(),
    };

    let mut presented = Vec::new();
    properties.retain(|p| {
        let token = p.name == BUNGEEGUARD_PROPERTY;
        if token {
            presented.push(p.value.clone());
        }
        !token
    });
    if !tokens.is_empty() {
        // Exactly one token: a duplicate could be a client-supplied property the proxy passed on.
        let [token] = presented.as_slice() else { return Err(ForwardError::BadToken) };
        let known = tokens.iter().fold(0u8, |ok, t| ok | t.as_bytes().ct_eq(token.as_bytes()).unwrap_u8());
        if known == 0 {
            return Err(ForwardError::BadToken);
        }
    }
    Ok(Forwarded { address, profile: checked_profile(uuid, name.to_owned(), properties)? })
}

fn parse_address(s: &str) -> Result<IpAddr, ForwardError> {
    // Tolerate `[v6]` brackets and a `%zone` suffix.
    let bare = s.trim_start_matches('[').trim_end_matches(']');
    let bare = bare.split_once('%').map_or(bare, |(a, _)| a);
    bare.parse().map_err(|_| ForwardError::BadAddress(s.to_owned()))
}

fn checked_profile(uuid: Uuid, name: String, properties: Vec<Property>) -> Result<GameProfile, ForwardError> {
    if uuid.is_nil() {
        return Err(ForwardError::BadUuid);
    }
    if name.is_empty() || name.chars().count() > 16 {
        return Err(ForwardError::BadName(name));
    }
    check_properties(&properties).map_err(|e| ForwardError::Malformed(e.into()))?;
    Ok(GameProfile { uuid, name, properties })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};
    use kiln_proto::WriteExt;

    pub(crate) const SECRET: &[u8] = b"kiln-test-secret";
    const UUID: &str = "069a79f4-44e9-4726-a5be-fca90e38aaf5";

    /// Builds forwarding data the way Velocity lays it out, signed with `secret`.
    pub(crate) fn velocity_payload(
        secret: &[u8],
        version: i32,
        address: &str,
        extra: impl FnOnce(&mut BytesMut),
    ) -> Vec<u8> {
        let mut data = BytesMut::new();
        data.put_varint(version);
        data.put_string(address);
        data.put_uuid(Uuid::parse_str(UUID).unwrap());
        data.put_string("Notch");
        data.put_varint(1);
        data.put_string("textures");
        data.put_string("eyJ0ZXh0dXJlcyI6e319");
        data.put_bool(true);
        data.put_string("c2lnbmF0dXJl");
        extra(&mut data);
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(&data);
        let mut payload = mac.finalize().into_bytes().to_vec();
        payload.extend_from_slice(&data);
        payload
    }

    #[test]
    fn hmac_sha256_rfc4231_case_2() {
        // Keeps the MAC construction honest independently of our own payload builder.
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        let hex: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, expected);
    }

    #[test]
    fn velocity_accepts_signed_v4() {
        let fwd = velocity_verify(SECRET, &velocity_payload(SECRET, 4, "203.0.113.7", |_| {})).unwrap();
        assert_eq!(fwd.address, "203.0.113.7".parse::<IpAddr>().unwrap());
        assert_eq!(fwd.profile.uuid.to_string(), UUID);
        assert_eq!(fwd.profile.name, "Notch");
        assert_eq!(fwd.profile.properties[0].name, "textures");
        assert_eq!(fwd.profile.properties[0].signature.as_deref(), Some("c2lnbmF0dXJl"));
    }

    #[test]
    fn velocity_accepts_a_payload_captured_from_velocity() {
        // Velocity 4.2.1-SNAPSHOT (build 32), offline mode, secret "kiln-test-secret", answering
        // our version 4 request for player ProxyBot connecting from 127.0.0.2.
        let payload = crate::auth::tests::hex(include_str!("testdata/velocity_v4_payload.hex"));
        let fwd = velocity_verify(SECRET, &payload).unwrap();
        assert_eq!(fwd.address, "127.0.0.2".parse::<IpAddr>().unwrap());
        assert_eq!(fwd.profile.name, "ProxyBot");
        assert_eq!(fwd.profile.uuid, crate::profile::offline_uuid("ProxyBot"));
        assert!(fwd.profile.properties.is_empty());
        assert_eq!(velocity_verify(b"kiln-test-secreT", &payload), Err(ForwardError::BadSignature));
    }

    #[test]
    fn velocity_accepts_v1_and_v6_address() {
        let fwd = velocity_verify(SECRET, &velocity_payload(SECRET, 1, "0:0:0:0:0:0:0:1", |_| {})).unwrap();
        assert_eq!(fwd.address, "::1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn velocity_skips_chat_key_in_v2_and_v3() {
        let key = |b: &mut BytesMut| {
            b.put_i64(1_700_000_000_000);
            b.put_varint(3);
            b.put_slice(&[1, 2, 3]);
            b.put_varint(2);
            b.put_slice(&[4, 5]);
        };
        assert!(velocity_verify(SECRET, &velocity_payload(SECRET, 2, "127.0.0.1", key)).is_ok());
        let v3 = |b: &mut BytesMut| {
            key(b);
            b.put_bool(true);
            b.put_uuid(Uuid::from_u128(7));
        };
        assert!(velocity_verify(SECRET, &velocity_payload(SECRET, 3, "127.0.0.1", v3)).is_ok());
        // v2 without the key is truncated.
        assert!(matches!(
            velocity_verify(SECRET, &velocity_payload(SECRET, 2, "127.0.0.1", |_| {})),
            Err(ForwardError::Malformed(_))
        ));
    }

    #[test]
    fn velocity_rejects_wrong_secret_and_tampering() {
        let payload = velocity_payload(b"another-secret", 4, "127.0.0.1", |_| {});
        assert_eq!(velocity_verify(SECRET, &payload), Err(ForwardError::BadSignature));

        let mut payload = velocity_payload(SECRET, 4, "127.0.0.1", |_| {});
        let last = payload.len() - 1;
        payload[last] ^= 1;
        assert_eq!(velocity_verify(SECRET, &payload), Err(ForwardError::BadSignature));

        let mut payload = velocity_payload(SECRET, 4, "127.0.0.1", |_| {});
        payload[0] ^= 0x80;
        assert_eq!(velocity_verify(SECRET, &payload), Err(ForwardError::BadSignature));

        assert!(matches!(velocity_verify(SECRET, &[0; 31]), Err(ForwardError::Malformed(_))));
    }

    #[test]
    fn velocity_rejects_unknown_version_and_trailing_bytes() {
        let payload = velocity_payload(SECRET, 5, "127.0.0.1", |_| {});
        assert_eq!(velocity_verify(SECRET, &payload), Err(ForwardError::UnsupportedVersion(5)));
        let payload = velocity_payload(SECRET, 4, "127.0.0.1", |b| b.put_u8(0));
        assert!(matches!(velocity_verify(SECRET, &payload), Err(ForwardError::Malformed(_))));
        let payload = velocity_payload(SECRET, 4, "not-an-ip", |_| {});
        assert!(matches!(velocity_verify(SECRET, &payload), Err(ForwardError::BadAddress(_))));
    }

    const TEXTURES: &str = r#"{"name":"textures","value":"e30=","signature":"c2ln"}"#;

    #[test]
    fn bungee_parses_forwarded_host() {
        let host = format!("mc.example.com\x00198.51.100.4\x00069a79f444e94726a5befca90e38aaf5\x00[{TEXTURES}]");
        let fwd = bungee_parse(&host, "Notch", &[]).unwrap();
        assert_eq!(fwd.address, "198.51.100.4".parse::<IpAddr>().unwrap());
        assert_eq!(fwd.profile.uuid.to_string(), UUID);
        assert_eq!(fwd.profile.name, "Notch");
        assert_eq!(
            fwd.profile.properties,
            vec![Property { name: "textures".into(), value: "e30=".into(), signature: Some("c2ln".into()) }]
        );
    }

    #[test]
    fn bungee_without_properties_and_with_query_suffix() {
        let host =
            "mc.example.com?_id=lobby&_o=example.com:25565\x002001:db8::1\x00069a79f4-44e9-4726-a5be-fca90e38aaf5";
        let fwd = bungee_parse(host, "Notch", &[]).unwrap();
        assert_eq!(fwd.address, "2001:db8::1".parse::<IpAddr>().unwrap());
        assert!(fwd.profile.properties.is_empty());
    }

    #[test]
    fn bungee_rejects_plain_or_malformed_hosts() {
        assert_eq!(bungee_parse("mc.example.com", "Notch", &[]), Err(ForwardError::Missing));
        assert_eq!(bungee_parse("mc.example.com\x00127.0.0.1", "Notch", &[]), Err(ForwardError::Missing));
        assert_eq!(bungee_parse("h\x00127.0.0.1\x00nope", "Notch", &[]), Err(ForwardError::BadUuid));
        assert!(matches!(
            bungee_parse("h\x00x.y\x00069a79f444e94726a5befca90e38aaf5", "N", &[]),
            Err(ForwardError::BadAddress(_))
        ));
        assert!(matches!(
            bungee_parse("h\x00127.0.0.1\x00069a79f444e94726a5befca90e38aaf5\x00[{]", "N", &[]),
            Err(ForwardError::Malformed(_))
        ));
    }

    #[test]
    fn bungeeguard_tokens() {
        let tokens = vec!["first-token".to_string(), "second-token".to_string()];
        let host = |props: &str| format!("h\x00127.0.0.1\x00069a79f444e94726a5befca90e38aaf5\x00[{props}]");
        let token = |t: &str| format!(r#"{{"name":"bungeeguard-token","value":"{t}"}}"#);

        let fwd = bungee_parse(&host(&format!("{TEXTURES},{}", token("second-token"))), "N", &tokens).unwrap();
        // The token is consumed, not forwarded as a profile property.
        assert_eq!(fwd.profile.properties.len(), 1);
        assert_eq!(fwd.profile.properties[0].name, "textures");

        assert_eq!(bungee_parse(&host(TEXTURES), "N", &tokens), Err(ForwardError::BadToken));
        assert_eq!(bungee_parse(&host(&token("wrong")), "N", &tokens), Err(ForwardError::BadToken));
        let twice = format!("{},{}", token("first-token"), token("first-token"));
        assert_eq!(bungee_parse(&host(&twice), "N", &tokens), Err(ForwardError::BadToken));
        // Without configured tokens a presented token is dropped and ignored.
        assert!(bungee_parse(&host(&token("x")), "N", &[]).unwrap().profile.properties.is_empty());
    }

    #[test]
    fn accepts_hosts_up_to_the_forwarding_limit() {
        let host = format!("h\x00127.0.0.1\x00069a79f444e94726a5befca90e38aaf5\x00[{TEXTURES}]");
        // Whitespace after the JSON array is still valid JSON.
        let long = format!("{host}{}", " ".repeat(MAX_FORWARDED_HOST - host.len()));
        let mut b = BytesMut::new();
        b.put_string(&long);
        assert!(Reader::new(&b).string(MAX_HOST).is_err());
        let read = Reader::new(&b).string(MAX_FORWARDED_HOST).unwrap();
        assert!(bungee_parse(read, "N", &[]).is_ok());
    }
}
