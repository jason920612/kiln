//! Resource identifiers (`namespace:path`).

use crate::value::{DataError, DataResult, Value};
use kiln_proto::{DecodeError, Reader, WriteExt};
use std::fmt;

/// A validated identifier in canonical `namespace:path` form (a missing namespace parses as
/// `minecraft`, as vanilla's `Identifier.parse` does).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier(String);

impl Identifier {
    pub fn parse(s: &str) -> Option<Identifier> {
        let (ns, path) = match s.find(':') {
            Some(0) => ("minecraft", &s[1..]),
            Some(i) => (&s[..i], &s[i + 1..]),
            None => ("minecraft", s),
        };
        let ns_ok = ns.bytes().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'));
        let path_ok = path.bytes().all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'/'));
        (ns_ok && path_ok).then(|| Identifier(format!("{ns}:{path}")))
    }

    /// An identifier known to be valid (e.g. from generated tables).
    pub fn new_unchecked(s: impl Into<String>) -> Identifier {
        Identifier(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn namespace(&self) -> &str {
        &self.0[..self.0.find(':').unwrap_or(0)]
    }

    pub fn path(&self) -> &str {
        &self.0[self.0.find(':').map_or(0, |i| i + 1)..]
    }

    /// `Identifier.STREAM_CODEC`: a string of at most 32767 characters.
    pub fn read(r: &mut Reader<'_>) -> Result<Identifier, DecodeError> {
        Identifier::parse(r.string(32767)?).ok_or(DecodeError::Invalid("identifier"))
    }

    pub fn write(&self, out: &mut bytes::BytesMut) {
        out.put_string(&self.0);
    }

    pub fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    pub fn from_value(v: &Value) -> DataResult<Identifier> {
        let s = v.as_str()?;
        Identifier::parse(s).ok_or_else(|| DataError(format!("invalid identifier {s:?}")))
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_vanilla() {
        assert_eq!(Identifier::parse("stick").unwrap().as_str(), "minecraft:stick");
        assert_eq!(Identifier::parse(":stick").unwrap().as_str(), "minecraft:stick");
        assert_eq!(Identifier::parse("kiln:a/b.c").unwrap().path(), "a/b.c");
        assert!(Identifier::parse("Stick").is_none());
        assert!(Identifier::parse("a:b:c").is_none());
        assert!(Identifier::parse("a/b:c").is_none());
    }
}
