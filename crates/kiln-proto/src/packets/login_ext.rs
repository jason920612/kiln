//! Login packets for encryption and proxy forwarding. Layouts follow the 26.3 bytecode
//! (`ClientboundHelloPacket`, `ServerboundKeyPacket`, `Clientbound`/`ServerboundCustomQuery*`).

use super::packet;
use crate::{DecodeError, Reader, WriteExt};
use bytes::{BufMut, Bytes};
use kiln_data::packets as ids;
use uuid::Uuid;

/// Largest custom query payload vanilla accepts in either direction.
pub const MAX_QUERY_PAYLOAD: usize = 1_048_576;

/// Encryption Request (`minecraft:hello`): the server id (empty since 1.7), the RSA public key
/// as X.509 SubjectPublicKeyInfo DER, and the challenge the client must echo encrypted.
pub fn encryption_request(server_id: &str, public_key: &[u8], challenge: &[u8], authenticate: bool) -> Bytes {
    let mut b = packet(ids::login::clientbound::HELLO);
    b.put_string(server_id);
    b.put_varint(public_key.len() as i32);
    b.put_slice(public_key);
    b.put_varint(challenge.len() as i32);
    b.put_slice(challenge);
    b.put_bool(authenticate);
    b.freeze()
}

/// Login Plugin Request (`minecraft:custom_query`); the payload is the unprefixed rest of the packet.
pub fn custom_query(transaction_id: i32, channel: &str, payload: &[u8]) -> Bytes {
    debug_assert!(payload.len() <= MAX_QUERY_PAYLOAD);
    let mut b = packet(ids::login::clientbound::CUSTOM_QUERY);
    b.put_varint(transaction_id);
    b.put_string(channel);
    b.put_slice(payload);
    b.freeze()
}

/// Login Disconnect with a translatable reason, so the client shows its own localized text.
pub fn login_disconnect_translated(key: &str) -> Bytes {
    let mut b = packet(ids::login::clientbound::LOGIN_DISCONNECT);
    b.put_string(&serde_json::json!({ "translate": key }).to_string());
    b.freeze()
}

/// Login Start (`minecraft:hello`, serverbound).
pub struct LoginStart<'a> {
    pub name: &'a str,
    pub uuid: Uuid,
}

pub fn read_login_start<'a>(r: &mut Reader<'a>) -> Result<LoginStart<'a>, DecodeError> {
    let name = r.string(16)?;
    let uuid = r.uuid()?;
    r.finish()?;
    Ok(LoginStart { name, uuid })
}

/// Encryption Response (`minecraft:key`): both fields are RSA ciphertexts.
pub struct EncryptionResponse<'a> {
    pub shared_secret: &'a [u8],
    pub challenge: &'a [u8],
}

pub fn read_encryption_response<'a>(r: &mut Reader<'a>) -> Result<EncryptionResponse<'a>, DecodeError> {
    let shared_secret = byte_array(r)?;
    let challenge = byte_array(r)?;
    r.finish()?;
    Ok(EncryptionResponse { shared_secret, challenge })
}

/// Login Plugin Response (`minecraft:custom_query_answer`): transaction id and the payload,
/// `None` when the client did not understand the channel.
pub fn read_custom_query_answer<'a>(r: &mut Reader<'a>) -> Result<(i32, Option<&'a [u8]>), DecodeError> {
    let id = r.varint()?;
    let payload = if r.bool()? { Some(r.rest()) } else { None };
    if payload.is_some_and(|p| p.len() > MAX_QUERY_PAYLOAD) {
        return Err(DecodeError::Invalid("custom query payload too large"));
    }
    r.finish()?;
    Ok((id, payload))
}

/// `FriendlyByteBuf.readByteArray()`: VarInt length, then the bytes.
fn byte_array<'a>(r: &mut Reader<'a>) -> Result<&'a [u8], DecodeError> {
    let len = r.len()?;
    r.bytes(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    fn body(pkt: &[u8], id: i32) -> Reader<'_> {
        let mut r = Reader::new(pkt);
        assert_eq!(r.varint().unwrap(), id);
        r
    }

    #[test]
    fn encryption_request_layout() {
        let pkt = encryption_request("", &[1, 2, 3], &[9, 8, 7, 6], true);
        assert_eq!(&pkt[..], &[ids::login::clientbound::HELLO as u8, 0, 3, 1, 2, 3, 4, 9, 8, 7, 6, 1]);
    }

    #[test]
    fn custom_query_layout() {
        let pkt = custom_query(300, "velocity:player_info", &[4]);
        let mut r = body(&pkt, ids::login::clientbound::CUSTOM_QUERY);
        assert_eq!(r.varint().unwrap(), 300);
        assert_eq!(r.string(32767).unwrap(), "velocity:player_info");
        assert_eq!(r.rest(), &[4]);
    }

    #[test]
    fn reads_encryption_response() {
        let mut b = BytesMut::new();
        b.put_varint(3);
        b.put_slice(&[1, 2, 3]);
        b.put_varint(2);
        b.put_slice(&[4, 5]);
        let resp = read_encryption_response(&mut Reader::new(&b)).unwrap();
        assert_eq!((resp.shared_secret, resp.challenge), (&[1u8, 2, 3][..], &[4u8, 5][..]));
        b.put_u8(0);
        assert!(read_encryption_response(&mut Reader::new(&b)).is_err());
    }

    #[test]
    fn reads_custom_query_answer() {
        // transaction 7, payload present
        let answer = read_custom_query_answer(&mut Reader::new(&[7, 1, 0xAA, 0xBB])).unwrap();
        assert_eq!(answer, (7, Some(&[0xAA, 0xBB][..])));
        // transaction 7, no payload (vanilla client answering an unknown channel)
        assert_eq!(read_custom_query_answer(&mut Reader::new(&[7, 0])).unwrap(), (7, None));
        assert!(read_custom_query_answer(&mut Reader::new(&[7, 0, 0])).is_err());
        assert!(read_custom_query_answer(&mut Reader::new(&[7, 2])).is_err());
    }

    #[test]
    fn reads_login_start() {
        let mut b = BytesMut::new();
        b.put_string("Notch");
        b.put_uuid(Uuid::from_u128(0x069a79f444e94726a5befca90e38aaf5));
        let start = read_login_start(&mut Reader::new(&b)).unwrap();
        assert_eq!(start.name, "Notch");
        assert_eq!(start.uuid.to_string(), "069a79f4-44e9-4726-a5be-fca90e38aaf5");
    }
}
