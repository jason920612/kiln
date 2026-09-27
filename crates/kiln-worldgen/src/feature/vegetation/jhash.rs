//! `java.util.HashSet` iteration order, which several features depend on: they fill a set of
//! positions and then draw random numbers while iterating it.
//!
//! This is a faithful model of `java.util.HashMap` (as of JDK 8 and still in JDK 25): a power
//! of two table of bins that doubles past 3/4 load, bins as linked lists in insertion order,
//! and bins of 9+ entries in a table of 64+ turned into red-black trees (`TreeNode`), which
//! reorder the bin's list (the root moves to the front, new nodes follow their tree parent).
//! Keys with equal spread hashes would be ordered by identity hash codes in a tree; features'
//! position sets never contain those.

use crate::pos::BlockPos;
use std::collections::HashSet;
use std::hash::Hash;

/// `Vec3i.hashCode`.
pub fn pos_hash(p: BlockPos) -> i32 {
    p.y.wrapping_add(p.z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(p.x)
}

const NIL: usize = usize::MAX;

#[derive(Clone, Debug)]
struct Node<T> {
    hash: i32,
    key: T,
    next: usize,
    prev: usize,
    parent: usize,
    left: usize,
    right: usize,
    red: bool,
}

/// A `HashSet` created with the default capacity and only added to.
#[derive(Clone, Debug)]
pub struct JavaHashSet<T> {
    nodes: Vec<Node<T>>,
    /// Bin heads.
    table: Vec<usize>,
    /// Whether a bin holds `TreeNode`s.
    tree: Vec<bool>,
    index: HashSet<T>,
}

/// `HashMap.hash`.
fn spread(h: i32) -> i32 {
    h ^ ((h as u32) >> 16) as i32
}

impl<T: Eq + Hash + Copy> JavaHashSet<T> {
    pub fn new() -> Self {
        Self { nodes: Vec::new(), table: vec![NIL; 16], tree: vec![false; 16], index: HashSet::new() }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, item: &T) -> bool {
        self.index.contains(item)
    }

    fn bin(&self, hash: i32) -> usize {
        hash as usize & (self.table.len() - 1)
    }

    /// `add` with the key's `hashCode`: false if it was already present.
    pub fn insert(&mut self, hash_code: i32, key: T) -> bool {
        if !self.index.insert(key) {
            return false;
        }
        let hash = spread(hash_code);
        let i = self.bin(hash);
        let x = self.nodes.len();
        self.nodes.push(Node { hash, key, next: NIL, prev: NIL, parent: NIL, left: NIL, right: NIL, red: false });
        if self.table[i] == NIL {
            self.table[i] = x;
        } else if self.tree[i] {
            self.put_tree(i, x);
        } else {
            let mut p = self.table[i];
            let mut count = 0;
            while self.nodes[p].next != NIL {
                p = self.nodes[p].next;
                count += 1;
            }
            self.nodes[p].next = x;
            if count >= 7 {
                self.treeify_bin(i);
            }
        }
        if self.nodes.len() > self.table.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    /// The keys in iteration order.
    pub fn iter_order(&self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.nodes.len());
        for &head in &self.table {
            let mut e = head;
            while e != NIL {
                out.push(self.nodes[e].key);
                e = self.nodes[e].next;
            }
        }
        out
    }

    fn list(&self, head: usize) -> Vec<usize> {
        let mut v = Vec::new();
        let mut e = head;
        while e != NIL {
            v.push(e);
            e = self.nodes[e].next;
        }
        v
    }

    /// Links `nodes` as the bin's list (plain nodes).
    fn link(&mut self, bin: usize, nodes: &[usize]) {
        self.table[bin] = nodes.first().copied().unwrap_or(NIL);
        for (k, &n) in nodes.iter().enumerate() {
            let node = &mut self.nodes[n];
            node.prev = if k == 0 { NIL } else { nodes[k - 1] };
            node.next = nodes.get(k + 1).copied().unwrap_or(NIL);
        }
    }

    /// `resize`: doubles the table, splitting each bin stably into a low and a high bin.
    fn resize(&mut self) {
        let old = self.table.len();
        let old_table = std::mem::replace(&mut self.table, vec![NIL; old * 2]);
        let old_tree = std::mem::replace(&mut self.tree, vec![false; old * 2]);
        for j in 0..old {
            let all = self.list(old_table[j]);
            let (lo, hi): (Vec<usize>, Vec<usize>) = all.iter().partition(|&&n| self.nodes[n].hash as usize & old == 0);
            if !old_tree[j] {
                self.link(j, &lo);
                self.link(j + old, &hi);
                continue;
            }
            for (bin, part, other) in [(j, &lo, &hi), (j + old, &hi, &lo)] {
                if part.is_empty() {
                    continue;
                }
                self.link(bin, part);
                if part.len() <= 6 {
                    self.tree[bin] = false;
                } else {
                    self.tree[bin] = true;
                    if !other.is_empty() {
                        self.treeify(bin);
                    }
                }
            }
        }
    }

    /// `treeifyBin`: resizes a small table instead.
    fn treeify_bin(&mut self, bin: usize) {
        if self.table.len() < 64 {
            self.resize();
        } else {
            self.tree[bin] = true;
            let nodes = self.list(self.table[bin]);
            self.link(bin, &nodes);
            self.treeify(bin);
        }
    }

    /// `TreeNode.treeify`: builds the tree from the bin's list order.
    fn treeify(&mut self, bin: usize) {
        let mut root = NIL;
        for x in self.list(self.table[bin]) {
            self.nodes[x].left = NIL;
            self.nodes[x].right = NIL;
            if root == NIL {
                self.nodes[x].parent = NIL;
                self.nodes[x].red = false;
                root = x;
                continue;
            }
            let h = self.nodes[x].hash;
            let mut p = root;
            loop {
                let ph = self.nodes[p].hash;
                let left = if ph > h {
                    true
                } else if ph < h {
                    false
                } else {
                    // tieBreakOrder: identity hash codes; see the module docs.
                    true
                };
                let child = if left { self.nodes[p].left } else { self.nodes[p].right };
                if child == NIL {
                    self.nodes[x].parent = p;
                    if left {
                        self.nodes[p].left = x;
                    } else {
                        self.nodes[p].right = x;
                    }
                    root = self.balance_insertion(root, x);
                    break;
                }
                p = child;
            }
        }
        self.move_root_to_front(bin, root);
    }

    /// `TreeNode.putTreeVal` for a new key.
    fn put_tree(&mut self, bin: usize, x: usize) {
        let mut root = self.table[bin];
        while self.nodes[root].parent != NIL {
            root = self.nodes[root].parent;
        }
        let h = self.nodes[x].hash;
        let mut p = root;
        loop {
            let left = self.nodes[p].hash >= h;
            let child = if left { self.nodes[p].left } else { self.nodes[p].right };
            if child == NIL {
                let xpn = self.nodes[p].next;
                if left {
                    self.nodes[p].left = x;
                } else {
                    self.nodes[p].right = x;
                }
                self.nodes[p].next = x;
                self.nodes[x].next = xpn;
                self.nodes[x].parent = p;
                self.nodes[x].prev = p;
                if xpn != NIL {
                    self.nodes[xpn].prev = x;
                }
                let root = self.balance_insertion(root, x);
                self.move_root_to_front(bin, root);
                return;
            }
            p = child;
        }
    }

    fn move_root_to_front(&mut self, bin: usize, root: usize) {
        let first = self.table[bin];
        if root == first {
            return;
        }
        self.table[bin] = root;
        let rp = self.nodes[root].prev;
        let rn = self.nodes[root].next;
        if rn != NIL {
            self.nodes[rn].prev = rp;
        }
        if rp != NIL {
            self.nodes[rp].next = rn;
        }
        if first != NIL {
            self.nodes[first].prev = root;
        }
        self.nodes[root].next = first;
        self.nodes[root].prev = NIL;
    }

    fn rotate_left(&mut self, mut root: usize, p: usize) -> usize {
        let r = self.nodes[p].right;
        if r == NIL {
            return root;
        }
        let rl = self.nodes[r].left;
        self.nodes[p].right = rl;
        if rl != NIL {
            self.nodes[rl].parent = p;
        }
        let pp = self.nodes[p].parent;
        self.nodes[r].parent = pp;
        if pp == NIL {
            root = r;
            self.nodes[r].red = false;
        } else if self.nodes[pp].left == p {
            self.nodes[pp].left = r;
        } else {
            self.nodes[pp].right = r;
        }
        self.nodes[r].left = p;
        self.nodes[p].parent = r;
        root
    }

    fn rotate_right(&mut self, mut root: usize, p: usize) -> usize {
        let l = self.nodes[p].left;
        if l == NIL {
            return root;
        }
        let lr = self.nodes[l].right;
        self.nodes[p].left = lr;
        if lr != NIL {
            self.nodes[lr].parent = p;
        }
        let pp = self.nodes[p].parent;
        self.nodes[l].parent = pp;
        if pp == NIL {
            root = l;
            self.nodes[l].red = false;
        } else if self.nodes[pp].right == p {
            self.nodes[pp].right = l;
        } else {
            self.nodes[pp].left = l;
        }
        self.nodes[l].right = p;
        self.nodes[p].parent = l;
        root
    }

    fn balance_insertion(&mut self, mut root: usize, mut x: usize) -> usize {
        self.nodes[x].red = true;
        loop {
            let mut xp = self.nodes[x].parent;
            if xp == NIL {
                self.nodes[x].red = false;
                return x;
            }
            let mut xpp = self.nodes[xp].parent;
            if !self.nodes[xp].red || xpp == NIL {
                return root;
            }
            let xppl = self.nodes[xpp].left;
            if xp == xppl {
                let xppr = self.nodes[xpp].right;
                if xppr != NIL && self.nodes[xppr].red {
                    self.nodes[xppr].red = false;
                    self.nodes[xp].red = false;
                    self.nodes[xpp].red = true;
                    x = xpp;
                } else {
                    if x == self.nodes[xp].right {
                        x = xp;
                        root = self.rotate_left(root, x);
                        xp = self.nodes[x].parent;
                        xpp = if xp == NIL { NIL } else { self.nodes[xp].parent };
                    }
                    if xp != NIL {
                        self.nodes[xp].red = false;
                        if xpp != NIL {
                            self.nodes[xpp].red = true;
                            root = self.rotate_right(root, xpp);
                        }
                    }
                }
            } else if xppl != NIL && self.nodes[xppl].red {
                self.nodes[xppl].red = false;
                self.nodes[xp].red = false;
                self.nodes[xpp].red = true;
                x = xpp;
            } else {
                if x == self.nodes[xp].left {
                    x = xp;
                    root = self.rotate_right(root, x);
                    xp = self.nodes[x].parent;
                    xpp = if xp == NIL { NIL } else { self.nodes[xp].parent };
                }
                if xp != NIL {
                    self.nodes[xp].red = false;
                    if xpp != NIL {
                        self.nodes[xpp].red = true;
                        root = self.rotate_left(root, xpp);
                    }
                }
            }
        }
    }
}

impl<T: Eq + Hash + Copy> Default for JavaHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A `HashSet<BlockPos>`.
#[derive(Clone, Debug, Default)]
pub struct PosSet(JavaHashSet<BlockPos>);

impl PosSet {
    pub fn new() -> Self {
        Self(JavaHashSet::new())
    }

    pub fn insert(&mut self, p: BlockPos) -> bool {
        self.0.insert(pos_hash(p), p)
    }

    pub fn contains(&self, p: BlockPos) -> bool {
        self.0.contains(&p)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter_order(&self) -> Vec<BlockPos> {
        self.0.iter_order()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_sets_iterate_by_bucket_then_insertion() {
        let mut s = PosSet::new();
        for p in [BlockPos::new(3, 0, 0), BlockPos::new(1, 0, 0), BlockPos::new(17, 0, 0), BlockPos::new(2, 0, 0)] {
            s.insert(p);
        }
        let order: Vec<i32> = s.iter_order().iter().map(|p| p.x).collect();
        assert_eq!(order, [1, 17, 2, 3]);
    }

    #[test]
    fn treeified_bins_keep_every_key() {
        let mut s = JavaHashSet::new();
        for i in 0..200 {
            s.insert(i * 64, i);
        }
        let mut order = s.iter_order();
        assert_eq!(order.len(), 200);
        order.sort();
        assert_eq!(order, (0..200).collect::<Vec<_>>());
    }

    /// Compares with orders printed by a JDK (`HashOrder.java` in the scratchpad) when
    /// `KILN_JAVA_HASH_ORDER` names its output.
    #[test]
    fn matches_jdk() {
        let Some(path) = std::env::var_os("KILN_JAVA_HASH_ORDER") else { return };
        let text = std::fs::read_to_string(path).unwrap();
        let hash = |c: i32, id: i32| -> i32 {
            match c {
                0 => (id % 3) + id * 4096,
                1 => id * 64 + (id * 7919) % 5,
                2 => (id * 31 + (id % 11) * 961) ^ (id << 20),
                _ => (id - 50) * 1024 + (id & 1),
            }
        };
        let mut lines = text.lines();
        for c in 0..4 {
            let mut s = JavaHashSet::new();
            for id in 0..300 {
                s.insert(hash(c, id), id);
                if [20, 90, 299].contains(&id) {
                    let mine: String = s.iter_order().iter().map(|i| format!("{i},")).collect();
                    assert_eq!(lines.next().unwrap().trim_end(), format!("{c} {id} {mine}"));
                }
            }
        }
    }
}
