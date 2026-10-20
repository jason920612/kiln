//! Java's hashes and the order a `HashSet` iterates in: the management server's `set` methods compare
//! the lists as sets (`Collectors.toSet()`) and remove and add in the order of those sets.

use kiln_link::access::java_string_hash;
use uuid::Uuid;

/// `UUID.hashCode`.
pub fn uuid_hash(u: &Uuid) -> i32 {
    let (msb, lsb) = u.as_u64_pair();
    let hilo = msb ^ lsb;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// `Objects.hashCode(String)`.
pub fn string_hash(s: Option<&str>) -> i32 {
    s.map_or(0, java_string_hash)
}

/// A record's `hashCode`: the components combined as `31 * hash + component`.
pub fn record_hash(components: &[i32]) -> i32 {
    components.iter().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(*c))
}

/// `Optional<Instant>.hashCode`.
pub fn instant_hash(instant: Option<(i64, u32)>) -> i32 {
    match instant {
        None => 0,
        Some((secs, nanos)) => {
            let s = secs as u64;
            ((s ^ (s >> 32)) as i32).wrapping_add(51i32.wrapping_mul(nanos as i32))
        }
    }
}

/// `NameAndId.hashCode`.
pub fn name_and_id_hash(id: &Uuid, name: &str) -> i32 {
    record_hash(&[uuid_hash(id), string_hash(Some(name))])
}

/// The items in the order a `java.util.HashSet` that they were added to, one after the other, iterates in
/// (items must be distinct). The table starts with 16 buckets and doubles when it is three quarters full;
/// an item's bucket is its spread hash modulo the table size; a bucket keeps insertion order.
pub fn set_order<T>(items: Vec<(i32, T)>) -> Vec<T> {
    let mut capacity = 16usize;
    for size in 1..=items.len() {
        if size > capacity * 3 / 4 {
            capacity *= 2;
        }
    }
    let bucket = |h: i32| {
        let h = h as u32;
        ((h ^ (h >> 16)) as usize) & (capacity - 1)
    };
    // (A resize splits each bucket in two and keeps the order inside, so the final order is the order by
    // final bucket, then insertion.)
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&k| (bucket(items[k].0), k));
    let mut slots: Vec<Option<T>> = items.into_iter().map(|(_, t)| Some(t)).collect();
    order.into_iter().filter_map(|k| slots[k].take()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hashes() {
        assert_eq!(string_hash(Some("hello")), 99_162_322);
        assert_eq!(string_hash(None), 0);
        // UUID.fromString("00000000-0000-0000-0000-000000000007").hashCode() == 7
        assert_eq!(uuid_hash(&Uuid::from_u128(7)), 7);
        assert_eq!(uuid_hash(&Uuid::from_u128(0x0000_0001_0000_0000_0000_0000_0000_0003)), 1 ^ 3);
        assert_eq!(record_hash(&[1, 2, 3]), (31 + 2) * 31 + 3);
        assert_eq!(instant_hash(None), 0);
        assert_eq!(instant_hash(Some((1_893_456_000, 123_000_000))), 1_893_456_000i32.wrapping_add(51 * 123_000_000));
    }

    #[test]
    fn hash_sets_iterate_by_bucket() {
        // Small hashes land in their own buckets, in numeric order.
        assert_eq!(set_order(vec![(3, "c"), (1, "a"), (2, "b")]), vec!["a", "b", "c"]);
        // 17 and 1 share bucket 1 (16 buckets): insertion order inside.
        assert_eq!(set_order(vec![(17, "x"), (1, "y"), (0, "z")]), vec!["z", "x", "y"]);
        // Thirteen items grow the table to 32 buckets: 17 now follows its own bucket, after 1.
        let mut items: Vec<(i32, i32)> = (0..12).map(|i| (i, i)).collect();
        items.push((17, 17));
        assert_eq!(set_order(items).last(), Some(&17));
        // The high bits are folded in: 0x10000 spreads to 1.
        assert_eq!(set_order(vec![(2, "b"), (0x10000, "a")]), vec!["a", "b"]);
    }
}
