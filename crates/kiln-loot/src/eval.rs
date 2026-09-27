//! The evaluation state vanilla's `LootContext` carries: the context, the random source and the
//! tables being visited (for loop detection).

use crate::context::{EntityTarget, LootContext};
use crate::data::LootData;
use crate::parse::Ref;
use crate::table::LootTable;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use std::sync::Arc;

/// A consumer of produced stacks that may itself evaluate (item modifiers do).
pub type Sink<'s, 'a> = dyn FnMut(&mut Eval<'a>, ItemStack) + 's;

pub struct Eval<'a> {
    pub data: &'a LootData,
    pub ctx: &'a dyn LootContext,
    pub rng: &'a mut dyn RandomSource,
    /// `LootContext.visitedElements` (tables only: nothing else is tracked at run time).
    visited: Vec<(bool, usize)>,
}

impl<'a> Eval<'a> {
    pub fn new(data: &'a LootData, ctx: &'a dyn LootContext, rng: &'a mut dyn RandomSource) -> Self {
        Eval { data, ctx, rng, visited: Vec::new() }
    }

    /// `EnchantmentHelper.getEnchantmentLevel(enchantment, living entity)` for a target.
    pub fn entity_enchantment_level(&self, target: EntityTarget, enchantment: i32) -> i32 {
        if !self.ctx.has_entity(target) {
            return 0;
        }
        let slots = self.data.enchantment(enchantment).map_or(&[][..], |e| e.slots.as_slice());
        self.ctx.entity_enchantment_level(target, enchantment, slots)
    }

    /// `LootTable.getRandomItemsRaw(context, output)`: the table's pools through its modifier;
    /// a table already being evaluated yields nothing ("Detected infinite loop").
    pub fn table_items_raw(&mut self, table: &Ref<LootTable>, sink: &mut Sink<'_, 'a>) {
        let data = self.data;
        let (key, t): ((bool, usize), &LootTable) = match table {
            Ref::Direct(t) => ((false, Arc::as_ptr(t) as usize), t),
            Ref::Named(i) => match data.tables.get(*i) {
                Some(t) => ((true, *i), t),
                None => return,
            },
        };
        if self.visited.contains(&key) {
            return;
        }
        self.visited.push(key);
        match &t.modifier {
            Some(m) => {
                let m = m.clone();
                let mut decorated = |ev: &mut Eval<'a>, s: ItemStack| {
                    let s = ev.apply_fn(&m, s);
                    sink(ev, s);
                };
                for pool in &t.pools {
                    self.pool_items(pool, &mut decorated);
                }
            }
            None => {
                for pool in &t.pools {
                    self.pool_items(pool, sink);
                }
            }
        }
        self.visited.retain(|k| *k != key);
    }
}
