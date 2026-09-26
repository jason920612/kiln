//! A small JSON reader that keeps number literals as text.
//!
//! Vanilla decodes datapack numbers from Gson's lazily parsed literals: `Codec.FLOAT` rounds
//! the decimal text straight to `float` and `Codec.DOUBLE` to `double`. Going through `f64`
//! first could double-round a float, so numbers stay strings until their codec is known.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

impl Json {
    pub fn parse(text: &str) -> Result<Json, ParseError> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }

    /// `Codec.FLOAT`: the literal rounded directly to `f32`.
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Json::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    /// `Codec.DOUBLE`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    /// `Codec.INT`; integral literals only.
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Json::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, message: &'static str) -> ParseError {
        ParseError { offset: self.i, message }
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, lit: &[u8]) -> Result<(), ParseError> {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn value(&mut self) -> Result<Json, ParseError> {
        self.ws();
        match self.peek().ok_or_else(|| self.err("unexpected end"))? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Json::String(self.string()?)),
            b't' => self.expect(b"true").map(|_| Json::Bool(true)),
            b'f' => self.expect(b"false").map(|_| Json::Bool(false)),
            b'n' => self.expect(b"null").map(|_| Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn object(&mut self) -> Result<Json, ParseError> {
        self.i += 1;
        let mut fields = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected key"));
            }
            let key = self.string()?;
            self.ws();
            self.expect(b":")?;
            fields.push((key, self.value()?));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(self.err("expected , or }")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, ParseError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.err("expected , or ]")),
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && !matches!(self.s[self.i], b'"' | b'\\') {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("invalid utf-8"))?);
            match self.peek() {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("unexpected end"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("short escape"))?;
                            let code = std::str::from_utf8(hex)
                                .ok()
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .ok_or_else(|| self.err("bad escape"))?;
                            self.i += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                _ => return Err(self.err("unterminated string")),
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        Ok(Json::Number(String::from_utf8_lossy(&self.s[start..self.i]).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_number_text() {
        let v = Json::parse(r#"{"a": [0.6666667, -4064, 1.0E-5], "b": "x\"y", "c": null}"#).unwrap();
        let a = v.get("a").unwrap().as_array().unwrap();
        assert_eq!(a[0], Json::Number("0.6666667".into()));
        assert_eq!(a[0].as_f32(), Some(0.6666667f32));
        assert_eq!(a[1].as_i32(), Some(-4064));
        assert_eq!(a[2].as_f64(), Some(1.0e-5));
        assert_eq!(v.get("b").unwrap().as_str(), Some("x\"y"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(Json::parse("{").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse("1 2").is_err());
    }
}
