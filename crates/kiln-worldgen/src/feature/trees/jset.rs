//! `java.util.HashSet<BlockPos>` with Java's iteration order, which tree placement depends
//! on: decorators walk the log and leaf sets (stably sorted by y) and draw randoms per
//! position, and `TreeFeature.updateLeaves` pops positions in iteration order.
//!
//! Mirrors `HashMap`: power-of-two table (16 initially, load factor 0.75), the hash spread
//! `h ^ (h >>> 16)`, bins appended at the tail and split in order on resize, and red-black
//! tree bins (`HashMap.TreeNode`) once a bin exceeds 8 entries in a table of 64 or more,
//! whose root is moved to the front of the bin and whose insertions link after their tree
//! parent. `BlockPos` is not directly `Comparable` to itself, so equal hashes fall back to
//! `tieBreakOrder` (identity hashes in Java, unreproducible; insertion order here).

use crate::pos::BlockPos;

/// `HashMap.hash(key)` of a `BlockPos` (`Vec3i.hashCode` spread).
fn hash(p: BlockPos) -> i32 {
    let h = p.y.wrapping_add(p.z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(p.x) as u32;
    (h ^ (h >> 16)) as i32
}

const NIL: usize = usize::MAX;
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;

#[derive(Clone, Debug)]
struct Node {
    pos: BlockPos,
    hash: i32,
    next: usize,
    prev: usize,
    parent: usize,
    left: usize,
    right: usize,
    red: bool,
}

#[derive(Clone, Debug, Default)]
pub struct JHashSet {
    nodes: Vec<Node>,
    /// First node of each bin.
    table: Vec<usize>,
    /// Whether each bin is a tree bin.
    tree: Vec<bool>,
    len: usize,
}

impl JHashSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn index(&self, h: i32) -> usize {
        h as u32 as usize & (self.table.len() - 1)
    }

    fn find(&self, p: BlockPos) -> Option<usize> {
        if self.table.is_empty() {
            return None;
        }
        let mut e = self.table[self.index(hash(p))];
        while e != NIL {
            if self.nodes[e].pos == p {
                return Some(e);
            }
            e = self.nodes[e].next;
        }
        None
    }

    pub fn contains(&self, p: BlockPos) -> bool {
        self.find(p).is_some()
    }

    fn new_node(&mut self, pos: BlockPos, h: i32) -> usize {
        self.nodes.push(Node { pos, hash: h, next: NIL, prev: NIL, parent: NIL, left: NIL, right: NIL, red: false });
        self.nodes.len() - 1
    }

    /// `add` (`HashMap.putVal`): false if already present.
    pub fn insert(&mut self, p: BlockPos) -> bool {
        if self.table.is_empty() {
            self.table = vec![NIL; 16];
            self.tree = vec![false; 16];
        }
        if self.contains(p) {
            return false;
        }
        let h = hash(p);
        let i = self.index(h);
        if self.table[i] == NIL {
            let x = self.new_node(p, h);
            self.table[i] = x;
        } else if self.tree[i] {
            self.put_tree_val(i, p, h);
        } else {
            let mut last = self.table[i];
            let mut count = 1;
            while self.nodes[last].next != NIL {
                last = self.nodes[last].next;
                count += 1;
            }
            let x = self.new_node(p, h);
            self.nodes[last].next = x;
            self.nodes[x].prev = last;
            if count >= TREEIFY_THRESHOLD {
                self.treeify_bin(i);
            }
        }
        self.len += 1;
        if self.len > self.table.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    /// `HashMap.treeifyBin`.
    fn treeify_bin(&mut self, i: usize) {
        if self.table.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
        } else {
            self.tree[i] = true;
            self.treeify(i);
        }
    }

    /// `TreeNode.treeify` of the list in bin `i`.
    fn treeify(&mut self, i: usize) {
        let mut root = NIL;
        let mut x = self.table[i];
        while x != NIL {
            let next = self.nodes[x].next;
            self.nodes[x].left = NIL;
            self.nodes[x].right = NIL;
            if root == NIL {
                self.nodes[x].parent = NIL;
                self.nodes[x].red = false;
                root = x;
            } else {
                let h = self.nodes[x].hash;
                let mut p = root;
                loop {
                    let dir = if self.nodes[p].hash > h { -1 } else { 1 };
                    let xp = p;
                    p = if dir <= 0 { self.nodes[p].left } else { self.nodes[p].right };
                    if p == NIL {
                        self.nodes[x].parent = xp;
                        if dir <= 0 {
                            self.nodes[xp].left = x;
                        } else {
                            self.nodes[xp].right = x;
                        }
                        root = self.balance_insertion(root, x);
                        break;
                    }
                }
            }
            x = next;
        }
        self.move_root_to_front(i, root);
    }

    /// `TreeNode.putTreeVal` for a key known to be absent.
    fn put_tree_val(&mut self, i: usize, pos: BlockPos, h: i32) {
        let root = self.root_of(self.table[i]);
        let mut p = root;
        loop {
            let dir = if self.nodes[p].hash > h { -1 } else { 1 };
            let xp = p;
            p = if dir <= 0 { self.nodes[p].left } else { self.nodes[p].right };
            if p == NIL {
                let xpn = self.nodes[xp].next;
                let x = self.new_node(pos, h);
                self.nodes[x].next = xpn;
                if dir <= 0 {
                    self.nodes[xp].left = x;
                } else {
                    self.nodes[xp].right = x;
                }
                self.nodes[xp].next = x;
                self.nodes[x].parent = xp;
                self.nodes[x].prev = xp;
                if xpn != NIL {
                    self.nodes[xpn].prev = x;
                }
                let r = self.balance_insertion(root, x);
                self.move_root_to_front(i, r);
                return;
            }
        }
    }

    fn root_of(&self, mut r: usize) -> usize {
        while self.nodes[r].parent != NIL {
            r = self.nodes[r].parent;
        }
        r
    }

    /// `TreeNode.moveRootToFront`.
    fn move_root_to_front(&mut self, i: usize, root: usize) {
        if root == NIL {
            return;
        }
        let first = self.table[i];
        if root != first {
            self.table[i] = root;
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
    }

    fn rotate_left(&mut self, mut root: usize, p: usize) -> usize {
        if p != NIL && self.nodes[p].right != NIL {
            let r = self.nodes[p].right;
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
        }
        root
    }

    fn rotate_right(&mut self, mut root: usize, p: usize) -> usize {
        if p != NIL && self.nodes[p].left != NIL {
            let l = self.nodes[p].left;
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
        }
        root
    }

    fn red(&self, x: usize) -> bool {
        x != NIL && self.nodes[x].red
    }

    /// `TreeNode.balanceInsertion`.
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
                if self.red(xppr) {
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
            } else if self.red(xppl) {
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

    /// `TreeNode.balanceDeletion`.
    fn balance_deletion(&mut self, mut root: usize, mut x: usize) -> usize {
        loop {
            if x == NIL || x == root {
                return root;
            }
            let mut xp = self.nodes[x].parent;
            if xp == NIL {
                self.nodes[x].red = false;
                return x;
            }
            if self.nodes[x].red {
                self.nodes[x].red = false;
                return root;
            }
            let xpl = self.nodes[xp].left;
            if xpl == x {
                let mut xpr = self.nodes[xp].right;
                if self.red(xpr) {
                    self.nodes[xpr].red = false;
                    self.nodes[xp].red = true;
                    root = self.rotate_left(root, xp);
                    xp = self.nodes[x].parent;
                    xpr = if xp == NIL { NIL } else { self.nodes[xp].right };
                }
                if xpr == NIL {
                    x = xp;
                } else {
                    let sl = self.nodes[xpr].left;
                    let mut sr = self.nodes[xpr].right;
                    if !self.red(sr) && !self.red(sl) {
                        self.nodes[xpr].red = true;
                        x = xp;
                    } else {
                        if !self.red(sr) {
                            if sl != NIL {
                                self.nodes[sl].red = false;
                            }
                            self.nodes[xpr].red = true;
                            root = self.rotate_right(root, xpr);
                            xp = self.nodes[x].parent;
                            xpr = if xp == NIL { NIL } else { self.nodes[xp].right };
                        }
                        if xpr != NIL {
                            self.nodes[xpr].red = xp != NIL && self.nodes[xp].red;
                            sr = self.nodes[xpr].right;
                            if sr != NIL {
                                self.nodes[sr].red = false;
                            }
                        }
                        if xp != NIL {
                            self.nodes[xp].red = false;
                            root = self.rotate_left(root, xp);
                        }
                        x = root;
                    }
                }
            } else {
                let mut xpl = xpl;
                if self.red(xpl) {
                    self.nodes[xpl].red = false;
                    self.nodes[xp].red = true;
                    root = self.rotate_right(root, xp);
                    xp = self.nodes[x].parent;
                    xpl = if xp == NIL { NIL } else { self.nodes[xp].left };
                }
                if xpl == NIL {
                    x = xp;
                } else {
                    let mut sl = self.nodes[xpl].left;
                    let sr = self.nodes[xpl].right;
                    if !self.red(sl) && !self.red(sr) {
                        self.nodes[xpl].red = true;
                        x = xp;
                    } else {
                        if !self.red(sl) {
                            if sr != NIL {
                                self.nodes[sr].red = false;
                            }
                            self.nodes[xpl].red = true;
                            root = self.rotate_left(root, xpl);
                            xp = self.nodes[x].parent;
                            xpl = if xp == NIL { NIL } else { self.nodes[xp].left };
                        }
                        if xpl != NIL {
                            self.nodes[xpl].red = xp != NIL && self.nodes[xp].red;
                            sl = self.nodes[xpl].left;
                            if sl != NIL {
                                self.nodes[sl].red = false;
                            }
                        }
                        if xp != NIL {
                            self.nodes[xp].red = false;
                            root = self.rotate_right(root, xp);
                        }
                        x = root;
                    }
                }
            }
        }
    }

    /// `TreeNode.removeTreeNode(map, tab, movable = false)` (iterator removal).
    fn remove_tree_node(&mut self, i: usize, p: usize) {
        let succ = self.nodes[p].next;
        let pred = self.nodes[p].prev;
        let mut first = self.table[i];
        if pred == NIL {
            self.table[i] = succ;
            first = succ;
        } else {
            self.nodes[pred].next = succ;
        }
        if succ != NIL {
            self.nodes[succ].prev = pred;
        }
        if first == NIL {
            self.tree[i] = false;
            return;
        }
        let mut root = self.root_of(first);
        let (pl, pr) = (self.nodes[p].left, self.nodes[p].right);
        let replacement;
        if pl != NIL && pr != NIL {
            let mut s = pr;
            while self.nodes[s].left != NIL {
                s = self.nodes[s].left;
            }
            let c = self.nodes[s].red;
            self.nodes[s].red = self.nodes[p].red;
            self.nodes[p].red = c;
            let sr = self.nodes[s].right;
            let pp = self.nodes[p].parent;
            if s == pr {
                self.nodes[p].parent = s;
                self.nodes[s].right = p;
            } else {
                let sp = self.nodes[s].parent;
                self.nodes[p].parent = sp;
                if sp != NIL {
                    if s == self.nodes[sp].left {
                        self.nodes[sp].left = p;
                    } else {
                        self.nodes[sp].right = p;
                    }
                }
                self.nodes[s].right = pr;
                if pr != NIL {
                    self.nodes[pr].parent = s;
                }
            }
            self.nodes[p].left = NIL;
            self.nodes[p].right = sr;
            if sr != NIL {
                self.nodes[sr].parent = p;
            }
            self.nodes[s].left = pl;
            if pl != NIL {
                self.nodes[pl].parent = s;
            }
            self.nodes[s].parent = pp;
            if pp == NIL {
                root = s;
            } else if p == self.nodes[pp].left {
                self.nodes[pp].left = s;
            } else {
                self.nodes[pp].right = s;
            }
            replacement = if sr != NIL { sr } else { p };
        } else if pl != NIL {
            replacement = pl;
        } else if pr != NIL {
            replacement = pr;
        } else {
            replacement = p;
        }
        if replacement != p {
            let pp = self.nodes[p].parent;
            self.nodes[replacement].parent = pp;
            if pp == NIL {
                root = replacement;
                self.nodes[replacement].red = false;
            } else if p == self.nodes[pp].left {
                self.nodes[pp].left = replacement;
            } else {
                self.nodes[pp].right = replacement;
            }
            self.nodes[p].left = NIL;
            self.nodes[p].right = NIL;
            self.nodes[p].parent = NIL;
        }
        if !self.nodes[p].red {
            self.balance_deletion(root, replacement);
        }
        if replacement == p {
            let pp = self.nodes[p].parent;
            self.nodes[p].parent = NIL;
            if pp != NIL {
                if p == self.nodes[pp].left {
                    self.nodes[pp].left = NIL;
                } else if p == self.nodes[pp].right {
                    self.nodes[pp].right = NIL;
                }
            }
        }
    }

    /// `HashMap.resize`: doubles the table, splitting bins in order (`TreeNode.split` for trees).
    fn resize(&mut self) {
        let n = self.table.len();
        let old = std::mem::replace(&mut self.table, vec![NIL; n * 2]);
        let old_tree = std::mem::replace(&mut self.tree, vec![false; n * 2]);
        for (j, head) in old.into_iter().enumerate() {
            let (mut lo_h, mut lo_t, mut hi_h, mut hi_t) = (NIL, NIL, NIL, NIL);
            let (mut lc, mut hc) = (0, 0);
            let mut e = head;
            while e != NIL {
                let next = self.nodes[e].next;
                self.nodes[e].next = NIL;
                let (h, t, c) = if self.nodes[e].hash as u32 as usize & n == 0 {
                    (&mut lo_h, &mut lo_t, &mut lc)
                } else {
                    (&mut hi_h, &mut hi_t, &mut hc)
                };
                self.nodes[e].prev = *t;
                if *t == NIL {
                    *h = e;
                } else {
                    self.nodes[*t].next = e;
                }
                *t = e;
                *c += 1;
                e = next;
            }
            self.table[j] = lo_h;
            self.table[j + n] = hi_h;
            if !old_tree[j] {
                continue;
            }
            for (idx, h, c, other) in [(j, lo_h, lc, hi_h), (j + n, hi_h, hc, lo_h)] {
                if h == NIL {
                    continue;
                }
                if c <= UNTREEIFY_THRESHOLD {
                    self.untreeify(h);
                } else {
                    self.tree[idx] = true;
                    if other != NIL {
                        self.treeify(idx);
                    }
                }
            }
        }
    }

    /// `TreeNode.untreeify`: plain nodes in the same order.
    fn untreeify(&mut self, mut e: usize) {
        while e != NIL {
            let n = &mut self.nodes[e];
            n.parent = NIL;
            n.left = NIL;
            n.right = NIL;
            n.red = false;
            e = n.next;
        }
    }

    /// `iterator().next()` followed by `remove()`.
    pub fn pop_first(&mut self) -> Option<BlockPos> {
        let i = self.table.iter().position(|&h| h != NIL)?;
        let e = self.table[i];
        if self.tree[i] {
            self.remove_tree_node(i, e);
        } else {
            let next = self.nodes[e].next;
            self.table[i] = next;
            if next != NIL {
                self.nodes[next].prev = NIL;
            }
        }
        self.len -= 1;
        Some(self.nodes[e].pos)
    }

    /// Iteration order.
    pub fn iter(&self) -> impl Iterator<Item = BlockPos> + '_ {
        self.table.iter().flat_map(move |&head| {
            let mut e = head;
            std::iter::from_fn(move || {
                if e == NIL {
                    return None;
                }
                let p = self.nodes[e].pos;
                e = self.nodes[e].next;
                Some(p)
            })
        })
    }

    /// `new ObjectArrayList<>(set)` sorted stably by y (`TreeDecorator.Context`).
    pub fn sorted_by_y(&self) -> Vec<BlockPos> {
        let mut v: Vec<BlockPos> = self.iter().collect();
        v.sort_by_key(|p| p.y);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_javamath::random::{LegacyRandom, RandomSource};

    /// Random inserts and iterator removals over heavily colliding positions (tree bins,
    /// untreeify on resize, tree removals). The checksum is FNV-1a over the popped positions
    /// and final iteration order printed by the same program run against `java.util.HashSet`.
    #[test]
    fn matches_java_hash_set() {
        let mut out = String::new();
        for seed in 0..40 {
            let mut r = LegacyRandom::new(seed);
            let mut s = JHashSet::new();
            for _ in 0..3000 {
                if r.next_int_bounded(10) < 7 {
                    let x = 64 * r.next_int_bounded(40) + r.next_int_bounded(3);
                    s.insert(BlockPos::new(x, r.next_int_bounded(2), 0));
                } else if let Some(p) = s.pop_first() {
                    out += &format!("{},{};", p.x, p.y);
                }
            }
            for p in s.iter() {
                out += &format!("{},{};", p.x, p.y);
            }
            out.push('\n');
        }
        let mut h: u64 = 0xcbf29ce484222325;
        for b in out.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        assert_eq!(h, 0xe8a28d23b38c79ae);
    }
}
