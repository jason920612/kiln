//! `experience` / `xp` (vanilla `ExperienceCommand`): add, set and query points or levels.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// `ExperienceCommand.Type`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum XpKind {
    Points,
    Levels,
}

impl XpKind {
    fn name(self) -> &'static str {
        match self {
            XpKind::Points => "points",
            XpKind::Levels => "levels",
        }
    }
}

fn add<S: Host>(c: &CommandContext<S>, s: &mut S, kind: XpKind) -> Result<i32> {
    let targets = c.selector("target").players(s)?;
    let amount = c.integer("amount");
    for p in &targets {
        s.add_experience(p, amount, kind);
    }
    let key = format!("commands.experience.add.{}.success", kind.name());
    let text = match &targets[..] {
        [one] => tr!(format!("{key}.single"), amount, one.display_name()),
        many => tr!(format!("{key}.multiple"), amount, many.len() as i32),
    };
    s.send_success(text, true);
    Ok(targets.len() as i32)
}

fn set<S: Host>(c: &CommandContext<S>, s: &mut S, kind: XpKind) -> Result<i32> {
    let targets = c.selector("target").players(s)?;
    let amount = c.integer("amount");
    let changed = targets.iter().filter(|p| s.set_experience(p, amount, kind)).count() as i32;
    if changed == 0 {
        return Err(CommandError::new(tr!("commands.experience.set.points.invalid")));
    }
    let key = format!("commands.experience.set.{}.success", kind.name());
    let text = match &targets[..] {
        [one] => tr!(format!("{key}.single"), amount, one.display_name()),
        many => tr!(format!("{key}.multiple"), amount, many.len() as i32),
    };
    s.send_success(text, true);
    Ok(targets.len() as i32)
}

fn query<S: Host>(c: &CommandContext<S>, s: &mut S, kind: XpKind) -> Result<i32> {
    let target = c.selector("target").players(s)?.into_iter().next().expect("a single player");
    let value = s.query_experience(&target, kind);
    s.send_success(tr!(format!("commands.experience.query.{}", kind.name()), target.display_name(), value), false);
    Ok(value)
}

pub fn experience<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let amount_then = |min: Option<i32>, run: fn(&CommandContext<S>, &mut S, XpKind) -> Result<i32>| {
        let amount = match min {
            Some(m) => ArgumentType::integer_min(m),
            None => ArgumentType::integer(),
        };
        argument("amount", amount)
            .executes(move |c, s: &mut S| run(c, s, XpKind::Points))
            .then(literal("points").executes(move |c, s: &mut S| run(c, s, XpKind::Points)))
            .then(literal("levels").executes(move |c, s: &mut S| run(c, s, XpKind::Levels)))
    };
    let node = d.register(
        literal("experience")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("add").then(argument("target", ArgumentType::players()).then(amount_then(None, add))))
            .then(literal("set").then(argument("target", ArgumentType::players()).then(amount_then(Some(0), set))))
            .then(
                literal("query").then(
                    argument("target", ArgumentType::player())
                        .then(literal("points").executes(|c, s: &mut S| query(c, s, XpKind::Points)))
                        .then(literal("levels").executes(|c, s: &mut S| query(c, s, XpKind::Levels))),
                ),
            ),
    );
    d.register(literal("xp").requires(LEVEL_GAMEMASTERS).redirect(node));
}
