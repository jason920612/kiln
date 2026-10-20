//! Decoding what a client sends the way the vanilla codecs do (`JsonOps` over DataFixerUpper's
//! `Codec`s), with their error texts: the text of a failure is the `data` of the "Invalid params" reply.

use super::json::J;
use uuid::Uuid;

pub type R<T> = Result<T, String>;

/// Errors of the fields of a record, joined the way `DataResult.apply` joins them.
#[derive(Default)]
pub struct Errs(Vec<String>);

impl Errs {
    pub fn take<T>(&mut self, r: R<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.0.push(e);
                None
            }
        }
    }

    pub fn finish(self) -> R<()> {
        // (`DataResult.apply` puts the error of the later field first.)
        if self.0.is_empty() {
            Ok(())
        } else {
            let mut errors = self.0;
            errors.reverse();
            Err(errors.join("; "))
        }
    }
}

pub fn boolean(j: &J) -> R<bool> {
    match j {
        J::Bool(b) => Ok(*b),
        _ => Err(format!("Not a boolean: {j}")),
    }
}

/// `Number.intValue()` of a `LazilyParsedNumber`: `parseInt`, else `(int) parseLong`, else `BigDecimal.intValue`.
pub fn int_value(lexeme: &str) -> i32 {
    if let Ok(i) = lexeme.parse::<i32>() {
        return i;
    }
    if let Ok(l) = lexeme.parse::<i64>() {
        return l as i32;
    }
    // BigDecimal.intValue: the integer part, its low 32 bits.
    let (neg, rest) = match lexeme.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, lexeme),
    };
    let (mantissa, exp) = match rest.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i64>().unwrap_or(0)),
        None => (rest, 0),
    };
    let (int_digits, frac_digits) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits: Vec<u8> = int_digits.bytes().chain(frac_digits.bytes()).map(|b| b - b'0').collect();
    // Value = digits * 10^(exp - frac_len); the integer part keeps the digits left of the point.
    let shift = exp - frac_digits.len() as i64;
    if shift < 0 {
        let cut = (-shift) as usize;
        let keep = digits.len().saturating_sub(cut);
        digits.truncate(keep);
    }
    let mut acc: u32 = 0;
    for d in digits {
        acc = acc.wrapping_mul(10).wrapping_add(u32::from(d));
    }
    if shift > 0 {
        for _ in 0..shift.min(100_000) {
            acc = acc.wrapping_mul(10);
        }
    }
    if neg { (acc as i32).wrapping_neg() } else { acc as i32 }
}

pub fn integer(j: &J) -> R<i32> {
    match j {
        J::Num(n) => Ok(int_value(n)),
        _ => Err(format!("Not a number: {j}")),
    }
}

pub fn string(j: &J) -> R<String> {
    match j {
        J::Str(s) => Ok(s.clone()),
        _ => Err(format!("Not a string: {j}")),
    }
}

pub fn array(j: &J) -> R<&[J]> {
    match j {
        J::Arr(a) => Ok(a),
        _ => Err(format!("Not a json array: {j}")),
    }
}

pub fn object(j: &J) -> R<&[(String, J)]> {
    match j {
        J::Obj(m) => Ok(m),
        _ => Err(format!("Not a JSON object: {j}")),
    }
}

/// A required field (`fieldOf`).
pub fn required<'a>(obj: &'a J, key: &str) -> R<&'a J> {
    obj.get(key).ok_or_else(|| format!("No key {key} in MapLike[{obj}]"))
}

/// A list: the elements that decode, or the errors of those that do not.
pub fn list<T>(j: &J, element: impl Fn(&J) -> R<T>) -> R<Vec<T>> {
    let items = array(j)?;
    let mut out = Vec::new();
    let mut errs = Errs::default();
    for item in items {
        if let Some(v) = errs.take(element(item)) {
            out.push(v);
        }
    }
    errs.finish()?;
    Ok(out)
}

/// `Long.parseLong(s, begin, end, 16)`: the message of the `NumberFormatException` on failure.
fn parse_hex_range(chars: &[char], begin: usize, end: usize) -> Result<i64, String> {
    let shown: String = chars[begin..end].iter().collect();
    let at = |i: usize| format!("Error at index {} in: \"{shown}\"", i - begin);
    if begin == end {
        return Err("For input string: \"\" under radix 16".into());
    }
    let (mut i, mut negative, mut limit) = (begin, false, -i64::MAX);
    if chars[i] < '0' {
        match chars[i] {
            '-' => {
                negative = true;
                limit = i64::MIN;
            }
            '+' => {}
            _ => return Err(at(i)),
        }
        i += 1;
        if i == end {
            return Err(at(i));
        }
    }
    let multmin = limit / 16;
    let mut result: i64 = 0;
    while i < end {
        let Some(digit) = chars[i].to_digit(16).map(i64::from) else { return Err(at(i)) };
        if result < multmin {
            return Err(at(i));
        }
        result *= 16;
        if result < limit + digit {
            return Err(at(i));
        }
        i += 1;
        result -= digit;
    }
    Ok(if negative { result } else { -result })
}

/// `UUIDUtil.STRING_CODEC`: `UUID.fromString` (JDK 25), with the text of its failures.
pub fn uuid_string(s: &str) -> Result<Uuid, String> {
    let fail = |m: String| format!("Invalid UUID {s}: {m}");
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len > 36 {
        return Err(fail("UUID string too large".into()));
    }
    let dash = |from: usize| chars.iter().skip(from).position(|c| *c == '-').map(|p| p + from);
    let d1 = dash(0);
    let d2 = d1.and_then(|d| dash(d + 1));
    let d3 = d2.and_then(|d| dash(d + 1));
    let d4 = d3.and_then(|d| dash(d + 1));
    let d5 = d4.and_then(|d| dash(d + 1));
    let (Some(d1), Some(d2), Some(d3), Some(d4), None) = (d1, d2, d3, d4, d5) else {
        return Err(fail(format!("Invalid UUID string: {s}")));
    };
    let part = |b: usize, e: usize| parse_hex_range(&chars, b, e).map_err(&fail);
    let mut msb = part(0, d1)? & 0xffff_ffff;
    msb <<= 16;
    msb |= part(d1 + 1, d2)? & 0xffff;
    msb <<= 16;
    msb |= part(d2 + 1, d3)? & 0xffff;
    let mut lsb = part(d3 + 1, d4)? & 0xffff;
    lsb <<= 48;
    lsb |= part(d4 + 1, len)? & 0xffff_ffff_ffff;
    Ok(Uuid::from_u64_pair(msb as u64, lsb as u64))
}

/// A decoded player (`PlayerDto`): either part may be missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerDto {
    pub id: Option<Uuid>,
    pub name: Option<String>,
}

pub fn player_dto(j: &J) -> R<PlayerDto> {
    object(j)?;
    let mut errs = Errs::default();
    let id = j.get("id").and_then(|v| errs.take(string(v).and_then(|s| uuid_string(&s))));
    let name = j.get("name").and_then(|v| errs.take(string(v)));
    errs.finish()?;
    Ok(PlayerDto { id, name })
}

/// A message (`Message`): literal text or a translation with optional arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub literal: Option<String>,
    pub translatable: Option<String>,
    pub params: Option<Vec<String>>,
}

pub fn message(j: &J) -> R<Message> {
    object(j)?;
    let mut errs = Errs::default();
    let literal = j.get("literal").and_then(|v| errs.take(string(v)));
    let translatable = j.get("translatable").and_then(|v| errs.take(string(v)));
    // (`lenientOptionalFieldOf`: arguments that do not decode are as good as none.)
    let params = j.get("translatableParams").and_then(|v| list(v, string).ok());
    errs.finish()?;
    Ok(Message { literal, translatable, params })
}

/// `Message.asComponent`: translation first, then the literal; `None` for a message with neither.
pub fn message_text(m: &Message) -> Option<kiln_link::mgmt::MgmtText> {
    use kiln_link::mgmt::MgmtText;
    if let Some(key) = &m.translatable {
        return Some(MgmtText::Translatable { key: key.clone(), args: m.params.clone().unwrap_or_default() });
    }
    m.literal.clone().map(MgmtText::Literal)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `Instant.toString`: `2030-01-01T00:00:00Z`, with the fraction in groups of three digits when there is one.
pub fn instant_string(secs: i64, nanos: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let mut out = if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, rem / 60 % 60, rem % 60)
    } else {
        format!("{}{:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", if y > 9999 { "+" } else { "-" }, y.abs(), rem / 3600, rem / 60 % 60, rem % 60)
    };
    if nanos != 0 {
        if nanos % 1_000_000 == 0 {
            out.push_str(&format!(".{:03}", nanos / 1_000_000));
        } else if nanos % 1000 == 0 {
            out.push_str(&format!(".{:06}", nanos / 1000));
        } else {
            out.push_str(&format!(".{nanos:09}"));
        }
    }
    out.push('Z');
    out
}

/// `ExtraCodecs.INSTANT_ISO8601`: `DateTimeFormatter.ISO_INSTANT.parse` -> seconds and nanos since the epoch.
pub fn instant(j: &J) -> R<(i64, u32)> {
    let s = string(j)?;
    parse_instant(&s)
}

pub fn parse_instant(text: &str) -> R<(i64, u32)> {
    let b: Vec<char> = text.chars().collect();
    let at = |i: usize| format!("Text '{text}' could not be parsed at index {i}");
    let mut i = 0;
    // uuuu: four to ten digits (a sign for more than four).
    let mut sign = 1i64;
    if matches!(b.first(), Some('+' | '-')) {
        if b[0] == '-' {
            sign = -1;
        }
        i = 1;
    }
    let digits = |i: &mut usize, min: usize, max: usize| -> Option<i64> {
        let start = *i;
        let mut v = 0i64;
        while *i < b.len() && *i - start < max && b[*i].is_ascii_digit() {
            v = v * 10 + i64::from(b[*i] as u8 - b'0');
            *i += 1;
        }
        if *i - start < min { None } else { Some(v) }
    };
    let year = digits(&mut i, 4, 10).ok_or_else(|| at(i))? * sign;
    let expect = |i: &mut usize, c: char| -> Result<(), String> {
        if b.get(*i) == Some(&c) {
            *i += 1;
            Ok(())
        } else {
            Err(at(*i))
        }
    };
    expect(&mut i, '-')?;
    let month = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
    expect(&mut i, '-')?;
    let day = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
    expect(&mut i, 'T')?;
    let hour = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
    expect(&mut i, ':')?;
    let minute = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
    expect(&mut i, ':')?;
    let second = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
    let mut nanos = 0u32;
    if b.get(i) == Some(&'.') {
        i += 1;
        let start = i;
        let mut v = 0u64;
        while i < b.len() && i - start < 9 && b[i].is_ascii_digit() {
            v = v * 10 + u64::from(b[i] as u8 - b'0');
            i += 1;
        }
        if i == start {
            return Err(at(i));
        }
        nanos = (v * 10u64.pow(9 - (i - start) as u32)) as u32;
    }
    // The offset: Z, or +hh:mm with optional :ss.
    let mut offset = 0i64;
    match b.get(i) {
        Some('Z') => i += 1,
        Some(&c @ ('+' | '-')) => {
            i += 1;
            let h = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
            expect(&mut i, ':')?;
            let m = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
            let mut s = 0;
            if b.get(i) == Some(&':') {
                i += 1;
                s = digits(&mut i, 2, 2).ok_or_else(|| at(i))?;
            }
            offset = (h * 3600 + m * 60 + s) * if c == '-' { -1 } else { 1 };
        }
        _ => return Err(at(i)),
    }
    if i != b.len() {
        return Err(format!("Text '{text}' could not be parsed, unparsed text found at index {i}"));
    }
    // (A field out of range or a day that does not exist: `ISO_INSTANT` fails at the start.)
    let range = |_name: &str, v: i64, lo: i64, hi: i64| -> R<()> { if (lo..=hi).contains(&v) { Ok(()) } else { Err(at(0)) } };
    range("MonthOfYear", month, 1, 12)?;
    range("DayOfMonth", day, 1, 31)?;
    range("HourOfDay", hour, 0, 23)?;
    range("MinuteOfHour", minute, 0, 59)?;
    range("SecondOfMinute", second, 0, 59)?;
    let days_in_month = |y: i64, m: i64| match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day > days_in_month(year, month) {
        return Err(at(0));
    }
    let secs = days_from_civil(year, month as u32, day as u32) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Ok((secs, nanos))
}
