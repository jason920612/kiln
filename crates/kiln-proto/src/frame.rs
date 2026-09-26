//! Packet framing: VarInt length prefix and optional zlib compression.

use crate::codec::{Reader, WriteExt, varint_len};
use bytes::{Buf, BufMut, BytesMut};

/// Largest frame the length prefix may announce (3-byte VarInt).
pub const MAX_FRAME_LEN: usize = 2_097_151;
/// Largest decompressed packet the vanilla server accepts.
pub const MAX_UNCOMPRESSED_LEN: usize = 8_388_608;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame length prefix longer than 3 bytes")]
    BadLength,
    #[error("frame of {0} bytes exceeds the limit")]
    TooLarge(usize),
    #[error("compressed packet declares {0} bytes, below the compression threshold")]
    BelowThreshold(usize),
    #[error("decompressed packet of {0} bytes exceeds the limit")]
    DecompressedTooLarge(usize),
    #[error("corrupt compressed packet")]
    BadCompression,
    #[error("malformed frame")]
    Malformed,
}

/// Per-connection framing state. Compression is off until `set_threshold`.
pub struct FrameCodec {
    threshold: Option<usize>,
    level: i32,
    compressor: Option<libdeflater::Compressor>,
    decompressor: libdeflater::Decompressor,
    scratch: Vec<u8>,
}

impl Default for FrameCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCodec {
    pub fn new() -> Self {
        Self {
            threshold: None,
            level: 4,
            compressor: None,
            decompressor: libdeflater::Decompressor::new(),
            scratch: Vec::new(),
        }
    }

    pub fn threshold(&self) -> Option<usize> {
        self.threshold
    }

    /// Enables compression for packets of at least `threshold` bytes (`None` disables it).
    pub fn set_threshold(&mut self, threshold: Option<usize>) {
        self.threshold = threshold;
        if threshold.is_some() && self.compressor.is_none() {
            let level = libdeflater::CompressionLvl::new(self.level).unwrap_or_default();
            self.compressor = Some(libdeflater::Compressor::new(level));
        }
    }

    /// Splits one packet (packet id + data, decompressed) off the front of `buf`.
    /// Returns `Ok(None)` if the frame is not complete yet.
    pub fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<BytesMut>, FrameError> {
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
        let mut frame = buf.split_to(len);

        let Some(threshold) = self.threshold else { return Ok(Some(frame)) };
        let mut r = Reader::new(&frame);
        let data_len = r.varint().map_err(|_| FrameError::Malformed)?;
        let prefix = frame.len() - r.remaining();
        if data_len == 0 {
            frame.advance(prefix);
            return Ok(Some(frame));
        }
        let data_len = usize::try_from(data_len).map_err(|_| FrameError::Malformed)?;
        if data_len < threshold {
            return Err(FrameError::BelowThreshold(data_len));
        }
        if data_len > MAX_UNCOMPRESSED_LEN {
            return Err(FrameError::DecompressedTooLarge(data_len));
        }
        // The declared size is bounded above, and must match the zlib stream exactly.
        let mut out = BytesMut::zeroed(data_len);
        let n = self
            .decompressor
            .zlib_decompress(&frame[prefix..], &mut out)
            .map_err(|_| FrameError::BadCompression)?;
        if n != data_len {
            return Err(FrameError::BadCompression);
        }
        Ok(Some(out))
    }

    /// Appends a framed packet (`packet` = packet id + data) to `out`.
    pub fn encode(&mut self, packet: &[u8], out: &mut BytesMut) -> Result<(), FrameError> {
        match (self.threshold, self.compressor.as_mut()) {
            (Some(threshold), Some(compressor)) if packet.len() >= threshold => {
                if packet.len() > MAX_UNCOMPRESSED_LEN {
                    return Err(FrameError::DecompressedTooLarge(packet.len()));
                }
                let bound = compressor.zlib_compress_bound(packet.len());
                self.scratch.resize(bound, 0);
                let n = compressor
                    .zlib_compress(packet, &mut self.scratch)
                    .map_err(|_| FrameError::BadCompression)?;
                let data_len = packet.len() as i32;
                let frame_len = varint_len(data_len) + n;
                if frame_len > MAX_FRAME_LEN {
                    return Err(FrameError::TooLarge(frame_len));
                }
                out.reserve(3 + frame_len);
                out.put_varint(frame_len as i32);
                out.put_varint(data_len);
                out.put_slice(&self.scratch[..n]);
            }
            (Some(_), _) => {
                let frame_len = 1 + packet.len();
                if frame_len > MAX_FRAME_LEN {
                    return Err(FrameError::TooLarge(frame_len));
                }
                out.reserve(3 + frame_len);
                out.put_varint(frame_len as i32);
                out.put_u8(0);
                out.put_slice(packet);
            }
            (None, _) => {
                if packet.len() > MAX_FRAME_LEN {
                    return Err(FrameError::TooLarge(packet.len()));
                }
                out.reserve(3 + packet.len());
                out.put_varint(packet.len() as i32);
                out.put_slice(packet);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(codec_threshold: Option<usize>, packet: &[u8]) {
        let mut tx = FrameCodec::new();
        let mut rx = FrameCodec::new();
        tx.set_threshold(codec_threshold);
        rx.set_threshold(codec_threshold);
        let mut wire = BytesMut::new();
        tx.encode(packet, &mut wire).unwrap();
        tx.encode(b"\x01second", &mut wire).unwrap();
        // Feed byte by byte to exercise partial frames.
        let mut buf = BytesMut::new();
        let mut got = Vec::new();
        for b in wire.iter() {
            buf.put_u8(*b);
            while let Some(p) = rx.decode(&mut buf).unwrap() {
                got.push(p.to_vec());
            }
        }
        assert_eq!(got, vec![packet.to_vec(), b"\x01second".to_vec()]);
    }

    #[test]
    fn uncompressed_roundtrip() {
        roundtrip(None, &[0x00, 1, 2, 3]);
    }

    #[test]
    fn compressed_roundtrip_above_and_below_threshold() {
        roundtrip(Some(256), &[7u8; 10]);
        roundtrip(Some(256), &vec![0x42u8; 100_000]);
    }

    #[test]
    fn rejects_compressed_frame_declaring_size_below_threshold() {
        let mut rx = FrameCodec::new();
        rx.set_threshold(Some(256));
        // frame: len=3, data_len=10 (compressed but below threshold), junk
        let mut buf = BytesMut::from(&[3u8, 10, 0x78, 0x9c][..]);
        buf.truncate(4);
        assert!(matches!(rx.decode(&mut buf), Err(FrameError::BelowThreshold(10))));
    }

    #[test]
    fn rejects_four_byte_length_prefix() {
        let mut rx = FrameCodec::new();
        let mut buf = BytesMut::from(&[0x80u8, 0x80, 0x80, 0x01][..]);
        assert!(matches!(rx.decode(&mut buf), Err(FrameError::BadLength)));
    }

    #[test]
    fn rejects_oversized_declared_decompression() {
        let mut rx = FrameCodec::new();
        rx.set_threshold(Some(256));
        let mut buf = BytesMut::new();
        let mut inner = BytesMut::new();
        inner.put_varint(MAX_UNCOMPRESSED_LEN as i32 + 1);
        inner.put_slice(&[0x78, 0x9c, 0, 0]);
        buf.put_varint(inner.len() as i32);
        buf.put_slice(&inner);
        assert!(matches!(rx.decode(&mut buf), Err(FrameError::DecompressedTooLarge(_))));
    }
}
