//! A small JSON tree that keeps object key order and number spelling, as Gson's `JsonElement`
//! does (vanilla decodes datapack files through `JsonOps`, so key order is visible in, for
//! example, the entry order of a decoded component patch).

use kiln_item::Value;
use std::fmt::{self, Write};

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number as written (Gson's `LazilyParsedNumber`).
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    /// Entries in file order; a repeated key replaces the earlier value in place.
    Obj(Vec<(String, Json)>),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("JSON syntax error at byte {pos}: {msg}")]
pub struct SyntaxError {
    pub pos: usize,
    pub msg: &'static str,
}

impl Json {
    pub fn parse(text: &str) -> Result<Json, SyntaxError> {
        let mut p = JsonParser { s: text.as_bytes(), pos: 0 };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.pos != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Json::Num(_))
    }

    /// `JsonPrimitive.getAsBoolean`: booleans, and strings equal to "true" ignoring case.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// `LazilyParsedNumber.intValue()`: an int, else a long truncated, else a decimal truncated.
    pub fn as_i32(&self) -> Option<i32> {
        let Json::Num(s) = self else { return None };
        if let Ok(v) = s.parse::<i32>() {
            return Some(v);
        }
        if let Ok(v) = s.parse::<i64>() {
            return Some(v as i32);
        }
        s.parse::<f64>().ok().map(|d| decimal_to_i64(d) as i32)
    }

    /// `LazilyParsedNumber.longValue()`.
    pub fn as_i64(&self) -> Option<i64> {
        let Json::Num(s) = self else { return None };
        if let Ok(v) = s.parse::<i64>() {
            return Some(v);
        }
        s.parse::<f64>().ok().map(decimal_to_i64)
    }

    /// `LazilyParsedNumber.floatValue()` (`Float.parseFloat`).
    pub fn as_f32(&self) -> Option<f32> {
        let Json::Num(s) = self else { return None };
        s.parse::<f32>().ok()
    }

    pub fn as_f64(&self) -> Option<f64> {
        let Json::Num(s) = self else { return None };
        s.parse::<f64>().ok()
    }

    /// Compact form with object keys sorted, used as a stable key for predicates.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        self.write_canonical(&mut out);
        out
    }

    fn write_canonical(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(n),
            Json::Str(s) => write_str(out, s),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_canonical(out);
                }
                out.push(']');
            }
            Json::Obj(entries) => {
                let mut sorted: Vec<&(String, Json)> = entries.iter().collect();
                sorted.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                out.push('{');
                for (i, (k, v)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(out, k);
                    out.push(':');
                    v.write_canonical(out);
                }
                out.push('}');
            }
        }
    }

    /// The value `JsonOps` presents to a codec, in kiln-item's format-independent form.
    pub fn to_value(&self) -> Value {
        match self {
            Json::Null => Value::Empty,
            Json::Bool(b) => Value::Bool(*b),
            Json::Num(s) => {
                if let Ok(v) = s.parse::<i32>() {
                    Value::Int(v)
                } else if let Ok(v) = s.parse::<i64>() {
                    Value::Long(v)
                } else {
                    Value::Double(s.parse().unwrap_or(0.0))
                }
            }
            Json::Str(s) => Value::String(s.clone()),
            Json::Arr(items) => Value::List(items.iter().map(Json::to_value).collect()),
            Json::Obj(entries) => {
                Value::Map(entries.iter().map(|(k, v)| (Value::String(k.clone()), v.to_value())).collect())
            }
        }
    }
}

/// `new BigDecimal(s).longValue()` for the decimals JSON can spell (truncation toward zero,
/// wrapping like `BigInteger.longValue`).
fn decimal_to_i64(d: f64) -> i64 {
    if d.is_finite() && d.abs() < 9.2e18 { d.trunc() as i64 } else { 0 }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl JsonParser<'_> {
    fn err(&self, msg: &'static str) -> SyntaxError {
        SyntaxError { pos: self.pos, msg }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.pos) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, SyntaxError> {
        if depth > 512 {
            return Err(self.err("nesting too deep"));
        }
        match self.s.get(self.pos) {
            None => Err(self.err("unexpected end")),
            Some(b'{') => {
                self.pos += 1;
                let mut entries: Vec<(String, Json)> = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Ok(Json::Obj(entries));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return Err(self.err("expected ':'"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    match entries.iter_mut().find(|(k, _)| *k == key) {
                        Some(slot) => slot.1 = v,
                        None => entries.push((key, v)),
                    }
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Json::Obj(entries));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(Json::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Json::Arr(items));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.pos;
                while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') = self.s.get(self.pos) {
                    self.pos += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.pos]).map_err(|_| self.err("bad number"))?;
                if text.parse::<f64>().is_err() {
                    return Err(self.err("bad number"));
                }
                Ok(Json::Num(text.to_owned()))
            }
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn string(&mut self) -> Result<String, SyntaxError> {
        if !self.eat("\"") {
            return Err(self.err("expected string"));
        }
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(&b) = self.s.get(self.pos) {
                if b == b'"' || b == b'\\' {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.pos]).map_err(|_| self.err("invalid UTF-8"))?);
            match self.s.get(self.pos) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                _ => {
                    self.pos += 1;
                    let esc = *self.s.get(self.pos).ok_or_else(|| self.err("unterminated escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&hi) && self.eat("\\u") {
                                let lo = self.hex4()?;
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF))
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(c.unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, SyntaxError> {
        let digits = self.s.get(self.pos..self.pos + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.pos += 4;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_number_text() {
        let j = Json::parse(r#"{"b": 1.0, "a": [true, null, "xé"], "b": 2}"#).unwrap();
        let Json::Obj(entries) = &j else { panic!() };
        assert_eq!(entries[0], ("b".into(), Json::Num("2".into())));
        assert_eq!(j.canonical(), r#"{"a":[true,null,"xé"],"b":2}"#);
        assert_eq!(Json::Num("1.9".into()).as_i32(), Some(1));
        assert_eq!(Json::Num("-3.5".into()).as_i32(), Some(-3));
        assert_eq!(Json::Num("4294967297".into()).as_i32(), Some(1));
        assert!(Json::parse("[1,]").is_err());
    }
}
