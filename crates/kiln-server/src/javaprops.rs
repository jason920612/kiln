//! `java.util.Properties` as vanilla's `Settings` and `Eula` use it: the reader (comments, `=`
//! `:` and blank separators, escapes, continuation lines) and the writer (`store`: a comment
//! line, the date line, then the entries sorted by key, as the JDK 18+ writer does).

use std::collections::BTreeMap;
use std::time::SystemTime;

/// The entries of a properties file. Sorted by key, which is the order `store` writes them in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JavaProps {
    pub map: BTreeMap<String, String>,
}

impl JavaProps {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.map.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.map.remove(key)
    }

    /// `Properties.load(Reader)`.
    pub fn parse(text: &str) -> Self {
        let mut map = BTreeMap::new();
        let mut lines = text.split(['\n', '\r']).filter(|_| true).peekable();
        // `\r\n` splits into an empty line between the two, which is a blank line: harmless.
        while let Some(first) = lines.next() {
            let line = first.trim_start_matches([' ', '\t', '\u{c}']);
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            // Logical line: an odd number of trailing backslashes continues on the next line,
            // whose leading white space is dropped.
            let mut logical = line.to_owned();
            while ends_with_odd_backslashes(&logical) {
                logical.pop();
                match lines.next() {
                    Some(next) => logical.push_str(next.trim_start_matches([' ', '\t', '\u{c}'])),
                    None => break,
                }
            }
            let (key, value) = split_key_value(&logical);
            map.insert(unescape(key), unescape(value));
        }
        Self { map }
    }

    /// `Properties.store(Writer, comment)` at `now`: `#<comment>`, `#<date>`, the entries.
    pub fn store(&self, comment: &str, now: SystemTime) -> String {
        let nl = if cfg!(windows) { "\r\n" } else { "\n" };
        let mut out = String::new();
        out.push('#');
        out.push_str(comment);
        out.push_str(nl);
        out.push('#');
        out.push_str(&java_date(now));
        out.push_str(nl);
        for (k, v) in &self.map {
            out.push_str(&escape(k, true));
            out.push('=');
            out.push_str(&escape(v, false));
            out.push_str(nl);
        }
        out
    }
}

fn ends_with_odd_backslashes(s: &str) -> bool {
    s.bytes().rev().take_while(|b| *b == b'\\').count() % 2 == 1
}

/// The key ends at the first unescaped `=`, `:` or white space; then white space, one optional
/// `=` or `:`, and more white space come before the value.
fn split_key_value(line: &str) -> (&str, &str) {
    let b = line.as_bytes();
    let mut i = 0;
    let mut escaped = false;
    while i < b.len() {
        let c = b[i];
        if escaped {
            escaped = false;
        } else if c == b'\\' {
            escaped = true;
        } else if matches!(c, b'=' | b':' | b' ' | b'\t' | 0x0c) {
            break;
        }
        i += 1;
    }
    let key = &line[..i];
    let mut rest = line[i..].trim_start_matches([' ', '\t', '\u{c}']);
    if let Some(r) = rest.strip_prefix(['=', ':']) {
        rest = r.trim_start_matches([' ', '\t', '\u{c}']);
    }
    (key, rest)
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => out.push('\u{fffd}'),
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// `Properties.saveConvert` with `escUnicode` off (a Writer): a space is escaped in a key or at
/// the start of a value; `\`, the control characters and `= : # !` get a backslash.
fn escape(s: &str, key: bool) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        match c {
            ' ' => {
                if i == 0 || key {
                    out.push('\\');
                }
                out.push(' ');
            }
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '=' | ':' | '#' | '!' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// `new Date().toString()` in UTC: `Mon Oct 19 20:43:39 UTC 2026`.
pub fn java_date(now: SystemTime) -> String {
    let secs = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} UTC {}",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(m - 1) as usize],
        d,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        y
    )
}

/// Days since 1970-01-01 to (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reads_what_java_reads() {
        let p = JavaProps::parse(
            "# comment\n! another\n\n  spaced = value with spaces \nkey:colon\nplain value\nescaped\\ key=a\\=b\\\\c\nlong=one\\\n    two\nuni=\\u00e9\\n\nempty=\n",
        );
        assert_eq!(p.get("spaced"), Some("value with spaces "));
        assert_eq!(p.get("key"), Some("colon"));
        assert_eq!(p.get("plain"), Some("value"));
        assert_eq!(p.get("escaped key"), Some("a=b\\c"));
        assert_eq!(p.get("long"), Some("onetwo"));
        assert_eq!(p.get("uni"), Some("\u{e9}\n"));
        assert_eq!(p.get("empty"), Some(""));
        assert_eq!(JavaProps::parse("a=1\r\nb=2\r\n").map.len(), 2);
    }

    #[test]
    fn writes_sorted_with_escapes() {
        let mut p = JavaProps::default();
        p.set("level-type", "minecraft:normal");
        p.set("a b", " lead=trail ");
        p.set("motd", "A Minecraft Server");
        // 2026-10-19 20:43:39 UTC was a Monday (the date line vanilla wrote on the VM).
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_442_619);
        let nl = if cfg!(windows) { "\r\n" } else { "\n" };
        let want = ["#Minecraft server properties", "#Mon Oct 19 20:43:39 UTC 2026", "a\\ b=\\ lead\\=trail ", "level-type=minecraft\\:normal", "motd=A Minecraft Server", ""]
            .join(nl);
        assert_eq!(p.store("Minecraft server properties", t), want);
        assert_eq!(JavaProps::parse(&want), p);
    }
}
