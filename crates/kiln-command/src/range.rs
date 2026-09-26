//! `MinMaxBounds`: `5`, `..5`, `1..`, `1..5` as used by selector options.

use crate::error::CommandError;
use crate::reader::StringReader;

/// Inclusive bounds; `None` is unbounded on that side.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bounds<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

pub type IntRange = Bounds<i32>;
pub type DoubleRange = Bounds<f64>;
/// `MinMaxBounds.FloatDegrees`, used by `x_rotation` and `y_rotation`.
pub type FloatRange = Bounds<f32>;

impl<T: Copy + PartialOrd + std::str::FromStr> Bounds<T> {
    pub fn exactly(v: T) -> Self {
        Bounds { min: Some(v), max: Some(v) }
    }

    pub fn is_any(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }

    pub fn matches(&self, v: T) -> bool {
        self.min.is_none_or(|m| m <= v) && self.max.is_none_or(|m| v <= m)
    }

    fn read(reader: &mut StringReader, invalid: fn(&str) -> CommandError, ordered: bool) -> Result<Self, CommandError> {
        if !reader.can_read() {
            return Err(CommandError::range_empty().at(reader));
        }
        let start = reader.cursor();
        let result = (|| {
            let min = read_number(reader, invalid)?;
            let max = if reader.can_read_n(2) && reader.peek() == '.' && reader.peek_at(1) == '.' {
                reader.skip();
                reader.skip();
                read_number(reader, invalid)?
            } else {
                min
            };
            if min.is_none() && max.is_none() {
                return Err(CommandError::range_empty().at(reader));
            }
            Ok(Bounds { min, max })
        })();
        let bounds = result.map_err(|e| {
            // Vanilla rethrows with the cursor at the start of the range.
            let input = e.input().unwrap_or(reader.string()).to_owned();
            e.with_context(&input, start)
        })?;
        if let (true, Some(a), Some(b)) = (ordered, bounds.min, bounds.max)
            && a > b
        {
            reader.set_cursor(start);
            return Err(CommandError::range_swapped().at(reader));
        }
        Ok(bounds)
    }
}

impl IntRange {
    pub fn parse(reader: &mut StringReader) -> Result<Self, CommandError> {
        Self::read(reader, CommandError::invalid_int, true)
    }
}

impl DoubleRange {
    pub fn parse(reader: &mut StringReader) -> Result<Self, CommandError> {
        Self::read(reader, CommandError::invalid_double, true)
    }

    /// Compares squared distances like `MinMaxBounds.Doubles.matchesSqr`.
    pub fn matches_sqr(&self, dist_sqr: f64) -> bool {
        self.min.is_none_or(|m| m * m <= dist_sqr) && self.max.is_none_or(|m| dist_sqr <= m * m)
    }
}

impl FloatRange {
    /// Degrees wrap around, so `min > max` is allowed (`MinMaxBounds.FloatDegrees`).
    pub fn parse(reader: &mut StringReader) -> Result<Self, CommandError> {
        Self::read(reader, CommandError::invalid_float, false)
    }
}

fn is_range_char(reader: &StringReader) -> bool {
    let c = reader.peek();
    c.is_ascii_digit() || c == '-' || (c == '.' && !(reader.can_read_n(2) && reader.peek_at(1) == '.'))
}

fn read_number<T: std::str::FromStr>(
    reader: &mut StringReader,
    invalid: fn(&str) -> CommandError,
) -> Result<Option<T>, CommandError> {
    let start = reader.cursor();
    while reader.can_read() && is_range_char(reader) {
        reader.skip();
    }
    let s = &reader.string()[start..reader.cursor()];
    if s.is_empty() {
        return Ok(None);
    }
    s.parse().map(Some).map_err(|_| invalid(s).at(reader))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(s: &str) -> Result<IntRange, CommandError> {
        IntRange::parse(&mut StringReader::new(s))
    }

    #[test]
    fn forms() {
        assert_eq!(int("5").unwrap(), IntRange::exactly(5));
        assert_eq!(int("..5").unwrap(), Bounds { min: None, max: Some(5) });
        assert_eq!(int("-3..").unwrap(), Bounds { min: Some(-3), max: None });
        assert_eq!(int("1..5]").unwrap(), Bounds { min: Some(1), max: Some(5) });
        let d = DoubleRange::parse(&mut StringReader::new("0.5..2.")).unwrap();
        assert_eq!(d, Bounds { min: Some(0.5), max: Some(2.0) });
        assert!(d.matches_sqr(1.0) && !d.matches_sqr(0.2) && !d.matches_sqr(4.1));
    }

    #[test]
    fn errors() {
        assert_eq!(int("..").unwrap_err().key(), Some("argument.range.empty"));
        assert_eq!(int("").unwrap_err().key(), Some("argument.range.empty"));
        let e = int("5..1").unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("argument.range.swapped"), Some(0)));
        let e = int("1.5").unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("parsing.int.invalid"), Some(0)));
    }
}
