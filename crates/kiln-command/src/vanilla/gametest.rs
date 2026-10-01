//! `test` (vanilla `TestCommand`, the game test framework's command tree): the tree and
//! argument handling; the host runs the subcommands ([`Host::test_command`]).
//!
//! Vanilla registers `test` on dedicated servers too. Its `export*` subcommands exist only
//! when the game runs from an IDE, so they are not part of the tree here either.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{Builder, CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, TestCommand, TestSelection};
use crate::types::wildcard_match;

const REGISTRY: &str = "minecraft:test_instance";

type Result<T> = std::result::Result<T, CommandError>;

/// The ids `tests` selects (`ResourceSelectorArgument.getSelectedResources`), in registry order.
fn selected<S: Host>(c: &CommandContext<S>, s: &S) -> Vec<String> {
    let pattern = c.string("tests");
    s.registry_ids(REGISTRY).into_iter().filter(|id| wildcard_match(id, pattern)).collect()
}

/// The `numberOfTimes [untilFailed [rotationSteps [testsPerRow]]]` tail of the run commands.
/// `rotation` adds `rotationSteps` and `testsPerRow` below `untilFailed`.
fn run_options<S: Host + 'static>(
    node: Builder<S>,
    rotation: bool,
    finder: fn(&CommandContext<S>, &S) -> TestSelection,
    copies: fn(&CommandContext<S>) -> i32,
) -> Builder<S> {
    let run = move |c: &CommandContext<S>, s: &mut S, tries: i32, halt: bool, steps: i32, per_row: i32| {
        let select = finder(c, s);
        s.test_command(&TestCommand::Run { select, copies: copies(c), tries, halt_on_failure: halt, rotation_steps: steps, per_row })
    };
    let until_failed = {
        let mut n = argument("untilFailed", ArgumentType::Bool)
            .executes(move |c, s: &mut S| run(c, s, c.integer("numberOfTimes"), c.bool("untilFailed"), 0, 8));
        if rotation {
            n = n.then(
                argument("rotationSteps", ArgumentType::integer())
                    .executes(move |c, s: &mut S| run(c, s, c.integer("numberOfTimes"), c.bool("untilFailed"), c.integer("rotationSteps"), 8))
                    .then(argument("testsPerRow", ArgumentType::integer()).executes(move |c, s: &mut S| {
                        run(c, s, c.integer("numberOfTimes"), c.bool("untilFailed"), c.integer("rotationSteps"), c.integer("testsPerRow"))
                    })),
            );
        }
        n
    };
    node.executes(move |c, s: &mut S| run(c, s, 1, true, 0, 8))
        .then(argument("numberOfTimes", ArgumentType::integer_min(0)).executes(move |c, s: &mut S| run(c, s, c.integer("numberOfTimes"), false, 0, 8)).then(until_failed))
}

fn no_copies<S: Host>(_: &CommandContext<S>) -> i32 {
    1
}

fn nearby<S: Host>(_: &CommandContext<S>, _: &S) -> TestSelection {
    TestSelection::Nearby
}

fn nearest<S: Host>(_: &CommandContext<S>, _: &S) -> TestSelection {
    TestSelection::Nearest
}

fn looked_at<S: Host>(_: &CommandContext<S>, _: &S) -> TestSelection {
    TestSelection::LookedAt
}

fn by_ids<S: Host>(c: &CommandContext<S>, s: &S) -> TestSelection {
    TestSelection::Ids(selected(c, s))
}

fn failed<S: Host>(_: &CommandContext<S>, _: &S) -> TestSelection {
    TestSelection::Failed { only_required: false }
}

fn failed_required<S: Host>(c: &CommandContext<S>, _: &S) -> TestSelection {
    TestSelection::Failed { only_required: c.bool("onlyRequiredTests") }
}

pub fn test<S: Host + 'static>(d: &mut Dispatcher<S>) {
    // `suggestTestInstanceIds`: ids of the test registry, without `minecraft:`.
    let tests = || {
        argument("tests", ArgumentType::ResourceSelector { registry: REGISTRY }).suggests_server(|_, s: &S, b| {
            let ids = s.registry_ids(REGISTRY);
            let shown: Vec<&str> = ids.iter().map(|id| id.strip_prefix("minecraft:").unwrap_or(id)).collect();
            b.suggest_resources(shown, "");
        })
    };
    let reset = |select: fn(&CommandContext<S>, &S) -> TestSelection| move |c: &CommandContext<S>, s: &mut S| -> Result<i32> {
        let select = select(c, s);
        s.test_command(&TestCommand::Reset(select))
    };
    let clear = |select: fn(&CommandContext<S>, &S) -> TestSelection| move |c: &CommandContext<S>, s: &mut S| -> Result<i32> {
        let select = select(c, s);
        s.test_command(&TestCommand::Clear(select))
    };
    let create = |c: &CommandContext<S>, s: &mut S, size: [i32; 3]| -> Result<i32> {
        s.test_command(&TestCommand::Create { id: c.identifier("id").clone(), size })
    };
    d.register(
        literal("test")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("run").then(run_options(tests(), true, by_ids, no_copies)))
            .then(
                literal("runmultiple").then(tests().executes(|c, s: &mut S| {
                    let ids = selected(c, s);
                    s.test_command(&TestCommand::Run { select: TestSelection::Ids(ids), copies: 1, tries: 1, halt_on_failure: true, rotation_steps: 0, per_row: 8 })
                }).then(argument("amount", ArgumentType::integer()).executes(|c, s: &mut S| {
                    let ids = selected(c, s);
                    s.test_command(&TestCommand::Run {
                        select: TestSelection::Ids(ids),
                        copies: c.integer("amount"),
                        tries: 1,
                        halt_on_failure: true,
                        rotation_steps: 0,
                        per_row: 8,
                    })
                })),
            ))
            .then(run_options(literal("runthese"), false, nearby, no_copies))
            .then(run_options(literal("runclosest"), false, nearest, no_copies))
            .then(run_options(literal("runthat"), false, looked_at, no_copies))
            .then(
                run_options(literal("runfailed"), true, failed, no_copies)
                    .then(run_options(argument("onlyRequiredTests", ArgumentType::Bool), true, failed_required, no_copies)),
            )
            .then(literal("verify").then(tests().executes(|c, s: &mut S| {
                let ids = selected(c, s);
                s.test_command(&TestCommand::Verify { ids })
            })))
            .then(literal("locate").then(tests().executes(|c, s: &mut S| {
                let ids = selected(c, s);
                s.test_command(&TestCommand::Locate { ids })
            })))
            .then(literal("resetclosest").executes(reset(nearest)))
            .then(literal("resetthese").executes(reset(nearby)))
            .then(literal("resetthat").executes(reset(looked_at)))
            .then(literal("clearthat").executes(clear(looked_at)))
            .then(literal("clearthese").executes(clear(nearby)))
            .then(
                literal("clearall")
                    .executes(|_, s: &mut S| s.test_command(&TestCommand::Clear(TestSelection::Radius(250))))
                    .then(argument("radius", ArgumentType::integer()).executes(|c, s: &mut S| {
                        s.test_command(&TestCommand::Clear(TestSelection::Radius(c.integer("radius").clamp(0, 1024))))
                    })),
            )
            .then(literal("stop").executes(|_, s: &mut S| s.test_command(&TestCommand::Stop)))
            .then(
                literal("pos")
                    .executes(|_, s: &mut S| s.test_command(&TestCommand::Pos("pos".to_owned())))
                    .then(argument("var", ArgumentType::word()).executes(|c, s: &mut S| s.test_command(&TestCommand::Pos(c.string("var").to_owned())))),
            )
            .then(
                literal("create").then(
                    argument("id", ArgumentType::ResourceLocation)
                        .executes(move |c, s: &mut S| create(c, s, [5, 5, 5]))
                        .then(
                            argument("width", ArgumentType::integer())
                                .executes(move |c, s: &mut S| {
                                    let w = c.integer("width");
                                    create(c, s, [w, w, w])
                                })
                                .then(argument("height", ArgumentType::integer()).then(argument("depth", ArgumentType::integer()).executes(
                                    move |c, s: &mut S| create(c, s, [c.integer("width"), c.integer("height"), c.integer("depth")]),
                                ))),
                        ),
                ),
            ),
    );
}
