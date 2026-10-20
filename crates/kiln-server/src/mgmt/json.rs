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

/// Where `Gson`'s `JsonReader.getPath` is: arrays count what they have read, objects hold the last name.
enum Seg {
    Arr(usize),
    Obj(Option<String>),
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    path: Vec<Seg>,
}

enum Fail {
    /// The text ended inside a value (`EOFException`).
    Eof,
    /// A syntax error; `consumed` is how many chars the reader had taken when it found it.
    Bad { consumed: usize, why: &'static str },
}

const LENIENT: &str = "Use JsonReader.setStrictness(Strictness.LENIENT) to accept malformed JSON";
const TROUBLESHOOTING: &str = "\nSee https://github.com/google/gson/blob/main/Troubleshooting.md#malformed-json";

/// `JsonReader.isLiteral` (lenient): what an unquoted value or name is made of.
fn literal_char(c: char) -> bool {
    !matches!(c, '/' | '\\' | ';' | '#' | '=' | '{' | '}' | '[' | ']' | ':' | ',' | ' ' | '\t' | '\u{c}' | '\r' | '\n')
}

impl Parser<'_> {
    fn bad<T>(&self, consumed: usize, why: &'static str) -> Result<T, Fail> {
        Err(Fail::Bad { consumed, why })
    }

    /// The next char that is not white space or inside a comment (`nextNonWhitespace`, lenient).
    fn next_token(&mut self) -> Result<Option<char>, Fail> {
        loop {
            let Some(&c) = self.s.get(self.i) else { return Ok(None) };
            match c {
                ' ' | '\t' | '\n' | '\r' => self.i += 1,
                '/' => match self.s.get(self.i + 1) {
                    Some('*') => {
                        let mut j = self.i + 2;
                        loop {
                            if j + 1 >= self.s.len() {
                                return self.bad(self.s.len(), "Unterminated comment");
                            }
                            if self.s[j] == '*' && self.s[j + 1] == '/' {
                                break;
                            }
                            j += 1;
                        }
                        self.i = j + 2;
                    }
                    Some('/') => self.skip_line(),
                    _ => return self.bad(self.i + 1, LENIENT),
                },
                '#' => self.skip_line(),
                _ => return Ok(Some(c)),
            }
        }
    }

    fn skip_line(&mut self) {
        while self.i < self.s.len() && !matches!(self.s[self.i], '\n' | '\r') {
            self.i += 1;
        }
    }

    fn quoted(&mut self, quote: char) -> Result<String, Fail> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return self.bad(self.s.len(), "Unterminated string") };
            self.i += 1;
            if c == quote {
                return Ok(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let Some(&e) = self.s.get(self.i) else { return self.bad(self.s.len(), "Unterminated escape sequence") };
            self.i += 1;
            match e {
                '"' | '\\' | '/' | '\'' => out.push(e),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                '\n' => out.push('\n'),
                'u' => {
                    if self.i + 4 > self.s.len() {
                        return self.bad(self.i, "Unterminated escape sequence");
                    }
                    let hex: String = self.s[self.i..self.i + 4].iter().collect();
                    let Ok(code) = u32::from_str_radix(&hex, 16) else { return self.bad(self.i + 4, "Malformed Unicode escape") };
                    self.i += 4;
                    // (A lone surrogate half cannot be a Rust char.)
                    out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                }
                _ => return self.bad(self.i, "Invalid escape sequence"),
            }
        }
    }

    /// An unquoted run (`nextUnquotedValue`).
    fn literal(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() && literal_char(self.s[self.i]) {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }

    /// What an unquoted run is: `true`, `false`, `null`, a number, or text.
    fn primitive(text: String) -> J {
        match text.as_str() {
            "true" | "TRUE" => return J::Bool(true),
            "false" | "FALSE" => return J::Bool(false),
            "null" | "NULL" => return J::Null,
            _ => {}
        }
        let b = text.as_bytes();
        let mut i = usize::from(b.first() == Some(&b'-'));
        let int_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let int_len = i - int_start;
        let mut ok = int_len > 0 && !(int_len > 1 && b[int_start] == b'0');
        if ok && i < b.len() && b[i] == b'.' {
            i += 1;
            let f = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            ok = i > f;
        }
        if ok && i < b.len() && matches!(b[i], b'e' | b'E') {
            i += 1;
            if i < b.len() && matches!(b[i], b'+' | b'-') {
                i += 1;
            }
            let e = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            ok = i > e;
        }
        if ok && i == b.len() { J::Num(text) } else { J::Str(text) }
    }

    /// A value. `in_array` lets an empty element stand for null.
    fn value(&mut self, in_array: bool) -> Result<J, Fail> {
        let Some(c) = self.next_token()? else { return Err(Fail::Eof) };
        match c {
            '"' | '\'' => self.quoted(c).map(J::Str),
            '{' => {
                self.i += 1;
                self.object()
            }
            '[' => {
                self.i += 1;
                self.array()
            }
            ']' | ';' | ',' => {
                if in_array {
                    // (Lenient: an empty element is null; the separator is read by the caller.)
                    Ok(J::Null)
                } else {
                    self.bad(self.i + 1, "Unexpected value")
                }
            }
            c if literal_char(c) => Ok(Self::primitive(self.literal())),
            _ => self.bad(self.i, "Expected value"),
        }
    }

    fn array(&mut self) -> Result<J, Fail> {
        let mut items = Vec::new();
        self.path.push(Seg::Arr(0));
        match self.next_token()? {
            None => return Err(Fail::Eof),
            Some(']') => {
                self.i += 1;
                self.path.pop();
                return Ok(J::Arr(items));
            }
            Some(_) => {}
        }
        loop {
            items.push(self.value(true)?);
            if let Some(Seg::Arr(n)) = self.path.last_mut() {
                *n = items.len();
            }
            match self.next_token()? {
                None => return Err(Fail::Eof),
                Some(',' | ';') => self.i += 1,
                Some(']') => {
                    self.i += 1;
                    self.path.pop();
                    return Ok(J::Arr(items));
                }
                Some(_) => return self.bad(self.i + 1, "Unterminated array"),
            }
        }
    }

    fn object(&mut self) -> Result<J, Fail> {
        let mut members: Vec<(String, J)> = Vec::new();
        self.path.push(Seg::Obj(None));
        let mut first = true;
        loop {
            // A name, or the end of the object.
            let Some(c) = self.next_token()? else { return Err(Fail::Eof) };
            let name = match c {
                '}' if first => {
                    self.i += 1;
                    self.path.pop();
                    return Ok(J::Obj(members));
                }
                '}' => return self.bad(self.i + 1, "Expected name"),
                '"' | '\'' => self.quoted(c)?,
                c if literal_char(c) => self.literal(),
                _ => return self.bad(self.i, "Expected name"),
            };
            first = false;
            if let Some(Seg::Obj(n)) = self.path.last_mut() {
                *n = Some(name.clone());
            }
            // `:`, or `=` or `=>`.
            match self.next_token()? {
                None => return Err(Fail::Eof),
                Some(':') => self.i += 1,
                Some('=') => {
                    self.i += 1;
                    if self.s.get(self.i) == Some(&'>') {
                        self.i += 1;
                    }
                }
                Some(_) => return self.bad(self.i + 1, "Expected ':'"),
            }
            let v = self.value(false)?;
            // (`LinkedTreeMap.put`: a repeated name keeps its place and takes the new value.)
            match members.iter_mut().find(|(k, _)| *k == name) {
                Some(slot) => slot.1 = v,
                None => members.push((name, v)),
            }
            match self.next_token()? {
                None => return Err(Fail::Eof),
                Some(',' | ';') => self.i += 1,
                Some('}') => {
                    self.i += 1;
                    self.path.pop();
                    return Ok(J::Obj(members));
                }
                Some(_) => return self.bad(self.i + 1, "Unterminated object"),
            }
        }
    }

    fn path_string(&self) -> String {
        let mut p = String::from("$");
        for seg in &self.path {
            match seg {
                Seg::Arr(n) => p.push_str(&format!("[{n}]")),
                Seg::Obj(Some(name)) => p.push_str(&format!(".{name}")),
                Seg::Obj(None) => p.push('.'),
            }
        }
        p
    }
}

/// `JsonParser.parseString`: lenient, as Gson reads a document; an empty one is `null`.
pub fn parse(text: &str) -> Result<J, ParseError> {
    let chars: Vec<char> = text.chars().collect();
    let mut p = Parser { s: &chars, i: 0, path: Vec::new() };
    let position = |consumed: usize| {
        let mut line = 1;
        let mut start = 0;
        for (i, c) in chars.iter().enumerate().take(consumed) {
            if *c == '\n' {
                line += 1;
                start = i + 1;
            }
        }
        (line, consumed - start + 1)
    };
    // (Nothing but white space and comments is no document at all.)
    if let Ok(None) = p.next_token() {
        return Ok(J::Null);
    }
    p.i = 0;
    let result = p.value(false).and_then(|v| {
        // After the value only white space may follow (`JsonParser` checks with a strict reader).
        while matches!(p.s.get(p.i), Some(' ' | '\t' | '\n' | '\r')) {
            p.i += 1;
        }
        match p.s.get(p.i) {
            None => Ok(v),
            Some(_) => Err(Fail::Bad { consumed: p.i + 1, why: LENIENT }),
        }
    });
    match result {
        Ok(v) => Ok(v),
        Err(Fail::Eof) => {
            let (l, c) = position(chars.len());
            Err(ParseError(format!("com.google.gson.JsonSyntaxException: java.io.EOFException: End of input at line {l} column {c} path {}", p.path_string())))
        }
        Err(Fail::Bad { consumed, why }) => {
            let (l, c) = position(consumed);
            Err(ParseError(format!(
                "com.google.gson.JsonSyntaxException: com.google.gson.stream.MalformedJsonException: {why} at line {l} column {c} path {}{TROUBLESHOOTING}",
                p.path_string()
            )))
        }
    }
}
