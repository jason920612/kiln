//! `execute` (`ExecuteCommand`): modifiers (`as`, `at`, `positioned`, `rotated`, `facing`,
//! `align`, `anchored`, `in`, `on`, `summon`), conditions (`if`/`unless`), result storage
//! (`store`) and `run`. Every subcommand redirects back to `execute`; the entity-selecting
//! modifiers and all conditions fork.

use super::LEVEL_GAMEMASTERS;
use super::blocks::{BlockBox, dimension_arg, loaded_block_pos, test_block};
use crate::arguments::ArgumentType;
use crate::dispatcher::{Builder, CommandContext, Dispatcher, NodeId, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, SourceStack};
use crate::scoreboard::Scoreboard;
use crate::selector::SelectorTarget;
use crate::tr;
use crate::types::{Anchor, Identifier};
use kiln_data::blocks::default_state::AIR;
use kiln_proto::nbt::Tag;
use std::sync::Arc;

type Result<T> = std::result::Result<T, CommandError>;

/// `ExecuteCommand.MAX_TEST_AREA`.
const MAX_TEST_AREA: i64 = 32768;

pub fn execute<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let exec = d.register(literal("execute").requires(LEVEL_GAMEMASTERS));
    let root = d.root();
    d.register(
        literal("execute")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("run").redirect(root))
            .then(conditionals(exec, literal("if"), true))
            .then(conditionals(exec, literal("unless"), false))
            .then(literal("as").then(argument("targets", ArgumentType::entities()).fork(exec, |c, s: &mut S| {
                let targets = c.selector("targets").find_entities(s)?;
                Ok(targets.into_iter().map(|e| s.stack().clone().with_entity(e)).collect())
            })))
            .then(literal("at").then(argument("targets", ArgumentType::entities()).fork(exec, |c, s: &mut S| {
                let targets = c.selector("targets").find_entities(s)?;
                Ok(targets
                    .into_iter()
                    .map(|e| {
                        s.stack().clone().with_dimension(e.dimension()).with_position(e.position()).with_rotation(e.rotation())
                    })
                    .collect())
            })))
            .then(literal("store").then(stores(exec, literal("result"), true)).then(stores(exec, literal("success"), false)))
            .then(
                literal("positioned")
                    .then(argument("pos", ArgumentType::vec3()).redirect_with(exec, |c, s: &mut S| {
                        let pos = s.stack().resolve(c.coordinates("pos"));
                        Ok(s.stack().clone().with_position(pos).with_anchor(Anchor::Feet))
                    }))
                    .then(literal("as").then(argument("targets", ArgumentType::entities()).fork(exec, |c, s: &mut S| {
                        let targets = c.selector("targets").find_entities(s)?;
                        Ok(targets.into_iter().map(|e| s.stack().clone().with_position(e.position())).collect())
                    })))
                    .then(literal("over").then(argument("heightmap", ArgumentType::Heightmap).redirect_with(
                        exec,
                        |c, s: &mut S| {
                            let [x, _, z] = s.origin();
                            let (bx, bz) = (x.floor() as i32, z.floor() as i32);
                            let dimension = s.dimension().to_owned();
                            if !s.is_chunk_loaded(&dimension, bx >> 4, bz >> 4) {
                                return Err(CommandError::pos_unloaded());
                            }
                            let y = s.height(&dimension, c.heightmap("heightmap"), bx, bz);
                            Ok(s.stack().clone().with_position([x, y as f64, z]))
                        },
                    ))),
            )
            .then(
                literal("rotated")
                    .then(argument("rot", ArgumentType::Rotation).redirect_with(exec, |c, s: &mut S| {
                        let rotation = c.coordinates("rot").rotation(s.source_rotation());
                        Ok(s.stack().clone().with_rotation(rotation))
                    }))
                    .then(literal("as").then(argument("targets", ArgumentType::entities()).fork(exec, |c, s: &mut S| {
                        let targets = c.selector("targets").find_entities(s)?;
                        Ok(targets.into_iter().map(|e| s.stack().clone().with_rotation(e.rotation())).collect())
                    }))),
            )
            .then(
                literal("facing")
                    .then(literal("entity").then(argument("targets", ArgumentType::entities()).then(
                        argument("anchor", ArgumentType::EntityAnchor).fork(exec, |c, s: &mut S| {
                            let anchor = c.anchor("anchor");
                            let targets = c.selector("targets").find_entities(s)?;
                            Ok(targets
                                .into_iter()
                                .map(|e| s.stack().clone().facing(entity_anchor(&e, anchor)))
                                .collect())
                        }),
                    )))
                    .then(argument("pos", ArgumentType::vec3()).redirect_with(exec, |c, s: &mut S| {
                        let pos = s.stack().resolve(c.coordinates("pos"));
                        Ok(s.stack().clone().facing(pos))
                    })),
            )
            .then(literal("align").then(argument("axes", ArgumentType::Swizzle).redirect_with(exec, |c, s: &mut S| {
                let axes = c.swizzle("axes");
                let pos = s.origin();
                let aligned = std::array::from_fn(|i| if axes[i] { pos[i].floor() } else { pos[i] });
                Ok(s.stack().clone().with_position(aligned))
            })))
            .then(literal("anchored").then(argument("anchor", ArgumentType::EntityAnchor).redirect_with(
                exec,
                |c, s: &mut S| Ok(s.stack().clone().with_anchor(c.anchor("anchor"))),
            )))
            .then(literal("in").then(argument("dimension", ArgumentType::Dimension).redirect_with(
                exec,
                |c, s: &mut S| {
                    let dimension = dimension_arg(c, s, "dimension")?;
                    Ok(s.stack().clone().with_dimension(&dimension))
                },
            )))
            .then(literal("summon").then(
                argument("entity", ArgumentType::resource("minecraft:entity_type"))
                    .suggests(crate::dispatcher::SuggestionProvider::Named("minecraft:summonable_entities"))
                    .redirect_with(exec, |_, _: &mut S| Err(CommandError::new(tr!("commands.summon.failed")))),
            ))
            .then(relations(exec, literal("on"))),
    );
}

/// `EntityAnchorArgument.Anchor.apply(Entity)`.
fn entity_anchor<E: SelectorTarget>(e: &E, anchor: Anchor) -> [f64; 3] {
    let [x, y, z] = e.position();
    match anchor {
        Anchor::Feet => [x, y, z],
        Anchor::Eyes => [x, y + e.eye_height(), z],
    }
}

/// `createRelationOperations`: entities related to the executing one (what the host knows of
/// them: [`Host::related_entities`]); without an executing entity there are none.
fn relations<S: Host + 'static>(exec: NodeId, b: Builder<S>) -> Builder<S> {
    ["owner", "leasher", "target", "attacker", "vehicle", "controller", "origin", "passengers"].into_iter().fold(b, |b, relation| {
        b.then(literal(relation).fork(exec, move |_, s: &mut S| {
            let Some(entity) = s.source_entity() else { return Ok(Vec::new()) };
            let related = s.related_entities(relation, &entity);
            Ok(related.into_iter().map(|e| s.stack().clone().with_entity(e)).collect())
        }))
    })
}

/// `ExecuteCommand.expect`: the source if the test came out as wanted.
fn expect<S: Host>(s: &S, positive: bool, passed: bool) -> Vec<SourceStack<S>> {
    if positive == passed { vec![s.stack().clone()] } else { Vec::new() }
}

type Test<S> = fn(&CommandContext<S>, &mut S) -> Result<bool>;

/// `addConditional`: forks when the test matches, or reports it when the command ends here.
fn conditional<S: Host + 'static>(exec: NodeId, b: Builder<S>, positive: bool, test: Test<S>) -> Builder<S> {
    b.fork(exec, move |c, s: &mut S| {
        let passed = test(c, s)?;
        Ok(expect(s, positive, passed))
    })
    .executes(move |c, s: &mut S| {
        if test(c, s)? == positive {
            s.send_success(tr!("commands.execute.conditional.pass"), false);
            Ok(1)
        } else {
            Err(CommandError::new(tr!("commands.execute.conditional.fail")))
        }
    })
}

/// A conditional over a count (`createNumericConditionalHandler`).
fn numeric_conditional<S: Host + 'static, F>(exec: NodeId, b: Builder<S>, positive: bool, count: F) -> Builder<S>
where
    F: Fn(&CommandContext<S>, &mut S) -> Result<i32> + Clone + Send + Sync + 'static,
{
    let fork_count = count.clone();
    b.fork(exec, move |c, s: &mut S| {
        let n = fork_count(c, s)?;
        Ok(expect(s, positive, n > 0))
    })
    .executes(move |c, s: &mut S| {
        let n = count(c, s)?;
        numeric_result(s, positive, n)
    })
}

fn numeric_result<S: Host>(s: &mut S, positive: bool, n: i32) -> Result<i32> {
    match (positive, n) {
        (true, n) if n > 0 => {
            s.send_success(tr!("commands.execute.conditional.pass_count", n), false);
            Ok(n)
        }
        (true, _) => Err(CommandError::new(tr!("commands.execute.conditional.fail"))),
        (false, 0) => {
            s.send_success(tr!("commands.execute.conditional.pass"), false);
            Ok(1)
        }
        (false, n) => Err(CommandError::new(tr!("commands.execute.conditional.fail_count", n))),
    }
}

/// `execute if items block|entity <source> <slots> <item_predicate>` (`countItems`).
fn items_branch<S: Host + 'static>(exec: NodeId, positive: bool, block: bool) -> Builder<S> {
    let count = move |c: &CommandContext<S>, s: &mut S| -> Result<i32> {
        let source = super::item::Accessor::read(c, s, block, "source")?;
        let arg = super::item::SlotsArg::read(c, "slots");
        let tree = source.slots(s, &arg, true)?;
        let test = super::items::ItemTest::parse(c.string("item_predicate"))?;
        Ok(super::item::item_copies(&tree, s).iter().filter(|t| test.test(t)).map(super::items::item_count).sum())
    };
    literal(if block { "block" } else { "entity" }).then(argument("source", if block { ArgumentType::BlockPos } else { ArgumentType::entities() }).then(
        argument("slots", ArgumentType::SlotSource).then(numeric_conditional(exec, argument("item_predicate", ArgumentType::ItemPredicate), positive, count)),
    ))
}

/// `execute if slots block|entity <source> <slots>` (`countSlots`).
fn slots_branch<S: Host + 'static>(exec: NodeId, positive: bool, block: bool) -> Builder<S> {
    let count = move |c: &CommandContext<S>, s: &mut S| -> Result<i32> {
        let source = super::item::Accessor::read(c, s, block, "source")?;
        let arg = super::item::SlotsArg::read(c, "slots");
        let tree = source.slots(s, &arg, true)?;
        Ok(super::item::tree_size(&tree, s))
    };
    literal(if block { "block" } else { "entity" })
        .then(argument("source", if block { ArgumentType::BlockPos } else { ArgumentType::entities() }).then(numeric_conditional(exec, argument("slots", ArgumentType::SlotSource), positive, count)))
}

/// `addConditionals`: every `if`/`unless` test.
fn conditionals<S: Host + 'static>(exec: NodeId, b: Builder<S>, positive: bool) -> Builder<S> {
    let compare = |op: &'static str, test: Test<S>| {
        literal(op).then(score_holder_arg("source", false).then(conditional(
            exec,
            argument("sourceObjective", ArgumentType::Objective),
            positive,
            test,
        )))
    };
    b.then(literal("block").then(argument("pos", ArgumentType::BlockPos).then(conditional(
        exec,
        argument("block", ArgumentType::BlockPredicate),
        positive,
        |c, s| {
            let dimension = s.dimension().to_owned();
            let pos = loaded_block_pos(c, s, "pos", &dimension)?;
            Ok(test_block(s, &dimension, pos, c.block_predicate("block")))
        },
    ))))
    .then(literal("biome").then(argument("pos", ArgumentType::BlockPos).then(conditional(
        exec,
        argument("biome", ArgumentType::ResourceOrTag { registry: "minecraft:worldgen/biome" }),
        positive,
        |c, s| {
            let dimension = s.dimension().to_owned();
            let pos = loaded_block_pos(c, s, "pos", &dimension)?;
            let biome = s.biome(&dimension, pos).ok_or_else(|| CommandError::unsupported("Biome lookup"))?;
            Ok(c.resource_or_tag("biome").test("minecraft:worldgen/biome", &biome))
        },
    ))))
    .then(literal("loaded").then(conditional(exec, argument("pos", ArgumentType::BlockPos), positive, |c, s| {
        let pos = s.stack().resolve_block(c.coordinates("pos"));
        let dimension = s.dimension().to_owned();
        Ok(s.is_chunk_ticking(&dimension, pos[0] >> 4, pos[2] >> 4))
    })))
    .then(literal("dimension").then(conditional(exec, argument("dimension", ArgumentType::Dimension), positive, |c, s| {
        let dimension = dimension_arg(c, s, "dimension")?;
        Ok(dimension == s.dimension())
    })))
    .then(literal("score").then(score_holder_arg("target", false).then(
        argument("targetObjective", ArgumentType::Objective)
            .then(compare("=", |c, s| check_scores(c, s, |a, b| a == b)))
            .then(compare("<", |c, s| check_scores(c, s, |a, b| a < b)))
            .then(compare("<=", |c, s| check_scores(c, s, |a, b| a <= b)))
            .then(compare(">", |c, s| check_scores(c, s, |a, b| a > b)))
            .then(compare(">=", |c, s| check_scores(c, s, |a, b| a >= b)))
            .then(literal("matches").then(conditional(exec, argument("range", ArgumentType::IntRange), positive, |c, s| {
                let holder = c.score_holder("target").name(s)?;
                let objective = objective(c, s, "targetObjective")?;
                let range = c.int_range("range");
                Ok(s.scoreboard().and_then(|sb| sb.score(&holder, &objective)).is_some_and(|v| range.matches(v)))
            }))),
    )))
    .then(literal("blocks").then(argument("start", ArgumentType::BlockPos).then(argument("end", ArgumentType::BlockPos).then(
        argument("destination", ArgumentType::BlockPos)
            .then(blocks_conditional(exec, literal("all"), positive, false))
            .then(blocks_conditional(exec, literal("masked"), positive, true)),
    ))))
    .then(literal("entity").then(numeric_conditional(exec, argument("entities", ArgumentType::entities()), positive, |c, s| {
        Ok(c.selector("entities").find_entities(s)?.len() as i32)
    })))
    .then(literal("predicate").then(conditional(exec, argument("predicate", ArgumentType::LootPredicate), positive, |c, s| {
        // `ExecuteCommand.checkCustomPredicate`.
        let predicate = match c.get("predicate") {
            Some(crate::arguments::ArgumentValue::Nbt(t)) => crate::host::LootTableArg::Inline(t.clone()),
            Some(crate::arguments::ArgumentValue::Identifier(id)) => crate::host::LootTableArg::Id(id.to_string()),
            other => unreachable!("predicate argument {other:?}"),
        };
        s.test_loot_predicate(&predicate)
    })))
    .then(literal("function").then(
        argument("name", ArgumentType::Function).suggests_server(super::function::suggest_functions)
            .fork(exec, move |c, s: &mut S| super::function::function_condition(c, s, positive)),
    ))
    .then(literal("items").then(items_branch(exec, positive, true)).then(items_branch(exec, positive, false)))
    .then(literal("slots").then(slots_branch(exec, positive, true)).then(slots_branch(exec, positive, false)))
    .then(literal("stopwatch").then(argument("id", ArgumentType::ResourceLocation).then(conditional(
        exec,
        argument("range", ArgumentType::FloatRange),
        positive,
        |c, s| {
            // `ExecuteCommand.checkStopwatch`.
            let id = c.identifier("id").to_string();
            let Some(seconds) = s.stopwatch_seconds(&id) else {
                return Err(CommandError::new(tr!("commands.stopwatch.does_not_exist", id)));
            };
            Ok(c.double_range("range").matches(seconds))
        },
    ))))
    .then(data_conditionals(exec, positive))
}

/// `if data block|entity|storage <source> <path>`: the number of matching tags.
fn data_conditionals<S: Host + 'static>(exec: NodeId, positive: bool) -> Builder<S> {
    literal("data")
        .then(literal("block").then(argument("source", ArgumentType::BlockPos).then(numeric_conditional(
            exec,
            argument("path", ArgumentType::NbtPath),
            positive,
            |c: &CommandContext<S>, s: &mut S| {
                let dimension = s.dimension().to_owned();
                let pos = loaded_block_pos(c, s, "source", &dimension)?;
                let data = s.block_entity(&dimension, pos).ok_or_else(block_not_entity)?;
                Ok(c.nbt_path("path").count_matching(&data) as i32)
            },
        ))))
        .then(literal("entity").then(argument("source", ArgumentType::entity()).then(numeric_conditional(
            exec,
            argument("path", ArgumentType::NbtPath),
            positive,
            |c: &CommandContext<S>, s: &mut S| {
                let e = c.selector("source").entity(s)?;
                let data = super::data::Accessor::Entity(e).get(s)?;
                Ok(c.nbt_path("path").count_matching(&data) as i32)
            },
        ))))
        .then(literal("storage").then(argument("source", ArgumentType::ResourceLocation).then(numeric_conditional(
            exec,
            argument("path", ArgumentType::NbtPath),
            positive,
            |c: &CommandContext<S>, s: &mut S| {
                let id = c.identifier("source").to_string();
                let data = s.storage_mut().map_or(Tag::Compound(Vec::new()), |st| st.get(&id));
                Ok(c.nbt_path("path").count_matching(&data) as i32)
            },
        ))))
}

fn block_not_entity() -> CommandError {
    CommandError::new(tr!("commands.data.block.invalid"))
}

/// `ObjectiveArgument.getObjective`.
fn objective<S: Host>(c: &CommandContext<S>, s: &S, name: &str) -> Result<String> {
    let objective = c.string(name);
    match s.scoreboard().and_then(|sb| sb.objective(objective)) {
        Some(o) => Ok(o.name.clone()),
        None => Err(CommandError::new(tr!("arguments.objective.notFound", objective))),
    }
}

/// `checkScore(ctx, IntBiPredicate)`: false when either score is unset.
fn check_scores<S: Host>(c: &CommandContext<S>, s: &mut S, cmp: fn(i32, i32) -> bool) -> Result<bool> {
    let target = c.score_holder("target").name(s)?;
    let target_objective = objective(c, s, "targetObjective")?;
    let source = c.score_holder("source").name(s)?;
    let source_objective = objective(c, s, "sourceObjective")?;
    let sb = s.scoreboard();
    let a = sb.and_then(|sb| sb.score(&target, &target_objective));
    let b = sb.and_then(|sb| sb.score(&source, &source_objective));
    Ok(a.zip(b).is_some_and(|(a, b)| cmp(a, b)))
}

/// `addIfBlocksConditional`.
fn blocks_conditional<S: Host + 'static>(exec: NodeId, b: Builder<S>, positive: bool, masked: bool) -> Builder<S> {
    b.fork(exec, move |c, s: &mut S| {
        let matched = check_regions(c, s, masked)?;
        Ok(expect(s, positive, matched.is_some()))
    })
    .executes(move |c, s: &mut S| match (positive, check_regions(c, s, masked)?) {
        (true, Some(n)) => {
            s.send_success(tr!("commands.execute.conditional.pass_count", n), false);
            Ok(n)
        }
        (true, None) => Err(CommandError::new(tr!("commands.execute.conditional.fail"))),
        (false, Some(n)) => Err(CommandError::new(tr!("commands.execute.conditional.fail_count", n))),
        (false, None) => {
            s.send_success(tr!("commands.execute.conditional.pass"), false);
            Ok(1)
        }
    })
}

/// `checkRegions`: the number of compared blocks if both regions match, `None` otherwise.
/// `masked` skips air (only `minecraft:air`) in the source.
fn check_regions<S: Host>(c: &CommandContext<S>, s: &mut S, masked: bool) -> Result<Option<i32>> {
    let dimension = s.dimension().to_owned();
    let start = loaded_block_pos(c, s, "start", &dimension)?;
    let end = loaded_block_pos(c, s, "end", &dimension)?;
    let dest = loaded_block_pos(c, s, "destination", &dimension)?;
    let src = BlockBox::from_corners(start, end);
    let len = src.length();
    let dst = BlockBox::from_corners(dest, std::array::from_fn(|i| dest[i] + len[i]));
    let offset: [i32; 3] = std::array::from_fn(|i| dst.min[i] - src.min[i]);
    let volume = src.volume();
    if volume > MAX_TEST_AREA {
        return Err(CommandError::new(tr!("commands.execute.blocks.toobig", MAX_TEST_AREA as i32, volume)));
    }
    let mut count = 0;
    for pos in src.positions() {
        let other: [i32; 3] = std::array::from_fn(|i| pos[i] + offset[i]);
        let state = s.block_state(&dimension, pos);
        if masked && state == AIR {
            continue;
        }
        if state != s.block_state(&dimension, other) {
            return Ok(None);
        }
        let a = s.block_entity(&dimension, pos);
        if a.is_some() && a != s.block_entity(&dimension, other) {
            return Ok(None);
        }
        count += 1;
    }
    Ok(Some(count))
}

/// The numeric NBT types of `store ... <path> <type> <scale>`.
const NUMERIC_TYPES: [&str; 6] = ["byte", "short", "int", "long", "float", "double"];

/// `ScoreHolderArgument.SUGGEST_SCORE_HOLDERS`: tracked holders and selectors.
pub(super) fn score_holder_arg<S: Host + 'static>(name: &str, multiple: bool) -> Builder<S> {
    argument(name, ArgumentType::ScoreHolder { multiple }).suggests_server(|_, s: &S, b| {
        let mut names = s.scoreboard().map(Scoreboard::holders).unwrap_or_default();
        names.extend(s.player_names());
        names.push("*".to_owned());
        crate::selector::suggest(b, s.permission() >= crate::selector::SELECTOR_PERMISSION, &names);
    })
}

/// Resolves the accessor of an NBT store target (`ArgProvider.access`).
type AccessFn<S> = fn(&CommandContext<S>, &mut S) -> Result<super::data::Accessor<<S as crate::host::Source>::Entity>>;

/// `<path> (byte|short|int|long|float|double) <scale>` under an NBT store target:
/// `storeData`, whose failures (no such parent, player data) are dropped as in vanilla.
fn nbt_target<S: Host + 'static>(exec: NodeId, target: Builder<S>, access: AccessFn<S>, result: bool) -> Builder<S> {
    use super::data::Accessor;
    /// The accessor without the entity handle (callbacks must be `Send`).
    enum Key {
        Block(String, [i32; 3]),
        Entity(uuid::Uuid),
        Storage(String),
    }
    let path = NUMERIC_TYPES.into_iter().fold(argument("path", ArgumentType::NbtPath), |p, ty| {
        p.then(literal(ty).then(argument("scale", ArgumentType::double()).redirect_with(exec, move |c, s: &mut S| {
            let key = match access(c, s)? {
                Accessor::Block { dimension, pos } => Key::Block(dimension, pos),
                Accessor::Entity(e) => Key::Entity(e.uuid()),
                Accessor::Storage(id) => Key::Storage(id),
            };
            let path = c.nbt_path("path").clone();
            let scale = c.double("scale");
            Ok(s.stack().clone().with_callback(Arc::new(move |s: &mut S, success, value| {
                let v = if result { value } else { success as i32 };
                let acc = match &key {
                    Key::Block(dimension, pos) => Accessor::Block { dimension: dimension.clone(), pos: *pos },
                    Key::Entity(uuid) => match s.entity_by_uuid(*uuid) {
                        Some(e) => Accessor::Entity(e),
                        None => return,
                    },
                    Key::Storage(id) => Accessor::Storage(id.clone()),
                };
                if let Ok(mut data) = acc.get(s)
                    && path.set(&mut data, &numeric_tag(ty, v, scale)).is_ok()
                {
                    let _ = acc.set(s, data);
                }
            })))
        })))
    });
    target.then(path)
}

/// `wrapStores`: `score`, `bossbar`, and the NBT targets.
fn stores<S: Host + 'static>(exec: NodeId, b: Builder<S>, result: bool) -> Builder<S> {
    b.then(literal("score").then(
        score_holder_arg("targets", true).then(
            argument("objective", ArgumentType::Objective).redirect_with(exec, move |c, s: &mut S| {
                let wildcard = s.scoreboard().map(Scoreboard::holders);
                let holders = c.score_holder("targets").names(s, wildcard)?;
                let objective = objective(c, s, "objective")?;
                Ok(s.stack().clone().with_callback(Arc::new(move |s: &mut S, success, value| {
                    let v = if result { value } else { success as i32 };
                    if let Some(sb) = s.scoreboard_mut() {
                        for h in &holders {
                            sb.set_score(h, &objective, v);
                        }
                    }
                })))
            }),
        ),
    ))
    .then(literal("bossbar").then({
        let bar = move |max: bool| {
            move |c: &CommandContext<S>, s: &mut S| {
                let id: Identifier = c.identifier("id").clone();
                if !s.has_bossbar(&id) {
                    return Err(CommandError::new(tr!("commands.bossbar.unknown", id.to_string())));
                }
                Ok(s.stack().clone().with_callback(Arc::new(move |s: &mut S, success, value| {
                    let v = if result { value } else { success as i32 };
                    let _ = s.set_bossbar(&id, max, v);
                })))
            }
        };
        argument("id", ArgumentType::ResourceLocation)
            .suggests_server(|_, _: &S, _| {})
            .then(literal("value").redirect_with(exec, bar(false)))
            .then(literal("max").redirect_with(exec, bar(true)))
    }))
    .then(literal("block").then(nbt_target(
        exec,
        argument("target", ArgumentType::BlockPos),
        |c, s| {
            let dimension = s.dimension().to_owned();
            let pos = loaded_block_pos(c, s, "target", &dimension)?;
            s.block_entity(&dimension, pos).ok_or_else(block_not_entity)?;
            Ok(super::data::Accessor::Block { dimension, pos })
        },
        result,
    )))
    .then(literal("entity").then(nbt_target(
        exec,
        argument("target", ArgumentType::entity()),
        |c, s| Ok(super::data::Accessor::Entity(c.selector("target").entity(s)?)),
        result,
    )))
    .then(literal("storage").then(storage_target(exec, result)))
}

/// `storage <target> <path> <type> <scale>`: `storeData` into command storage.
fn storage_target<S: Host + 'static>(exec: NodeId, result: bool) -> Builder<S> {
    let path = NUMERIC_TYPES.into_iter().fold(argument("path", ArgumentType::NbtPath), |p, ty| {
        p.then(literal(ty).then(argument("scale", ArgumentType::double()).redirect_with(exec, move |c, s: &mut S| {
            if s.storage_mut().is_none() {
                return Err(CommandError::unsupported("Command storage"));
            }
            let id = c.identifier("target").to_string();
            let path = c.nbt_path("path").clone();
            let scale = c.double("scale");
            Ok(s.stack().clone().with_callback(Arc::new(move |s: &mut S, success, value| {
                let v = if result { value } else { success as i32 };
                if let Some(storage) = s.storage_mut() {
                    // Failures (no such parent) are dropped, as in vanilla.
                    let _ = storage.store(&id, &path, &numeric_tag(ty, v, scale));
                }
            })))
        })))
    });
    argument("target", ArgumentType::ResourceLocation).then(path)
}

/// The tag `store ... <type> <scale>` writes: `(type) (value * scale)` with Java's casts.
fn numeric_tag(ty: &str, value: i32, scale: f64) -> Tag {
    let v = f64::from(value) * scale;
    match ty {
        "byte" => Tag::Byte(v as i32 as i8),
        "short" => Tag::Short(v as i32 as i16),
        "int" => Tag::Int(v as i32),
        "long" => Tag::Long(v as i64),
        "float" => Tag::Float(v as f32),
        _ => Tag::Double(v),
    }
}
