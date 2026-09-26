//! SNBT as 26.x reads it (`TagParser` over `SnbtGrammar`): compounds, lists (mixed element
//! types allowed), typed arrays, quoted strings with escapes, bare words, `true`/`false`,
//! `bool(...)`/`uuid(...)`, and numbers with signs, underscores, hex/binary literals, signed
//! or unsigned integer suffixes and exponents. Trailing commas are accepted.
//!
//! The parser explores the grammar's alternatives in vanilla's order and reports errors the
//! way its packrat parser does: only errors at the furthest position count, and the first one
//! recorded there is thrown.

use crate::error::CommandError;
use crate::reader::{StringReader, is_allowed_in_unquoted_string, is_java_whitespace};
use crate::tr;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// `TagParser.parseAsArgument`: any SNBT value.
pub fn parse_tag(reader: &mut StringReader) -> Result<Tag> {
    let mut p = Parser { reader, errors: Errors::default(), silent: false, depth: 0 };
    let start = p.reader.cursor();
    match p.literal() {
        Some(tag) => Ok(tag),
        None => {
            p.reader.set_cursor(start);
            Err(p.errors.into_error(p.reader.string()))
        }
    }
}

/// `TagParser.parseCompoundAsArgument`: a value that must be a compound.
pub fn parse_compound(reader: &mut StringReader) -> Result<Tag> {
    let tag = parse_tag(reader)?;
    if matches!(tag, Tag::Compound(_)) { Ok(tag) } else { Err(expected_compound().at(reader)) }
}

/// A compound, returned as the text it was read from.
pub fn read_compound<'a>(reader: &mut StringReader<'a>) -> Result<&'a str> {
    let start = reader.cursor();
    parse_compound(reader)?;
    Ok(&reader.string()[start..reader.cursor()])
}

/// Any value, returned as the text it was read from.
pub fn read_value<'a>(reader: &mut StringReader<'a>) -> Result<&'a str> {
    let start = reader.cursor();
    parse_tag(reader)?;
    Ok(&reader.string()[start..reader.cursor()])
}

fn expected_compound() -> CommandError {
    CommandError::new(tr!("argument.nbt.expected.compound"))
}

fn snbt_error(key: &str) -> CommandError {
    CommandError::new(tr!(format!("snbt.parser.{key}")))
}

/// `ErrorCollector.LongestOnly`: errors at the furthest cursor, in the order recorded.
#[derive(Default)]
struct Errors {
    cursor: usize,
    entries: Vec<CommandError>,
}

impl Errors {
    fn store(&mut self, cursor: usize, error: CommandError) {
        if cursor > self.cursor {
            self.cursor = cursor;
            self.entries.clear();
        }
        if cursor == self.cursor {
            self.entries.push(error);
        }
    }

    fn into_error(self, input: &str) -> CommandError {
        let e = self.entries.into_iter().next().unwrap_or_else(|| snbt_error("expected_unquoted_string"));
        e.with_context(input, self.cursor)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    Plus,
    Minus,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Float,
    Double,
    Byte,
    Short,
    Int,
    Long,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Base {
    Binary,
    Decimal,
    Hex,
}

/// `SnbtGrammar.IntegerLiteral`: sign, digits (underscores kept), base and suffix.
struct IntegerLiteral {
    sign: Sign,
    digits: String,
    base: Base,
    signed: Option<bool>,
    kind: Option<Kind>,
}

struct Parser<'r, 'a> {
    reader: &'r mut StringReader<'a>,
    errors: Errors,
    /// Inside a lookahead: errors are not recorded.
    silent: bool,
    /// Nesting of values, bounded like `NbtAccounter`'s depth limit.
    depth: usize,
}

/// Deepest nesting accepted (vanilla's NBT depth limit).
const MAX_DEPTH: usize = 512;

impl Parser<'_, '_> {
    fn fail(&mut self, cursor: usize, error: CommandError) {
        if !self.silent {
            self.errors.store(cursor, error);
        }
    }

    fn skip_ws(&mut self) {
        while self.reader.can_read() && is_java_whitespace(self.reader.peek()) {
            self.reader.skip();
        }
    }

    /// `StringReaderTerms.characters`: skips whitespace, then one of `chars`.
    fn chars(&mut self, chars: &[char]) -> Option<char> {
        self.skip_ws();
        let at = self.reader.cursor();
        if self.reader.can_read() && chars.contains(&self.reader.peek()) {
            return Some(self.reader.read());
        }
        let expected: Vec<String> = chars.iter().map(char::to_string).collect();
        self.fail(at, CommandError::literal_incorrect(&expected.join("|")));
        None
    }

    fn char(&mut self, c: char) -> bool {
        self.chars(&[c]).is_some()
    }

    /// `Term.positiveLookahead(characters)`.
    fn lookahead(&mut self, accept: impl Fn(char) -> bool) -> bool {
        let at = self.reader.cursor();
        self.skip_ws();
        let ok = self.reader.can_read() && accept(self.reader.peek());
        self.reader.set_cursor(at);
        ok
    }

    /// `NumberRunParseRule`: digits of the base plus inner underscores.
    fn number_run(&mut self, accept: fn(char) -> bool, expected: &str) -> Option<String> {
        self.skip_ws();
        let start = self.reader.cursor();
        let s = self.reader.string();
        let len = s[start..].find(|c: char| !accept(c)).unwrap_or(s.len() - start);
        if len == 0 {
            self.fail(start, snbt_error(expected));
            return None;
        }
        let run = &s[start..start + len];
        if run.starts_with('_') || run.ends_with('_') {
            self.fail(start, snbt_error("underscore_not_allowed"));
            return None;
        }
        self.reader.set_cursor(start + len);
        Some(run.to_owned())
    }

    fn decimal(&mut self) -> Option<String> {
        self.number_run(|c| c.is_ascii_digit() || c == '_', "expected_decimal_numeral")
    }

    fn sign(&mut self) -> Option<Sign> {
        let at = self.reader.cursor();
        if self.char('+') {
            return Some(Sign::Plus);
        }
        self.reader.set_cursor(at);
        if self.char('-') {
            return Some(Sign::Minus);
        }
        self.reader.set_cursor(at);
        None
    }

    fn optional<T>(&mut self, f: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let at = self.reader.cursor();
        let r = f(self);
        if r.is_none() {
            self.reader.set_cursor(at);
        }
        r
    }

    /// `literal`: the top rule.
    fn literal(&mut self) -> Option<Tag> {
        if self.depth >= MAX_DEPTH {
            let here = self.reader.cursor();
            self.fail(here, CommandError::new(tr!("arguments.nbtpath.too_deep")));
            return None;
        }
        self.depth += 1;
        let r = self.literal_inner();
        self.depth -= 1;
        r
    }

    fn literal_inner(&mut self) -> Option<Tag> {
        let start = self.reader.cursor();
        if self.lookahead(can_start_number) {
            if let Some(t) = self.optional(|p| p.float_literal()) {
                return Some(t);
            }
            if let Some(i) = self.optional(|p| p.integer_literal()) {
                // The value is computed once the alternative matched: no fallback.
                let r = self.integer_tag(&i, i.kind.unwrap_or(Kind::Int));
                if r.is_none() {
                    self.reader.set_cursor(start);
                }
                return r;
            }
            self.reader.set_cursor(start);
        }
        if self.lookahead(|c| c == '"' || c == '\'') {
            let r = self.quoted_string().map(Tag::String);
            if r.is_none() {
                self.reader.set_cursor(start);
            }
            return r;
        }
        if self.lookahead(|c| c == '{') {
            let r = self.map_literal();
            if r.is_none() {
                self.reader.set_cursor(start);
            }
            return r;
        }
        if self.lookahead(|c| c == '[') {
            let r = self.list_literal();
            if r.is_none() {
                self.reader.set_cursor(start);
            }
            return r;
        }
        let r = self.unquoted_string_or_builtin();
        if r.is_none() {
            self.reader.set_cursor(start);
        }
        r
    }

    /// `integer_suffix`.
    fn integer_suffix(&mut self) -> Option<(Option<bool>, Kind)> {
        const TYPES: [(&[char], Kind); 4] =
            [(&['b', 'B'], Kind::Byte), (&['s', 'S'], Kind::Short), (&['i', 'I'], Kind::Int), (&['l', 'L'], Kind::Long)];
        let at = self.reader.cursor();
        for (prefix, signed) in [(&['u', 'U'][..], false), (&['s', 'S'][..], true)] {
            if self.chars(prefix).is_some() {
                let after = self.reader.cursor();
                for (chars, kind) in TYPES {
                    if self.chars(chars).is_some() {
                        return Some((Some(signed), kind));
                    }
                    self.reader.set_cursor(after);
                }
            }
            self.reader.set_cursor(at);
        }
        for (chars, kind) in TYPES {
            if self.chars(chars).is_some() {
                return Some((None, kind));
            }
            self.reader.set_cursor(at);
        }
        None
    }

    /// `integer_literal`.
    fn integer_literal(&mut self) -> Option<IntegerLiteral> {
        let sign = self.optional(|p| p.sign()).unwrap_or(Sign::Plus);
        let at = self.reader.cursor();
        let (base, digits) = if self.char('0') {
            // cut: no plain decimal alternative once a leading zero was read
            let after = self.reader.cursor();
            if self.chars(&['x', 'X']).is_some() {
                let hex = self.number_run(|c| c.is_ascii_hexdigit() || c == '_', "expected_hex_numeral")?;
                (Base::Hex, hex)
            } else {
                self.reader.set_cursor(after);
                let binary = if self.chars(&['b', 'B']).is_some() {
                    self.number_run(|c| matches!(c, '0' | '1' | '_'), "expected_binary_numeral")
                } else {
                    None
                };
                match binary {
                    Some(b) => (Base::Binary, b),
                    None => {
                        self.reader.set_cursor(after);
                        if self.decimal().is_some() {
                            let here = self.reader.cursor();
                            self.fail(here, snbt_error("leading_zero_not_allowed"));
                            return None;
                        }
                        self.reader.set_cursor(after);
                        (Base::Decimal, "0".to_owned())
                    }
                }
            }
        } else {
            self.reader.set_cursor(at);
            (Base::Decimal, self.decimal()?)
        };
        let (signed, kind) = match self.optional(|p| p.integer_suffix()) {
            Some((s, k)) => (s, Some(k)),
            None => (None, None),
        };
        Some(IntegerLiteral { sign, digits, base, signed, kind })
    }

    /// `IntegerLiteral.create`: range checks per type and signedness.
    fn integer_tag(&mut self, lit: &IntegerLiteral, kind: Kind) -> Option<Tag> {
        let signed = lit.signed.unwrap_or(lit.base == Base::Decimal);
        if !signed && lit.sign == Sign::Minus {
            let here = self.reader.cursor();
            self.fail(here, snbt_error("expected_non_negative_number"));
            return None;
        }
        let digits: String = lit.digits.chars().filter(|&c| c != '_').collect();
        let radix = match lit.base {
            Base::Binary => 2,
            Base::Decimal => 10,
            Base::Hex => 16,
        };
        let text = if lit.sign == Sign::Minus { format!("-{digits}") } else { digits.clone() };
        let parsed = if signed {
            match kind {
                Kind::Byte => i8::from_str_radix(&text, radix).map(Tag::Byte).map_err(|_| out_of_range(&text, radix)),
                Kind::Short => i16::from_str_radix(&text, radix).map(Tag::Short).map_err(|_| out_of_range(&text, radix)),
                Kind::Int => i32::from_str_radix(&text, radix).map(Tag::Int).map_err(|_| for_input(&text, radix)),
                Kind::Long => i64::from_str_radix(&text, radix).map(Tag::Long).map_err(|_| for_input(&text, radix)),
                _ => Err(String::new()),
            }
        } else {
            match kind {
                Kind::Byte => u8::from_str_radix(&text, radix)
                    .map(|v| Tag::Byte(v as i8))
                    .map_err(|_| format!("Value {text} exceeds unsigned byte range")),
                Kind::Short => u16::from_str_radix(&text, radix)
                    .map(|v| Tag::Short(v as i16))
                    .map_err(|_| format!("Value {text} is outside the range of an unsigned short")),
                Kind::Int => {
                    u32::from_str_radix(&text, radix).map(|v| Tag::Int(v as i32)).map_err(|_| for_input(&text, radix))
                }
                Kind::Long => {
                    u64::from_str_radix(&text, radix).map(|v| Tag::Long(v as i64)).map_err(|_| for_input(&text, radix))
                }
                _ => Err(String::new()),
            }
        };
        let here = self.reader.cursor();
        match (kind, parsed) {
            (Kind::Float | Kind::Double, _) => {
                self.fail(here, snbt_error("expected_integer_type"));
                None
            }
            (_, Ok(tag)) => Some(tag),
            (_, Err(message)) => {
                self.fail(here, CommandError::new(tr!("snbt.parser.number_parse_failure", message)));
                None
            }
        }
    }

    /// `float_literal`.
    fn float_literal(&mut self) -> Option<Tag> {
        let sign = self.optional(|p| p.sign()).unwrap_or(Sign::Plus);
        let at = self.reader.cursor();
        let parts = 'alt: {
            // whole '.' cut fraction? exponent? suffix?
            if let Some(whole) = self.decimal() {
                if self.char('.') {
                    let fraction = self.optional(|p| p.decimal());
                    let exponent = self.optional(|p| p.exponent());
                    let suffix = self.optional(|p| p.float_suffix());
                    break 'alt Some((Some(whole), fraction, exponent, suffix));
                }
            }
            self.reader.set_cursor(at);
            // '.' cut fraction exponent? suffix?
            if self.char('.') {
                let fraction = self.decimal();
                let Some(fraction) = fraction else { break 'alt None };
                let exponent = self.optional(|p| p.exponent());
                let suffix = self.optional(|p| p.float_suffix());
                break 'alt Some((None, Some(fraction), exponent, suffix));
            }
            self.reader.set_cursor(at);
            // whole exponent cut suffix?
            if let Some(whole) = self.decimal() {
                if let Some(exponent) = self.exponent() {
                    let suffix = self.optional(|p| p.float_suffix());
                    break 'alt Some((Some(whole), None, Some(exponent), suffix));
                }
            }
            self.reader.set_cursor(at);
            // whole exponent? suffix
            if let Some(whole) = self.decimal() {
                let exponent = self.optional(|p| p.exponent());
                if let Some(suffix) = self.float_suffix() {
                    break 'alt Some((Some(whole), None, exponent, Some(suffix)));
                }
            }
            None
        };
        let (whole, fraction, exponent, suffix) = parts?;
        let mut text = String::new();
        if sign == Sign::Minus {
            text.push('-');
        }
        let clean = |s: &str| s.chars().filter(|&c| c != '_').collect::<String>();
        if let Some(w) = &whole {
            text.push_str(&clean(w));
        }
        if let Some(f) = &fraction {
            text.push('.');
            text.push_str(&clean(f));
        }
        if let Some((neg, digits)) = &exponent {
            text.push('e');
            if *neg {
                text.push('-');
            }
            text.push_str(&clean(digits));
        }
        let here = self.reader.cursor();
        let tag = match suffix {
            Some(Kind::Float) => java_float(&text).filter(|v| v.is_finite()).map(Tag::Float),
            _ => java_double(&text).filter(|v| v.is_finite()).map(Tag::Double),
        };
        if tag.is_none() {
            self.fail(here, snbt_error("infinity_not_allowed"));
        }
        tag
    }

    /// `float_exponent_part`: `(negative, digits)`.
    fn exponent(&mut self) -> Option<(bool, String)> {
        self.chars(&['e', 'E'])?;
        let sign = self.optional(|p| p.sign());
        Some((sign == Some(Sign::Minus), self.decimal()?))
    }

    fn float_suffix(&mut self) -> Option<Kind> {
        let at = self.reader.cursor();
        if self.chars(&['f', 'F']).is_some() {
            return Some(Kind::Float);
        }
        self.reader.set_cursor(at);
        self.chars(&['d', 'D']).map(|_| Kind::Double)
    }

    /// `quoted_string_literal`.
    fn quoted_string(&mut self) -> Option<String> {
        let at = self.reader.cursor();
        for (quote, other) in [('"', '\''), ('\'', '"')] {
            if self.char(quote) {
                let contents = self.optional(|p| p.string_contents(quote, other)).unwrap_or_default();
                if self.char(quote) {
                    return Some(contents);
                }
                // cut after the opening quote
                return None;
            }
            self.reader.set_cursor(at);
        }
        None
    }

    /// `*_quoted_string_contents`: one or more chunks.
    fn string_contents(&mut self, quote: char, other: char) -> Option<String> {
        let mut out = String::new();
        let mut any = false;
        loop {
            let at = self.reader.cursor();
            // plain run
            let s = self.reader.string();
            let len = s[at..].find(|c: char| matches!(c, '"' | '\'' | '\\')).unwrap_or(s.len() - at);
            if len > 0 {
                out.push_str(&s[at..at + len]);
                self.reader.set_cursor(at + len);
                any = true;
                continue;
            }
            self.fail(at, snbt_error("invalid_string_contents"));
            if self.char('\\') {
                if let Some(e) = self.escape() {
                    out.push_str(&e);
                    any = true;
                    continue;
                }
            }
            self.reader.set_cursor(at);
            if self.char(other) {
                out.push(other);
                any = true;
                continue;
            }
            self.reader.set_cursor(at);
            let _ = quote;
            break;
        }
        any.then_some(out)
    }

    /// `string_escape_sequence`.
    fn escape(&mut self) -> Option<String> {
        const SIMPLE: [(char, &str); 9] = [
            ('b', "\u{8}"),
            ('s', " "),
            ('t', "\t"),
            ('n', "\n"),
            ('f', "\u{c}"),
            ('r', "\r"),
            ('\\', "\\"),
            ('\'', "'"),
            ('"', "\""),
        ];
        let at = self.reader.cursor();
        for (c, s) in SIMPLE {
            if self.char(c) {
                return Some(s.to_owned());
            }
            self.reader.set_cursor(at);
        }
        for (c, n) in [('x', 2), ('u', 4), ('U', 8)] {
            if self.char(c) {
                let digits_at = self.reader.cursor();
                let s = self.reader.string();
                let len = s[digits_at..].chars().take(n).take_while(char::is_ascii_hexdigit).count();
                if len < n {
                    self.fail(digits_at, CommandError::new(tr!("snbt.parser.expected_hex_escape", n as i32)));
                    self.reader.set_cursor(at);
                    return None;
                }
                self.reader.set_cursor(digits_at + n);
                let code = u32::from_str_radix(&s[digits_at..digits_at + n], 16).ok()?;
                return match char::from_u32(code) {
                    Some(ch) => Some(ch.to_string()),
                    None => {
                        let here = self.reader.cursor();
                        self.fail(here, CommandError::new(tr!("snbt.parser.invalid_codepoint", format!("U+{code:08X}"))));
                        None
                    }
                };
            }
            self.reader.set_cursor(at);
        }
        if self.char('N') && self.char('{') {
            let name_at = self.reader.cursor();
            let s = self.reader.string();
            let len =
                s[name_at..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ' ')).unwrap_or(s.len() - name_at);
            if len == 0 {
                self.fail(name_at, snbt_error("invalid_character_name"));
                return None;
            }
            self.reader.set_cursor(name_at + len);
            let here = self.reader.cursor();
            // Unicode character names are not tabulated here; vanilla resolves them with
            // `Character.codePointOf`.
            self.fail(here, snbt_error("invalid_character_name"));
            return None;
        }
        None
    }

    /// `unquoted_string_or_builtin`: bare words, booleans and operations.
    fn unquoted_string_or_builtin(&mut self) -> Option<Tag> {
        self.skip_ws();
        let start = self.reader.cursor();
        let word = self.reader.read_unquoted_string().to_owned();
        if word.is_empty() {
            self.fail(start, snbt_error("expected_unquoted_string"));
            return None;
        }
        let at = self.reader.cursor();
        let args = if self.char('(') {
            let args = self.repeated(|p| p.literal());
            if self.char(')') {
                Some(args)
            } else {
                self.reader.set_cursor(at);
                None
            }
        } else {
            self.reader.set_cursor(at);
            None
        };
        let here = self.reader.cursor();
        if !word.chars().next().is_some_and(|c| !can_start_number(c)) {
            self.fail(here, snbt_error("invalid_unquoted_start"));
            return None;
        }
        if let Some(args) = args {
            return self.operation(&word, args, here);
        }
        if word.eq_ignore_ascii_case("true") {
            return Some(Tag::Byte(1));
        }
        if word.eq_ignore_ascii_case("false") {
            return Some(Tag::Byte(0));
        }
        Some(Tag::String(word))
    }

    /// `SnbtOperations.BUILTIN_OPERATIONS`.
    fn operation(&mut self, name: &str, args: Vec<Tag>, here: usize) -> Option<Tag> {
        match (name, args.as_slice()) {
            ("bool", [v]) => match v {
                Tag::Byte(n) => Some(Tag::Byte((*n != 0) as i8)),
                Tag::Short(n) => Some(Tag::Byte((*n != 0) as i8)),
                Tag::Int(n) => Some(Tag::Byte((*n != 0) as i8)),
                Tag::Long(n) => Some(Tag::Byte((*n != 0) as i8)),
                Tag::Float(n) => Some(Tag::Byte((*n != 0.0) as i8)),
                Tag::Double(n) => Some(Tag::Byte((*n != 0.0) as i8)),
                _ => {
                    self.fail(here, snbt_error("expected_number_or_boolean"));
                    None
                }
            },
            ("uuid", [v]) => {
                let uuid = match v {
                    Tag::String(s) => crate::selector::java_uuid_from_string(s),
                    _ => None,
                };
                match uuid {
                    Some(u) => {
                        let (hi, lo) = u.as_u64_pair();
                        Some(Tag::IntArray(vec![(hi >> 32) as i32, hi as i32, (lo >> 32) as i32, lo as i32]))
                    }
                    None => {
                        self.fail(here, snbt_error("expected_string_uuid"));
                        None
                    }
                }
            }
            _ => {
                let key = format!("{name}/{}", args.len());
                self.fail(here, CommandError::new(tr!("snbt.parser.no_such_operation", key)));
                None
            }
        }
    }

    /// `Term.repeatedWithTrailingSeparator(element, ',')`.
    fn repeated<T>(&mut self, mut element: impl FnMut(&mut Self) -> Option<T>) -> Vec<T> {
        let mut out = Vec::new();
        loop {
            let at = self.reader.cursor();
            if !out.is_empty() && !self.char(',') {
                self.reader.set_cursor(at);
                break;
            }
            let before = self.reader.cursor();
            match element(self) {
                Some(v) => out.push(v),
                None => {
                    self.reader.set_cursor(before);
                    break;
                }
            }
        }
        out
    }

    /// `map_key`.
    fn map_key(&mut self) -> Option<String> {
        let at = self.reader.cursor();
        if let Some(s) = self.quoted_string() {
            return Some(s);
        }
        self.reader.set_cursor(at);
        self.skip_ws();
        let start = self.reader.cursor();
        let word = self.reader.read_unquoted_string();
        if word.is_empty() {
            self.fail(start, snbt_error("expected_unquoted_string"));
            self.reader.set_cursor(at);
            return None;
        }
        Some(word.to_owned())
    }

    /// `map_literal`.
    fn map_literal(&mut self) -> Option<Tag> {
        if !self.char('{') {
            return None;
        }
        let entries = self.repeated(|p| {
            let key = p.map_key()?;
            if !p.char(':') {
                return None;
            }
            let value = p.literal()?;
            if key.is_empty() {
                let here = p.reader.cursor();
                p.fail(here, snbt_error("empty_key"));
                return None;
            }
            Some((key, value))
        });
        if !self.char('}') {
            return None;
        }
        let mut fields: Vec<(String, Tag)> = Vec::with_capacity(entries.len());
        for (k, v) in entries {
            match fields.iter_mut().find(|(e, _)| *e == k) {
                Some(slot) => slot.1 = v,
                None => fields.push((k, v)),
            }
        }
        Some(Tag::Compound(fields))
    }

    /// `list_literal`: a list or `[B;...]`, `[I;...]`, `[L;...]`.
    fn list_literal(&mut self) -> Option<Tag> {
        if !self.char('[') {
            return None;
        }
        let at = self.reader.cursor();
        let prefix = self.chars(&['B', 'L', 'I']).filter(|_| self.char(';'));
        let entries = match prefix {
            Some(_) => Err(self.repeated(|p| p.integer_literal())),
            None => {
                self.reader.set_cursor(at);
                Ok(self.repeated(|p| p.literal()))
            }
        };
        if !self.char(']') {
            return None;
        }
        // The rule's action converts array elements once the brackets matched.
        let (entries, prefix) = match (entries, prefix) {
            (Ok(list), _) => return Some(Tag::List(list)),
            (Err(entries), Some(prefix)) => (entries, prefix),
            (Err(_), None) => unreachable!("arrays have a prefix"),
        };
        let (default, allowed): (Kind, &[Kind]) = match prefix {
            'B' => (Kind::Byte, &[]),
            'I' => (Kind::Int, &[Kind::Byte, Kind::Short]),
            _ => (Kind::Long, &[Kind::Byte, Kind::Short, Kind::Int]),
        };
        let mut values = Vec::with_capacity(entries.len());
        for lit in &entries {
            let kind = lit.kind.unwrap_or(default);
            if kind != default && !allowed.contains(&kind) {
                let here = self.reader.cursor();
                self.fail(here, snbt_error("invalid_array_element_type"));
                return None;
            }
            values.push(match self.integer_tag(lit, kind)? {
                Tag::Byte(v) => v as i64,
                Tag::Short(v) => v as i64,
                Tag::Int(v) => v as i64,
                Tag::Long(v) => v,
                _ => unreachable!("integer tags only"),
            });
        }
        Some(match prefix {
            'B' => Tag::ByteArray(values.into_iter().map(|v| v as i8).collect()),
            'I' => Tag::IntArray(values.into_iter().map(|v| v as i32).collect()),
            _ => Tag::LongArray(values),
        })
    }
}

/// `SnbtGrammar.canStartNumber`.
fn can_start_number(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '+' | '-' | '.')
}

/// `NumberFormatException` messages of `Byte.parseByte` and `Short.parseShort`.
fn out_of_range(text: &str, radix: u32) -> String {
    format!("Value out of range. Value:\"{text}\" Radix:{radix}")
}

/// `NumberFormatException.forInputString`.
fn for_input(text: &str, radix: u32) -> String {
    if radix == 10 {
        format!("For input string: \"{text}\"")
    } else {
        format!("For input string: \"{text}\" under radix {radix}")
    }
}

fn java_double(text: &str) -> Option<f64> {
    text.parse().ok()
}

fn java_float(text: &str) -> Option<f32> {
    text.parse().ok()
}

/// SNBT text of a tag as vanilla prints it (`SnbtPrinterTagVisitor` without indentation):
/// used for feedback such as block NBT in messages.
pub fn to_snbt(tag: &Tag) -> String {
    let mut out = String::new();
    write_snbt(tag, &mut out);
    out
}

fn write_snbt(tag: &Tag, out: &mut String) {
    use std::fmt::Write as _;
    match tag {
        Tag::Byte(v) => write!(out, "{v}b").unwrap(),
        Tag::Short(v) => write!(out, "{v}s").unwrap(),
        Tag::Int(v) => write!(out, "{v}").unwrap(),
        Tag::Long(v) => write!(out, "{v}L").unwrap(),
        Tag::Float(v) => write!(out, "{v:?}f").unwrap(),
        Tag::Double(v) => write!(out, "{v:?}d").unwrap(),
        Tag::String(s) => quote(s, out),
        Tag::ByteArray(v) => {
            out.push_str("[B;");
            let items: Vec<String> = v.iter().map(|b| format!("{b}B")).collect();
            out.push_str(&items.join(","));
            out.push(']');
        }
        Tag::IntArray(v) => {
            out.push_str("[I;");
            let items: Vec<String> = v.iter().map(i32::to_string).collect();
            out.push_str(&items.join(","));
            out.push(']');
        }
        Tag::LongArray(v) => {
            out.push_str("[L;");
            let items: Vec<String> = v.iter().map(|l| format!("{l}L")).collect();
            out.push_str(&items.join(","));
            out.push(']');
        }
        Tag::List(items) => {
            out.push('[');
            for (i, t) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_snbt(t, out);
            }
            out.push(']');
        }
        Tag::Compound(fields) => {
            out.push('{');
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !k.is_empty() && k.chars().all(is_allowed_in_unquoted_string) {
                    out.push_str(k);
                } else {
                    quote(k, out);
                }
                out.push(':');
                write_snbt(v, out);
            }
            out.push('}');
        }
    }
}

/// `StringTag.quoteAndEscape`: double quotes unless the string contains them.
fn quote(s: &str, out: &mut String) {
    let q = if s.contains('"') && !s.contains('\'') { '\'' } else { '"' };
    out.push(q);
    for c in s.chars() {
        if c == q || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(q);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> Tag {
        parse_tag(&mut StringReader::new(s)).unwrap()
    }

    fn err(s: &str) -> (String, usize, Vec<crate::text::Arg>) {
        let e = parse_tag(&mut StringReader::new(s)).unwrap_err();
        (e.key().unwrap().to_owned(), e.cursor().unwrap(), e.args().to_vec())
    }

    #[test]
    fn numbers() {
        assert_eq!(tag("1"), Tag::Int(1));
        assert_eq!(tag("-5b"), Tag::Byte(-5));
        assert_eq!(tag("12s"), Tag::Short(12));
        assert_eq!(tag("3L"), Tag::Long(3));
        assert_eq!(tag("1_000"), Tag::Int(1000));
        assert_eq!(tag("0xFF"), Tag::Int(255));
        assert_eq!(tag("0xFFub"), Tag::Byte(-1));
        assert_eq!(tag("0b101"), Tag::Int(5));
        assert_eq!(tag("0b"), Tag::Byte(0));
        assert_eq!(tag("0"), Tag::Int(0));
        assert_eq!(tag("255ub"), Tag::Byte(-1));
        assert_eq!(tag("1.5"), Tag::Double(1.5));
        assert_eq!(tag(".5f"), Tag::Float(0.5));
        assert_eq!(tag("1e3"), Tag::Double(1000.0));
        assert_eq!(tag("2f"), Tag::Float(2.0));
        assert_eq!(tag("1."), Tag::Double(1.0));
        assert_eq!(tag("true"), Tag::Byte(1));
        assert_eq!(tag("FALSE"), Tag::Byte(0));
        assert_eq!(err("128b").0, "snbt.parser.number_parse_failure");
        // The float rule is tried first and fails at the same position: its "." comes first.
        assert_eq!(err("01").2, [crate::text::Arg::Str(".".into())]);
        assert_eq!(err("0x").0, "snbt.parser.expected_hex_numeral");
        assert_eq!(err("-0xFub").0, "snbt.parser.expected_non_negative_number");
        assert_eq!(tag("_1"), Tag::String("_1".into()));
    }

    #[test]
    fn strings_and_words() {
        assert_eq!(tag("\"a\\\"b\\n\""), Tag::String("a\"b\n".into()));
        assert_eq!(tag("'it''"), Tag::String("it".into()));
        assert_eq!(tag("'say \"hi\"'"), Tag::String("say \"hi\"".into()));
        assert_eq!(tag("\"\\x41\\u00e9\""), Tag::String("Aé".into()));
        assert_eq!(tag("\"\""), Tag::String(String::new()));
        assert_eq!(tag("red"), Tag::String("red".into()));
        assert_eq!(tag("minecraft.stone"), Tag::String("minecraft.stone".into()));
        assert_eq!(err("\"a\\qb\"").0, "argument.literal.incorrect");
    }

    #[test]
    fn compounds_lists_arrays() {
        let t = tag("{a:1b, \"b c\":[I;1,2], d:{e:'x}'}, f:[{},[]], g:bool(2), h:[1,2,],}");
        let Tag::Compound(f) = t else { panic!() };
        assert_eq!(f[0], ("a".into(), Tag::Byte(1)));
        assert_eq!(f[1], ("b c".into(), Tag::IntArray(vec![1, 2])));
        assert_eq!(f[3].1, Tag::List(vec![Tag::Compound(vec![]), Tag::List(vec![])]));
        assert_eq!(f[4].1, Tag::Byte(1));
        assert_eq!(f[5].1, Tag::List(vec![Tag::Int(1), Tag::Int(2)]));
        assert_eq!(tag("[1, 'a', 2b]"), Tag::List(vec![Tag::Int(1), Tag::String("a".into()), Tag::Byte(2)]));
        assert_eq!(tag("[B;1b,2]"), Tag::ByteArray(vec![1, 2]));
        assert_eq!(tag("[L;1,2b]"), Tag::LongArray(vec![1, 2]));
        assert_eq!(err("[B;1L]").0, "snbt.parser.invalid_array_element_type");
        assert_eq!(tag("{ a : 1 , b : 2 }"), Tag::Compound(vec![("a".into(), Tag::Int(1)), ("b".into(), Tag::Int(2))]));
        assert_eq!(tag("{a:1,a:2}"), Tag::Compound(vec![("a".into(), Tag::Int(2))]));
        assert_eq!(
            tag("uuid(\"0-0-0-0-1\")"),
            Tag::IntArray(vec![0, 0, 0, 1]),
        );
    }

    #[test]
    fn errors_follow_the_longest_parse() {
        let (key, cursor, args) = err("{a:1");
        assert_eq!((key.as_str(), cursor), ("argument.literal.incorrect", 4));
        assert_eq!(args, [crate::text::Arg::Str(".".into())]);
        let (key, cursor, _) = err("{a:}");
        assert_eq!((key.as_str(), cursor), ("snbt.parser.expected_unquoted_string", 3));
        assert_eq!(err("{:1}").0, "argument.literal.incorrect");
        assert_eq!(err("{\"\":{}}").0, "snbt.parser.empty_key");
        let mut r = StringReader::new("1abc");
        assert_eq!((parse_tag(&mut r).unwrap(), r.cursor()), (Tag::Int(1), 1));
        assert_eq!(err("nope(1)").0, "snbt.parser.no_such_operation");
        assert_eq!(read_compound(&mut StringReader::new("[1]")).unwrap_err().key(), Some("argument.nbt.expected.compound"));
        let mut r = StringReader::new(r#"{a:1b}] rest"#);
        assert_eq!(read_compound(&mut r).unwrap(), "{a:1b}");
        assert_eq!(r.peek(), ']');
    }

    #[test]
    fn printing() {
        let t = tag("{a:1b,\"b c\":[I;1,2],s:'say \"x\"',l:[1L],d:1.5d}");
        assert_eq!(to_snbt(&t), "{a:1b,\"b c\":[I;1,2],s:'say \"x\"',l:[1L],d:1.5d}");
    }
}
