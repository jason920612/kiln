//! Plain-text rendering of chat components (JSON or network NBT), for disconnect reasons.

use kiln_proto::{DecodeError, Reader};
use serde_json::{Map, Value};

const MAX_DEPTH: usize = 32;

/// Renders a JSON text component; malformed JSON is returned as is.
pub fn from_json(json: &str) -> String {
    match serde_json::from_str::<Value>(json) {
        Ok(v) => render(&v),
        Err(_) => json.to_owned(),
    }
}

/// Reads a network NBT text component (nameless root) and renders it.
pub fn from_nbt(r: &mut Reader) -> Result<String, DecodeError> {
    let tag = r.u8()?;
    Ok(render(&nbt(r, tag, 0)?))
}

/// Concatenates `text` (or `translate` keys with their arguments) of a component tree.
pub fn render(v: &Value) -> String {
    let mut out = String::new();
    render_into(v, &mut out, 0);
    out
}

fn render_into(v: &Value, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::String(s) => out.push_str(s),
        Value::Array(items) => items.iter().for_each(|v| render_into(v, out, depth + 1)),
        Value::Object(m) => {
            if let Some(Value::String(t)) = m.get("text") {
                out.push_str(t);
            } else if let Some(Value::String(key)) = m.get("translate") {
                out.push_str(key);
                if let Some(Value::Array(args)) = m.get("with") {
                    out.push('(');
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        render_into(a, out, depth + 1);
                    }
                    out.push(')');
                }
            }
            if let Some(Value::Array(extra)) = m.get("extra") {
                extra.iter().for_each(|v| render_into(v, out, depth + 1));
            }
        }
        Value::Null => {}
        other => out.push_str(&other.to_string()),
    }
}

/// Converts an NBT payload into a JSON value; arrays of numbers are dropped.
fn nbt(r: &mut Reader, tag: u8, depth: usize) -> Result<Value, DecodeError> {
    if depth > MAX_DEPTH {
        return Err(DecodeError::Invalid("NBT nested too deeply"));
    }
    let skip = |r: &mut Reader, width: usize| -> Result<Value, DecodeError> {
        let n = r.i32()?;
        let n = usize::try_from(n).map_err(|_| DecodeError::NegativeLength(n))?;
        r.bytes(n.checked_mul(width).ok_or(DecodeError::Eof)?)?;
        Ok(Value::Null)
    };
    Ok(match tag {
        1 => r.i8()?.into(),
        2 => r.i16()?.into(),
        3 => r.i32()?.into(),
        4 => r.i64()?.into(),
        5 => r.f32()?.into(),
        6 => r.f64()?.into(),
        7 => skip(r, 1)?,
        8 => Value::String(mutf8(r)?),
        9 => {
            let elem = r.u8()?;
            let n = r.i32()?;
            let n = usize::try_from(n).map_err(|_| DecodeError::NegativeLength(n))?;
            let mut items = Vec::new();
            // An empty list may declare element type End.
            if elem != 0 {
                for _ in 0..n {
                    items.push(nbt(r, elem, depth + 1)?);
                }
            }
            Value::Array(items)
        }
        10 => {
            let mut m = Map::new();
            loop {
                let t = r.u8()?;
                if t == 0 {
                    break;
                }
                let name = mutf8(r)?;
                m.insert(name, nbt(r, t, depth + 1)?);
            }
            Value::Object(m)
        }
        11 => skip(r, 4)?,
        12 => skip(r, 8)?,
        _ => return Err(DecodeError::Invalid("NBT tag type")),
    })
}

/// Java modified UTF-8, decoded leniently (it only differs from UTF-8 for NUL and astral chars).
fn mutf8(r: &mut Reader) -> Result<String, DecodeError> {
    let len = r.u16()? as usize;
    Ok(String::from_utf8_lossy(r.bytes(len)?).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use kiln_proto::nbt::Tag;

    #[test]
    fn renders_json_components() {
        assert_eq!(from_json(r#""plain""#), "plain");
        assert_eq!(from_json(r#"{"text":"a","extra":["b",{"text":"c"}]}"#), "abc");
        assert_eq!(
            from_json(r#"{"translate":"multiplayer.disconnect.kicked","with":[{"text":"x"}]}"#),
            "multiplayer.disconnect.kicked(x)"
        );
        assert_eq!(from_json("not json"), "not json");
    }

    #[test]
    fn renders_nbt_components() {
        let tag = Tag::Compound(vec![
            ("text".into(), Tag::String("Kicked: ".into())),
            ("color".into(), Tag::String("red".into())),
            ("bold".into(), Tag::Byte(1)),
            ("ids".into(), Tag::IntArray(vec![1, 2])),
            ("extra".into(), Tag::List(vec![Tag::String("too fast".into())])),
        ]);
        let mut b = BytesMut::new();
        tag.write_network(&mut b);
        let mut r = Reader::new(&b);
        assert_eq!(from_nbt(&mut r).unwrap(), "Kicked: too fast");
        r.finish().unwrap();
    }

    #[test]
    fn rejects_truncated_nbt() {
        let mut b = BytesMut::new();
        Tag::Compound(vec![("text".into(), Tag::String("abc".into()))]).write_network(&mut b);
        let mut r = Reader::new(&b[..b.len() - 2]);
        assert_eq!(from_nbt(&mut r), Err(DecodeError::Eof));
    }
}
