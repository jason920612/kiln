//! Minimal NBT writer for network use (nameless root tag, as sent since 1.20.2).

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
    fn modified_utf8_nul_and_supplementary() {
        let mut b = BytesMut::new();
        put_mutf8(&mut b, "\0𝄞");
        // NUL -> C0 80; U+1D11E -> surrogates D834 DD1E, 3 bytes each.
        assert_eq!(&b[..], &[0, 8, 0xc0, 0x80, 0xed, 0xa0, 0xb4, 0xed, 0xb4, 0x9e]);
    }
}
