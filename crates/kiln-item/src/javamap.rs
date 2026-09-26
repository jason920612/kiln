//! Iteration order of `java.util.HashMap<String, _>`, which is the order `CompoundTag` writes
//! its entries in. Reproducing it is what makes NBT we build byte-identical to vanilla's.

/// `String.hashCode()`: UTF-16 units, `h = 31 * h + c`.
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// Indices of `keys` (distinct, in insertion order) in the order a `HashMap` created with the
/// default capacity iterates them after inserting them one by one.
pub fn iteration_order(keys: &[&str]) -> Vec<usize> {
    if keys.len() <= 1 {
        return (0..keys.len()).collect();
    }
    let hashes: Vec<u32> = keys
        .iter()
        .map(|k| {
            let h = string_hash(k) as u32;
            h ^ (h >> 16)
        })
        .collect();
    // Bucket chains hold indices in insertion order; resizing splits each chain in order.
    let mut cap = 16usize;
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); cap];
    let resize = |buckets: &mut Vec<Vec<usize>>, cap: &mut usize| {
        *cap *= 2;
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); *cap];
        for chain in buckets.iter() {
            for &i in chain {
                next[hashes[i] as usize & (*cap - 1)].push(i);
            }
        }
        *buckets = next;
    };
    for (i, &h) in hashes.iter().enumerate() {
        let b = h as usize & (cap - 1);
        buckets[b].push(i);
        // A ninth entry in one chain treeifies it, which resizes instead while the table is small.
        // (Tree bins themselves need 64+ buckets and colliding keys; not modelled.)
        if buckets[b].len() >= 9 && cap < 64 {
            resize(&mut buckets, &mut cap);
        }
        if i + 1 > cap * 3 / 4 {
            resize(&mut buckets, &mut cap);
        }
    }
    buckets.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_string_hash() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("text"), 3556653);
        assert_eq!(string_hash("minecraft:custom_name"), "minecraft:custom_name".encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32)));
    }

    #[test]
    fn matches_vanilla_compound_order() {
        // Vanilla wrote {color, text, bold, italic} for a text component encoded as text, color,
        // bold, italic.
        let keys = ["text", "color", "bold", "italic"];
        let order: Vec<&str> = iteration_order(&keys).into_iter().map(|i| keys[i]).collect();
        assert_eq!(order, ["color", "text", "bold", "italic"]);
        // ItemStack.CODEC: {components, count, id}.
        let keys = ["id", "count", "components"];
        let order: Vec<&str> = iteration_order(&keys).into_iter().map(|i| keys[i]).collect();
        assert_eq!(order, ["components", "count", "id"]);
    }
}
