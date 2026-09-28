//! `advancement` (vanilla `AdvancementCommands`: grant or revoke everything, one advancement
//! (or one criterion), or an advancement with its parents and/or descendants) and `recipe`
//! (`RecipeCommand`: give or take one recipe or all of them).

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use kiln_proto::packets::commands::StringKind;

type Result<T> = std::result::Result<T, CommandError>;

/// `AdvancementCommands.Mode`: which advancements around the given one take part.
#[derive(Clone, Copy)]
enum Mode {
    Only,
    Through,
    From,
    Until,
}

impl Mode {
    fn parents(self) -> bool {
        matches!(self, Mode::Through | Mode::Until)
    }
    fn children(self) -> bool {
        matches!(self, Mode::Through | Mode::From)
    }
}

fn action_name(revoke: bool) -> &'static str {
    if revoke { "revoke" } else { "grant" }
}

/// `ResourceKeyArgument.getAdvancement`.
fn advancement_arg<S: Host>(c: &CommandContext<S>, s: &S) -> Result<(String, Text)> {
    let id = c.identifier("advancement").to_string();
    match s.advancement_name(&id) {
        Some(name) => Ok((id, name)),
        None => Err(CommandError::new(tr!("advancement.advancementNotFound", id))),
    }
}

/// `getAdvancements`: parents from the root down to it, it, then its descendants.
fn around<S: Host>(s: &S, id: &str, mode: Mode) -> Vec<String> {
    let mut out = Vec::new();
    if mode.parents() {
        out.extend(s.advancement_parents(id));
    }
    out.push(id.to_owned());
    if mode.children() {
        out.extend(s.advancement_descendants(id));
    }
    out
}

/// A `CommandResponseTracker` over players: the total, and the only element (any, and non-zero).
struct Tracker<E> {
    total: i32,
    count: usize,
    only: Option<E>,
    nonzero: usize,
    only_nonzero: Option<E>,
}

impl<E: Clone> Tracker<E> {
    fn new() -> Self {
        Tracker { total: 0, count: 0, only: None, nonzero: 0, only_nonzero: None }
    }
    fn track(&mut self, e: &E, value: i32) {
        self.total += value;
        self.count += 1;
        self.only = if self.count == 1 { Some(e.clone()) } else { None };
        if value != 0 {
            self.nonzero += 1;
            self.only_nonzero = if self.nonzero == 1 { Some(e.clone()) } else { None };
        }
    }
}

fn perform<S: Host>(s: &mut S, targets: Vec<S::Entity>, revoke: bool, advancements: Vec<String>, show: bool) -> Result<i32> {
    let mut t = Tracker::new();
    for p in &targets {
        if !show {
            s.flush_advancements(p, true);
        }
        let n = advancements.iter().filter(|a| s.change_advancement(p, a, revoke)).count() as i32;
        if !show {
            s.flush_advancements(p, false);
        }
        t.track(p, n);
    }
    let action = action_name(revoke);
    if let [one] = advancements.as_slice() {
        let name = s.advancement_name(one).unwrap_or_else(|| Text::literal(one.clone()));
        if t.total == 0 {
            return Err(CommandError::new(match &t.only {
                Some(p) => tr!(format!("commands.advancement.{action}.one.to.one.failure"), name, p.display_name()),
                None => tr!(format!("commands.advancement.{action}.one.to.many.failure"), name, t.count as i32),
            }));
        }
        let text = match &t.only_nonzero {
            Some(p) => tr!(format!("commands.advancement.{action}.one.to.one.success"), name, p.display_name()),
            None => tr!(format!("commands.advancement.{action}.one.to.many.success"), name, t.nonzero as i32),
        };
        s.send_success(text, true);
        return Ok(t.total);
    }
    let size = advancements.len() as i32;
    if t.total == 0 {
        return Err(CommandError::new(match &t.only {
            Some(p) => tr!(format!("commands.advancement.{action}.many.to.one.failure"), size, p.display_name()),
            None => tr!(format!("commands.advancement.{action}.many.to.many.failure"), size, t.count as i32),
        }));
    }
    let text = match &t.only_nonzero {
        Some(p) => tr!(format!("commands.advancement.{action}.many.to.one.success"), size, p.display_name()),
        None => tr!(format!("commands.advancement.{action}.many.to.many.success"), size, t.nonzero as i32),
    };
    s.send_success(text, true);
    Ok(t.total)
}

fn perform_criterion<S: Host>(c: &CommandContext<S>, s: &mut S, revoke: bool) -> Result<i32> {
    let targets = c.selector("targets").players(s)?;
    let (id, name) = advancement_arg(c, s)?;
    let criterion = c.string("criterion").to_owned();
    if !s.advancement_criteria(&id).contains(&criterion) {
        return Err(CommandError::new(tr!("commands.advancement.criterionNotFound", name, criterion)));
    }
    let mut t = Tracker::new();
    for p in &targets {
        let changed = s.change_criterion(p, &id, &criterion, revoke);
        t.track(p, changed as i32);
    }
    let action = action_name(revoke);
    if t.total == 0 {
        return Err(CommandError::new(match &t.only {
            Some(p) => tr!(format!("commands.advancement.{action}.criterion.to.one.failure"), criterion, name, p.display_name()),
            None => tr!(format!("commands.advancement.{action}.criterion.to.many.failure"), criterion, name, t.count as i32),
        }));
    }
    let text = match &t.only_nonzero {
        Some(p) => tr!(format!("commands.advancement.{action}.criterion.to.one.success"), criterion, name, p.display_name()),
        None => tr!(format!("commands.advancement.{action}.criterion.to.many.success"), criterion, name, t.nonzero as i32),
    };
    s.send_success(text, true);
    Ok(t.total)
}

fn suggest_criteria<S: Host>(c: &CommandContext<S>, s: &S, b: &mut crate::suggestion::SuggestionsBuilder) {
    let id = c.identifier("advancement").to_string();
    let names = s.advancement_criteria(&id);
    b.suggest_matching(names.iter().map(String::as_str));
}

pub fn advancement<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let key = || ArgumentType::ResourceKey { registry: "minecraft:advancement" };
    let branch = |revoke: bool| {
        let mode = move |name: &'static str, m: Mode| {
            literal(name).then(argument("advancement", key()).executes(move |c, s: &mut S| {
                let targets = c.selector("targets").players(s)?;
                let (id, _) = advancement_arg(c, s)?;
                let list = around(s, &id, m);
                perform(s, targets, revoke, list, true)
            }))
        };
        literal(action_name(revoke)).then(
            argument("targets", ArgumentType::players())
                .then(
                    literal("only").then(
                        argument("advancement", key())
                            .executes(move |c, s: &mut S| {
                                let targets = c.selector("targets").players(s)?;
                                let (id, _) = advancement_arg(c, s)?;
                                let list = around(s, &id, Mode::Only);
                                perform(s, targets, revoke, list, true)
                            })
                            .then(
                                argument("criterion", ArgumentType::String(StringKind::Greedy))
                                    .suggests_server(suggest_criteria)
                                    .executes(move |c, s: &mut S| perform_criterion(c, s, revoke)),
                            ),
                    ),
                )
                .then(mode("from", Mode::From))
                .then(mode("until", Mode::Until))
                .then(mode("through", Mode::Through))
                .then(literal("everything").executes(move |c, s: &mut S| {
                    let targets = c.selector("targets").players(s)?;
                    let all = s.advancement_ids();
                    perform(s, targets, revoke, all, false)
                })),
        )
    };
    d.register(literal("advancement").requires(LEVEL_GAMEMASTERS).then(branch(false)).then(branch(true)));
}

fn recipes<S: Host>(c: &CommandContext<S>, s: &mut S, take: bool, all: bool) -> Result<i32> {
    let targets = c.selector("targets").players(s)?;
    let list = if all {
        s.recipe_ids()
    } else {
        let id = c.identifier("recipe").to_string();
        if !s.recipe_ids().contains(&id) {
            return Err(CommandError::new(tr!("recipe.notFound", id)));
        }
        vec![id]
    };
    let mut total = 0;
    for p in &targets {
        total += s.change_recipes(p, &list, take);
    }
    let (verb, failed) = if take { ("take", "commands.recipe.take.failed") } else { ("give", "commands.recipe.give.failed") };
    if total == 0 {
        return Err(CommandError::new(tr!(failed)));
    }
    let text = match &targets[..] {
        [one] => tr!(format!("commands.recipe.{verb}.success.single"), total, one.display_name()),
        many => tr!(format!("commands.recipe.{verb}.success.multiple"), total, many.len() as i32),
    };
    s.send_success(text, true);
    Ok(total)
}

pub fn recipe<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let branch = |name: &'static str, take: bool| {
        literal(name).then(
            argument("targets", ArgumentType::players())
                .then(literal("*").executes(move |c, s: &mut S| recipes(c, s, take, true)))
                .then(
                    argument("recipe", ArgumentType::ResourceKey { registry: "minecraft:recipe" })
                        .executes(move |c, s: &mut S| recipes(c, s, take, false)),
                ),
        )
    };
    d.register(literal("recipe").requires(LEVEL_GAMEMASTERS).then(branch("give", false)).then(branch("take", true)));
}
