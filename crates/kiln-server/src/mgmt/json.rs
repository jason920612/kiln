//! The JSON the management protocol speaks, as Gson (`JsonParser`, `JsonElement.toString`) reads and
//! writes it: objects keep the order of their members, numbers keep the text they came in as, and the
//! text of a parse error is Gson's.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    /// The number as written (`LazilyParsedNumber`).
    Num(String),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn int(i: i64) -> J {
        J::Num(i.to_string())
    }

    pub fn str(s: impl Into<String>) -> J {
        J::Str(s.into())
    }

    pub fn obj(members: Vec<(&str, J)>) -> J {
        J::Obj(members.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn is_object(&self) -> bool {
        matches!(self, J::Obj(_))
    }

    /// `JsonPrimitive.getAsString` for what is a primitive.
    pub fn primitive_string(&self) -> Option<String> {
        match self {
            J::Bool(b) => Some(b.to_string()),
            J::Num(n) => Some(n.clone()),
            J::Str(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// `GsonHelper.isNumberValue`.
    pub fn is_number(&self) -> bool {
        matches!(self, J::Num(_))
    }
}

fn write_string(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            '\u{8}' => f.write_str("\\b")?,
            '\u{c}' => f.write_str("\\f")?,
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => write!(f, "\\u{:04x}", c as u32)?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

/// `JsonElement.toString`: compact, members in order, no escaping of HTML characters.
impl fmt::Display for J {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            J::Null => f.write_str("null"),
            J::Bool(b) => write!(f, "{b}"),
            J::Num(n) => f.write_str(n),
            J::Str(s) => write_string(f, s),
            J::Arr(a) => {
                f.write_str("[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            J::Obj(m) => {
                f.write_str("{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write_string(f, k)?;
                    write!(f, ":{v}")?;
                }
                f.write_str("}")
            }
        }
    }
}

/// Why a text is not JSON, as the data of a "Parse error" reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    /// Gson's `JsonReader.getPath` of what is being read.
    path: Vec<String>,
}

enum Fail {
    /// The text ended inside a value (`EOFException`).
    Eof,
    /// Something that is no JSON at char `at` (consumed with it for Gson's column).
    Bad { at: usize, why: &'static str },
}

const LENIENT_HINT: &str = "Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON";
const TROUBLESHOOTING: &str = "\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json";

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], ' ' | '\t' | '\n' | '\r') {
            self.i += 1;
        }
    }

    fn path_string(&self) -> String {
        let mut p = String::from("$");
        for part in &self.path {
            p.push_str(part);
        }
        p
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn value(&mut self) -> Result<J, Fail> {
        self.skip_ws();
        let Some(c) = self.peek() else { return Err(Fail::Eof) };
        match c {
            '{' => self.object(),
            '[' => self.array(),
            '"' => self.string().map(J::Str),
            't' => self.word("true", J::Bool(true)),
            'f' => self.word("false", J::Bool(false)),
            'n' => self.word("null", J::Null),
            '-' | '0'..='9' => self.number(),
            _ => Err(Fail::Bad { at: self.i, why: LENIENT_HINT }),
        }
    }

    fn word(&mut self, w: &str, v: J) -> Result<J, Fail> {
        for (k, c) in w.chars().enumerate() {
            match self.s.get(self.i + k) {
                None => return Err(Fail::Eof),
                Some(&x) if x == c => {}
                Some(_) => return Err(Fail::Bad { at: self.i + k, why: LENIENT_HINT }),
            }
        }
        self.i += w.len();
        Ok(v)
    }

    fn number(&mut self) -> Result<J, Fail> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.i += 1;
            }
            p.i - from
        };
        if self.peek() == Some('-') {
            self.i += 1;
        }
        if digits(self) == 0 {
            return if self.peek().is_none() { Err(Fail::Eof) } else { Err(Fail::Bad { at: self.i, why: LENIENT_HINT }) };
        }
        if self.peek() == Some('.') {
            self.i += 1;
            if digits(self) == 0 {
                return if self.peek().is_none() { Err(Fail::Eof) } else { Err(Fail::Bad { at: self.i, why: LENIENT_HINT }) };
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.i += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return if self.peek().is_none() { Err(Fail::Eof) } else { Err(Fail::Bad { at: self.i, why: LENIENT_HINT }) };
            }
        }
        Ok(J::Num(self.s[start..self.i].iter().collect()))
    }

    fn string(&mut self) -> Result<String, Fail> {
        self.i += 1; // the quote
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(Fail::Eof) };
            self.i += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(e) = self.peek() else { return Err(Fail::Eof) };
                    self.i += 1;
                    match e {
                        '"' | '\\' | '/' | '\'' => out.push(e),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' => {
                            let mut code = 0u32;
                            for _ in 0..4 {
                                let Some(h) = self.peek() else { return Err(Fail::Eof) };
                                self.i += 1;
                                let Some(d) = h.to_digit(16) else { return Err(Fail::Bad { at: self.i - 1, why: "Malformed Unicode escape" }) };
                                code = code * 16 + d;
                            }
                            // (A lone surrogate half cannot be a Rust char.)
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(Fail::Bad { at: self.i - 1, why: "Invalid escape sequence" }),
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn array(&mut self) -> Result<J, Fail> {
        self.i += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.i += 1;
            return Ok(J::Arr(items));
        }
        loop {
            self.path.push(format!("[{}]", items.len()));
            items.push(self.value()?);
            self.path.pop();
            self.skip_ws();
            match self.peek() {
                None => return Err(Fail::Eof),
                Some(',') => self.i += 1,
                Some(']') => {
                    self.i += 1;
                    return Ok(J::Arr(items));
                }
                Some(_) => return Err(Fail::Bad { at: self.i, why: LENIENT_HINT }),
            }
        }
    }

    fn object(&mut self) -> Result<J, Fail> {
        self.i += 1;
        let mut members: Vec<(String, J)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok(J::Obj(members));
        }
        loop {
            self.skip_ws();
            let name = match self.peek() {
                None => return Err(Fail::Eof),
                Some('"') => self.string()?,
                Some(_) => return Err(Fail::Bad { at: self.i, why: LENIENT_HINT }),
            };
            self.skip_ws();
            match self.peek() {
                None => return Err(Fail::Eof),
                Some(':') => self.i += 1,
                Some(_) => return Err(Fail::Bad { at: self.i, why: LENIENT_HINT }),
            }
            self.path.push(format!(".{name}"));
            let v = self.value()?;
            self.path.pop();
            // (`LinkedTreeMap.put`: a repeated name keeps its place and takes the new value.)
            match members.iter_mut().find(|(k, _)| *k == name) {
                Some(slot) => slot.1 = v,
                None => members.push((name, v)),
            }
            self.skip_ws();
            match self.peek() {
                None => return Err(Fail::Eof),
                Some(',') => self.i += 1,
                Some('}') => {
                    self.i += 1;
                    return Ok(J::Obj(members));
                }
                Some(_) => return Err(Fail::Bad { at: self.i, why: LENIENT_HINT }),
            }
        }
    }
}

/// `JsonParser.parseString`.
pub fn parse(text: &str) -> Result<J, ParseError> {
    let chars: Vec<char> = text.chars().collect();
    let mut p = Parser { s: &chars, i: 0, path: Vec::new() };
    let position = |upto: usize| {
        let mut line = 1;
        let mut start = 0;
        for (i, c) in chars.iter().enumerate().take(upto) {
            if *c == '\n' {
                line += 1;
                start = i + 1;
            }
        }
        (line, upto - start + 1)
    };
    let result = p.value().and_then(|v| {
        p.skip_ws();
        match p.peek() {
            None => Ok(v),
            Some(_) => Err(Fail::Bad { at: p.i, why: LENIENT_HINT }),
        }
    });
    match result {
        Ok(v) => Ok(v),
        Err(Fail::Eof) => {
            let (l, c) = position(chars.len());
            Err(ParseError(format!(
                "com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line {l} column {c} path {}",
                p.path_string()
            )))
        }
        Err(Fail::Bad { at, why }) => {
            let (l, c) = position(at + 1);
            let tail = if why == LENIENT_HINT { TROUBLESHOOTING } else { "" };
            Err(ParseError(format!(
                "com.google.gson.JsonSyntaxException: com.google.gson.stream.MalformedJsonException: {why} at line {l} column {c} path {}{tail}",
                p.path_string()
            )))
        }
    }
}
