//! `/team` (`TeamCommand`) on the host's [`Scoreboard`](crate::scoreboard::Scoreboard) teams.

use super::LEVEL_GAMEMASTERS;
use super::execute::score_holder_arg;
use super::scoreboard::{Tracker, board, holders, resolved};
use crate::arguments::{ArgumentType, TEAM_COLORS};
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::scoreboard::{COLLISION_NAMES, Team, TeamRule, VISIBILITY_NAMES};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

const RULES: [TeamRule; 4] = [TeamRule::Always, TeamRule::Never, TeamRule::OtherTeams, TeamRule::OwnTeam];

pub fn team<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let team = || argument("team", ArgumentType::Team);
    // Vanilla lists the options in this order: never, hideForOtherTeams, hideForOwnTeam, always.
    let visibility = |name: &'static str, death: bool| {
        [1, 2, 3, 0].into_iter().fold(literal(name), move |b, i| {
            b.then(literal(VISIBILITY_NAMES[i]).executes(move |c, s: &mut S| set_visibility(c, s, death, RULES[i])))
        })
    };
    let collision = [1, 3, 2, 0].into_iter().fold(literal("collisionRule"), |b, i| {
        b.then(literal(COLLISION_NAMES[i]).executes(move |c, s: &mut S| set_collision(c, s, RULES[i])))
    });
    let modify = team()
        .then(literal("displayName").then(argument("displayName", ArgumentType::Component).executes(
            |c, s: &mut S| {
                let name = resolved(c, s, "displayName")?;
                set_display_name(c, s, name)
            },
        )))
        .then(
            literal("color")
                .then(literal("reset").executes(|c, s: &mut S| set_color(c, s, None)))
                .then(argument("value", ArgumentType::TeamColor).executes(|c, s: &mut S| {
                    let color = TEAM_COLORS.iter().position(|n| *n == c.string("value"));
                    set_color(c, s, color)
                })),
        )
        .then(literal("friendlyFire").then(
            argument("allowed", ArgumentType::Bool).executes(|c, s: &mut S| set_friendly_fire(c, s, c.bool("allowed"))),
        ))
        .then(literal("seeFriendlyInvisibles").then(
            argument("allowed", ArgumentType::Bool).executes(|c, s: &mut S| set_friendly_sight(c, s, c.bool("allowed"))),
        ))
        .then(visibility("nametagVisibility", false))
        .then(visibility("deathMessageVisibility", true))
        .then(collision)
        .then(literal("prefix").then(argument("prefix", ArgumentType::Component).executes(|c, s: &mut S| {
            let prefix = resolved(c, s, "prefix")?;
            set_affix(c, s, prefix, true)
        })))
        .then(literal("suffix").then(argument("suffix", ArgumentType::Component).executes(|c, s: &mut S| {
            let suffix = resolved(c, s, "suffix")?;
            set_affix(c, s, suffix, false)
        })));
    d.register(
        literal("team")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("list")
                    .executes(|_, s: &mut S| list_teams(s))
                    .then(team().executes(|c, s: &mut S| list_members(c, s))),
            )
            .then(
                literal("add").then(
                    argument("team", ArgumentType::word())
                        .executes(|c, s: &mut S| {
                            let name = c.string("team");
                            create_team(s, name, Text::literal(name))
                        })
                        .then(argument("displayName", ArgumentType::Component).executes(|c, s: &mut S| {
                            let display = resolved(c, s, "displayName")?;
                            create_team(s, c.string("team"), display)
                        })),
                ),
            )
            .then(literal("remove").then(team().executes(|c, s: &mut S| delete_team(c, s))))
            .then(literal("empty").then(team().executes(|c, s: &mut S| empty_team(c, s))))
            .then(
                literal("join").then(
                    team()
                        .executes(|c, s: &mut S| {
                            let me = super::source_entity(s)?;
                            join_team(c, s, vec![me.scoreboard_name()])
                        })
                        .then(score_holder_arg("members", true).executes(|c, s: &mut S| {
                            let members = holders(c, s, "members")?;
                            join_team(c, s, members)
                        })),
                ),
            )
            .then(literal("leave").then(score_holder_arg("members", true).executes(|c, s: &mut S| leave_team(c, s))))
            .then(literal("modify").then(modify)),
    );
}

/// `TeamArgument.getTeam`.
fn team_arg<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<Team> {
    let name = c.string("team");
    board(s)?.team(name).cloned().ok_or_else(|| CommandError::new(tr!("team.notFound", name)))
}

/// Changes the team named by the `team` argument and sends it; returns the changed team.
fn modify<S: Host>(c: &CommandContext<S>, s: &mut S, f: impl FnOnce(&mut Team)) -> Result<Team> {
    let name = team_arg(c, s)?.name;
    let board = board(s)?;
    board.modify_team(&name, f);
    Ok(board.team(&name).cloned().expect("team exists"))
}

fn list_teams<S: Host>(s: &mut S) -> Result<i32> {
    let teams: Vec<Text> = board(s)?.teams().into_iter().map(Team::formatted_display_name).collect();
    let n = teams.len() as i32;
    let text = if teams.is_empty() {
        tr!("commands.team.list.teams.empty")
    } else {
        tr!("commands.team.list.teams.success", n, Text::join(teams))
    };
    s.send_success(text, false);
    Ok(n)
}

fn list_members<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let team = team_arg(c, s)?;
    let mut members = team.players();
    // `ComponentUtils.formatList(Collection<String>)` sorts (`String.compareTo`).
    members.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let n = members.len() as i32;
    let text = if members.is_empty() {
        tr!("commands.team.list.members.empty", team.formatted_display_name())
    } else {
        let names = Text::join(members.iter().map(Text::literal));
        tr!("commands.team.list.members.success", team.formatted_display_name(), n, names)
    };
    s.send_success(text, false);
    Ok(n)
}

fn create_team<S: Host>(s: &mut S, name: &str, display: Text) -> Result<i32> {
    let board = board(s)?;
    if !board.add_team(name) {
        return Err(CommandError::new(tr!("commands.team.add.duplicate")));
    }
    board.modify_team(name, |t| t.display_name = display);
    let team = board.team(name).expect("just added").formatted_display_name();
    let n = board.teams().len() as i32;
    s.send_success(tr!("commands.team.add.success", team), true);
    Ok(n)
}

fn delete_team<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let team = team_arg(c, s)?;
    let board = board(s)?;
    board.remove_team(&team.name);
    let n = board.teams().len() as i32;
    s.send_success(tr!("commands.team.remove.success", team.formatted_display_name()), true);
    Ok(n)
}

fn empty_team<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let team = team_arg(c, s)?;
    let members = team.players();
    if members.is_empty() {
        return Err(CommandError::new(tr!("commands.team.empty.unchanged")));
    }
    let board = board(s)?;
    for m in &members {
        board.leave_team(m);
    }
    let n = members.len() as i32;
    s.send_success(tr!("commands.team.empty.success", n, team.formatted_display_name()), true);
    Ok(n)
}

fn join_team<S: Host>(c: &CommandContext<S>, s: &mut S, members: Vec<String>) -> Result<i32> {
    let team = team_arg(c, s)?;
    let mut tracker = Tracker::default();
    for m in &members {
        let joined = board(s)?.join_team(m, &team.name);
        tracker.track(m, joined as i32);
    }
    let t = board(s)?.team(&team.name).expect("team exists").formatted_display_name();
    Ok(tracker.send(
        s,
        false,
        |h, _| tr!("commands.team.join.success.single", h, t.clone()),
        |n, _| tr!("commands.team.join.success.multiple", n, t.clone()),
    ))
}

fn leave_team<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let members = holders(c, s, "members")?;
    let mut tracker = Tracker::default();
    for m in &members {
        let left = board(s)?.leave_team(m);
        tracker.track(m, left as i32);
    }
    Ok(tracker.send(
        s,
        false,
        |h, _| tr!("commands.team.leave.success.single", h),
        |n, _| tr!("commands.team.leave.success.multiple", n),
    ))
}

fn set_visibility<S: Host>(c: &CommandContext<S>, s: &mut S, death: bool, rule: TeamRule) -> Result<i32> {
    let team = team_arg(c, s)?;
    let (current, kind) = if death {
        (team.death_message_visibility, "deathMessageVisibility")
    } else {
        (team.name_tag_visibility, "nametagVisibility")
    };
    if current == rule {
        return Err(CommandError::new(Text::translate(format!("commands.team.option.{kind}.unchanged"), vec![])));
    }
    let team = modify(c, s, |t| {
        if death {
            t.death_message_visibility = rule;
        } else {
            t.name_tag_visibility = rule;
        }
    })?;
    let value = Text::translate(format!("team.visibility.{}", VISIBILITY_NAMES[rule as usize]), vec![]);
    let key = format!("commands.team.option.{kind}.success");
    s.send_success(Text::translate(key, vec![team.formatted_display_name().into(), value.into()]), true);
    Ok(0)
}

fn set_collision<S: Host>(c: &CommandContext<S>, s: &mut S, rule: TeamRule) -> Result<i32> {
    let team = team_arg(c, s)?;
    if team.collision_rule == rule {
        return Err(CommandError::new(tr!("commands.team.option.collisionRule.unchanged")));
    }
    let team = modify(c, s, |t| t.collision_rule = rule)?;
    let value = Text::translate(format!("team.collision.{}", COLLISION_NAMES[rule as usize]), vec![]);
    s.send_success(tr!("commands.team.option.collisionRule.success", team.formatted_display_name(), value), true);
    Ok(0)
}

/// `setFriendlyFire` and `setFriendlySight`: `commands.team.option.<option>.<state>`.
fn set_flag<S: Host>(
    c: &CommandContext<S>,
    s: &mut S,
    option: &str,
    allowed: bool,
    get: fn(&Team) -> bool,
    set: fn(&mut Team, bool),
) -> Result<i32> {
    let team = team_arg(c, s)?;
    if get(&team) == allowed {
        let state = if allowed { "alreadyEnabled" } else { "alreadyDisabled" };
        return Err(CommandError::new(Text::translate(format!("commands.team.option.{option}.{state}"), vec![])));
    }
    let team = modify(c, s, |t| set(t, allowed))?;
    let state = if allowed { "enabled" } else { "disabled" };
    let key = format!("commands.team.option.{option}.{state}");
    s.send_success(Text::translate(key, vec![team.formatted_display_name().into()]), true);
    Ok(0)
}

fn set_friendly_fire<S: Host>(c: &CommandContext<S>, s: &mut S, allowed: bool) -> Result<i32> {
    set_flag(c, s, "friendlyfire", allowed, |t| t.friendly_fire, |t, v| t.friendly_fire = v)
}

fn set_friendly_sight<S: Host>(c: &CommandContext<S>, s: &mut S, allowed: bool) -> Result<i32> {
    set_flag(c, s, "seeFriendlyInvisibles", allowed, |t| t.see_friendly_invisibles, |t, v| t.see_friendly_invisibles = v)
}

fn set_display_name<S: Host>(c: &CommandContext<S>, s: &mut S, name: Text) -> Result<i32> {
    let team = team_arg(c, s)?;
    if team.display_name.to_nbt() == name.to_nbt() {
        return Err(CommandError::new(tr!("commands.team.option.name.unchanged")));
    }
    let team = modify(c, s, |t| t.display_name = name)?;
    s.send_success(tr!("commands.team.option.name.success", team.formatted_display_name()), true);
    Ok(0)
}

/// `setColor` and `clearColor`.
fn set_color<S: Host>(c: &CommandContext<S>, s: &mut S, color: Option<usize>) -> Result<i32> {
    let team = team_arg(c, s)?;
    if team.color == color {
        return Err(CommandError::new(tr!("commands.team.option.color.unchanged")));
    }
    let team = modify(c, s, |t| t.color = color)?;
    let text = match color {
        Some(i) => tr!("commands.team.option.color.success", team.formatted_display_name(), TEAM_COLORS[i]),
        None => tr!("commands.team.option.color.clear.success", team.formatted_display_name()),
    };
    s.send_success(text, true);
    Ok(0)
}

/// `setPrefix` / `setSuffix`: always applied, feedback only to the source.
fn set_affix<S: Host>(c: &CommandContext<S>, s: &mut S, text: Text, prefix: bool) -> Result<i32> {
    modify(c, s, |t| if prefix { t.prefix = text.clone() } else { t.suffix = text.clone() })?;
    let key = if prefix { "commands.team.option.prefix.success" } else { "commands.team.option.suffix.success" };
    s.send_success(Text::translate(key, vec![text.into()]), false);
    Ok(1)
}
