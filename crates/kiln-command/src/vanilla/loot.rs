//! `loot` (vanilla `LootCommand`): items from a loot table, a fishing roll, a kill or a mined
//! block, dropped, given, inserted into a container or put into slots.

use super::blocks::loaded_block_pos;
use super::items::{item_count, item_id, with_count};
use super::tracker::{Counted, Tracker};
use super::{LEVEL_GAMEMASTERS, source_entity};
use crate::arguments::{ArgumentType, ArgumentValue};
use crate::dispatcher::{Builder, CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, ItemHolder, LootSource, LootTableArg};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use crate::types::ItemInput;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// A `loot_table` / `loot_modifier` argument.
pub(super) fn table_arg<S: Host>(c: &CommandContext<S>, name: &str) -> LootTableArg {
    match c.get(name) {
        Some(ArgumentValue::Nbt(Tag::String(id))) => {
            LootTableArg::Id(crate::types::Identifier::parse(id).map_or_else(|| id.clone(), |i| i.to_string()))
        }
        Some(ArgumentValue::Nbt(t)) => LootTableArg::Inline(t.clone()),
        Some(ArgumentValue::Identifier(id)) => LootTableArg::Id(id.to_string()),
        other => unreachable!("loot table argument {other:?}"),
    }
}

/// `ResourceOrIdArgument.getResource`: an id must name an element of `registry`; the error
/// points after the argument.
pub(super) fn resource_arg<S: Host>(c: &CommandContext<S>, s: &S, name: &str, registry: &str) -> Result<LootTableArg> {
    let arg = table_arg(c, name);
    if let LootTableArg::Id(id) = &arg
        && !s.registry_ids(registry).iter().any(|r| r == id)
    {
        let text = c.arg_text(name).unwrap_or("");
        let end = (text.as_ptr() as usize).saturating_sub(c.input().as_ptr() as usize) + text.len();
        return Err(CommandError::new(tr!("argument.resource_or_id.no_such_element", id.as_str(), registry)).with_context(c.input(), end));
    }
    Ok(arg)
}

/// `ItemInput.createItemStack(1)` as item stack NBT.
pub(super) fn item_input_nbt(item: &ItemInput, count: i32) -> Tag {
    let mut fields = vec![("id".to_owned(), Tag::String(item.item.to_string())), ("count".to_owned(), Tag::Int(count))];
    if !item.components.is_empty() {
        let comps = item
            .components
            .iter()
            .map(|(id, snbt)| match snbt {
                Some(s) => {
                    let v = crate::snbt::parse_tag(&mut crate::reader::StringReader::new(s)).unwrap_or(Tag::Compound(Vec::new()));
                    (id.to_string(), v)
                }
                None => (format!("!{id}"), Tag::Compound(Vec::new())),
            })
            .collect();
        fields.push(("components".to_owned(), Tag::Compound(comps)));
    }
    Tag::Compound(fields)
}

/// `ItemStack.isSameItemSameComponents` on item stack NBT.
pub(super) fn same_item(a: &Tag, b: &Tag) -> bool {
    let none = Tag::Compound(Vec::new());
    item_id(a) == item_id(b) && crate::nbt_path::nbt_eq(a.get("components").unwrap_or(&none), b.get("components").unwrap_or(&none))
}

/// `ItemStack.getDisplayName`: the hover name in brackets.
pub(super) fn display_name<S: Host>(s: &S, item: &Tag) -> Text {
    if item_id(item).is_none_or(|id| id == "minecraft:air") || item_count(item) <= 0 {
        return Text::translate("block.minecraft.air", Vec::new()).bracketed();
    }
    s.item_name(item).bracketed()
}

/// `getSourceHandItem`.
fn hand_item<S: Host>(s: &mut S, offhand: bool) -> Result<Option<Tag>> {
    let e = source_entity(s)?;
    match s.hand_item(&e, offhand) {
        Some(item) => Ok(item),
        None => Err(CommandError::new(tr!("commands.drop.no_held_items", e.display_name()))),
    }
}

/// What a source rolled: the stacks (empty stacks as `None`) and the table named.
struct Drops {
    items: Vec<Tag>,
    table: Option<String>,
}

fn roll<S: Host>(s: &mut S, source: LootSource<S::Entity>) -> Result<Drops> {
    let (items, table) = s.roll_loot(&source)?;
    Ok(Drops { items, table })
}

/// The empty stack (`ItemStack.EMPTY`) as NBT.
fn empty() -> Tag {
    Tag::Compound(vec![("id".into(), Tag::String("minecraft:air".into())), ("count".into(), Tag::Int(0))])
}

/// A target: puts the drops somewhere and tracks each stack placed.
type Target<S> = fn(&CommandContext<S>, &mut S, &[Tag], &mut Tracker<Tag>) -> Result<()>;

fn spawn<S: Host>(c: &CommandContext<S>, s: &mut S, items: &[Tag], t: &mut Tracker<Tag>) -> Result<()> {
    let pos = s.stack().resolve(c.coordinates("targetPos"));
    let dim = s.dimension().to_owned();
    for item in items {
        s.spawn_item(&dim, pos, item);
        t.track(item, 1);
    }
    Ok(())
}

fn give<S: Host>(c: &CommandContext<S>, s: &mut S, items: &[Tag], t: &mut Tracker<Tag>) -> Result<()> {
    let players = c.selector("players").players(s)?;
    for item in items {
        for p in &players {
            if s.give_stack(p, item) {
                t.track(item, 1);
            }
        }
    }
    for p in &players {
        s.inventory_changed(p);
    }
    Ok(())
}

/// `BlockItemAccessor.getContainer`.
fn container<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<(String, [i32; 3], i32)> {
    let dim = s.dimension().to_owned();
    let pos = loaded_block_pos(c, s, "targetPos", &dim)?;
    match s.container_size(&dim, pos) {
        Some(size) => Ok((dim, pos, size)),
        None => Err(CommandError::new(tr!("commands.item.target.not_a_container", pos[0], pos[1], pos[2]))),
    }
}

fn insert<S: Host>(c: &CommandContext<S>, s: &mut S, items: &[Tag], t: &mut Tracker<Tag>) -> Result<()> {
    let (dimension, pos, size) = container(c, s)?;
    let holder = ItemHolder::Block { dimension, pos };
    for item in items {
        // `distributeToContainer`.
        let mut left = item_count(item);
        let mut changed = false;
        for slot in 0..size {
            if left <= 0 || item_id(item).is_none_or(|id| id == "minecraft:air") {
                break;
            }
            match s.slot_item(&holder, slot) {
                Some(None) => {
                    changed = true;
                    s.set_slot_item(&holder, slot, Some(&with_count(item, left)));
                    break;
                }
                Some(Some(there)) => {
                    let max = s.item_max_stack(&there);
                    let n = item_count(&there);
                    if n <= max && same_item(&there, item) {
                        let moved = (max - n).min(left);
                        left -= moved;
                        if moved > 0 {
                            s.set_slot_item(&holder, slot, Some(&with_count(&there, n + moved)));
                        }
                        changed = true;
                    }
                }
                None => {}
            }
        }
        if changed {
            t.track(item, 1);
        }
    }
    Ok(())
}

fn count_arg<S: Host>(c: &CommandContext<S>, items: &[Tag]) -> i32 {
    if c.has("count") { c.integer("count") } else { items.len() as i32 }
}

fn replace_block<S: Host>(c: &CommandContext<S>, s: &mut S, items: &[Tag], t: &mut Tracker<Tag>) -> Result<()> {
    let (dimension, pos, size) = container(c, s)?;
    let slot = c.integer("slot");
    if slot < 0 || slot >= size {
        return Err(CommandError::new(tr!("commands.item.target.no_such_slot", slot)));
    }
    let holder = ItemHolder::Block { dimension, pos };
    for j in 0..count_arg(c, items) {
        let item = items.get(j as usize).cloned().unwrap_or_else(empty);
        let put = (item_count(&item) > 0).then_some(&item);
        if s.set_slot_item(&holder, slot + j, put) {
            t.track(&item, 1);
        }
    }
    Ok(())
}

fn replace_entity<S: Host>(c: &CommandContext<S>, s: &mut S, items: &[Tag], t: &mut Tracker<Tag>) -> Result<()> {
    let targets = c.selector("entities").entities(s)?;
    let slot = c.integer("slot");
    let count = count_arg(c, items);
    for e in &targets {
        let holder = ItemHolder::Entity(e.clone());
        for j in 0..count {
            let item = items.get(j as usize).cloned().unwrap_or_else(empty);
            if s.slot_item(&holder, slot + j).is_none() {
                continue;
            }
            let put = (item_count(&item) > 0).then_some(&item);
            if s.set_slot_item(&holder, slot + j, put) {
                t.track(&item, 1);
            }
        }
        if e.is_player() {
            s.inventory_changed(e);
        }
    }
    Ok(())
}

/// Places the drops and sends `commands.drop.success.*`.
fn finish<S: Host>(c: &CommandContext<S>, s: &mut S, target: Target<S>, drops: Drops) -> Result<i32> {
    let mut t = Tracker::new();
    target(c, s, &drops.items, &mut t)?;
    let single = |s: &S, item: &Tag| (item_count(item), display_name(s, item));
    let text = match &drops.table {
        Some(table) => t.message(
            Counted::NonZero,
            |item, _| {
                let (n, name) = single(s, item);
                tr!("commands.drop.success.single_with_table", n, name, table.as_str())
            },
            |count, _| tr!("commands.drop.success.multiple_with_table", count, table.as_str()),
        ),
        None => t.message(
            Counted::NonZero,
            |item, _| {
                let (n, name) = single(s, item);
                tr!("commands.drop.success.single", n, name)
            },
            |count, _| tr!("commands.drop.success.multiple", count),
        ),
    };
    s.send_success(text, false);
    Ok(t.total())
}

/// The four sources under one target.
fn sources<S: Host + 'static>(b: Builder<S>, target: Target<S>) -> Builder<S> {
    let table_source = |c: &CommandContext<S>, s: &mut S| -> Result<LootSource<S::Entity>> {
        Ok(LootSource::Table {
            table: resource_arg(c, s, "loot_table", "minecraft:loot_table")?,
            origin: s.stack().position,
            dimension: s.dimension().to_owned(),
            this: s.source_entity(),
        })
    };
    let tool_branches = move |b: Builder<S>, run: fn(&CommandContext<S>, &mut S, Target<S>, Option<Tag>, bool) -> Result<i32>| {
        b.executes(move |c, s: &mut S| run(c, s, target, None, false))
            .then(argument("tool", ArgumentType::ItemStack).executes(move |c, s: &mut S| {
                let tool = item_input_nbt(c.item("tool"), 1);
                run(c, s, target, Some(tool), false)
            }))
            .then(literal("mainhand").executes(move |c, s: &mut S| {
                let tool = hand_item(s, false)?;
                run(c, s, target, tool, true)
            }))
            .then(literal("offhand").executes(move |c, s: &mut S| {
                let tool = hand_item(s, true)?;
                run(c, s, target, tool, true)
            }))
    };
    b.then(literal("fish").then(argument("loot_table", ArgumentType::LootResource { registry: "minecraft:loot_table" }).then(
        tool_branches(argument("pos", ArgumentType::BlockPos), |c, s, target, tool, _| {
            let table = resource_arg(c, s, "loot_table", "minecraft:loot_table")?;
            let dimension = s.dimension().to_owned();
            let pos = loaded_block_pos(c, s, "pos", &dimension)?;
            let this = s.source_entity();
            let drops = roll(s, LootSource::Fish { table, pos, dimension, tool, this })?;
            finish(c, s, target, drops)
        }),
    )))
    .then(literal("loot").then(argument("loot_table", ArgumentType::LootResource { registry: "minecraft:loot_table" }).executes(
        move |c, s: &mut S| {
            let source = table_source(c, s)?;
            let drops = roll(s, source)?;
            finish(c, s, target, drops)
        },
    )))
    .then(literal("kill").then(argument("target", ArgumentType::entity()).executes(move |c, s: &mut S| {
        let victim = c.selector("target").entity(s)?;
        let origin = s.stack().position;
        let killer = s.source_entity();
        let drops = roll(s, LootSource::Kill { target: victim, origin, killer })?;
        finish(c, s, target, drops)
    })))
    .then(literal("mine").then(tool_branches(argument("pos", ArgumentType::BlockPos), |c, s, target, tool, _| {
        let dimension = s.dimension().to_owned();
        let pos = loaded_block_pos(c, s, "pos", &dimension)?;
        let this = s.source_entity();
        let drops = roll(s, LootSource::Mine { pos, dimension, tool, this })?;
        finish(c, s, target, drops)
    })))
}

pub fn loot<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let slot_count = |target: Target<S>| {
        argument("slot", ArgumentType::ItemSlot)
            .then(sources(argument("count", ArgumentType::integer_min(0)), target))
    };
    let slot_node = |target: Target<S>| sources(slot_count(target), target);
    d.register(
        literal("loot")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("replace")
                    .then(literal("entity").then(argument("entities", ArgumentType::entities()).then(slot_node(replace_entity))))
                    .then(literal("block").then(argument("targetPos", ArgumentType::BlockPos).then(slot_node(replace_block)))),
            )
            .then(literal("insert").then(sources(argument("targetPos", ArgumentType::BlockPos), insert)))
            .then(literal("give").then(sources(argument("players", ArgumentType::players()), give)))
            .then(literal("spawn").then(sources(argument("targetPos", ArgumentType::vec3()), spawn))),
    );
}
