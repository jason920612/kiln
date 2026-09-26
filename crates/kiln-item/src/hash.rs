//! Vanilla's `HashOps.CRC32C_INSTANCE`: the hash a client sends per data component in
//! `container_click` (`HashedPatchMap`). Every value is hashed as a CRC32C of a small tagged
//! byte stream (Guava `Hasher` semantics: primitives little endian, strings as UTF-16 units);
//! lists hash their elements' hashes in order, maps their (key, value) hash pairs sorted by
//! unsigned key hash, then unsigned value hash.

use crate::value::Value;

const TAG_EMPTY: u8 = 1;
const TAG_MAP_START: u8 = 2;
const TAG_MAP_END: u8 = 3;
const TAG_LIST_START: u8 = 4;
const TAG_LIST_END: u8 = 5;
const TAG_BYTE: u8 = 6;
const TAG_SHORT: u8 = 7;
const TAG_INT: u8 = 8;
const TAG_LONG: u8 = 9;
const TAG_FLOAT: u8 = 10;
const TAG_DOUBLE: u8 = 11;
const TAG_STRING: u8 = 12;
const TAG_BOOLEAN: u8 = 13;
const TAG_BYTE_ARRAY_START: u8 = 14;
const TAG_BYTE_ARRAY_END: u8 = 15;
const TAG_INT_ARRAY_START: u8 = 16;
const TAG_INT_ARRAY_END: u8 = 17;
const TAG_LONG_ARRAY_START: u8 = 18;
const TAG_LONG_ARRAY_END: u8 = 19;

/// The 32-bit hash of `value`, as `HashCode.asInt()` returns it.
pub fn hash(value: &Value) -> i32 {
    hash_u32(value) as i32
}

fn hash_u32(value: &Value) -> u32 {
    let mut h = Crc32c::new();
    match value {
        Value::Empty => h.byte(TAG_EMPTY),
        Value::Bool(b) => {
            h.byte(TAG_BOOLEAN);
            h.byte(*b as u8);
        }
        Value::Byte(v) => {
            h.byte(TAG_BYTE);
            h.byte(*v as u8);
        }
        Value::Short(v) => {
            h.byte(TAG_SHORT);
            h.bytes(&v.to_le_bytes());
        }
        Value::Int(v) => {
            h.byte(TAG_INT);
            h.bytes(&v.to_le_bytes());
        }
        Value::Long(v) => {
            h.byte(TAG_LONG);
            h.bytes(&v.to_le_bytes());
        }
        Value::Float(v) => {
            h.byte(TAG_FLOAT);
            h.bytes(&v.to_bits().to_le_bytes());
        }
        Value::Double(v) => {
            h.byte(TAG_DOUBLE);
            h.bytes(&v.to_bits().to_le_bytes());
        }
        Value::String(s) => {
            h.byte(TAG_STRING);
            let units: Vec<u16> = s.encode_utf16().collect();
            h.bytes(&(units.len() as i32).to_le_bytes());
            for u in units {
                h.bytes(&u.to_le_bytes());
            }
        }
        Value::List(items) => {
            h.byte(TAG_LIST_START);
            for item in items {
                h.bytes(&hash_u32(item).to_le_bytes());
            }
            h.byte(TAG_LIST_END);
        }
        Value::Map(entries) => {
            let mut pairs: Vec<(u32, u32)> = entries.iter().map(|(k, v)| (hash_u32(k), hash_u32(v))).collect();
            pairs.sort_unstable();
            h.byte(TAG_MAP_START);
            for (k, v) in pairs {
                h.bytes(&k.to_le_bytes());
                h.bytes(&v.to_le_bytes());
            }
            h.byte(TAG_MAP_END);
        }
        Value::ByteList(v) => {
            h.byte(TAG_BYTE_ARRAY_START);
            for b in v {
                h.byte(*b as u8);
            }
            h.byte(TAG_BYTE_ARRAY_END);
        }
        Value::IntList(v) => {
            h.byte(TAG_INT_ARRAY_START);
            for x in v {
                h.bytes(&x.to_le_bytes());
            }
            h.byte(TAG_INT_ARRAY_END);
        }
        Value::LongList(v) => {
            h.byte(TAG_LONG_ARRAY_START);
            for x in v {
                h.bytes(&x.to_le_bytes());
            }
            h.byte(TAG_LONG_ARRAY_END);
        }
    }
    h.finish()
}

/// CRC-32C (Castagnoli), reflected, as Guava's `Hashing.crc32c()`.
struct Crc32c(u32);

static TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

impl Crc32c {
    fn new() -> Self {
        Self(!0)
    }

    fn byte(&mut self, b: u8) {
        self.0 = TABLE[((self.0 ^ b as u32) & 0xff) as usize] ^ (self.0 >> 8);
    }

    fn bytes(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
        }
    }

    fn finish(&self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_check_value() {
        let mut c = Crc32c::new();
        c.bytes(b"123456789");
        assert_eq!(c.finish(), 0xE306_9283);
    }

    #[test]
    fn matches_vanilla_for_simple_values() {
        // From the vanilla corpus: damage=5, repair_cost=7, unbreakable={}.
        assert_eq!(hash(&Value::Int(5)), 645064431);
        assert_eq!(hash(&Value::Int(7)), -1726626450);
        assert_eq!(hash(&Value::Map(Vec::new())), -982207288);
        assert_eq!(hash(&Value::Bool(true)), -1019818302);
        assert_eq!(hash(&Value::String("x".into())), -1651626295);
    }
}
