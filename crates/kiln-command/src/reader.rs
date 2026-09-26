//! Brigadier's `StringReader`. Cursors are byte offsets into the command text.

use crate::error::CommandError;

#[derive(Debug, Clone)]
pub struct StringReader<'a> {
    s: &'a str,
    cursor: usize,
}

type Result<T> = std::result::Result<T, CommandError>;

impl<'a> StringReader<'a> {
    pub fn new(s: &'a str) -> Self {
        StringReader { s, cursor: 0 }
    }

    pub fn string(&self) -> &'a str {
        self.s
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        debug_assert!(self.s.is_char_boundary(cursor));
        self.cursor = cursor;
    }

    pub fn total_len(&self) -> usize {
        self.s.len()
    }

    pub fn remaining(&self) -> &'a str {
        &self.s[self.cursor..]
    }

    pub fn read_so_far(&self) -> &'a str {
        &self.s[..self.cursor]
    }

    pub fn can_read(&self) -> bool {
        self.cursor < self.s.len()
    }

    pub fn can_read_n(&self, n: usize) -> bool {
        self.cursor + n <= self.s.len()
    }

    /// The next character; `'\0'` at the end (callers check [`can_read`](Self::can_read)).
    pub fn peek(&self) -> char {
        self.remaining().chars().next().unwrap_or('\0')
    }

    /// The character `n` characters ahead.
    pub fn peek_at(&self, n: usize) -> char {
        self.remaining().chars().nth(n).unwrap_or('\0')
    }

    pub fn read(&mut self) -> char {
        let c = self.peek();
        self.cursor += c.len_utf8();
        c
    }

    pub fn skip(&mut self) {
        self.read();
    }

    pub fn skip_whitespace(&mut self) {
        while self.can_read() && is_java_whitespace(self.peek()) {
            self.skip();
        }
    }

    fn take_while(&mut self, f: impl Fn(char) -> bool) -> &'a str {
        let start = self.cursor;
        while self.can_read() && f(self.peek()) {
            self.skip();
        }
        &self.s[start..self.cursor]
    }

    fn read_number<T: std::str::FromStr>(
        &mut self,
        expected: fn() -> CommandError,
        invalid: fn(&str) -> CommandError,
    ) -> Result<T> {
        let start = self.cursor;
        let number = self.take_while(is_allowed_number);
        if number.is_empty() {
            return Err(expected().at(self));
        }
        number.parse().map_err(|_| {
            self.cursor = start;
            invalid(number).at(self)
        })
    }

    pub fn read_int(&mut self) -> Result<i32> {
        self.read_number(CommandError::expected_int, CommandError::invalid_int)
    }

    pub fn read_long(&mut self) -> Result<i64> {
        self.read_number(CommandError::expected_long, CommandError::invalid_long)
    }

    pub fn read_double(&mut self) -> Result<f64> {
        self.read_number(CommandError::expected_double, CommandError::invalid_double)
    }

    pub fn read_float(&mut self) -> Result<f32> {
        self.read_number(CommandError::expected_float, CommandError::invalid_float)
    }

    pub fn read_unquoted_string(&mut self) -> &'a str {
        self.take_while(is_allowed_in_unquoted_string)
    }

    pub fn read_quoted_string(&mut self) -> Result<String> {
        if !self.can_read() {
            return Ok(String::new());
        }
        let quote = self.peek();
        if !is_quoted_string_start(quote) {
            return Err(CommandError::expected_start_of_quote().at(self));
        }
        self.skip();
        self.read_string_until(quote)
    }

    pub fn read_string_until(&mut self, terminator: char) -> Result<String> {
        let mut out = String::new();
        let mut escaped = false;
        while self.can_read() {
            let c = self.read();
            if escaped {
                if c == terminator || c == '\\' {
                    out.push(c);
                    escaped = false;
                } else {
                    self.cursor -= c.len_utf8();
                    return Err(CommandError::invalid_escape(c).at(self));
                }
            } else if c == '\\' {
                escaped = true;
            } else if c == terminator {
                return Ok(out);
            } else {
                out.push(c);
            }
        }
        Err(CommandError::expected_end_of_quote().at(self))
    }

    /// A quoted string if the next character is a quote, otherwise an unquoted one.
    pub fn read_string(&mut self) -> Result<String> {
        if !self.can_read() {
            return Ok(String::new());
        }
        let next = self.peek();
        if is_quoted_string_start(next) {
            self.skip();
            return self.read_string_until(next);
        }
        Ok(self.read_unquoted_string().to_owned())
    }

    pub fn read_boolean(&mut self) -> Result<bool> {
        let start = self.cursor;
        let value = self.read_string()?;
        match value.as_str() {
            "" => Err(CommandError::expected_bool().at(self)),
            "true" => Ok(true),
            "false" => Ok(false),
            _ => {
                self.cursor = start;
                Err(CommandError::invalid_bool(&value).at(self))
            }
        }
    }

    pub fn expect(&mut self, c: char) -> Result<()> {
        if !self.can_read() || self.peek() != c {
            return Err(CommandError::expected_symbol(c).at(self));
        }
        self.skip();
        Ok(())
    }
}

pub fn is_allowed_number(c: char) -> bool {
    c.is_ascii_digit() || c == '.' || c == '-'
}

pub fn is_quoted_string_start(c: char) -> bool {
    c == '"' || c == '\''
}

pub fn is_allowed_in_unquoted_string(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

/// `Character.isWhitespace`: Unicode spaces except no-break spaces, plus the ASCII separators.
pub fn is_java_whitespace(c: char) -> bool {
    matches!(c, '\u{1c}'..='\u{1f}')
        || (c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}' | '\u{85}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(e: CommandError) -> String {
        e.key().unwrap().to_owned()
    }

    #[test]
    fn numbers() {
        let mut r = StringReader::new("12 -3.5 x 1.5 1-2");
        assert_eq!(r.read_int().unwrap(), 12);
        r.skip();
        assert_eq!(r.read_double().unwrap(), -3.5);
        r.skip();
        let e = r.read_int().unwrap_err();
        assert_eq!((key(e.clone()), e.cursor()), ("parsing.int.expected".into(), Some(8)));
        r.set_cursor(10);
        let e = r.read_int().unwrap_err();
        assert_eq!((key(e.clone()), e.cursor()), ("parsing.int.invalid".into(), Some(10)));
        assert_eq!(e.args(), &[crate::text::Arg::Str("1.5".into())]);
        r.set_cursor(14);
        assert_eq!(key(r.read_float().unwrap_err()), "parsing.float.invalid");
        assert_eq!(StringReader::new("1.").read_double().unwrap(), 1.0);
        assert_eq!(StringReader::new(".5").read_float().unwrap(), 0.5);
        assert_eq!(key(StringReader::new("9999999999").read_int().unwrap_err()), "parsing.int.invalid");
        assert_eq!(StringReader::new("9999999999").read_long().unwrap(), 9_999_999_999);
    }

    #[test]
    fn strings() {
        let mut r = StringReader::new(r#"hello_world+1.x "quoted \"string\" \\" 'single "inner"' rest"#);
        assert_eq!(r.read_unquoted_string(), "hello_world+1.x");
        r.skip();
        assert_eq!(r.read_string().unwrap(), r#"quoted "string" \"#);
        r.skip();
        assert_eq!(r.read_string().unwrap(), r#"single "inner""#);
        r.skip();
        assert_eq!(r.read_string().unwrap(), "rest");
        assert!(!r.can_read());
        assert_eq!(r.read_string().unwrap(), "");
    }

    #[test]
    fn string_errors() {
        let e = StringReader::new(r#""abc"#).read_string().unwrap_err();
        assert_eq!((key(e.clone()), e.cursor()), ("parsing.quote.expected.end".into(), Some(4)));
        let e = StringReader::new(r#""a\nb""#).read_string().unwrap_err();
        assert_eq!((key(e.clone()), e.cursor()), ("parsing.quote.escape".into(), Some(3)));
        assert_eq!(e.args(), &[crate::text::Arg::Str("n".into())]);
        let e = StringReader::new("abc").read_quoted_string().unwrap_err();
        assert_eq!(key(e), "parsing.quote.expected.start");
    }

    #[test]
    fn booleans_and_expect() {
        assert!(StringReader::new("true").read_boolean().unwrap());
        assert!(!StringReader::new("\"false\"").read_boolean().unwrap());
        let e = StringReader::new("yes").read_boolean().unwrap_err();
        assert_eq!((key(e.clone()), e.cursor()), ("parsing.bool.invalid".into(), Some(0)));
        assert_eq!(key(StringReader::new(" ").read_boolean().unwrap_err()), "parsing.bool.expected");
        let e = StringReader::new("a").expect('[').unwrap_err();
        assert_eq!(e.args(), &[crate::text::Arg::Str("[".into())]);
    }

    #[test]
    fn unicode_cursor() {
        let mut r = StringReader::new("é x");
        assert_eq!(r.read(), 'é');
        assert_eq!(r.cursor(), 2);
        r.skip_whitespace();
        assert_eq!(r.peek(), 'x');
    }
}
