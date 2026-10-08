//! Client-side framing, tuned for many connections per process.
//!
//! Inbound frames are split off the read buffer without copying. A compressed frame is only
//! inflated in full when the bot acts on its packet; otherwise just enough of the zlib stream
//! is inflated to read the packet id. The inflater is per thread rather than per bot, and no
//! compressor is kept: bots practically never send a packet above the threshold.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use flate2::{Compression, Decompress, FlushDecompress, Status};
use kiln_proto::codec::varint_len;
use kiln_proto::frame::FrameError;
use kiln_proto::{MAX_FRAME_LEN, MAX_UNCOMPRESSED_LEN, Reader, WriteExt};
use std::cell::RefCell;
use std::io::Write;

/// Body bytes after the packet id that `decode_peek` shows its callback.
pub const PEEK: usize = 8;

thread_local! {
    static INFLATER: RefCell<Decompress> = RefCell::new(Decompress::new(true));
}

/// One inbound packet.
#[derive(Debug)]
pub struct Frame {
    pub id: i32,
    /// Packet data after the id; only present when the caller asked for it.
    pub body: Option<Bytes>,
}

/// Inbound framing state. Compression is off until `set_threshold`.
#[derive(Debug, Default)]
pub struct Inbound {
    threshold: Option<usize>,
}

impl Inbound {
    pub fn set_threshold(&mut self, threshold: Option<usize>) {
        self.threshold = threshold;
    }

    /// Splits the next packet off `buf`; `want(id)` decides whether its body is needed.
    /// Returns `Ok(None)` if the frame is not complete yet.
    pub fn decode(&mut self, buf: &mut BytesMut, want: impl Fn(i32) -> bool) -> Result<Option<Frame>, FrameError> {
        self.decode_peek(buf, |id, _| want(id))
    }

    /// Like [`Inbound::decode`], but `want(id, head)` also sees the first bytes of the body
    /// (up to [`PEEK`] of them), enough to read a chunk's coordinates before deciding to inflate it.
    pub fn decode_peek(
        &mut self,
        buf: &mut BytesMut,
        want: impl Fn(i32, &[u8]) -> bool,
    ) -> Result<Option<Frame>, FrameError> {
        let mut len = 0usize;
        let mut header = 0;
        loop {
            if header == 3 {
                return Err(FrameError::BadLength);
            }
            let Some(&b) = buf.get(header) else { return Ok(None) };
            len |= ((b & 0x7f) as usize) << (7 * header);
            header += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
        if len > MAX_FRAME_LEN {
            return Err(FrameError::TooLarge(len));
        }
        if buf.len() < header + len {
            buf.reserve(header + len - buf.len());
            return Ok(None);
        }
        buf.advance(header);

        let mut r = Reader::new(&buf[..len]);
        let data_len = match self.threshold {
            Some(_) => r.varint().map_err(|_| FrameError::Malformed)?,
            None => 0,
        };
        if data_len == 0 {
            let id = r.varint().map_err(|_| FrameError::Malformed)?;
            let offset = len - r.remaining();
            let head = &buf[offset..len.min(offset + PEEK)];
            let body = if want(id, head) {
                buf.advance(offset);
                Some(buf.split_to(len - offset).freeze())
            } else {
                buf.advance(len);
                None
            };
            return Ok(Some(Frame { id, body }));
        }

        let data_len = usize::try_from(data_len).map_err(|_| FrameError::Malformed)?;
        if self.threshold.is_some_and(|t| data_len < t) {
            return Err(FrameError::BelowThreshold(data_len));
        }
        if data_len > MAX_UNCOMPRESSED_LEN {
            return Err(FrameError::DecompressedTooLarge(data_len));
        }
        let compressed = r.rest();
        let result = INFLATER.with_borrow_mut(|inflater| {
            let (id, id_len, head, got) = peek_id(inflater, compressed, data_len)?;
            let body = match want(id, &head[id_len..got]) {
                true => Some(Bytes::from(inflate(inflater, compressed, data_len)?).slice(id_len..)),
                false => None,
            };
            Ok(Frame { id, body })
        });
        buf.advance(len);
        result.map(Some)
    }
}

/// Inflates only the first few bytes of a zlib stream: enough for the packet id and its
/// encoded length. Corruption past that point goes unnoticed for packets we skip.
type Peeked = (i32, usize, [u8; 5 + PEEK], usize);

fn peek_id(inflater: &mut Decompress, input: &[u8], data_len: usize) -> Result<Peeked, FrameError> {
    inflater.reset(true);
    let mut head = [0u8; 5 + PEEK];
    let n = data_len.min(head.len());
    inflater.decompress(input, &mut head[..n], FlushDecompress::None).map_err(|_| FrameError::BadCompression)?;
    let got = inflater.total_out() as usize;
    let mut r = Reader::new(&head[..got]);
    let id = r.varint().map_err(|_| FrameError::BadCompression)?;
    Ok((id, got - r.remaining(), head, got))
}

/// Inflates a whole zlib stream that must decompress to exactly `len` bytes.
fn inflate(inflater: &mut Decompress, input: &[u8], len: usize) -> Result<Vec<u8>, FrameError> {
    inflater.reset(true);
    let mut out = vec![0u8; len];
    let status =
        inflater.decompress(input, &mut out, FlushDecompress::Finish).map_err(|_| FrameError::BadCompression)?;
    if status != Status::StreamEnd || inflater.total_out() as usize != len {
        return Err(FrameError::BadCompression);
    }
    Ok(out)
}

/// Appends a framed packet (`packet` = packet id + data) to `out`.
pub fn encode(threshold: Option<usize>, packet: &[u8], out: &mut BytesMut) {
    debug_assert!(packet.len() <= MAX_FRAME_LEN - 5, "bots only send small packets");
    match threshold {
        Some(t) if packet.len() >= t => {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), Compression::fast());
            z.write_all(packet).expect("writing to a Vec cannot fail");
            let z = z.finish().expect("writing to a Vec cannot fail");
            let data_len = packet.len() as i32;
            out.put_varint((varint_len(data_len) + z.len()) as i32);
            out.put_varint(data_len);
            out.put_slice(&z);
        }
        Some(_) => {
            out.put_varint(packet.len() as i32 + 1);
            out.put_u8(0);
            out.put_slice(packet);
        }
        None => {
            out.put_varint(packet.len() as i32);
            out.put_slice(packet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_proto::FrameCodec;

    fn packet(id: i32, data: &[u8]) -> Vec<u8> {
        let mut b = BytesMut::new();
        b.put_varint(id);
        b.put_slice(data);
        b.to_vec()
    }

    /// Frames encoded by the server's codec, fed byte by byte.
    fn server_frames(threshold: Option<usize>, packets: &[Vec<u8>]) -> BytesMut {
        let mut tx = FrameCodec::new();
        tx.set_threshold(threshold);
        let mut wire = BytesMut::new();
        for p in packets {
            tx.encode(p, &mut wire).unwrap();
        }
        wire
    }

    fn decode_all(threshold: Option<usize>, wire: &[u8], want: impl Fn(i32) -> bool + Copy) -> Vec<Frame> {
        let mut rx = Inbound::default();
        rx.set_threshold(threshold);
        let mut buf = BytesMut::new();
        let mut out = Vec::new();
        for b in wire {
            buf.put_u8(*b);
            while let Some(f) = rx.decode(&mut buf, want).unwrap() {
                out.push(f);
            }
        }
        assert!(buf.is_empty());
        out
    }

    #[test]
    fn reads_ids_and_wanted_bodies_from_server_frames() {
        let big: Vec<u8> = (0..70_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let packets = vec![
            packet(0x2e, &big),        // compressed, not wanted: id only
            packet(0x21, b"small"),    // uncompressed, wanted
            packet(300, &big[..1000]), // two-byte id, compressed, wanted
            packet(0x05, b"skip me"),  // uncompressed, not wanted
        ];
        for threshold in [None, Some(256)] {
            let wire = server_frames(threshold, &packets);
            let frames = decode_all(threshold, &wire, |id| id == 0x21 || id == 300);
            let ids: Vec<i32> = frames.iter().map(|f| f.id).collect();
            assert_eq!(ids, [0x2e, 0x21, 300, 0x05]);
            assert!(frames[0].body.is_none());
            assert_eq!(frames[1].body.as_deref(), Some(&b"small"[..]));
            assert_eq!(frames[2].body.as_deref(), Some(&big[..1000]));
            assert!(frames[3].body.is_none());
        }
    }

    #[test]
    fn server_decodes_our_frames() {
        let big = vec![0x42u8; 5000];
        for threshold in [None, Some(256)] {
            let mut wire = BytesMut::new();
            encode(threshold, &packet(1, b"hi"), &mut wire);
            encode(threshold, &packet(2, &big), &mut wire);
            let mut rx = FrameCodec::new();
            rx.set_threshold(threshold);
            assert_eq!(rx.decode(&mut wire).unwrap().unwrap().to_vec(), packet(1, b"hi"));
            assert_eq!(rx.decode(&mut wire).unwrap().unwrap().to_vec(), packet(2, &big));
            assert!(wire.is_empty());
        }
    }

    #[test]
    fn rejects_wanted_frame_with_wrong_declared_length() {
        let mut body = BytesMut::new();
        body.put_varint(1000); // declared, but the stream holds 600 bytes
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), Compression::fast());
        z.write_all(&packet(7, &[1u8; 599])).unwrap();
        body.put_slice(&z.finish().unwrap());
        let mut wire = BytesMut::new();
        wire.put_varint(body.len() as i32);
        wire.put_slice(&body);

        let mut rx = Inbound::default();
        rx.set_threshold(Some(256));
        let mut peek = wire.clone();
        assert_eq!(rx.decode(&mut peek, |_| false).unwrap().unwrap().id, 7);
        assert!(matches!(rx.decode(&mut wire, |_| true), Err(FrameError::BadCompression)));
    }

    #[test]
    fn rejects_compressed_frame_below_threshold() {
        let mut rx = Inbound::default();
        rx.set_threshold(Some(256));
        let mut buf = BytesMut::from(&[3u8, 10, 0x78, 0x9c][..]);
        assert!(matches!(rx.decode(&mut buf, |_| true), Err(FrameError::BelowThreshold(10))));
    }
}
