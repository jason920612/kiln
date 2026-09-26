//! `/scoreboard` (`ScoreboardCommand`) on the host's [`Scoreboard`]. Per-score display
//! names and number formats (`players display ...`, `objectives modify ... numberformat`)
//! are parsed and reported like vanilla but not stored.

use super::LEVEL_GAMEMASTERS;
use super::execute::score_holder_arg;
use crate::arguments::{ArgumentType, Operation, ScoreHolderArg};
use crate::dispatcher::{Builder, CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::scoreboard::{Objective, Scoreboard};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

pub fn scoreboard<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let objective = || argument("objective", ArgumentType::Objective);
    let targets = || score_holder_arg("targets", true);
    let objectives = literal("objectives")
        .then(literal("list").executes(|_, s: &mut S| list_objectives(s)))
        .then(
            literal("add").then(
                argument("objective", ArgumentType::word()).then(
                    argument("criteria", ArgumentType::ObjectiveCriteria)
                        .executes(|c, s: &mut S| {
                            let name = c.string("objective");
                            add_objective(s, name, c.string("criteria"), Text::literal(name))
                        })
                        .then(argument("displayName", ArgumentType::Component).executes(|c, s: &mut S| {
                            let display = resolved(c, s, "displayName")?;
                            add_objective(s, c.string("objective"), c.string("criteria"), display)
                        })),
                ),
            ),
        )
        .then(
            literal("modify").then(
                objective()
                    .then(literal("displayname").then(argument("displayName", ArgumentType::Component).executes(
                        |c, s: &mut S| {
                            let display = resolved(c, s, "displayName")?;
                            set_display_name(c, s, display)
                        },
                    )))
                    .then(["hearts", "integer"].into_iter().fold(literal("rendertype"), |b, kind| {
                        b.then(literal(kind).executes(move |c, s: &mut S| set_render_type(c, s, kind)))
                    }))
                    .then(literal("displayautoupdate").then(
                        argument("value", ArgumentType::Bool)
                            .executes(|c, s: &mut S| set_display_auto_update(c, s, c.bool("value"))),
                    ))
                    .then(number_formats(literal("numberformat"), set_objective_format)),
            ),
        )
        .then(literal("remove").then(objective().executes(|c, s: &mut S| remove_objective(c, s))))
        .then(
            literal("setdisplay").then(
                argument("slot", ArgumentType::ScoreboardSlot)
                    .executes(|c, s: &mut S| clear_display_slot(s, c.string("slot")))
                    .then(objective().executes(|c, s: &mut S| set_display_slot(c, s))),
            ),
        );
    let players = literal("players")
        .then(
            literal("list")
                .executes(|_, s: &mut S| list_holders(s))
                .then(score_holder_arg("target", false).executes(|c, s: &mut S| list_holder_scores(c, s))),
        )
        .then(literal("set").then(targets().then(objective().then(
            argument("score", ArgumentType::integer()).executes(|c, s: &mut S| set_score(c, s, c.integer("score"))),
        ))))
        .then(literal("get").then(
            score_holder_arg("target", false).then(objective().executes(|c, s: &mut S| get_score(c, s))),
        ))
        .then(literal("add").then(targets().then(objective().then(
            argument("score", ArgumentType::integer_min(0)).executes(|c, s: &mut S| add_score(c, s, false)),
        ))))
        .then(literal("remove").then(targets().then(objective().then(
            argument("score", ArgumentType::integer_min(0)).executes(|c, s: &mut S| add_score(c, s, true)),
        ))))
        .then(literal("reset").then(
            targets()
                .executes(|c, s: &mut S| reset_scores(c, s, false))
                .then(objective().executes(|c, s: &mut S| reset_scores(c, s, true))),
        ))
        .then(literal("enable").then(targets().then(objective().suggests_server(suggest_triggers).executes(
            |c, s: &mut S| enable_trigger(c, s),
        ))))
        .then(
            literal("display")
                .then(literal("name").then(targets().then(
                    objective()
                        .then(argument("name", ArgumentType::Component).executes(|c, s: &mut S| {
                            let name = resolved(c, s, "name")?;
                            set_score_display(c, s, Some(name))
                        }))
                        .executes(|c, s: &mut S| set_score_display(c, s, None)),
                )))
                .then(literal("numberformat").then(targets().then(number_formats(objective(), set_score_format)))),
        )
        .then(literal("operation").then(targets().then(argument("targetObjective", ArgumentType::Objective).then(
            argument("operation", ArgumentType::Operation).then(
                score_holder_arg("source", true)
                    .then(argument("sourceObjective", ArgumentType::Objective).executes(|c, s: &mut S| operation(c, s))),
            ),
        ))));
    d.register(literal("scoreboard").requires(LEVEL_GAMEMASTERS).then(objectives).then(players));
}

/// A number format as the commands see it: set (`blank`, `fixed`, `styled`) or cleared.
type FormatFn<S> = fn(&CommandContext<S>, &mut S, bool) -> Result<i32>;

/// `addNumberFormats`: `blank`, `fixed <contents>`, `styled <style>`, or nothing to clear.
fn number_formats<S: Host + 'static>(b: Builder<S>, run: FormatFn<S>) -> Builder<S> {
    b.then(literal("blank").executes(move |c, s: &mut S| run(c, s, true)))
        .then(literal("fixed").then(argument("contents", ArgumentType::Component).executes(move |c, s: &mut S| {
            resolved(c, s, "contents")?;
            run(c, s, true)
        })))
        .then(literal("styled").then(argument("style", ArgumentType::Style).executes(move |c, s: &mut S| run(c, s, true))))
        .executes(move |c, s: &mut S| run(c, s, false))
}

/// `ComponentArgument.getResolvedComponent`.
fn resolved<S: Host>(c: &CommandContext<S>, s: &mut S, name: &str) -> Result<Text> {
    let me = s.source_entity();
    Ok(c.component(name).resolve(s, me.as_ref())?.to_text())
}

fn board<S: Host>(s: &mut S) -> Result<&mut Scoreboard> {
    s.scoreboard_mut().ok_or_else(|| CommandError::unsupported("The scoreboard"))
}

/// `ObjectiveArgument.getObjective`.
fn objective<S: Host>(c: &CommandContext<S>, s: &mut S, arg: &str) -> Result<Objective> {
    let name = c.string(arg);
    board(s)?.objective(name).cloned().ok_or_else(|| CommandError::new(tr!("arguments.objective.notFound", name)))
}

/// `ObjectiveArgument.getWritableObjective`.
fn writable_objective<S: Host>(c: &CommandContext<S>, s: &mut S, arg: &str) -> Result<Objective> {
    let objective = objective(c, s, arg)?;
    if objective.is_read_only() {
        return Err(CommandError::new(tr!("arguments.objective.readonly", objective.name)));
    }
    Ok(objective)
}

/// `ScoreHolderArgument.getNamesWithDefaultWildcard`.
fn holders<S: Host>(c: &CommandContext<S>, s: &mut S, arg: &str) -> Result<Vec<String>> {
    let tracked = s.scoreboard().map(Scoreboard::holders);
    c.score_holder(arg).names(s, tracked)
}

/// `ScoreHolder.getFeedbackDisplayName`: a player's display name, else the name.
fn feedback_name<S: Host>(s: &S, holder: &str) -> Text {
    s.players().iter().find(|p| p.scoreboard_name() == holder).map_or_else(|| Text::literal(holder), |p| p.display_name())
}

/// `CommandResponseTracker`: the total of the tracked values and the only (non-zero)
/// holder, which picks between the single- and multiple-holder messages.
#[derive(Default)]
struct Tracker {
    total: i32,
    count: i32,
    only: Option<String>,
    non_zero: i32,
    only_non_zero: Option<String>,
}

impl Tracker {
    fn track(&mut self, holder: &str, value: i32) {
        self.total = self.total.wrapping_add(value);
        self.count += 1;
        self.only = (self.count == 1).then(|| holder.to_owned());
        if value != 0 {
            self.non_zero += 1;
            self.only_non_zero = (self.non_zero == 1).then(|| holder.to_owned());
        }
    }

    /// `sendFeedback`: `any` is `ElementType.ANY`, else `NON_ZERO`; returns the total.
    fn send<S: Host>(
        &self,
        s: &mut S,
        any: bool,
        single: impl FnOnce(Text, i32) -> Text,
        multiple: impl FnOnce(i32, i32) -> Text,
    ) -> i32 {
        let (only, count) = if any { (&self.only, self.count) } else { (&self.only_non_zero, self.non_zero) };
        let text = match only {
            Some(holder) => single(feedback_name(s, holder), self.total),
            None => multiple(count, self.total),
        };
        s.send_success(text, true);
        self.total
    }
}

fn list_objectives<S: Host>(s: &mut S) -> Result<i32> {
    let names: Vec<Text> = board(s)?.objectives().into_iter().map(Objective::formatted_display_name).collect();
    let n = names.len() as i32;
    let text = if names.is_empty() {
        tr!("commands.scoreboard.objectives.list.empty")
    } else {
        tr!("commands.scoreboard.objectives.list.success", n, Text::join(names))
    };
    s.send_success(text, false);
    Ok(n)
}

fn add_objective<S: Host>(s: &mut S, name: &str, criterion: &str, display_name: Text) -> Result<i32> {
    let board = board(s)?;
    let objective = Objective {
        name: name.to_owned(),
        criterion: criterion.to_owned(),
        display_name,
        render_type: if criterion == "health" { "hearts" } else { "integer" },
        display_auto_update: false,
    };
    let formatted = objective.formatted_display_name();
    if !board.add_objective(objective) {
        return Err(CommandError::new(tr!("commands.scoreboard.objectives.add.duplicate")));
    }
    let n = board.objectives().len() as i32;
    s.send_success(tr!("commands.scoreboard.objectives.add.success", formatted), true);
    Ok(n)
}

fn remove_objective<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let objective = objective(c, s, "objective")?;
    let board = board(s)?;
    board.remove_objective(&objective.name);
    let n = board.objectives().len() as i32;
    s.send_success(tr!("commands.scoreboard.objectives.remove.success", objective.formatted_display_name()), true);
    Ok(n)
}

/// Applies `f` to the objective named by the `objective` argument; returns the changed copy.
fn modify<S: Host>(c: &CommandContext<S>, s: &mut S, f: impl FnOnce(&mut Objective) -> bool) -> Result<Option<Objective>> {
    let name = objective(c, s, "objective")?.name;
    let objective = board(s)?.objective_mut(&name).expect("objective exists");
    Ok(f(objective).then(|| objective.clone()))
}

fn set_display_name<S: Host>(c: &CommandContext<S>, s: &mut S, display: Text) -> Result<i32> {
    let changed = modify(c, s, |o| {
        let differs = o.display_name.to_nbt() != display.to_nbt();
        if differs {
            o.display_name = display;
        }
        differs
    })?;
    if let Some(o) = changed {
        s.send_success(tr!("commands.scoreboard.objectives.modify.displayname", o.name.as_str(), o.formatted_display_name()), true);
    }
    Ok(0)
}

fn set_render_type<S: Host>(c: &CommandContext<S>, s: &mut S, kind: &'static str) -> Result<i32> {
    let changed = modify(c, s, |o| std::mem::replace(&mut o.render_type, kind) != kind)?;
    if let Some(o) = changed {
        s.send_success(tr!("commands.scoreboard.objectives.modify.rendertype", o.formatted_display_name()), true);
    }
    Ok(0)
}

fn set_display_auto_update<S: Host>(c: &CommandContext<S>, s: &mut S, value: bool) -> Result<i32> {
    let changed = modify(c, s, |o| std::mem::replace(&mut o.display_auto_update, value) != value)?;
    if let Some(o) = changed {
        let key = if value { "enable" } else { "disable" };
        let key = format!("commands.scoreboard.objectives.modify.displayAutoUpdate.{key}");
        s.send_success(Text::translate(key, vec![o.name.as_str().into(), o.formatted_display_name().into()]), true);
    }
    Ok(0)
}

fn set_objective_format<S: Host>(c: &CommandContext<S>, s: &mut S, set: bool) -> Result<i32> {
    let name = objective(c, s, "objective")?.name;
    let key = if set { "set" } else { "clear" };
    let key = format!("commands.scoreboard.objectives.modify.objectiveFormat.{key}");
    s.send_success(Text::translate(key, vec![name.into()]), true);
    Ok(0)
}

fn clear_display_slot<S: Host>(s: &mut S, slot: &str) -> Result<i32> {
    let board = board(s)?;
    if board.display(slot).is_none() {
        return Err(CommandError::new(tr!("commands.scoreboard.objectives.display.alreadyEmpty")));
    }
    board.set_display(slot, None);
    s.send_success(tr!("commands.scoreboard.objectives.display.cleared", slot), true);
    Ok(0)
}

fn set_display_slot<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let slot = c.string("slot");
    let objective = objective(c, s, "objective")?;
    let board = board(s)?;
    if board.display(slot) == Some(objective.name.as_str()) {
        return Err(CommandError::new(tr!("commands.scoreboard.objectives.display.alreadySet")));
    }
    board.set_display(slot, Some(&objective.name));
    s.send_success(tr!("commands.scoreboard.objectives.display.set", slot, objective.display_name), true);
    Ok(0)
}

fn list_holders<S: Host>(s: &mut S) -> Result<i32> {
    let holders = board(s)?.holders();
    let n = holders.len() as i32;
    let text = if holders.is_empty() {
        tr!("commands.scoreboard.players.list.empty")
    } else {
        tr!("commands.scoreboard.players.list.success", n, Text::join(holders.iter().map(Text::literal)))
    };
    s.send_success(text, false);
    Ok(n)
}

fn list_holder_scores<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let holder = c.score_holder("target").name(s)?;
    let name = feedback_name(s, &holder);
    let board = board(s)?;
    let scores: Vec<(Text, i32)> = board
        .scores_of(&holder)
        .into_iter()
        .filter_map(|(o, v)| Some((board.objective(o)?.formatted_display_name(), v)))
        .collect();
    let n = scores.len() as i32;
    if scores.is_empty() {
        s.send_success(tr!("commands.scoreboard.players.list.entity.empty", name), false);
    } else {
        s.send_success(tr!("commands.scoreboard.players.list.entity.success", name, n), false);
        for (objective, value) in scores {
            s.send_success(tr!("commands.scoreboard.players.list.entity.entry", objective, value), false);
        }
    }
    Ok(n)
}

fn get_score<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let holder = c.score_holder("target").name(s)?;
    let objective = objective(c, s, "objective")?;
    let name = feedback_name(s, &holder);
    let Some(value) = board(s)?.score(&holder, &objective.name) else {
        return Err(CommandError::new(tr!("commands.scoreboard.players.get.null", objective.name.as_str(), name)));
    };
    s.send_success(tr!("commands.scoreboard.players.get.success", name, value, objective.formatted_display_name()), false);
    Ok(value)
}

fn set_score<S: Host>(c: &CommandContext<S>, s: &mut S, value: i32) -> Result<i32> {
    let holders = holders(c, s, "targets")?;
    let objective = writable_objective(c, s, "objective")?;
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for holder in &holders {
        board.set_score(holder, &objective.name, value);
        tracker.track(holder, value);
    }
    let o = objective.formatted_display_name();
    Ok(tracker.send(
        s,
        false,
        |h, _| tr!("commands.scoreboard.players.set.success.single", o.clone(), h, value),
        |n, _| tr!("commands.scoreboard.players.set.success.multiple", o.clone(), n, value),
    ))
}

/// `addScore`, or `removeScore`.
fn add_score<S: Host>(c: &CommandContext<S>, s: &mut S, remove: bool) -> Result<i32> {
    let amount = c.integer("score");
    let delta = if remove { amount.wrapping_neg() } else { amount };
    let holders = holders(c, s, "targets")?;
    let objective = writable_objective(c, s, "objective")?;
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for holder in &holders {
        let score = board.score_mut(holder, &objective.name);
        score.value = score.value.wrapping_add(delta);
        tracker.track(holder, score.value);
    }
    let o = objective.formatted_display_name();
    let kind = if remove { "remove" } else { "add" };
    Ok(tracker.send(
        s,
        true,
        |h, total| {
            let key = format!("commands.scoreboard.players.{kind}.success.single");
            Text::translate(key, vec![amount.into(), o.clone().into(), h.into(), total.into()])
        },
        |n, _| {
            let key = format!("commands.scoreboard.players.{kind}.success.multiple");
            Text::translate(key, vec![amount.into(), o.clone().into(), n.into()])
        },
    ))
}

fn reset_scores<S: Host>(c: &CommandContext<S>, s: &mut S, one: bool) -> Result<i32> {
    let holders = holders(c, s, "targets")?;
    let objective = if one { Some(objective(c, s, "objective")?) } else { None };
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for holder in &holders {
        board.reset(holder, objective.as_ref().map(|o| o.name.as_str()));
        tracker.track(holder, 1);
    }
    Ok(match objective {
        Some(o) => {
            let o = o.formatted_display_name();
            tracker.send(
                s,
                false,
                |h, _| tr!("commands.scoreboard.players.reset.specific.single", o.clone(), h),
                |n, _| tr!("commands.scoreboard.players.reset.specific.multiple", o.clone(), n),
            )
        }
        None => tracker.send(
            s,
            false,
            |h, _| tr!("commands.scoreboard.players.reset.all.single", h),
            |n, _| tr!("commands.scoreboard.players.reset.all.multiple", n),
        ),
    })
}

fn enable_trigger<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let holders = holders(c, s, "targets")?;
    let objective = objective(c, s, "objective")?;
    if objective.criterion != "trigger" {
        return Err(CommandError::new(tr!("commands.scoreboard.players.enable.invalid")));
    }
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for holder in &holders {
        let score = board.score_mut(holder, &objective.name);
        if score.locked {
            score.locked = false;
            tracker.track(holder, 1);
        }
    }
    if tracker.non_zero == 0 {
        return Err(CommandError::new(tr!("commands.scoreboard.players.enable.failed")));
    }
    let o = objective.formatted_display_name();
    Ok(tracker.send(
        s,
        false,
        |h, _| tr!("commands.scoreboard.players.enable.success.single", o.clone(), h),
        |n, _| tr!("commands.scoreboard.players.enable.success.multiple", o.clone(), n),
    ))
}

/// `suggestTriggers`: trigger objectives some target cannot use yet. Selectors cannot be
/// resolved here, so they stand for the online players.
fn suggest_triggers<S: Host>(c: &CommandContext<S>, s: &S, b: &mut crate::suggestion::SuggestionsBuilder) {
    let Some(board) = s.scoreboard() else { return };
    let targets = match c.score_holder("targets") {
        ScoreHolderArg::Name(n) => vec![n.clone()],
        ScoreHolderArg::Wildcard => board.holders(),
        ScoreHolderArg::Selector(_) => s.player_names(),
    };
    let names: Vec<&str> = board
        .objectives()
        .into_iter()
        .filter(|o| o.criterion == "trigger")
        .filter(|o| targets.iter().any(|t| board.score_info(t, &o.name).is_none_or(|i| i.locked)))
        .map(|o| o.name.as_str())
        .collect();
    b.suggest_matching(names);
}

fn set_score_display<S: Host>(c: &CommandContext<S>, s: &mut S, name: Option<Text>) -> Result<i32> {
    let holders = holders(c, s, "targets")?;
    let objective = objective(c, s, "objective")?;
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for holder in &holders {
        board.score_mut(holder, &objective.name);
        tracker.track(holder, 1);
    }
    let o = objective.formatted_display_name();
    Ok(match name {
        None => tracker.send(
            s,
            false,
            |h, _| tr!("commands.scoreboard.players.display.name.clear.success.single", h, o.clone()),
            |n, _| tr!("commands.scoreboard.players.display.name.clear.success.multiple", n, o.clone()),
        ),
        Some(name) => tracker.send(
            s,
            false,
            |h, _| tr!("commands.scoreboard.players.display.name.set.success.single", name.clone(), h, o.clone()),
            |n, _| tr!("commands.scoreboard.players.display.name.set.success.multiple", name.clone(), n, o.clone()),
        ),
    })
}

fn set_score_format<S: Host>(c: &CommandContext<S>, s: &mut S, set: bool) -> Result<i32> {
    let holders = holders(c, s, "targets")?;
    let objective = objective(c, s, "objective")?;
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for holder in &holders {
        board.score_mut(holder, &objective.name);
        tracker.track(holder, 1);
    }
    let o = objective.formatted_display_name();
    let kind = if set { "set" } else { "clear" };
    Ok(tracker.send(
        s,
        false,
        |h, _| {
            let key = format!("commands.scoreboard.players.display.numberFormat.{kind}.success.single");
            Text::translate(key, vec![h.into(), o.clone().into()])
        },
        |n, _| {
            let key = format!("commands.scoreboard.players.display.numberFormat.{kind}.success.multiple");
            Text::translate(key, vec![n.into(), o.clone().into()])
        },
    ))
}

/// `performOperation`: every target against every source, creating missing scores as 0.
fn operation<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let targets = holders(c, s, "targets")?;
    let target_objective = writable_objective(c, s, "targetObjective")?;
    let op = c.operation("operation");
    let sources = holders(c, s, "source")?;
    let source_objective = objective(c, s, "sourceObjective")?;
    let mut tracker = Tracker::default();
    let board = board(s)?;
    for target in &targets {
        board.score_mut(target, &target_objective.name);
        for source in &sources {
            let b = board.score_mut(source, &source_objective.name).value;
            let a = board.score_mut(target, &target_objective.name).value;
            let (a, b) = apply(op, a, b)?;
            board.score_mut(source, &source_objective.name).value = b;
            board.score_mut(target, &target_objective.name).value = a;
        }
        tracker.track(target, board.score_mut(target, &target_objective.name).value);
    }
    let o = target_objective.formatted_display_name();
    Ok(tracker.send(
        s,
        true,
        |h, total| tr!("commands.scoreboard.players.operation.success.single", o.clone(), h, total),
        |n, _| tr!("commands.scoreboard.players.operation.success.multiple", o.clone(), n),
    ))
}

/// `OperationArgument.Operation.apply`: the new (target, source) values.
fn apply(op: Operation, a: i32, b: i32) -> Result<(i32, i32)> {
    let div0 = || CommandError::new(tr!("arguments.operation.div0"));
    Ok(match op {
        Operation::Assign => (b, b),
        Operation::Add => (a.wrapping_add(b), b),
        Operation::Subtract => (a.wrapping_sub(b), b),
        Operation::Multiply => (a.wrapping_mul(b), b),
        // `Mth.floorDiv` / `Mth.positiveModulo`.
        Operation::Divide if b == 0 => return Err(div0()),
        Operation::Divide => {
            let q = a.wrapping_div(b);
            (if a.wrapping_rem(b) != 0 && (a ^ b) < 0 { q - 1 } else { q }, b)
        }
        Operation::Modulo if b == 0 => return Err(div0()),
        Operation::Modulo => {
            let r = a.wrapping_rem(b);
            (if r != 0 && (r ^ b) < 0 { r + b } else { r }, b)
        }
        Operation::Min => (a.min(b), b),
        Operation::Max => (a.max(b), b),
        Operation::Swap => (b, a),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_follow_java_integer_math() {
        assert_eq!(apply(Operation::Divide, -7, 2).unwrap(), (-4, 2));
        assert_eq!(apply(Operation::Divide, i32::MIN, -1).unwrap(), (i32::MIN, -1));
        assert_eq!(apply(Operation::Modulo, -7, 2).unwrap(), (1, 2));
        assert_eq!(apply(Operation::Modulo, 7, -2).unwrap(), (-1, -2));
        assert_eq!(apply(Operation::Add, i32::MAX, 1).unwrap(), (i32::MIN, 1));
        assert_eq!(apply(Operation::Swap, 1, 2).unwrap(), (2, 1));
        assert!(apply(Operation::Modulo, 1, 0).is_err());
    }
}
