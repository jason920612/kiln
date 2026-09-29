//! `item` (vanilla `ItemCommands`): `replace`, `fill`, `override` and `modify` on the slots a
//! slot source selects in a container block or in entities.
//!
//! The pieces follow vanilla's `SlotCollection` (here [`SlotTree`]), `SlotSelector`,
//! `ItemProvider` and `CommandResponseTracker`.

use super::blocks::loaded_block_pos;
use super::items::{item_count, item_id};
use super::loot::{display_name, item_input_nbt};
use super::tracker::{Counted, Tracker};
use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{argument, literal, Builder, CommandContext, Dispatcher};
use crate::error::CommandError;
use crate::host::{Host, ItemHolder, LootTableArg, SlotTree};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use kiln_proto::nbt::Tag;
use std::cell::Cell;
use std::rc::Rc;

type Result<T> = std::result::Result<T, CommandError>;
type Pred = Rc<dyn Fn(&Tag) -> bool>;

fn empty() -> Tag {
    Tag::Compound(vec![("id".into(), Tag::String("minecraft:air".into())), ("count".into(), Tag::Int(0))])
}

fn is_empty(item: &Tag) -> bool {
    item_id(item).is_none_or(|id| id == "minecraft:air") || item_count(item) <= 0
}

/// `SlotSelector`: which slots take part, and how many are left (`TrackingSelector`; the count is
/// shared by every selector derived from it).
#[derive(Clone)]
struct Selector {
    preds: Vec<Pred>,
    limit: i32,
    count: Rc<Cell<i32>>,
}

impl Selector {
    fn tracking(base: Option<Pred>, count: Rc<Cell<i32>>) -> Self {
        Selector { preds: base.into_iter().collect(), limit: i32::MAX, count }
    }

    fn try_select(&self, item: &Tag) -> bool {
        if self.count.get() < self.limit && self.preds.iter().all(|p| p(item)) {
            self.count.set(self.count.get() + 1);
            true
        } else {
            false
        }
    }

    fn filter(&self, p: Pred) -> Selector {
        let mut s = self.clone();
        s.preds.push(p);
        s
    }

    fn limit(&self, n: usize) -> Selector {
        let n = n.min(i32::MAX as usize) as i32;
        let mut s = self.clone();
        if n < s.limit {
            s.limit = n;
        }
        s
    }
}

/// `ItemProvider`: `of` (each stack once), `cycle`, either with `orElseProvide`.
struct Provider {
    items: Vec<Tag>,
    pos: usize,
    cycle: bool,
    fallback: Option<Tag>,
    using_fallback: bool,
}

impl Provider {
    fn inner_has_next(&self) -> bool {
        if self.cycle { !self.items.is_empty() } else { self.pos < self.items.len() }
    }

    fn has_next(&self) -> bool {
        self.fallback.is_some() || self.inner_has_next()
    }

    fn restart(&mut self) {
        self.pos = 0;
        self.using_fallback = false;
    }

    fn next(&mut self) -> Tag {
        if !self.using_fallback {
            if self.inner_has_next() {
                let i = if self.cycle { self.pos % self.items.len() } else { self.pos };
                self.pos += 1;
                return self.items[i].clone();
            }
            self.using_fallback = true;
        }
        self.fallback.clone().unwrap_or_else(empty)
    }
}

fn get<S: Host>(s: &mut S, holder: &ItemHolder<S::Entity>, slot: i32) -> Tag {
    match s.slot_item(holder, slot) {
        Some(Some(t)) => t,
        _ => empty(),
    }
}

fn set<S: Host>(s: &mut S, holder: &ItemHolder<S::Entity>, slot: i32, item: &Tag) -> bool {
    s.set_slot_item(holder, slot, (!is_empty(item)).then_some(item))
}

/// `SlotCollection.replaceSlotItems`.
fn replace<S: Host>(tree: &SlotTree<S::Entity>, s: &mut S, p: &mut Provider, sel: &Selector) -> i32 {
    match tree {
        SlotTree::Empty => 0,
        SlotTree::Slots(slots) => {
            let mut n = 0;
            for (holder, slot) in slots {
                if !p.has_next() {
                    break;
                }
                let current = get(s, holder, *slot);
                if sel.try_select(&current) {
                    let item = p.next();
                    if set(s, holder, *slot, &item) {
                        n += 1;
                    }
                }
            }
            n
        }
        SlotTree::Concat(parts) => parts.iter().map(|t| replace(t, s, p, sel)).sum(),
        SlotTree::Filtered(inner, f) => replace(inner, s, p, &sel.filter(f.clone())),
        SlotTree::Limited(inner, n) => replace(inner, s, p, &sel.limit(*n)),
    }
}

/// `SlotCollection.modifySlots`.
fn modify_slots<S: Host>(tree: &SlotTree<S::Entity>, s: &mut S, sel: &Selector, f: &mut dyn FnMut(&mut S, &ItemHolder<S::Entity>, i32)) {
    match tree {
        SlotTree::Empty => {}
        SlotTree::Slots(slots) => {
            for (holder, slot) in slots {
                let current = get(s, holder, *slot);
                if sel.try_select(&current) {
                    f(s, holder, *slot);
                }
            }
        }
        SlotTree::Concat(parts) => {
            for t in parts {
                modify_slots(t, s, sel, f);
            }
        }
        SlotTree::Filtered(inner, p) => modify_slots(inner, s, &sel.filter(p.clone()), f),
        SlotTree::Limited(inner, n) => modify_slots(inner, s, &sel.limit(*n), f),
    }
}

/// `SlotCollection.itemCopies`.
pub(super) fn item_copies<S: Host>(tree: &SlotTree<S::Entity>, s: &mut S) -> Vec<Tag> {
    match tree {
        SlotTree::Empty => Vec::new(),
        SlotTree::Slots(slots) => slots.iter().map(|(h, slot)| get(s, h, *slot)).collect(),
        SlotTree::Concat(parts) => parts.iter().flat_map(|t| item_copies(t, s)).collect(),
        SlotTree::Filtered(inner, p) => item_copies(inner, s).into_iter().filter(|t| p(t)).collect(),
        SlotTree::Limited(inner, n) => {
            let mut v = item_copies(inner, s);
            v.truncate(*n);
            v
        }
    }
}

/// `SlotCollection.size`.
pub(super) fn tree_size<S: Host>(tree: &SlotTree<S::Entity>, s: &mut S) -> i32 {
    match tree {
        SlotTree::Empty => 0,
        SlotTree::Slots(slots) => slots.len() as i32,
        SlotTree::Concat(parts) => parts.iter().map(|t| tree_size(t, s)).sum(),
        SlotTree::Filtered(..) => item_copies(tree, s).len() as i32,
        SlotTree::Limited(inner, n) => tree_size(inner, s).min(*n as i32),
    }
}

/// `SlotSourceArgument.Result`: a slot range or a slot source, and the name errors show.
pub(super) enum SlotsArg {
    Range { name: String, ids: &'static [i32] },
    Source { name: Option<String>, source: LootTableArg },
}

impl SlotsArg {
    pub(super) fn read<S: Host>(c: &CommandContext<S>, name: &str) -> SlotsArg {
        let text = c.string(name);
        if let Some(ids) = crate::slots::by_name(text) {
            return SlotsArg::Range { name: text.to_owned(), ids };
        }
        if text.starts_with(['{', '[', '"', '\'']) {
            let source = crate::snbt::parse_tag(&mut crate::reader::StringReader::new(text)).unwrap_or(Tag::Compound(Vec::new()));
            if let Tag::String(id) = &source {
                let id = crate::types::Identifier::parse(id).map_or_else(|| id.clone(), |i| i.to_string());
                return SlotsArg::Source { name: Some(id.clone()), source: LootTableArg::Id(id) };
            }
            return SlotsArg::Source { name: None, source: LootTableArg::Inline(source) };
        }
        let id = crate::types::Identifier::parse(text).map_or_else(|| text.to_owned(), |i| i.to_string());
        SlotsArg::Source { name: Some(id.clone()), source: LootTableArg::Id(id) }
    }

    fn name(&self) -> Option<&str> {
        match self {
            SlotsArg::Range { name, .. } => Some(name),
            SlotsArg::Source { name, .. } => name.as_deref(),
        }
    }

    /// `getSlotsFromProvider` for one slot owner.
    pub(super) fn tree<S: Host>(&self, s: &mut S, holder: &ItemHolder<S::Entity>) -> Result<SlotTree<S::Entity>> {
        match self {
            SlotsArg::Range { ids, .. } => {
                let mut slots = Vec::new();
                for &id in *ids {
                    if s.slot_item(holder, id).is_some() {
                        slots.push((holder.clone(), id));
                    }
                }
                Ok(SlotTree::Slots(slots))
            }
            SlotsArg::Source { source, .. } => s.slot_source_tree(source, holder),
        }
    }
}

/// `ItemAccessor`: a container block or entities.
pub(super) enum Accessor<E> {
    Block { dimension: String, pos: [i32; 3] },
    Entities(Vec<E>),
}

impl<E: Clone> Accessor<E> {
    /// `BlockItemAccessor.getContainer`.
    fn check_container<S: Host<Entity = E>>(&self, s: &mut S, error: &'static str) -> Result<()> {
        if let Accessor::Block { dimension, pos } = self
            && s.container_size(dimension, *pos).is_none()
        {
            return Err(CommandError::new(tr!(error, pos[0], pos[1], pos[2])));
        }
        Ok(())
    }

    /// `getSlots`: the slots of every target, as one collection.
    pub(super) fn slots<S: Host<Entity = E>>(&self, s: &mut S, arg: &SlotsArg, source_side: bool) -> Result<SlotTree<E>> {
        match self {
            Accessor::Block { dimension, pos } => {
                self.check_container(s, if source_side { "commands.item.source.not_a_container" } else { "commands.item.target.not_a_container" })?;
                arg.tree(s, &ItemHolder::Block { dimension: dimension.clone(), pos: *pos })
            }
            Accessor::Entities(entities) => {
                let mut parts = Vec::new();
                for e in entities {
                    parts.push(arg.tree(s, &ItemHolder::Entity(e.clone()))?);
                }
                Ok(SlotTree::Concat(parts))
            }
        }
    }

    pub(super) fn read<S: Host<Entity = E>>(c: &CommandContext<S>, s: &mut S, block: bool, name: &str) -> Result<Accessor<E>> {
        if block {
            let dimension = s.dimension().to_owned();
            let pos = loaded_block_pos(c, s, name, &dimension)?;
            Ok(Accessor::Block { dimension, pos })
        } else {
            Ok(Accessor::Entities(c.selector(name).entities(s)?))
        }
    }
}

/// `ItemAccessor.setItems`: calls `f` with every target and its slots.
fn set_items<S: Host>(
    target: &Accessor<S::Entity>,
    s: &mut S,
    arg: &SlotsArg,
    f: &mut dyn FnMut(&mut S, Option<&S::Entity>, &SlotTree<S::Entity>) -> i32,
) -> Result<()> {
    match target {
        Accessor::Block { .. } => {
            let tree = target.slots(s, arg, false)?;
            f(s, None, &tree);
        }
        Accessor::Entities(entities) => {
            for e in entities {
                let tree = arg.tree(s, &ItemHolder::Entity(e.clone()))?;
                let n = f(s, Some(e), &tree);
                if n > 0 && e.is_player() {
                    s.inventory_changed(e);
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Mode {
    Replace,
    Fill,
    Override,
}

/// What a tracker element is: a block position or an entity.
#[derive(Clone)]
enum Element<E> {
    Block([i32; 3]),
    Entity(E),
}

fn inapplicable(arg: &SlotsArg, source_side: bool) -> CommandError {
    let (named, unnamed) = if source_side {
        ("commands.item.source.no_such_slot", "commands.item.source.no_such_slot.unnamed")
    } else {
        ("commands.item.target.no_such_slot", "commands.item.target.no_such_slot.unnamed")
    };
    match arg.name() {
        Some(n) => CommandError::new(tr!(named, n)),
        None => CommandError::new(tr!(unnamed)),
    }
}

fn track_target<E: Clone>(t: &mut Tracker<Element<E>>, target: &Accessor<E>, entity: Option<&E>, n: i32) {
    let element = match (target, entity) {
        (Accessor::Block { pos, .. }, _) => Element::Block(*pos),
        (_, Some(e)) => Element::Entity(e.clone()),
        _ => return,
    };
    t.track(&element, n);
}

/// The feedback of `getReplaceSuccess`.
fn replace_feedback<S: Host>(s: &mut S, t: &Tracker<Element<S::Entity>>, block_target: bool, known: Option<&Tag>) -> Result<i32> {
    let block = |e: &Element<S::Entity>| match e {
        Element::Block(p) => *p,
        Element::Entity(_) => [0; 3],
    };
    let name = |e: &Element<S::Entity>| match e {
        Element::Entity(e) => e.display_name(),
        Element::Block(_) => Text::literal(""),
    };
    let is_block = block_target;
    let item_name = known.map(|k| display_name(s, k));
    let error = match &item_name {
        Some(n) => CommandError::new(tr!("commands.item.target.failed.known_item", n.clone())),
        None => CommandError::new(tr!("commands.item.target.failed")),
    };
    let text = t.message(
        Counted::NonZero,
        |e, total| {
            if is_block {
                let [x, y, z] = block(e);
                match &item_name {
                    Some(n) => tr!("commands.item.block.replace.success.known_item", total, x, y, z, n.clone()),
                    None => tr!("commands.item.block.replace.success", total, x, y, z),
                }
            } else {
                match &item_name {
                    Some(n) => tr!("commands.item.entity.replace.success.single.known_item", total, name(e), n.clone()),
                    None => tr!("commands.item.entity.replace.success.single", total, name(e)),
                }
            }
        },
        |count, _| match &item_name {
            Some(n) => tr!("commands.item.entity.replace.success.multiple.known_item", count, n.clone()),
            None => tr!("commands.item.entity.replace.success.multiple", count),
        },
    );
    if t.count(Counted::NonZero) == 0 {
        return Err(error);
    }
    s.send_success(text, true);
    Ok(t.total())
}

/// The feedback of `getModifySuccess`.
fn modify_feedback<S: Host>(s: &mut S, t: &Tracker<Element<S::Entity>>) -> Result<i32> {
    if t.count(Counted::NonZero) == 0 {
        return Err(CommandError::new(tr!("commands.item.target.failed")));
    }
    let text = t.message(
        Counted::NonZero,
        |e, total| match e {
            Element::Block([x, y, z]) => tr!("commands.item.block.modify.success", total, *x, *y, *z),
            Element::Entity(e) => tr!("commands.item.entity.modify.success.single", total, e.display_name()),
        },
        |count, _| tr!("commands.item.entity.modify.success.multiple", count),
    );
    s.send_success(text, true);
    Ok(t.total())
}

/// `ItemCommands.setItems`.
fn do_set<S: Host>(
    s: &mut S,
    target: &Accessor<S::Entity>,
    arg: &SlotsArg,
    mode: Mode,
    items: Vec<Tag>,
    known: Option<&Tag>,
) -> Result<i32> {
    let selected = Rc::new(Cell::new(0));
    let selector = Selector::tracking(None, selected.clone());
    let mut provider = match mode {
        Mode::Replace => Provider { items, pos: 0, cycle: false, fallback: None, using_fallback: false },
        Mode::Fill => Provider { items, pos: 0, cycle: true, fallback: None, using_fallback: false },
        Mode::Override => Provider { items, pos: 0, cycle: false, fallback: Some(empty()), using_fallback: false },
    };
    let mut tracker = Tracker::new();
    set_items(target, s, arg, &mut |s, entity, tree| {
        provider.restart();
        let n = replace(tree, s, &mut provider, &selector);
        track_target(&mut tracker, target, entity, n);
        n
    })?;
    if selected.get() == 0 {
        return Err(inapplicable(arg, false));
    }
    replace_feedback(s, &tracker, matches!(target, Accessor::Block { .. }), known)
}

/// `ItemCommands.getItems`: the source slots' items (through the modifier).
fn get_items<S: Host>(
    s: &mut S,
    source: &Accessor<S::Entity>,
    arg: &SlotsArg,
    modifier: Option<&LootTableArg>,
) -> Result<Vec<Tag>> {
    let tree = source.slots(s, arg, true)?;
    let mut items = item_copies(&tree, s);
    if let Some(m) = modifier {
        for item in &mut items {
            *item = s.apply_item_modifier(m, item)?;
        }
    }
    if items.is_empty() {
        return Err(inapplicable(arg, true));
    }
    Ok(items)
}

fn do_modify<S: Host>(s: &mut S, target: &Accessor<S::Entity>, arg: &SlotsArg, modifier: &LootTableArg) -> Result<i32> {
    let selected = Rc::new(Cell::new(0));
    let non_empty: Pred = Rc::new(|t: &Tag| !is_empty(t));
    let selector = Selector::tracking(Some(non_empty), selected.clone());
    let mut tracker = Tracker::new();
    let mut failure: Option<CommandError> = None;
    set_items(target, s, arg, &mut |s, entity, tree| {
        let mut changed = 0;
        modify_slots(tree, s, &selector, &mut |s, holder, slot| {
            let current = get(s, holder, slot);
            match s.apply_item_modifier(modifier, &current) {
                Ok(new) => {
                    if set(s, holder, slot, &new) {
                        changed += 1;
                    }
                }
                Err(e) => failure = failure.take().or(Some(e)),
            }
        });
        track_target(&mut tracker, target, entity, changed);
        changed
    })?;
    if let Some(e) = failure {
        return Err(e);
    }
    if selected.get() == 0 {
        return Err(inapplicable(arg, false));
    }
    modify_feedback(s, &tracker)
}

fn modifier_arg<S: Host>(c: &CommandContext<S>) -> LootTableArg {
    super::loot::table_arg(c, "modifier")
}

/// `replace`, `fill` and `override`: the `from` and `with` branches of a slots argument.
fn set_branch<S: Host + 'static>(mode: Mode, block: bool) -> Builder<S> {
    let target_arg = if block { argument("target", ArgumentType::BlockPos) } else { argument("target", ArgumentType::entities()) };
    let mut slots = argument("slots", ArgumentType::SlotSource);
    for source_block in [false, true] {
        let source_arg = if source_block { argument("source", ArgumentType::BlockPos) } else { argument("source", ArgumentType::entities()) };
        let run = move |c: &CommandContext<S>, s: &mut S, with_modifier: bool| -> Result<i32> {
            let target = Accessor::read(c, s, block, "target")?;
            let arg = SlotsArg::read(c, "slots");
            let source = Accessor::read(c, s, source_block, "source")?;
            let source_arg = SlotsArg::read(c, "sourceSlots");
            let modifier = with_modifier.then(|| modifier_arg(c));
            let items = get_items(s, &source, &source_arg, modifier.as_ref())?;
            do_set(s, &target, &arg, mode, items, None)
        };
        let from = literal("from").then(
            literal(if source_block { "block" } else { "entity" }).then(
                source_arg.then(
                    argument("sourceSlots", ArgumentType::SlotSource)
                        .executes(move |c, s: &mut S| run(c, s, false))
                        .then(argument("modifier", ArgumentType::LootResource { registry: "minecraft:item_modifier" }).executes(move |c, s: &mut S| run(c, s, true))),
                ),
            ),
        );
        slots = slots.then(from);
    }
    let with = move |c: &CommandContext<S>, s: &mut S, count: i32| -> Result<i32> {
        let target = Accessor::read(c, s, block, "target")?;
        let arg = SlotsArg::read(c, "slots");
        let input = c.item("item");
        let max = s.max_stack_size(input);
        if count > max {
            return Err(CommandError::new(tr!("arguments.item.overstacked", input.item.to_string(), max)));
        }
        let item = item_input_nbt(input, count);
        do_set(s, &target, &arg, mode, vec![item.clone()], Some(&item))
    };
    slots = slots.then(
        literal("with").then(
            argument("item", ArgumentType::ItemStack)
                .executes(move |c, s: &mut S| with(c, s, 1))
                .then(argument("count", ArgumentType::integer_range(1, 99)).executes(move |c, s: &mut S| with(c, s, c.integer("count")))),
        ),
    );
    literal(if block { "block" } else { "entity" }).then(target_arg.then(slots))
}

fn modify_branch<S: Host + 'static>(block: bool) -> Builder<S> {
    let target_arg = if block { argument("target", ArgumentType::BlockPos) } else { argument("target", ArgumentType::entities()) };
    literal(if block { "block" } else { "entity" }).then(target_arg.then(argument("slots", ArgumentType::SlotSource).then(
        argument("modifier", ArgumentType::LootResource { registry: "minecraft:item_modifier" }).executes(move |c, s: &mut S| {
            let target = Accessor::read(c, s, block, "target")?;
            let arg = SlotsArg::read(c, "slots");
            let modifier = modifier_arg(c);
            do_modify(s, &target, &arg, &modifier)
        }),
    )))
}

pub fn item<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let mut root = literal("item").requires(LEVEL_GAMEMASTERS);
    for (name, mode) in [("replace", Mode::Replace), ("fill", Mode::Fill), ("override", Mode::Override)] {
        root = root.then(literal(name).then(set_branch(mode, false)).then(set_branch(mode, true)));
    }
    root = root.then(literal("modify").then(modify_branch(false)).then(modify_branch(true)));
    d.register(root);
}
