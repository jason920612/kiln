//! `help`, `list`, `time`, `weather`, `gamerule`, `seed`, `stop`, `op`, `deop`, `difficulty`,
//! `setworldspawn` and Kiln's `kiln`.

use super::players::{spawn_point, spawn_pos};
use super::{LEVEL_ADMINS, LEVEL_GAMEMASTERS, LEVEL_OWNERS, gamerules, profile_of, resolve_profiles};
use crate::arguments::{ArgumentType, ArgumentValue};
use crate::dispatcher::{Builder, CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{GameRuleValue, Host, TimeAction, Weather};
use crate::selector::SelectorTarget;
use crate::text::{ClickEvent, Text};
use crate::tr;
use crate::types::{Difficulty, Identifier};

pub fn help<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("help")
            .executes(|c, s: &mut S| {
                let d = c.dispatcher();
                let usage = d.smart_usage(d.root(), s);
                for (_, u) in &usage {
                    s.send_success(Text::literal(format!("/{u}")), false);
                }
                Ok(usage.len() as i32)
            })
            .then(argument("command", ArgumentType::greedy_string()).executes(|c, s: &mut S| {
                let d = c.dispatcher();
                let parse = d.parse(c.string("command"), s);
                let Some(last) = parse.nodes().last() else {
                    return Err(CommandError::new(tr!("commands.help.failed")));
                };
                let usage = d.smart_usage(last, s);
                for (_, u) in &usage {
                    s.send_success(Text::literal(format!("/{} {u}", parse.input())), false);
                }
                Ok(usage.len() as i32)
            })),
    );
}

pub fn list<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn format<S: Host>(s: &mut S, uuids: bool) -> i32 {
        let players = s.players();
        let names = Text::join(players.iter().map(|p| {
            if uuids {
                tr!("commands.list.nameAndId", Text::literal(p.name()), Text::literal(p.uuid().to_string()))
            } else {
                p.display_name()
            }
        }));
        s.send_success(tr!("commands.list.players", players.len() as i32, s.max_players() as i32, names), false);
        players.len() as i32
    }
    d.register(
        literal("list")
            .executes(|_, s: &mut S| Ok(format(s, false)))
            .then(literal("uuids").executes(|_, s: &mut S| Ok(format(s, true)))),
    );
}

type ClockFn<S> = fn(&CommandContext<S>) -> Option<Identifier>;

/// The subcommands shared by `/time` (default clock) and `/time of <clock>`.
fn clock_nodes<S: Host + 'static>(b: Builder<S>, clock: ClockFn<S>, top_level: bool) -> Builder<S> {
    let run = move |action: fn(&CommandContext<S>) -> TimeAction| {
        move |c: &CommandContext<S>, s: &mut S| s.time(clock(c).as_ref(), &action(c))
    };
    let mut query = literal("query");
    if top_level {
        query = query.then(literal("gametime").executes(run(|_| TimeAction::QueryGameTime)));
    }
    query = query.then(literal("time").executes(run(|_| TimeAction::QueryTime))).then(
        argument("timeline", ArgumentType::resource("minecraft:timeline"))
            .suggests_server(move |c, s: &S, b| {
                let ids = s.timelines(clock(c).as_ref());
                b.suggest_resources(ids.iter().map(String::as_str), "");
            })
            .executes(run(|c| TimeAction::QueryTimeline {
                timeline: c.identifier("timeline").clone(),
                repetitions: false,
            }))
            .then(literal("repetition").executes(run(|c| TimeAction::QueryTimeline {
                timeline: c.identifier("timeline").clone(),
                repetitions: true,
            }))),
    );
    b.then(
        literal("add").then(
            argument("time", ArgumentType::time_min(i32::MIN)).executes(run(|c| TimeAction::Add(c.time("time")))),
        ),
    )
    .then(literal("pause").executes(run(|_| TimeAction::Pause)))
    .then(query)
    .then(literal("rate").then(
        argument("rate", ArgumentType::float_range(1e-5, 1000.0)).executes(run(|c| TimeAction::Rate(c.float("rate")))),
    ))
    .then(literal("resume").executes(run(|_| TimeAction::Resume)))
    .then(
        literal("set")
            .then(argument("time", ArgumentType::time()).executes(run(|c| TimeAction::Set(c.time("time")))))
            .then(
                argument("timemarker", ArgumentType::ResourceLocation)
                    .suggests_server(move |c, s: &S, b| {
                        let ids = s.time_markers(clock(c).as_ref());
                        b.suggest_resources(ids.iter().map(String::as_str), "");
                    })
                    .executes(run(|c| TimeAction::SetMarker(c.identifier("timemarker").clone()))),
            ),
    )
}

pub fn time<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let of = literal("of").then(clock_nodes(
        argument("clock", ArgumentType::resource("minecraft:world_clock")),
        |c| match c.get("clock") {
            Some(ArgumentValue::Identifier(id)) => Some(id.clone()),
            _ => None,
        },
        false,
    ));
    d.register(clock_nodes(literal("time").requires(LEVEL_GAMEMASTERS), |_| None, true).then(of));
}

pub fn weather<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let mut root = literal("weather").requires(LEVEL_GAMEMASTERS);
    for weather in [Weather::Clear, Weather::Rain, Weather::Thunder] {
        let set = move |c: &CommandContext<S>, s: &mut S| {
            let duration = c.get("duration").map(|_| c.time("duration"));
            let ticks = s.set_weather(weather, duration);
            s.send_success(tr!(format!("commands.weather.set.{}", weather.name())), true);
            Ok(ticks)
        };
        root = root.then(
            literal(weather.name()).executes(set).then(argument("duration", ArgumentType::time_min(1)).executes(set)),
        );
    }
    d.register(root);
}

pub fn gamerule<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let mut root = literal("gamerule").requires(LEVEL_GAMEMASTERS);
    for &rule in gamerules::rules() {
        for name in [gamerules::short_name(rule), rule] {
            let query = move |_: &CommandContext<S>, s: &mut S| {
                let value = s.game_rule(rule);
                s.send_success(tr!("commands.gamerule.query", gamerules::short_name(rule), value.serialize()), false);
                Ok(value.command_result())
            };
            let set = move |c: &CommandContext<S>, s: &mut S| {
                let value = match c.get("value") {
                    Some(ArgumentValue::Bool(b)) => GameRuleValue::Bool(*b),
                    Some(ArgumentValue::Integer(v)) => GameRuleValue::Int(*v),
                    other => unreachable!("game rule value {other:?}"),
                };
                let id = gamerules::short_name(rule);
                if s.game_rule(rule) == value {
                    return Err(CommandError::new(tr!("commands.gamerule.not_set", id, value.serialize())));
                }
                s.set_game_rule(rule, value);
                s.send_success(tr!("commands.gamerule.set", id, value.serialize()), true);
                Ok(value.command_result())
            };
            root = root
                .then(literal(name).executes(query).then(argument("value", gamerules::value_type(rule)).executes(set)));
        }
    }
    d.register(root);
}

pub fn seed<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("seed").requires(LEVEL_GAMEMASTERS).executes(|_, s: &mut S| {
        let seed = s.seed();
        s.send_success(tr!("commands.seed.success", copy_on_click(&seed.to_string())), false);
        Ok(seed as i32)
    }));
}

/// `ComponentUtils.copyOnClickText`.
pub(super) fn copy_on_click(text: &str) -> Text {
    Text::literal(text)
        .color("green")
        .click(ClickEvent::CopyToClipboard(text.to_owned()))
        .hover(tr!("chat.copy.click"))
        .insertion(text)
        .bracketed()
}

pub fn stop<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("stop").requires(LEVEL_OWNERS).executes(|_, s: &mut S| {
        s.send_success(tr!("commands.stop.stopping"), true);
        s.stop();
        Ok(1)
    }));
}

pub fn op<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("op").requires(LEVEL_ADMINS).then(
            argument("targets", ArgumentType::GameProfile)
                .suggests_server(|_, s: &S, b| {
                    let names: Vec<String> = s
                        .players()
                        .iter()
                        .filter(|p| !s.is_operator(&profile_of(*p)))
                        .map(SelectorTarget::name)
                        .collect();
                    b.suggest_matching(names.iter().map(String::as_str));
                })
                .executes(|c, s: &mut S| set_operators(c, s, true)),
        ),
    );
}

pub fn deop<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("deop").requires(LEVEL_ADMINS).then(
            argument("targets", ArgumentType::GameProfile)
                .suggests_server(|_, s: &S, b| {
                    let names = s.operator_names();
                    b.suggest_matching(names.iter().map(String::as_str));
                })
                .executes(|c, s: &mut S| set_operators(c, s, false)),
        ),
    );
}

fn set_operators<S: Host>(c: &CommandContext<S>, s: &mut S, op: bool) -> Result<i32, CommandError> {
    let profiles = resolve_profiles(c.game_profile("targets"), s)?;
    let (key, failed) = if op {
        ("commands.op.success", "commands.op.failed")
    } else {
        ("commands.deop.success", "commands.deop.failed")
    };
    let mut changed = 0;
    for p in &profiles {
        if s.is_operator(p) != op {
            s.set_operator(p, op);
            changed += 1;
            s.send_success(tr!(key, p.name.as_str()), true);
        }
    }
    if changed == 0 {
        return Err(CommandError::new(tr!(failed)));
    }
    Ok(changed)
}

pub fn difficulty<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let mut root = literal("difficulty").requires(LEVEL_GAMEMASTERS);
    for difficulty in Difficulty::ALL {
        root = root.then(literal(difficulty.name()).executes(move |_, s: &mut S| {
            if s.difficulty() == difficulty {
                return Err(CommandError::new(tr!("commands.difficulty.failure", difficulty.display_name())));
            }
            s.set_difficulty(difficulty);
            s.send_success(tr!("commands.difficulty.success", difficulty.display_name()), true);
            Ok(0)
        }));
    }
    d.register(root.executes(|_, s: &mut S| {
        let current = s.difficulty();
        s.send_success(tr!("commands.difficulty.query", current.display_name()), false);
        Ok(current.id())
    }));
}

pub fn setworldspawn<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let set = |c: &CommandContext<S>, s: &mut S| -> Result<i32, CommandError> {
        let pos = spawn_pos(c, s, "pos")?;
        let spawn = spawn_point(c, s, pos);
        s.set_world_spawn(&spawn)?;
        let [x, y, z] = spawn.pos;
        s.send_success(
            tr!("commands.setworldspawn.success", x, y, z, spawn.yaw, spawn.pitch, spawn.dimension.as_str()),
            true,
        );
        Ok(1)
    };
    d.register(
        literal("setworldspawn").requires(LEVEL_GAMEMASTERS).executes(set).then(
            argument("pos", ArgumentType::BlockPos)
                .executes(set)
                .then(argument("rotation", ArgumentType::Rotation).executes(set)),
        ),
    );
}

pub fn kiln<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let report = |lines: Vec<Text>, s: &mut S| {
        let n = lines.len() as i32;
        for line in lines {
            s.send_success(line, false);
        }
        n
    };
    d.register(
        literal("kiln")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("tick").executes(move |_, s: &mut S| {
                let lines = s.kiln_tick();
                Ok(report(lines, s))
            }))
            .then(literal("regions").executes(move |_, s: &mut S| {
                let lines = s.kiln_regions();
                Ok(report(lines, s))
            }))
            .then(literal("use").then(argument("targets", ArgumentType::players()).then(
                argument("pos", ArgumentType::BlockPos).executes(|c, s: &mut S| {
                    let targets = c.selector("targets").players(s)?;
                    let pos = s.stack().resolve_block(c.coordinates("pos"));
                    Ok(targets.iter().filter(|p| s.kiln_use(p, pos)).count() as i32)
                }),
            )))
            .then(literal("recipebook").then(argument("targets", ArgumentType::players()).executes(|c, s: &mut S| {
                let targets = c.selector("targets").players(s)?;
                Ok(targets.iter().filter(|p| s.kiln_open_recipe_book(p)).count() as i32)
            }))),
    );
}
