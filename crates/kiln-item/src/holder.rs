//! Registry holders and holder sets as data components reference them.

use crate::ident::Identifier;
use crate::registry::Registry;
use crate::value::{DataError, DataResult, Value};
use crate::wire::WireResult;
use bytes::BytesMut;
use kiln_proto::{DecodeError, Reader, WriteExt};

/// `Holder<T>` of a registry that also accepts inline definitions (`ByteBufCodecs.holder`,
/// `RegistryFileCodec`): a registry entry by network id, or a direct value.
#[derive(Debug, Clone, PartialEq)]
pub enum Holder<T> {
    Reference(i32),
    Direct(Box<T>),
}

impl<T> Holder<T> {
    /// VarInt `id + 1`, or 0 followed by the direct value.
    pub fn read(reg: Registry, r: &mut Reader<'_>, direct: impl FnOnce(&mut Reader<'_>) -> WireResult<T>) -> WireResult<Self> {
        match r.varint()? {
            0 => Ok(Holder::Direct(Box::new(direct(r)?))),
            n if n > 0 && (n as usize) <= reg.len() => Ok(Holder::Reference(n - 1)),
            _ => Err(DecodeError::Invalid("holder id out of range")),
        }
    }

    pub fn write(&self, out: &mut BytesMut, direct: impl FnOnce(&T, &mut BytesMut)) {
        match self {
            Holder::Reference(id) => out.put_varint(id + 1),
            Holder::Direct(v) => {
                out.put_varint(0);
                direct(v, out);
            }
        }
    }

    /// The entry name, or the direct value's own encoding.
    pub fn to_value(&self, reg: Registry, direct: impl FnOnce(&T) -> Value) -> Value {
        match self {
            Holder::Reference(id) => reg.id_to_value(*id),
            Holder::Direct(v) => direct(v),
        }
    }

    pub fn from_value(reg: Registry, v: &Value, direct: impl FnOnce(&Value) -> DataResult<T>) -> DataResult<Self> {
        match v {
            Value::String(_) => Ok(Holder::Reference(reg.id_from_value(v)?)),
            _ => Ok(Holder::Direct(Box::new(direct(v)?))),
        }
    }
}

/// `HolderSet<T>`: a tag, or an explicit list of entries (network ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HolderSet {
    Tag(Identifier),
    Direct(Vec<i32>),
}

impl HolderSet {
    /// `ByteBufCodecs.holderSet`: VarInt 0 and the tag id, or `n + 1` and n VarInt ids.
    pub fn read(reg: Registry, r: &mut Reader<'_>) -> WireResult<Self> {
        let n = r.varint()?;
        if n == 0 {
            return Ok(HolderSet::Tag(Identifier::read(r)?));
        }
        let n = usize::try_from(n - 1).map_err(|_| DecodeError::Invalid("holder set size"))?;
        if n > r.remaining() {
            return Err(DecodeError::Eof);
        }
        (0..n).map(|_| reg.read_id(r)).collect::<WireResult<_>>().map(HolderSet::Direct)
    }

    pub fn write(&self, out: &mut BytesMut) {
        match self {
            HolderSet::Tag(tag) => {
                out.put_varint(0);
                tag.write(out);
            }
            HolderSet::Direct(ids) => {
                out.put_varint(ids.len() as i32 + 1);
                for &id in ids {
                    out.put_varint(id);
                }
            }
        }
    }

    /// `RegistryCodecs.homogeneousList`: `"#tag"`, a single entry name, or a list of names.
    pub fn to_value(&self, reg: Registry) -> Value {
        self.to_value_with(reg, false)
    }

    /// As [`HolderSet::to_value`]; `always_list` writes one entry as a list too.
    pub fn to_value_with(&self, reg: Registry, always_list: bool) -> Value {
        match self {
            HolderSet::Tag(tag) => Value::String(format!("#{tag}")),
            HolderSet::Direct(ids) if ids.len() == 1 && !always_list => reg.id_to_value(ids[0]),
            HolderSet::Direct(ids) => Value::List(ids.iter().map(|&id| reg.id_to_value(id)).collect()),
        }
    }

    pub fn from_value(reg: Registry, v: &Value) -> DataResult<Self> {
        match v {
            Value::String(s) if s.starts_with('#') => Identifier::parse(&s[1..])
                .map(HolderSet::Tag)
                .ok_or_else(|| DataError(format!("invalid tag {s:?}"))),
            Value::String(_) => Ok(HolderSet::Direct(vec![reg.id_from_value(v)?])),
            _ => v.as_list()?.iter().map(|e| reg.id_from_value(e)).collect::<DataResult<_>>().map(HolderSet::Direct),
        }
    }
}
