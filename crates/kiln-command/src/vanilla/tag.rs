//! `tag` (vanilla `TagCommand`): add, remove and list entity tags.

use super::LEVEL_GAMEMASTERS;
use super::tracker::{Counted, Tracker};
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// `ComponentUtils.formatList(Collection<String>)`: sorted, green, comma separated.
pub(super) fn format_string_list(items: impl IntoIterator<Item = String>) -> Text {
    let mut items: Vec<String> = items.into_iter().collect();
    items.sort_by(|a, b| crate::nbt_text::java_cmp(a, b));
    Text::join(items.into_iter().map(|s| Text::literal(s).color("green")))
}

fn change<S: Host>(c: &CommandContext<S>, s: &mut S, add: bool) -> Result<i32> {
    let targets = c.selector("targets").entities(s)?;
    let name = c.string("name").to_owned();
    let mut t = Tracker::new();
    for e in &targets {
        let changed = if add { s.add_entity_tag(e, &name) } else { s.remove_entity_tag(e, &name) };
        t.track_bool(e, changed);
    }
    let (verb, error) = if add { ("add", "commands.tag.add.failed") } else { ("remove", "commands.tag.remove.failed") };
    let n = name.clone();
    t.send(
        s,
        true,
        Counted::NonZero,
        Some(CommandError::new(tr!(error))),
        |e, _| tr!(format!("commands.tag.{verb}.success.single"), name.as_str(), e.display_name()),
        |count, _| tr!(format!("commands.tag.{verb}.success.multiple"), n.as_str(), count),
    )
}

fn list<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let targets = c.selector("targets").entities(s)?;
    let mut t = Tracker::new();
    let mut all: Vec<String> = Vec::new();
    for e in &targets {
        let tags = s.entity_tags(e);
        for tag in &tags {
            if !all.contains(tag) {
                all.push(tag.clone());
            }
        }
        t.track(e, tags.len() as i32);
    }
    let size = all.len() as i32;
    if all.is_empty() {
        t.send(
            s,
            false,
            Counted::NonZero,
            None,
            |e, _| tr!("commands.tag.list.single.empty", e.display_name()),
            |count, _| tr!("commands.tag.list.multiple.empty", count),
        )?;
    } else {
        let list = format_string_list(all);
        let l2 = list.clone();
        t.send(
            s,
            false,
            Counted::NonZero,
            None,
            |e, _| tr!("commands.tag.list.single.success", e.display_name(), size, list),
            |count, _| tr!("commands.tag.list.multiple.success", count, size, l2),
        )?;
    }
    Ok(size)
}

pub fn tag<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("tag").requires(LEVEL_GAMEMASTERS).then(
            argument("targets", ArgumentType::entities())
                .then(literal("add").then(argument("name", ArgumentType::word()).executes(|c, s: &mut S| change(c, s, true))))
                .then(
                    literal("remove").then(
                        argument("name", ArgumentType::word())
                            .suggests_server(|_, s: &S, b| {
                                // Vanilla suggests the targets' tags; resolving selectors needs a
                                // mutable source, so this offers the players' tags.
                                let mut tags: Vec<String> = s.players().iter().flat_map(|p| p.tags().to_vec()).collect();
                                tags.sort();
                                tags.dedup();
                                for t in tags {
                                    b.suggest(&t);
                                }
                            })
                            .executes(|c, s: &mut S| change(c, s, false)),
                    ),
                )
                .then(literal("list").executes(list)),
        ),
    );
}
