//! Loot pool entries (`LootPoolEntryContainer` and the `LootPoolEntry` choices they expand to).

use crate::condition::Condition;
use crate::eval::{Eval, Sink};
use crate::function::Function;
use crate::json::Json;
use crate::parse::{IdSet, PResult, Parser, Ref, boolean, fail, ident, int, list, opt, opt_or, req};
use crate::slot::SlotSource;
use crate::table::LootTable;
use kiln_item::registry;
use kiln_item::{Identifier, ItemStack};

#[derive(Debug, Clone)]
pub enum EntryKind {
    Empty,
    Item(i32),
    /// `loot_table`: one or more tables (`value`), each its own choice when expanded.
    LootTable { tables: Vec<Ref<LootTable>>, expand: bool },
    Dynamic(Identifier),
    Tag { items: IdSet, expand: bool },
    Slots(Ref<SlotSource>),
    Alternatives(Vec<Entry>),
    Sequence(Vec<Entry>),
    Group(Vec<Entry>),
}

/// A pool entry container with its condition, modifier, weight and quality.
#[derive(Debug, Clone)]
pub struct Entry {
    pub condition: Option<Ref<Condition>>,
    pub modifier: Option<Ref<Function>>,
    pub weight: i32,
    pub quality: i32,
    pub kind: EntryKind,
}

/// What a chosen [`PoolEntry`] produces.
#[derive(Debug, Clone, Copy)]
pub enum Choice<'e> {
    /// A singleton entry (item, empty, dynamic, slots).
    Single(&'e EntryKind),
    /// One item of an expanded tag.
    TagItem(i32),
    /// Every item of an unexpanded tag.
    TagAll(&'e IdSet),
    /// One table of an expanded `loot_table` entry.
    Table(&'e Ref<LootTable>),
    /// Every table of an unexpanded `loot_table` entry.
    TableAll(&'e [Ref<LootTable>]),
}

/// `LootPoolEntry`: a weighted choice and the modifiers wrapped around it, innermost first.
#[derive(Debug, Clone)]
pub struct PoolEntry<'e> {
    pub weight: i32,
    pub quality: i32,
    pub modifiers: Vec<&'e Ref<Function>>,
    pub choice: Choice<'e>,
}

impl PoolEntry<'_> {
    /// `UniformContainerBase.EntryBase.getWeight`: `max(floor(weight + quality * luck), 0)`.
    pub fn weight(&self, luck: f32) -> i32 {
        kiln_javamath::math::floor_f32(self.weight as f32 + self.quality as f32 * luck).max(0)
    }
}

impl Entry {
    /// `LootPoolEntries.CODEC`: dispatched on `type`.
    pub fn parse(p: &Parser, j: &Json) -> PResult<Entry> {
        let ty = req(j, "type", ident)?;
        let condition = opt(j, "condition", |v| Condition::parse_ref(p, v))?;
        let modifier = opt(j, "modifier", |v| Function::parse_ref(p, v))?;
        let children = || opt_or(j, "children", Vec::new(), |v| list(v, |c| Entry::parse(p, c)));
        let composite = matches!(ty.as_str(), "minecraft:alternatives" | "minecraft:sequence" | "minecraft:group");
        let (weight, quality) =
            if composite { (1, 0) } else { (opt_or(j, "weight", 1, int)?, opt_or(j, "quality", 0, int)?) };
        let expand = || opt_or(j, "expand", false, boolean);
        let kind = match ty.as_str() {
            "minecraft:empty" => EntryKind::Empty,
            "minecraft:item" => EntryKind::Item(req(j, "name", |v| p.id(v, registry::ITEM))?),
            "minecraft:loot_table" => EntryKind::LootTable {
                tables: req(j, "value", |v| p.holder_list(v, crate::data::Kind::Table, LootTable::parse))?,
                expand: expand()?,
            },
            "minecraft:dynamic" => EntryKind::Dynamic(req(j, "name", ident)?),
            "minecraft:tag" => EntryKind::Tag { items: req(j, "items", |v| p.id_set(v, registry::ITEM))?, expand: expand()? },
            "minecraft:slots" => EntryKind::Slots(req(j, "slot_source", |v| SlotSource::parse_ref(p, v))?),
            "minecraft:alternatives" => EntryKind::Alternatives(children()?),
            "minecraft:sequence" => EntryKind::Sequence(children()?),
            "minecraft:group" => EntryKind::Group(children()?),
            other => return fail(format!("unknown loot pool entry type {other}")),
        };
        Ok(Entry { condition, modifier, weight, quality, kind })
    }
}

impl<'a> Eval<'a> {
    /// `LootPoolEntryContainer.expand`: the entries this container offers, or `false` when
    /// its condition fails (which composites use to decide).
    pub fn expand_entry<'e>(&mut self, e: &'e Entry, out: &mut Vec<PoolEntry<'e>>) -> bool {
        if let Some(c) = &e.condition
            && !self.test(c)
        {
            return false;
        }
        let start = out.len();
        let base = |choice: Choice<'e>| PoolEntry { weight: e.weight, quality: e.quality, modifiers: Vec::new(), choice };
        let result = match &e.kind {
            EntryKind::Empty | EntryKind::Item(_) | EntryKind::Dynamic(_) | EntryKind::Slots(_) => {
                out.push(base(Choice::Single(&e.kind)));
                true
            }
            EntryKind::Tag { items, expand } => {
                if *expand {
                    out.extend(items.ids().iter().map(|&id| base(Choice::TagItem(id))));
                } else {
                    out.push(base(Choice::TagAll(items)));
                }
                true
            }
            EntryKind::LootTable { tables, expand } => {
                if *expand {
                    out.extend(tables.iter().map(|t| base(Choice::Table(t))));
                } else {
                    out.push(base(Choice::TableAll(tables)));
                }
                true
            }
            EntryKind::Alternatives(children) => {
                let mut any = false;
                for c in children {
                    if self.expand_entry(c, out) {
                        any = true;
                        break;
                    }
                }
                any
            }
            EntryKind::Sequence(children) => {
                let mut all = true;
                for c in children {
                    if !self.expand_entry(c, out) {
                        all = false;
                        break;
                    }
                }
                all
            }
            EntryKind::Group(children) => {
                for c in children {
                    self.expand_entry(c, out);
                }
                true
            }
        };
        if let Some(m) = &e.modifier {
            for pe in &mut out[start..] {
                pe.modifiers.push(m);
            }
        }
        result
    }

    /// `LootPoolEntry.createItemStack` through the entry's modifiers.
    pub fn create_items(&mut self, pe: &PoolEntry<'_>, sink: &mut Sink<'_, 'a>) {
        let modifiers = pe.modifiers.clone();
        let mut decorated = |ev: &mut Eval<'a>, stack: ItemStack| {
            let mut s = stack;
            for m in &modifiers {
                s = ev.apply_fn(m, s);
            }
            sink(ev, s);
        };
        match pe.choice {
            Choice::Single(EntryKind::Item(item)) => decorated(self, ItemStack::new(*item, 1)),
            Choice::Single(EntryKind::Dynamic(name)) => {
                let ctx = self.ctx;
                let mut items = Vec::new();
                ctx.dynamic_drops(name, &mut |s| items.push(s));
                for s in items {
                    decorated(self, s);
                }
            }
            Choice::Single(EntryKind::Slots(source)) => {
                let items = self.slot_items(source);
                for s in items.into_iter().filter(|s| !s.is_empty()) {
                    decorated(self, s);
                }
            }
            Choice::Single(_) => {}
            Choice::TagItem(id) => decorated(self, ItemStack::new(id, 1)),
            Choice::TagAll(set) => {
                for &id in set.ids() {
                    decorated(self, ItemStack::new(id, 1));
                }
            }
            Choice::Table(t) => self.table_items_raw(t, &mut decorated),
            Choice::TableAll(tables) => {
                for t in tables {
                    self.table_items_raw(t, &mut decorated);
                }
            }
        }
    }
}
