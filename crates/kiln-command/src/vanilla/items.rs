//! `clear` and `enchant` (vanilla `ClearInventoryCommands`, `EnchantCommand`): inventories
//! seen through the host's item slots, items as item stack NBT.

use super::tracker::{Counted, Tracker};
use super::{LEVEL_GAMEMASTERS, source_player};
use crate::arguments::ArgumentType;
use crate::dispatcher::{Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{EnchantOutcome, Host, ItemHolder};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// An `item_predicate` argument as far as Kiln evaluates it: any item, an item or an item tag.
/// Component tests (`[...]`) are not evaluated.
pub(super) enum ItemTest {
    Any,
    Item(String),
    Tag(&'static [i32]),
}

impl ItemTest {
    pub(super) fn parse(text: &str) -> Result<ItemTest> {
        if text.contains('[') {
            return Err(CommandError::unsupported("Item component predicates"));
        }
        if text == "*" {
            return Ok(ItemTest::Any);
        }
        if let Some(tag) = text.strip_prefix('#') {
            let id = crate::types::Identifier::parse(tag).map_or_else(|| tag.to_owned(), |i| i.to_string());
            let ids = crate::blocks::registry_tag("minecraft:item", &id).unwrap_or(&[]);
            return Ok(ItemTest::Tag(ids));
        }
        let id = crate::types::Identifier::parse(text).map_or_else(|| text.to_owned(), |i| i.to_string());
        Ok(ItemTest::Item(id))
    }

    /// Tests item stack NBT.
    pub(super) fn test(&self, item: &Tag) -> bool {
        let Some(id) = item_id(item) else { return false };
        match self {
            ItemTest::Any => true,
            ItemTest::Item(want) => id == want,
            ItemTest::Tag(ids) => kiln_data::builtin_id("minecraft:item", id).is_some_and(|n| ids.contains(&n)),
        }
    }
}

pub(super) fn item_id(item: &Tag) -> Option<&str> {
    match item {
        Tag::Compound(f) => f.iter().find(|(k, _)| k == "id").and_then(|(_, v)| if let Tag::String(s) = v { Some(s.as_str()) } else { None }),
        _ => None,
    }
}

pub(super) fn item_count(item: &Tag) -> i32 {
    match item {
        Tag::Compound(f) => f
            .iter()
            .find(|(k, _)| k == "count")
            .and_then(|(_, v)| match v {
                Tag::Int(n) => Some(*n),
                Tag::Byte(n) => Some(*n as i32),
                Tag::Short(n) => Some(*n as i32),
                _ => None,
            })
            .unwrap_or(1),
        _ => 0,
    }
}

pub(super) fn with_count(item: &Tag, count: i32) -> Tag {
    let mut t = item.clone();
    if let Tag::Compound(f) = &mut t {
        f.retain(|(k, _)| k != "count");
        f.insert(1.min(f.len()), ("count".into(), Tag::Int(count)));
    }
    t
}

/// `Inventory.clearOrCountMatchingItems`: `max` -1 clears every match, 0 only counts.
fn clear_or_count<S: Host>(s: &mut S, player: &S::Entity, test: &ItemTest, max: i32) -> i32 {
    let counting = max == 0;
    let holder = ItemHolder::Entity(player.clone());
    let mut total = 0;
    for slot in s.clear_slots(player) {
        let Some(Some(item)) = s.slot_item(&holder, slot) else { continue };
        if !test.test(&item) {
            continue;
        }
        let n = item_count(&item);
        if counting {
            total += n;
            continue;
        }
        let limit = max - total;
        let take = if limit < 0 { n } else { limit.min(n) };
        if take > 0 {
            let rest = n - take;
            let new = (rest > 0).then(|| with_count(&item, rest));
            s.set_slot_item(&holder, slot, new.as_ref());
        }
        total += take;
    }
    total
}

pub fn clear<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(s: &mut S, targets: Vec<S::Entity>, test: ItemTest, max: i32) -> Result<i32> {
        let counting = max == 0;
        let mut t = Tracker::new();
        for p in &targets {
            let n = clear_or_count(s, p, &test, max);
            t.track(p, n);
            if !counting {
                s.inventory_changed(p);
            }
        }
        if t.total() == 0 {
            let e = t.message(
                Counted::All,
                |p, _| tr!("clear.failed.single", p.display_name()),
                |count, _| tr!("clear.failed.multiple", count),
            );
            return Err(CommandError::new(e));
        }
        let (single, multiple) =
            if counting { ("commands.clear.test.single", "commands.clear.test.multiple") } else { ("commands.clear.success.single", "commands.clear.success.multiple") };
        t.send(s, true, Counted::NonZero, None, |p, total| tr!(single, total, p.display_name()), |count, total| tr!(multiple, total, count))
    }
    d.register(
        literal("clear")
            .requires(LEVEL_GAMEMASTERS)
            .executes(|_, s: &mut S| {
                let p = source_player(s)?;
                run(s, vec![p], ItemTest::Any, -1)
            })
            .then(
                argument("targets", ArgumentType::players())
                    .executes(|c, s: &mut S| {
                        let targets = c.selector("targets").players(s)?;
                        run(s, targets, ItemTest::Any, -1)
                    })
                    .then(
                        argument("item", ArgumentType::ItemPredicate)
                            .executes(|c, s: &mut S| {
                                let targets = c.selector("targets").players(s)?;
                                run(s, targets, ItemTest::parse(c.string("item"))?, -1)
                            })
                            .then(argument("maxCount", ArgumentType::integer_min(0)).executes(|c, s: &mut S| {
                                let targets = c.selector("targets").players(s)?;
                                run(s, targets, ItemTest::parse(c.string("item"))?, c.integer("maxCount"))
                            })),
                    ),
            ),
    );
}

/// `Enchantment.getFullname`: the name, and the level unless the enchantment has only one
/// (curses red, others gray).
fn full_name(enchantment: &str, level: i32, max_level: i32) -> Text {
    let (ns, path) = enchantment.split_once(':').unwrap_or(("minecraft", enchantment));
    let curse = kiln_data::synced_id("minecraft:enchantment", enchantment)
        .zip(crate::blocks::registry_tag("minecraft:enchantment", "minecraft:curse"))
        .is_some_and(|(id, curses)| curses.contains(&id));
    let mut name = Text::translate(format!("enchantment.{ns}.{path}"), Vec::new());
    if level != 1 || max_level != 1 {
        name = name.append(Text::literal(" ")).append(Text::translate(format!("enchantment.level.{level}"), Vec::new()));
    }
    name.color(if curse { "red" } else { "gray" })
}

pub fn enchant<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(s: &mut S, targets: Vec<S::Entity>, enchantment: &str, level: i32) -> Result<i32> {
        let Some(max) = s.enchantment_max_level(enchantment) else {
            return Err(CommandError::unknown_resource(enchantment, "minecraft:enchantment"));
        };
        if level > max {
            return Err(CommandError::new(tr!("commands.enchant.failed.level", level, max)));
        }
        let single_target = targets.len() == 1;
        let mut t = Tracker::new();
        for e in &targets {
            match s.enchant_held(e, enchantment, level) {
                EnchantOutcome::Applied => t.track(e, 1),
                EnchantOutcome::Incompatible(name) if single_target => {
                    return Err(CommandError::new(tr!("commands.enchant.failed.incompatible", name)));
                }
                EnchantOutcome::NoItem if single_target => {
                    return Err(CommandError::new(tr!("commands.enchant.failed.itemless", e.display_name())));
                }
                EnchantOutcome::NotLiving if single_target => {
                    return Err(CommandError::new(tr!("commands.enchant.failed.entity", e.display_name())));
                }
                _ => {}
            }
        }
        let name = full_name(enchantment, level, max);
        let name2 = name.clone();
        t.send(
            s,
            true,
            Counted::NonZero,
            Some(CommandError::new(tr!("commands.enchant.failed"))),
            |e, _| tr!("commands.enchant.success.single", name, e.display_name()),
            |count, _| tr!("commands.enchant.success.multiple", name2, count),
        )
    }
    d.register(
        literal("enchant").requires(LEVEL_GAMEMASTERS).then(
            argument("targets", ArgumentType::entities()).then(
                argument("enchantment", ArgumentType::resource("minecraft:enchantment"))
                    .executes(|c, s: &mut S| {
                        let targets = c.selector("targets").entities(s)?;
                        let id = c.identifier("enchantment").to_string();
                        run(s, targets, &id, 1)
                    })
                    .then(argument("level", ArgumentType::integer_min(0)).executes(|c, s: &mut S| {
                        let targets = c.selector("targets").entities(s)?;
                        let id = c.identifier("enchantment").to_string();
                        run(s, targets, &id, c.integer("level"))
                    })),
            ),
        ),
    );
}
