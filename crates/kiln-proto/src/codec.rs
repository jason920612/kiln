use bytes::{BufMut, BytesMut};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("unexpected end of packet")]
    Eof,
    #[error("VarInt too long")]
    VarIntTooLong,
    #[error("string longer than {max} characters")]
    StringTooLong { max: usize },
    #[error("invalid UTF-8 in string")]
    InvalidUtf8,
    #[error("negative length {0}")]
    NegativeLength(i32),
    #[error("{0} trailing bytes after packet")]
    TrailingBytes(usize),
    #[error("invalid value: {0}")]
    Invalid(&'static str),
}

/// Cursor over a packet body.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len()
    }

    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.buf)
    }

    /// Errors if any bytes are left; call after decoding a whole packet.
    pub fn finish(&self) -> Result<(), DecodeError> {
        match self.buf.len() {
            0 => Ok(()),
            n => Err(DecodeError::TrailingBytes(n)),
        }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.buf.len() < n {
            return Err(DecodeError::Eof);
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }
    pub fn i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.u8()? as i8)
    }
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Invalid("boolean")),
        }
    }
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_be_bytes(self.array()?))
    }
    pub fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_be_bytes(self.array()?))
    }
    pub fn uuid(&mut self) -> Result<Uuid, DecodeError> {
        Ok(Uuid::from_bytes(self.array()?))
    }

    pub fn varint(&mut self) -> Result<i32, DecodeError> {
        let mut value = 0u32;
        for i in 0..5 {
            let b = self.u8()?;
            value |= ((b & 0x7f) as u32) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        Err(DecodeError::VarIntTooLong)
    }

    pub fn varlong(&mut self) -> Result<i64, DecodeError> {
        let mut value = 0u64;
        for i in 0..10 {
            let b = self.u8()?;
            value |= ((b & 0x7f) as u64) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value as i64);
            }
        }
        Err(DecodeError::VarIntTooLong)
    }

    /// Length prefix for arrays/strings; rejects negatives.
    pub fn len(&mut self) -> Result<usize, DecodeError> {
        let n = self.varint()?;
        usize::try_from(n).map_err(|_| DecodeError::NegativeLength(n))
    }

    /// A String (n): `max` is in UTF-16 code units, as in vanilla.
    pub fn string(&mut self, max: usize) -> Result<&'a str, DecodeError> {
        let len = self.len()?;
        if len > max * 3 {
            return Err(DecodeError::StringTooLong { max });
        }
        let s = std::str::from_utf8(self.bytes(len)?).map_err(|_| DecodeError::InvalidUtf8)?;
        if s.encode_utf16().count() > max {
            return Err(DecodeError::StringTooLong { max });
        }
        Ok(s)
    }
}

/// Writers for the wire types, on top of `BytesMut`.
pub trait WriteExt {
    fn put_varint(&mut self, v: i32);
    fn put_varlong(&mut self, v: i64);
    fn put_bool(&mut self, v: bool);
    fn put_string(&mut self, s: &str);
    fn put_uuid(&mut self, u: Uuid);
    /// Block position packed as x:26, z:26, y:12.
    fn put_position(&mut self, x: i32, y: i32, z: i32);
    /// `ByteBufCodecs.BIT_SET` as of 26.3: VarInt byte count, then `java.util.BitSet.toByteArray()`
    /// (little-endian bit order, trailing zero bytes dropped). Not the long-array form.
    fn put_bitset(&mut self, words: &[u64]);
}

impl WriteExt for BytesMut {
    fn put_varint(&mut self, v: i32) {
        let mut v = v as u32;
        loop {
            if v & !0x7f == 0 {
                self.put_u8(v as u8);
                return;
            }
            self.put_u8((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }

    fn put_varlong(&mut self, v: i64) {
        let mut v = v as u64;
        loop {
            if v & !0x7f == 0 {
                self.put_u8(v as u8);
                return;
            }
            self.put_u8((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }

    fn put_bool(&mut self, v: bool) {
        self.put_u8(v as u8);
    }

    fn put_string(&mut self, s: &str) {
        self.put_varint(s.len() as i32);
        self.put_slice(s.as_bytes());
    }

    fn put_uuid(&mut self, u: Uuid) {
        self.put_slice(u.as_bytes());
    }

    fn put_position(&mut self, x: i32, y: i32, z: i32) {
        let packed = ((x as i64 & 0x3FF_FFFF) << 38) | ((z as i64 & 0x3FF_FFFF) << 12) | (y as i64 & 0xFFF);
        self.put_i64(packed);
    }

    fn put_bitset(&mut self, words: &[u64]) {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let len = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        self.put_varint(len as i32);
        self.put_slice(&bytes[..len]);
    }
}

pub fn varint_len(v: i32) -> usize {
    match v as u32 {
        0..=0x7f => 1,
        0x80..=0x3fff => 2,
        0x4000..=0x1f_ffff => 3,
        0x20_0000..=0xfff_ffff => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_known_vectors() {
        // Vectors from the protocol documentation.
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (25565, &[0xdd, 0xc7, 0x01]),
            (2097151, &[0xff, 0xff, 0x7f]),
            (i32::MAX, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (i32::MIN, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];
        for &(v, bytes) in cases {
            let mut b = BytesMut::new();
            b.put_varint(v);
            assert_eq!(&b[..], bytes, "encode {v}");
            assert_eq!(varint_len(v), bytes.len(), "len {v}");
            let mut r = Reader::new(bytes);
            assert_eq!(r.varint().unwrap(), v, "decode {v}");
            r.finish().unwrap();
        }
    }

    #[test]
    fn varint_rejects_six_bytes() {
        let mut r = Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]);
        assert_eq!(r.varint(), Err(DecodeError::VarIntTooLong));
    }

    #[test]
    fn position_packing() {
        // Example from the protocol documentation: (18357644, 831, -20882616).
        let mut b = BytesMut::new();
        b.put_position(18357644, 831, -20882616);
        assert_eq!(&b[..], &0b01000110000001110110001100_10110000010101101101001000_001100111111u64.to_be_bytes());
    }

    #[test]
    fn bitset_matches_java_to_byte_array() {
        // java.util.BitSet: {} -> [], {0} -> [01], {1..=25} -> [FE FF FF 03], {64} -> 8 zeros then 01
        let enc = |words: &[u64]| {
            let mut b = BytesMut::new();
            b.put_bitset(words);
            b.to_vec()
        };
        assert_eq!(enc(&[0]), vec![0]);
        assert_eq!(enc(&[1]), vec![1, 0x01]);
        assert_eq!(enc(&[0x3FF_FFFE]), vec![4, 0xFE, 0xFF, 0xFF, 0x03]);
        assert_eq!(enc(&[0, 1]), vec![9, 0, 0, 0, 0, 0, 0, 0, 0, 0x01]);
    }

    #[test]
    fn string_limit_counts_utf16_units() {
        let mut b = BytesMut::new();
        b.put_string("𝄞𝄞"); // 2 scalar values, 4 UTF-16 units
        assert!(Reader::new(&b).string(4).is_ok());
        assert_eq!(Reader::new(&b).string(3), Err(DecodeError::StringTooLong { max: 3 }));
    }
}
