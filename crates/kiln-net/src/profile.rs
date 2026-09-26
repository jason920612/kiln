//! Player identity as established at login: offline, authenticated, or forwarded by a proxy.

use kiln_link::Property;
use kiln_proto::packets::ProfileProperty;
use kiln_proto::{DecodeError, Reader};
use serde::Deserialize;
use uuid::Uuid;

/// `ByteBufCodecs.GAME_PROFILE_PROPERTIES` limits; the client rejects a Login Finished beyond them.
pub const MAX_PROPERTIES: usize = 16;
const MAX_PROPERTY_NAME: usize = 64;
const MAX_PROPERTY_VALUE: usize = 32767;
const MAX_PROPERTY_SIGNATURE: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfile {
    pub uuid: Uuid,
    pub name: String,
    pub properties: Vec<Property>,
}

impl GameProfile {
    /// Borrowed properties for `packets::login_finished`.
    pub fn wire_properties(&self) -> Vec<ProfileProperty<'_>> {
        self.properties
            .iter()
            .map(|p| ProfileProperty { name: &p.name, value: &p.value, signature: p.signature.as_deref() })
            .collect()
    }
}

/// Vanilla's player name rule: 1 to 16 characters from `[A-Za-z0-9_]`.
/// Vanilla's rule (StringUtil.isValidPlayerName): 1-16 characters, each in '!'..='~'.
/// Proxies such as Geyser/Floodgate rely on names outside [A-Za-z0-9_].
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.bytes().all(|b| (33..=126).contains(&b))
}

/// UUID the vanilla server assigns in offline mode: v3 of "OfflinePlayer:<name>" without a namespace.
pub fn offline_uuid(name: &str) -> Uuid {
    use md5::{Digest, Md5};
    let mut h: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Uuid::from_bytes(h)
}

/// Rejects properties the client could not decode when they are echoed back to it.
pub fn check_properties(properties: &[Property]) -> Result<(), &'static str> {
    let utf16 = |s: &str| s.encode_utf16().count();
    if properties.len() > MAX_PROPERTIES {
        return Err("too many profile properties");
    }
    for p in properties {
        if utf16(&p.name) > MAX_PROPERTY_NAME
            || utf16(&p.value) > MAX_PROPERTY_VALUE
            || p.signature.as_deref().is_some_and(|s| utf16(s) > MAX_PROPERTY_SIGNATURE)
        {
            return Err("profile property too long");
        }
    }
    Ok(())
}

/// Properties in the binary layout shared by the protocol and Velocity forwarding.
pub fn read_properties(r: &mut Reader) -> Result<Vec<Property>, DecodeError> {
    let n = r.len()?;
    if n > MAX_PROPERTIES {
        return Err(DecodeError::Invalid("too many profile properties"));
    }
    (0..n)
        .map(|_| {
            let name = r.string(MAX_PROPERTY_NAME)?.to_owned();
            let value = r.string(MAX_PROPERTY_VALUE)?.to_owned();
            let signature = if r.bool()? { Some(r.string(MAX_PROPERTY_SIGNATURE)?.to_owned()) } else { None };
            Ok(Property { name, value, signature })
        })
        .collect()
}

/// A property as authlib serializes it to JSON (session server responses, BungeeCord forwarding).
#[derive(Deserialize)]
pub struct PropertyJson {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub signature: Option<String>,
}

impl From<PropertyJson> for Property {
    fn from(p: PropertyJson) -> Self {
        Property { name: p.name, value: p.value, signature: p.signature }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use kiln_proto::WriteExt;

    #[test]
    fn offline_uuid_matches_vanilla() {
        // Java: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(offline_uuid("Notch").to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }

    #[test]
    fn property_limits() {
        let prop = |name: &str, value: &str| Property { name: name.into(), value: value.into(), signature: None };
        assert!(check_properties(&[prop("textures", "e30=")]).is_ok());
        assert!(check_properties(&vec![prop("a", "b"); 17]).is_err());
        assert!(check_properties(&[prop(&"n".repeat(65), "b")]).is_err());
        let signed = Property { signature: Some("s".repeat(1025)), ..prop("a", "b") };
        assert!(check_properties(&[signed]).is_err());
    }

    #[test]
    fn reads_binary_properties() {
        let mut b = BytesMut::new();
        b.put_varint(2);
        b.put_string("textures");
        b.put_string("dmFsdWU=");
        b.put_bool(true);
        b.put_string("c2ln");
        b.put_string("other");
        b.put_string("x");
        b.put_bool(false);
        let props = read_properties(&mut Reader::new(&b)).unwrap();
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].signature.as_deref(), Some("c2ln"));
        assert_eq!(props[1], Property { name: "other".into(), value: "x".into(), signature: None });
    }
}
