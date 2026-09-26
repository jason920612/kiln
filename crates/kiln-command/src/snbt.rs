//! Structural SNBT scanning for arguments that carry NBT or component values. Values are
//! validated for shape (balanced compounds, lists, quoted strings) and kept as source text
//! for the host to interpret.

use crate::error::CommandError;
use crate::reader::StringReader;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

fn expected_compound() -> CommandError {
    CommandError::new(tr!("argument.nbt.expected.compound"))
}

fn expected_key() -> CommandError {
    CommandError::new(tr!("argument.nbt.expected.key"))
}

fn expected_value() -> CommandError {
    CommandError::new(tr!("argument.nbt.expected.value"))
}

fn is_unquoted(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

/// Reads a `{...}` compound (`TagParser.parseCompoundAsArgument`) and returns its text.
pub fn read_compound<'a>(reader: &mut StringReader<'a>) -> Result<&'a str> {
    let start = reader.cursor();
    reader.skip_whitespace();
    if !reader.can_read() || reader.peek() != '{' {
        return Err(expected_compound().at(reader));
    }
    skip_value(reader, 0)?;
    Ok(&reader.string()[start..reader.cursor()])
}

/// Reads any SNBT value and returns its text.
pub fn read_value<'a>(reader: &mut StringReader<'a>) -> Result<&'a str> {
    let start = reader.cursor();
    skip_value(reader, 0)?;
    Ok(&reader.string()[start..reader.cursor()])
}

fn skip_value(reader: &mut StringReader, depth: usize) -> Result<()> {
    if depth > 512 {
        return Err(expected_value().at(reader));
    }
    reader.skip_whitespace();
    if !reader.can_read() {
        return Err(expected_value().at(reader));
    }
    match reader.peek() {
        '{' => {
            reader.skip();
            reader.skip_whitespace();
            while reader.can_read() && reader.peek() != '}' {
                let key_start = reader.cursor();
                let key = reader.read_string()?;
                if key.is_empty() && reader.cursor() == key_start {
                    return Err(expected_key().at(reader));
                }
                reader.skip_whitespace();
                reader.expect(':')?;
                skip_value(reader, depth + 1)?;
                reader.skip_whitespace();
                if reader.can_read() && reader.peek() == ',' {
                    reader.skip();
                    reader.skip_whitespace();
                } else {
                    break;
                }
            }
            reader.expect('}')
        }
        '[' => {
            reader.skip();
            if reader.can_read_n(2) && matches!(reader.peek(), 'B' | 'I' | 'L') && reader.peek_at(1) == ';' {
                reader.skip();
                reader.skip();
            }
            reader.skip_whitespace();
            while reader.can_read() && reader.peek() != ']' {
                skip_value(reader, depth + 1)?;
                reader.skip_whitespace();
                if reader.can_read() && reader.peek() == ',' {
                    reader.skip();
                    reader.skip_whitespace();
                } else {
                    break;
                }
            }
            reader.expect(']')
        }
        '"' | '\'' => reader.read_quoted_string().map(drop),
        _ => {
            let start = reader.cursor();
            while reader.can_read() && is_unquoted(reader.peek()) {
                reader.skip();
            }
            if reader.cursor() == start {
                return Err(expected_value().at(reader));
            }
            // Operations such as `bool(1)` or `uuid("...")`.
            if reader.can_read() && reader.peek() == '(' {
                reader.skip();
                reader.skip_whitespace();
                while reader.can_read() && reader.peek() != ')' {
                    skip_value(reader, depth + 1)?;
                    reader.skip_whitespace();
                    if reader.can_read() && reader.peek() == ',' {
                        reader.skip();
                    } else {
                        break;
                    }
                }
                reader.expect(')')?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compounds() {
        let mut r = StringReader::new(r#"{a:1b,"b c":[I;1,2],d:{e:'x}'},f:[{},[]],g:bool(1)}] rest"#);
        assert_eq!(read_compound(&mut r).unwrap(), r#"{a:1b,"b c":[I;1,2],d:{e:'x}'},f:[{},[]],g:bool(1)}"#);
        assert_eq!(r.peek(), ']');
        assert_eq!(
            read_compound(&mut StringReader::new("[1]")).unwrap_err().key(),
            Some("argument.nbt.expected.compound")
        );
        assert_eq!(
            read_compound(&mut StringReader::new("{a:}")).unwrap_err().key(),
            Some("argument.nbt.expected.value")
        );
        assert_eq!(read_compound(&mut StringReader::new("{a:1")).unwrap_err().key(), Some("parsing.expected"));
        assert_eq!(read_compound(&mut StringReader::new("{:1}")).unwrap_err().key(), Some("argument.nbt.expected.key"));
    }
}
