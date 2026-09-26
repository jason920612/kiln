//! Network codec building blocks (`ByteBufCodecs`) on top of kiln-proto's reader and writer.

use crate::value::Value;
use bytes::BytesMut;
use kiln_proto::nbt::{self, Tag};
use kiln_proto::{DecodeError, Reader, WriteExt};
use uuid::Uuid;

pub type WireResult<T> = Result<T, DecodeError>;

/// Largest NBT value read from untrusted input (`NbtAccounter.defaultQuota()`, 2 MiB).
pub const MAX_NBT_BYTES: usize = 2 * 1024 * 1024;

/// `ByteBufCodecs.readCount`: a VarInt count, at most `max`.
pub fn read_count(r: &mut Reader<'_>, max: usize) -> WireResult<usize> {
    let n = r.varint()?;
    let n = usize::try_from(n).map_err(|_| DecodeError::NegativeLength(n))?;
    if n > max {
        return Err(DecodeError::Invalid("collection too large"));
    }
    Ok(n)
}

/// `ByteBufCodecs.collection` / `list`: a count, then the elements.
pub fn read_list<T>(r: &mut Reader<'_>, max: usize, mut f: impl FnMut(&mut Reader<'_>) -> WireResult<T>) -> WireResult<Vec<T>> {
    let n = read_count(r, max)?;
    // Every element takes at least one byte; don't preallocate beyond the input.
    let mut out = Vec::with_capacity(n.min(r.remaining()));
    for _ in 0..n {
        out.push(f(r)?);
    }
    Ok(out)
}

pub fn write_list<T>(out: &mut BytesMut, items: &[T], mut f: impl FnMut(&T, &mut BytesMut)) {
    out.put_varint(items.len() as i32);
    for item in items {
        f(item, out);
    }
}

/// `ByteBufCodecs.optional`: a boolean, then the value if present.
pub fn read_opt<T>(r: &mut Reader<'_>, f: impl FnOnce(&mut Reader<'_>) -> WireResult<T>) -> WireResult<Option<T>> {
    if r.bool()? { f(r).map(Some) } else { Ok(None) }
}

pub fn write_opt<T>(out: &mut BytesMut, value: &Option<T>, f: impl FnOnce(&T, &mut BytesMut)) {
    out.put_bool(value.is_some());
    if let Some(v) = value {
        f(v, out);
    }
}

/// `ByteBufCodecs.stringUtf8(max)` / `STRING_UTF8` (max 32767).
pub fn read_string(r: &mut Reader<'_>, max: usize) -> WireResult<String> {
    Ok(r.string(max)?.to_owned())
}

/// `ByteBufCodecs.TAG` / `COMPOUND_TAG`: a network NBT root.
pub fn read_nbt(r: &mut Reader<'_>) -> WireResult<Tag> {
    let rest = r.rest();
    let (tag, used) = nbt::read_network(rest).map_err(|_| DecodeError::Invalid("NBT"))?;
    if used > MAX_NBT_BYTES {
        return Err(DecodeError::Invalid("NBT too large"));
    }
    *r = Reader::new(&rest[used..]);
    Ok(tag)
}

pub fn write_nbt(out: &mut BytesMut, tag: &Tag) {
    tag.write_network(out);
}

/// `ByteBufCodecs.fromCodec(WithRegistries)`: the persistent form as network NBT.
pub fn read_value(r: &mut Reader<'_>) -> WireResult<Value> {
    Ok(Value::from_nbt(&read_nbt(r)?))
}

pub fn write_value(out: &mut BytesMut, value: &Value) {
    value.to_nbt().write_network(out);
}

/// `UUIDUtil.STREAM_CODEC`: two longs.
pub fn read_uuid(r: &mut Reader<'_>) -> WireResult<Uuid> {
    r.uuid()
}

/// `ByteBufCodecs.idMapper` over an enum's ordinals, checked against its size.
pub fn read_enum(r: &mut Reader<'_>, count: usize) -> WireResult<i32> {
    let v = r.varint()?;
    if v < 0 || v as usize >= count {
        return Err(DecodeError::Invalid("enum ordinal out of range"));
    }
    Ok(v)
}

/// UUIDs in persistent form (`UUIDUtil.CODEC`): four ints, most significant first.
pub fn uuid_to_value(u: Uuid) -> Value {
    let b = u.as_u128();
    Value::IntList(vec![(b >> 96) as i32, (b >> 64) as i32, (b >> 32) as i32, b as i32])
}

pub fn uuid_from_value(v: &Value) -> crate::value::DataResult<Uuid> {
    let ints = v.as_int_stream()?;
    if ints.len() != 4 {
        return crate::value::err(format!("UUID needs 4 ints, got {}", ints.len()));
    }
    let n = ints.iter().fold(0u128, |acc, &i| (acc << 32) | i as u32 as u128);
    Ok(Uuid::from_u128(n))
}
