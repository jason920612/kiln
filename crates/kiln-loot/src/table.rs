//! Loot tables and pools, and the entry points vanilla offers on `LootTable`: raw item
//! generation, stack-split generation (`getRandomItems`) and container filling (`fill`).

use crate::condition::Condition;
use crate::context::LootContext;
use crate::data::LootData;
use crate::entry::{Entry, PoolEntry};
use crate::eval::{Eval, Sink};
use crate::function::Function;
use crate::json::Json;
use crate::number::{FloatProvider, IntProvider};
use crate::parse::{PResult, Parser, Ref, ident, list, opt, opt_or, req};
use crate::random::{RandomSequences, RngExt, seeded};
use crate::stack;
use kiln_item::{Identifier, ItemStack};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use std::sync::Arc;

/// `LootPool`.
#[derive(Debug, Clone)]
pub struct LootPool {
    pub entries: Vec<Entry>,
    pub condition: Option<Ref<Condition>>,
    pub modifier: Option<Ref<Function>>,
    pub rolls: Ref<IntProvider>,
    pub bonus_rolls: Ref<FloatProvider>,
}

/// `LootTable`.
#[derive(Debug, Clone)]
pub struct LootTable {
    /// The `type` (a `LootContextParamSets` name such as `minecraft:block`), if valid.
    pub param_set: Option<Identifier>,
    pub random_sequence: Option<Identifier>,
    pub pools: Vec<LootPool>,
    pub modifier: Option<Ref<Function>>,
}

impl LootPool {
    pub fn parse(p: &Parser, j: &Json) -> PResult<LootPool> {
        Ok(LootPool {
            entries: req(j, "entries", |v| list(v, |e| Entry::parse(p, e)))?,
            condition: opt(j, "condition", |v| Condition::parse_ref(p, v))?,
            modifier: opt(j, "modifier", |v| Function::parse_ref(p, v))?,
            rolls: req(j, "rolls", |v| IntProvider::parse_ref(p, v))?,
            bonus_rolls: opt_or(j, "bonus_rolls", Ref::direct(FloatProvider::Constant(0.0)), |v| FloatProvider::parse_ref(p, v))?,
        })
    }
}

impl LootTable {
    /// `LootTable.DIRECT_CODEC`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<LootTable> {
        crate::parse::obj(j)?;
        Ok(LootTable {
            // `lenientOptionalFieldOf`: an invalid type falls back to the default.
            param_set: j.get("type").and_then(|v| ident(v).ok()),
            random_sequence: opt(j, "random_sequence", ident)?,
            pools: opt_or(j, "pools", Vec::new(), |v| list(v, |pool| LootPool::parse(p, pool)))?,
            modifier: opt(j, "modifier", |v| Function::parse_ref(p, v))?,
        })
    }
}

impl<'a> Eval<'a> {
    /// `LootPool.addRandomItems`: the pool's condition, then `rolls + floor(bonus_rolls * luck)`
    /// weighted picks.
    pub fn pool_items(&mut self, pool: &LootPool, sink: &mut Sink<'_, 'a>) {
        if let Some(c) = &pool.condition
            && !self.test(c)
        {
            return;
        }
        let run = |ev: &mut Eval<'a>, sink: &mut Sink<'_, 'a>| {
            let rolls = ev.int(&pool.rolls);
            let bonus = ev.float(&pool.bonus_rolls);
            let n = rolls.wrapping_add(kiln_javamath::math::floor_f32(bonus * ev.ctx.luck()));
            for _ in 0..n {
                ev.pool_item(pool, sink);
            }
        };
        match &pool.modifier {
            Some(m) => {
                let m = m.clone();
                let mut decorated = |ev: &mut Eval<'a>, s: ItemStack| {
                    let s = ev.apply_fn(&m, s);
                    sink(ev, s);
                };
                run(self, &mut decorated);
            }
            None => run(self, sink),
        }
    }

    /// `LootPool.addRandomItem`: expand the entries, keep those with a positive weight, pick one.
    fn pool_item(&mut self, pool: &LootPool, sink: &mut Sink<'_, 'a>) {
        let luck = self.ctx.luck();
        let mut expanded: Vec<PoolEntry<'_>> = Vec::new();
        let mut kept: Vec<PoolEntry<'_>> = Vec::new();
        let mut total = 0i32;
        for e in &pool.entries {
            expanded.clear();
            self.expand_entry(e, &mut expanded);
            for pe in expanded.drain(..) {
                let w = pe.weight(luck);
                if w > 0 {
                    total = total.wrapping_add(w);
                    kept.push(pe);
                }
            }
        }
        if total == 0 || kept.is_empty() {
            return;
        }
        if kept.len() == 1 {
            self.create_items(&kept[0], sink);
            return;
        }
        let mut r = self.rng.bounded(total);
        for pe in &kept {
            r -= pe.weight(luck);
            if r < 0 {
                self.create_items(pe, sink);
                return;
            }
        }
    }
}

/// The random source of one table evaluation: an explicit seed's own source, or a borrowed one
/// (a random sequence or the level's random).
pub enum LootRandom<'r> {
    Seeded(LegacyRandom),
    Borrowed(&'r mut dyn RandomSource),
}

impl LootRandom<'_> {
    pub fn source(&mut self) -> &mut dyn RandomSource {
        match self {
            LootRandom::Seeded(r) => r,
            LootRandom::Borrowed(r) => *r,
        }
    }
}

impl LootTable {
    /// `LootContext.Builder.withOptionalRandomSeed(seed).create(randomSequence)`: a non-zero
    /// seed gets its own `LegacyRandomSource`; otherwise the table's random sequence, or the
    /// level's random for a table without one.
    pub fn random<'r>(
        &self,
        seed: i64,
        sequences: &'r mut RandomSequences,
        level: &'r mut dyn RandomSource,
    ) -> LootRandom<'r> {
        if seed != 0 {
            return LootRandom::Seeded(seeded(seed));
        }
        match &self.random_sequence {
            Some(id) => LootRandom::Borrowed(sequences.get(id)),
            None => LootRandom::Borrowed(level),
        }
    }
}

/// A table to evaluate: one of the data's tables by id, or a table decoded on its own (for
/// example with [`LootData::parse_table`]).
#[derive(Debug, Clone, Copy)]
pub enum TableRef<'t> {
    Id(&'t Identifier),
    Inline(&'t Arc<LootTable>),
}

impl<'t> From<&'t Identifier> for TableRef<'t> {
    fn from(id: &'t Identifier) -> Self {
        TableRef::Id(id)
    }
}

impl<'t> From<&'t Arc<LootTable>> for TableRef<'t> {
    fn from(t: &'t Arc<LootTable>) -> Self {
        TableRef::Inline(t)
    }
}

impl LootData {
    /// `LootTable.getRandomItemsRaw`: every stack the table produces, unsplit (empty stacks
    /// from `discard` included).
    pub fn random_items_raw<'t>(
        &self,
        table: impl Into<TableRef<'t>>,
        ctx: &dyn LootContext,
        rng: &mut dyn RandomSource,
        sink: &mut dyn FnMut(ItemStack),
    ) {
        let r = match table.into() {
            TableRef::Id(id) => match self.table_index(id) {
                Some(i) => Ref::Named(i),
                None => return,
            },
            TableRef::Inline(t) => Ref::Direct(t.clone()),
        };
        let mut ev = Eval::new(self, ctx, rng);
        ev.table_items_raw(&r, &mut |_ev: &mut Eval<'_>, s: ItemStack| sink(s));
    }

    /// `LootTable.getRandomItems`: the table's stacks, split to their maximum stack size and with
    /// disabled items removed.
    pub fn random_items<'t>(
        &self,
        table: impl Into<TableRef<'t>>,
        ctx: &dyn LootContext,
        rng: &mut dyn RandomSource,
    ) -> Vec<ItemStack> {
        let mut out = Vec::new();
        self.random_items_raw(table, ctx, rng, &mut |s| stack::split_stack(ctx, s, &mut |p| out.push(p)));
        out
    }

    /// `LootTable.fill`: generates the table's stacks and places them in random empty slots of
    /// `container`, splitting stacks to use the free space (`shuffleAndSplitItems`).
    pub fn fill<'t>(
        &self,
        table: impl Into<TableRef<'t>>,
        ctx: &dyn LootContext,
        rng: &mut dyn RandomSource,
        container: &mut [ItemStack],
    ) {
        let mut items = self.random_items(table, ctx, rng);
        let mut slots: Vec<usize> = (0..container.len()).filter(|&i| container[i].is_empty()).collect();
        shuffle(&mut slots, rng);
        shuffle_and_split(&mut items, slots.len(), rng);
        for item in items {
            let Some(slot) = slots.pop() else { return };
            container[slot] = if item.is_empty() { ItemStack::empty() } else { item };
        }
    }
}

/// `Util.shuffle`.
pub fn shuffle<T>(list: &mut [T], rng: &mut dyn RandomSource) {
    let mut i = list.len();
    while i > 1 {
        let j = rng.bounded(i as i32) as usize;
        list.swap(i - 1, j);
        i -= 1;
    }
}

/// `LootTable.shuffleAndSplitItems`.
pub fn shuffle_and_split(items: &mut Vec<ItemStack>, empty_slots: usize, rng: &mut dyn RandomSource) {
    let mut split = Vec::new();
    items.retain(|s| {
        if s.is_empty() {
            return false;
        }
        if s.count() > 1 {
            split.push(s.clone());
            return false;
        }
        true
    });
    while empty_slots as i64 - items.len() as i64 - split.len() as i64 > 0 && !split.is_empty() {
        let idx = rng.next_int_between(0, split.len() as i32 - 1) as usize;
        let mut s: ItemStack = split.remove(idx);
        let n = rng.next_int_between(1, s.count() / 2);
        let part = s.split(n);
        if s.count() > 1 && rng.next_bool() {
            split.push(s);
        } else {
            items.push(s);
        }
        if part.count() > 1 && rng.next_bool() {
            split.push(part);
        } else {
            items.push(part);
        }
    }
    items.append(&mut split);
    shuffle(items, rng);
}
