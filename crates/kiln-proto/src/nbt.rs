//! NBT: a writer for network use (nameless root tag, as sent since 1.20.2) and a bounded
//! reader for both the network form and the named-root file form.

use bytes::{BufMut, BytesMut};

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(Vec<(String, Tag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(_) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    /// Writes this tag as a network root: type id, then payload (no name).
    pub fn write_network(&self, out: &mut BytesMut) {
        out.put_u8(self.id());
        self.write_payload(out);
    }

    /// Writes this tag as a file-format root: type id, name, payload.
    pub fn write_named(&self, name: &str, out: &mut BytesMut) {
        out.put_u8(self.id());
        put_mutf8(out, name);
        self.write_payload(out);
    }

    /// A list that may mix element types: when they differ, non-compound elements (and
    /// compounds that would be ambiguous) are wrapped as `{"": value}`, as vanilla does.
    pub fn heterogeneous_list(items: Vec<Tag>) -> Tag {
        let mixed = items.windows(2).any(|w| w[0].id() != w[1].id());
        if !mixed {
            return Tag::List(items);
        }
        Tag::List(
            items
                .into_iter()
                .map(|t| match t {
                    Tag::Compound(ref f) if !(f.len() == 1 && f[0].0.is_empty()) => t,
                    t => Tag::Compound(vec![(String::new(), t)]),
                })
                .collect(),
        )
    }

    fn write_payload(&self, out: &mut BytesMut) {
        match self {
            Tag::Byte(v) => out.put_i8(*v),
            Tag::Short(v) => out.put_i16(*v),
            Tag::Int(v) => out.put_i32(*v),
            Tag::Long(v) => out.put_i64(*v),
            Tag::Float(v) => out.put_f32(*v),
            Tag::Double(v) => out.put_f64(*v),
            Tag::ByteArray(v) => {
                out.put_i32(v.len() as i32);
                for b in v {
                    out.put_i8(*b);
                }
            }
            Tag::String(s) => put_mutf8(out, s),
            Tag::List(items) => {
                let id = items.first().map_or(0, Tag::id);
                out.put_u8(id);
                out.put_i32(items.len() as i32);
                for t in items {
                    debug_assert_eq!(t.id(), id, "NBT list elements must share a type");
                    t.write_payload(out);
                }
            }
            Tag::Compound(fields) => {
                for (name, t) in fields {
                    out.put_u8(t.id());
                    put_mutf8(out, name);
                    t.write_payload(out);
                }
                out.put_u8(0);
            }
            Tag::IntArray(v) => {
                out.put_i32(v.len() as i32);
                for x in v {
                    out.put_i32(*x);
                }
            }
            Tag::LongArray(v) => {
                out.put_i32(v.len() as i32);
                for x in v {
                    out.put_i64(*x);
                }
            }
        }
    }
}

/// Maximum nesting depth accepted by the reader (as in vanilla).
pub const MAX_DEPTH: usize = 512;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NbtError {
    #[error("unexpected end of NBT data")]
    Eof,
    #[error("unknown tag type {0}")]
    BadType(u8),
    #[error("NBT nested deeper than {MAX_DEPTH}")]
    TooDeep,
    #[error("negative or oversized length")]
    BadLength,
    #[error("invalid modified UTF-8")]
    BadString,
}

struct NbtReader<'a> {
    buf: &'a [u8],
}

impl<'a> NbtReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], NbtError> {
        if self.buf.len() < n {
            return Err(NbtError::Eof);
        }
        let (a, b) = self.buf.split_at(n);
        self.buf = b;
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32, NbtError> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// A length prefix for `elem`-byte elements; rejects lengths the remaining input cannot hold.
    fn len(&mut self, elem: usize) -> Result<usize, NbtError> {
        let n = usize::try_from(self.i32()?).map_err(|_| NbtError::BadLength)?;
        if n.checked_mul(elem).is_none_or(|b| b > self.buf.len()) {
            return Err(NbtError::BadLength);
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, NbtError> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        decode_mutf8(self.take(n)?)
    }

    fn payload(&mut self, ty: u8, depth: usize) -> Result<Tag, NbtError> {
        if depth > MAX_DEPTH {
            return Err(NbtError::TooDeep);
        }
        Ok(match ty {
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(i16::from_be_bytes(self.take(2)?.try_into().unwrap())),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(i64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            5 => Tag::Float(f32::from_be_bytes(self.take(4)?.try_into().unwrap())),
            6 => Tag::Double(f64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            7 => {
                let n = self.len(1)?;
                Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let elem_ty = self.u8()?;
                // Every element takes at least one byte, except in lists of End tags,
                // which must be empty.
                let n = self.len(if elem_ty == 0 { 0 } else { 1 })?;
                if elem_ty == 0 {
                    return if n == 0 { Ok(Tag::List(Vec::new())) } else { Err(NbtError::BadLength) };
                }
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.payload(elem_ty, depth + 1)?);
                }
                Tag::List(items)
            }
            10 => {
                let mut fields = Vec::new();
                loop {
                    let t = self.u8()?;
                    if t == 0 {
                        break;
                    }
                    let name = self.string()?;
                    fields.push((name, self.payload(t, depth + 1)?));
                }
                Tag::Compound(fields)
            }
            11 => {
                let n = self.len(4)?;
                let raw = self.take(n * 4)?;
                Tag::IntArray(raw.chunks_exact(4).map(|c| i32::from_be_bytes(c.try_into().unwrap())).collect())
            }
            12 => {
                let n = self.len(8)?;
                let raw = self.take(n * 8)?;
                Tag::LongArray(raw.chunks_exact(8).map(|c| i64::from_be_bytes(c.try_into().unwrap())).collect())
            }
            t => return Err(NbtError::BadType(t)),
        })
    }
}

/// Reads a file-format root: type, name, payload. Returns the name and the tag.
pub fn read_named(data: &[u8]) -> Result<(String, Tag), NbtError> {
    let mut r = NbtReader { buf: data };
    let ty = r.u8()?;
    let name = r.string()?;
    Ok((name, r.payload(ty, 0)?))
}

/// Reads a network root (no name) from the front of `data`; returns the tag and bytes consumed.
pub fn read_network(data: &[u8]) -> Result<(Tag, usize), NbtError> {
    let mut r = NbtReader { buf: data };
    let ty = r.u8()?;
    let tag = r.payload(ty, 0)?;
    Ok((tag, data.len() - r.buf.len()))
}

fn decode_mutf8(b: &[u8]) -> Result<String, NbtError> {
    // Fast path: plain UTF-8 without the encodings modified UTF-8 uses for NUL and surrogates.
    if !b.iter().any(|&c| c == 0xc0 || c == 0xed) {
        if let Ok(s) = std::str::from_utf8(b) {
            return Ok(s.to_owned());
        }
    }
    let mut units = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as u16;
        let (unit, len) = if c < 0x80 {
            (c, 1)
        } else if c & 0xe0 == 0xc0 && i + 1 < b.len() {
            (((c & 0x1f) << 6) | (b[i + 1] as u16 & 0x3f), 2)
        } else if c & 0xf0 == 0xe0 && i + 2 < b.len() {
            (((c & 0x0f) << 12) | ((b[i + 1] as u16 & 0x3f) << 6) | (b[i + 2] as u16 & 0x3f), 3)
        } else {
            return Err(NbtError::BadString);
        };
        units.push(unit);
        i += len;
    }
    String::from_utf16(&units).map_err(|_| NbtError::BadString)
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v as i64),
            Tag::Short(v) => Some(v as i64),
            Tag::Int(v) => Some(v as i64),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }

    /// Any numeric tag as a double (vanilla reads numbers leniently, e.g. `getFloatOr`).
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Tag::Float(v) => Some(v as f64),
            Tag::Double(v) => Some(v),
            _ => self.as_i64().map(|v| v as f64),
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_long_array(&self) -> Option<&[i64]> {
        match self {
            Tag::LongArray(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_byte_array(&self) -> Option<&[i8]> {
        match self {
            Tag::ByteArray(v) => Some(v),
            _ => None,
        }
    }

    /// Elements of heterogeneous lists are wrapped as `{"": value}` (since 1.21.5).
    pub fn unwrap_list_element(&self) -> &Tag {
        match self {
            Tag::Compound(f) if f.len() == 1 && f[0].0.is_empty() => &f[0].1,
            t => t,
        }
    }
}

/// Java "modified UTF-8" with a u16 byte-length prefix.
fn put_mutf8(out: &mut BytesMut, s: &str) {
    let mut buf = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            0x0001..=0x007f => buf.push(unit as u8),
            0x0000 | 0x0080..=0x07ff => {
                buf.push(0xc0 | (unit >> 6) as u8);
                buf.push(0x80 | (unit & 0x3f) as u8);
            }
            _ => {
                buf.push(0xe0 | (unit >> 12) as u8);
                buf.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                buf.push(0x80 | (unit & 0x3f) as u8);
            }
        }
    }
    let len = buf.len().min(u16::MAX as usize);
    out.put_u16(len as u16);
    out.put_slice(&buf[..len]);
}

/// A plain-text chat component: a bare string tag.
pub fn text(s: &str) -> Tag {
    Tag::String(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_compound_layout() {
        let t = Tag::Compound(vec![("text".into(), Tag::String("hi".into())), ("bold".into(), Tag::Byte(1))]);
        let mut b = BytesMut::new();
        t.write_network(&mut b);
        let expected: &[u8] = &[
            10, // compound, no root name
            8, 0, 4, b't', b'e', b'x', b't', 0, 2, b'h', b'i', //
            1, 0, 4, b'b', b'o', b'l', b'd', 1, //
            0,
        ];
        assert_eq!(&b[..], expected);
    }

    #[test]
    fn read_back_what_we_write() {
        let t = Tag::Compound(vec![
            ("a".into(), Tag::List(vec![Tag::Int(1), Tag::Int(2)])),
            ("s".into(), Tag::String("\0𝄞 hé".into())),
            ("l".into(), Tag::LongArray(vec![1, -1])),
            ("n".into(), Tag::Compound(vec![("x".into(), Tag::Byte(-3))])),
        ]);
        let mut b = BytesMut::new();
        t.write_network(&mut b);
        let (back, used) = read_network(&b).unwrap();
        assert_eq!((back, used), (t, b.len()));
    }

    #[test]
    fn rejects_hostile_lengths_and_depth() {
        // A list claiming i32::MAX ints with no data behind it.
        let mut b = vec![9u8, 3];
        b.extend_from_slice(&i32::MAX.to_be_bytes());
        assert_eq!(read_network(&b), Err(NbtError::BadLength));
        // Lists nested deeper than the limit.
        let mut b = vec![9u8];
        for _ in 0..600 {
            b.extend_from_slice(&[9, 0, 0, 0, 1]);
        }
        assert!(matches!(read_network(&b), Err(NbtError::TooDeep)));
    }

    #[test]
    fn modified_utf8_nul_and_supplementary() {
        let mut b = BytesMut::new();
        put_mutf8(&mut b, "\0𝄞");
        // NUL -> C0 80; U+1D11E -> surrogates D834 DD1E, 3 bytes each.
        assert_eq!(&b[..], &[0, 8, 0xc0, 0x80, 0xed, 0xa0, 0xb4, 0xed, 0xb4, 0x9e]);
    }
}
