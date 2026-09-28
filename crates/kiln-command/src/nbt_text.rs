//! NBT as vanilla prints it: `Tag.toString` (`StringTagVisitor`, compact SNBT with sorted
//! keys) for error arguments and `NbtUtils.toPrettyComponent` (`TextComponentTagVisitor` with
//! no indentation, keys in `HashMap` order) for `/data get` feedback.

use crate::scoreboard::JavaHashSet;
use crate::text::Text;
use kiln_proto::nbt::Tag;

/// `Float.toString`.
pub fn java_float(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    java_decimal(v == 0.0, v.is_sign_negative(), &format!("{:e}", v.abs()), v.abs() as f64)
}

/// `Double.toString`.
pub fn java_double(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    java_decimal(v == 0.0, v.is_sign_negative(), &format!("{:e}", v.abs()), v.abs())
}

/// The shortest round-trip digits laid out as Java does: plain for magnitudes in
/// [1e-3, 1e7), else `d.dddE±n`, always with a fractional digit.
fn java_decimal(zero: bool, negative: bool, sci: &str, magnitude: f64) -> String {
    let sign = if negative { "-" } else { "" };
    if zero {
        return format!("{sign}0.0");
    }
    let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if (1e-3..1e7).contains(&magnitude) {
        let point = exp + 1;
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{exp}", &digits[..1], frac)
    }
}

/// `SnbtGrammar.escapeControlCharacters`.
fn control_escape(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{8}' => "b",
        '\t' => "t",
        '\n' => "n",
        '\u{c}' => "f",
        '\r' => "r",
        _ => return None,
    })
}

/// `StringTag.quoteAndEscape`: quoted with `"` unless the first quote in the string is one.
pub fn quote_and_escape(s: &str) -> String {
    let mut out = String::from(" ");
    let mut quote: Option<char> = None;
    for c in s.chars() {
        if c == '\\' {
            out.push_str("\\\\");
        } else if c == '"' || c == '\'' {
            let q = *quote.get_or_insert(if c == '"' { '\'' } else { '"' });
            if q == c {
                out.push('\\');
            }
            out.push(c);
        } else if let Some(e) = control_escape(c) {
            out.push('\\');
            out.push_str(e);
        } else {
            out.push(c);
        }
    }
    let q = quote.unwrap_or('"');
    out.replace_range(0..1, q.encode_utf8(&mut [0; 4]));
    out.push(q);
    out
}

fn unquoted_key(k: &str) -> bool {
    // `[A-Za-z._]+[A-Za-z0-9._+-]*`
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '.' || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// `Tag.toString` (`StringTagVisitor`).
pub fn snbt(tag: &Tag) -> String {
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
        Tag::Float(v) => write!(out, "{}f", java_float(*v)).unwrap(),
        Tag::Double(v) => write!(out, "{}d", java_double(*v)).unwrap(),
        Tag::String(s) => out.push_str(&quote_and_escape(s)),
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
            let mut sorted: Vec<&(String, Tag)> = fields.iter().collect();
            sorted.sort_by(|a, b| java_cmp(&a.0, &b.0));
            out.push('{');
            for (i, (k, v)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !k.eq_ignore_ascii_case("true") && !k.eq_ignore_ascii_case("false") && unquoted_key(k) {
                    out.push_str(k);
                } else {
                    out.push_str(&quote_and_escape(k));
                }
                out.push(':');
                write_snbt(v, out);
            }
            out.push('}');
        }
    }
}

/// `String.compareTo`: by UTF-16 code units.
pub fn java_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The keys of a compound in `HashMap` iteration order (insertion order within a bucket).
pub fn hash_order(fields: &[(String, Tag)]) -> Vec<&(String, Tag)> {
    let mut set = JavaHashSet::default();
    for (k, _) in fields {
        set.insert(k);
    }
    set.iter().filter_map(|k| fields.iter().find(|(f, _)| f == k)).collect()
}

/// Builds the pretty component as styled literal parts (`RichStyling`).
struct Pretty {
    parts: Vec<Text>,
    depth: usize,
}

impl Pretty {
    fn plain(&mut self, s: &str) {
        match self.parts.last_mut() {
            Some(last) if last.style == Default::default() && last.extra.is_empty() => {
                if let crate::text::Content::Literal(l) = &mut last.content {
                    l.push_str(s);
                    return;
                }
                self.parts.push(Text::literal(s));
            }
            _ => self.parts.push(Text::literal(s)),
        }
    }

    fn styled(&mut self, s: &str, color: &'static str) {
        self.parts.push(Text::literal(s).color(color));
    }

    fn number(&mut self, v: String, suffix: &str) {
        self.styled(&v, "gold");
        if !suffix.is_empty() {
            self.styled(suffix, "red");
        }
    }

    fn string(&mut self, s: &str) {
        let q = quote_and_escape(s);
        let (open, rest) = q.split_at(1);
        let (body, close) = rest.split_at(rest.len() - 1);
        self.plain(open);
        self.styled(body, "green");
        self.plain(close);
    }

    fn key(&mut self, k: &str) {
        // `SIMPLE_VALUE`: `[A-Za-z0-9._+-]+`.
        if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) {
            self.styled(k, "aqua");
        } else {
            let q = quote_and_escape(k);
            let (open, rest) = q.split_at(1);
            let (body, close) = rest.split_at(rest.len() - 1);
            self.plain(open);
            self.styled(body, "aqua");
            self.plain(close);
        }
    }

    fn array<T: Copy>(&mut self, prefix: &str, items: &[T], show: impl Fn(T) -> String, suffix: &str) {
        self.plain("[");
        self.styled(prefix, "red");
        self.plain(";");
        for (i, v) in items.iter().take(128).enumerate() {
            self.plain(" ");
            self.number(show(*v), suffix);
            if i != items.len() - 1 {
                self.plain(",");
            }
        }
        if items.len() > 128 {
            self.plain("<...>");
        }
        self.plain("]");
    }

    fn sub(&mut self, tag: &Tag) {
        self.depth += 1;
        self.visit(tag);
        self.depth -= 1;
    }

    fn visit(&mut self, tag: &Tag) {
        match tag {
            Tag::Byte(v) => self.number(v.to_string(), "b"),
            Tag::Short(v) => self.number(v.to_string(), "s"),
            Tag::Int(v) => self.number(v.to_string(), ""),
            Tag::Long(v) => self.number(v.to_string(), "L"),
            Tag::Float(v) => self.number(java_float(*v), "f"),
            Tag::Double(v) => self.number(java_double(*v), "d"),
            Tag::String(s) => self.string(s),
            Tag::ByteArray(v) => self.array("B", v, |b| b.to_string(), "b"),
            Tag::IntArray(v) => self.array("I", v, |b| b.to_string(), ""),
            Tag::LongArray(v) => self.array("L", v, |b| b.to_string(), "L"),
            Tag::List(items) => {
                if items.is_empty() {
                    self.plain("[]");
                } else if self.depth >= 64 {
                    self.plain("[<...>]");
                } else {
                    let numeric = |t: &Tag| {
                        matches!(t, Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_))
                    };
                    let wrap = items.len() < 8 && !items.iter().all(numeric);
                    self.plain("[");
                    let shown = if wrap { items.len().min(128) } else { items.len() };
                    for (i, t) in items.iter().take(shown).enumerate() {
                        if i > 0 {
                            self.plain(", ");
                        }
                        self.sub(t);
                    }
                    if wrap && items.len() > 128 {
                        self.plain("<...>");
                    }
                    self.plain("]");
                }
            }
            Tag::Compound(fields) => {
                if fields.is_empty() {
                    self.plain("{}");
                } else if self.depth >= 64 {
                    self.plain("{<...>}");
                } else {
                    self.plain("{");
                    for (i, (k, v)) in hash_order(fields).into_iter().enumerate() {
                        if i > 0 {
                            self.plain(", ");
                        }
                        self.key(k);
                        self.plain(": ");
                        self.sub(v);
                    }
                    self.plain("}");
                }
            }
        }
    }
}

/// `NbtUtils.toPrettyComponent`.
pub fn pretty(tag: &Tag) -> Text {
    let mut p = Pretty { parts: Vec::new(), depth: 0 };
    p.visit(tag);
    let mut out = Text::empty();
    out.extra = p.parts;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Tag {
        crate::snbt::parse_tag(&mut crate::reader::StringReader::new(s)).unwrap()
    }

    #[test]
    fn formats() {
        assert_eq!(java_float(1.0), "1.0");
        assert_eq!(java_float(0.1), "0.1");
        assert_eq!(java_double(1e7), "1.0E7");
        assert_eq!(java_double(-2.5e-4), "-2.5E-4");
        assert_eq!(quote_and_escape("a\"b"), "'a\"b'");
        assert_eq!(quote_and_escape("it's \"x\""), "\"it's \\\"x\\\"\"");
        assert_eq!(snbt(&parse("{b:1b,a:[1.5f,2d],\"x y\":\"s\",true:1}")), "{a:[1.5f,2.0d],b:1b,\"true\":1,\"x y\":\"s\"}");
        assert_eq!(pretty(&parse("{a:[B;1b,2b],b:[{}],c:\"x\"}")).to_plain(), "{a: [B; 1b, 2b], b: [{}], c: \"x\"}");
        assert_eq!(pretty(&parse("[1,2,3]")).to_plain(), "[1, 2, 3]");
    }
}
