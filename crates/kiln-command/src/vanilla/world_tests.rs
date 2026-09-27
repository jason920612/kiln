//! `setblock`, `fill`, `clone`, `execute` and `tellraw` against the mock host (stone below
//! y 64, air above, chunks within 10 of the origin loaded).

use super::tests::{Mock, dispatcher, err_key};
use crate::host::{GameRuleValue, Source};
use crate::scoreboard::Objective;
use crate::text::Text;
use kiln_data::blocks::default_state as b;
use kiln_data::blocks_types::block_by_name;
use kiln_proto::nbt::Tag;

fn objective(name: &str) -> Objective {
    Objective::new(name, "dummy", Text::literal(name))
}

#[test]
fn setblock_modes() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    assert_eq!(s.run(&d, "setblock 1 64 1 gold_block"), Ok(1));
    assert_eq!(s.block([1, 64, 1]), b::GOLD_BLOCK);
    assert_eq!(s.feedback.pop().unwrap(), ("commands.setblock.success[1, 64, 1]".into(), true));
    assert_eq!(err_key(s.run(&d, "setblock 1 64 1 gold_block")), "commands.setblock.failed", "unchanged");
    assert_eq!(err_key(s.run(&d, "setblock 1 64 1 stone keep")), "commands.setblock.failed", "not air");
    assert_eq!(s.run(&d, "setblock 1 65 1 stone keep"), Ok(1));
    assert_eq!(s.run(&d, "setblock 1 65 1 oak_log[axis=x] strict"), Ok(1));
    let log = block_by_name("minecraft:oak_log").unwrap();
    assert_eq!(log.property(s.block([1, 65, 1]), "axis"), Some("x"));
    assert_eq!(s.run(&d, "setblock 1 63 1 air destroy"), Ok(1));
    assert_eq!(s.effects.pop().unwrap(), "destroy [1, 63, 1] true");
    // Destroying into air when it is already air places nothing but succeeds.
    assert_eq!(s.run(&d, "setblock 1 63 1 air destroy"), Ok(1));
    assert_eq!(s.run(&d, "setblock 2 64 2 chest{Lock:\"k\"}"), Ok(1));
    assert_eq!(s.effects.pop().unwrap(), "nbt [2, 64, 2] {Lock:\"k\"}");
    assert_eq!(err_key(s.run(&d, "setblock 1000 64 0 stone")), "argument.pos.unloaded");
    assert_eq!(err_key(s.run(&d, "setblock 0 320 0 stone")), "argument.pos.outofworld");
    assert_eq!(err_key(s.run(&d, "setblock 0 64 0 #minecraft:logs")), "argument.block.tag.disallowed");
    // Relative to the source: Alice stands at 0.5 64 0.5.
    let p = &mut Mock::new(2);
    assert_eq!(p.run(&d, "setblock ~ ~1 ~ glass"), Ok(1));
    assert_eq!(p.block([0, 65, 0]), b::GLASS);
}

#[test]
fn fill_modes_and_limits() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    assert_eq!(s.run(&d, "fill 0 64 0 2 66 2 dirt"), Ok(27));
    assert_eq!(s.feedback.pop().unwrap(), ("commands.fill.success[27]".into(), true));
    assert_eq!(err_key(s.run(&d, "fill 0 64 0 2 66 2 dirt")), "commands.fill.failed");
    assert_eq!(s.run(&d, "fill 0 64 0 2 66 2 glass hollow"), Ok(27), "outer 26 glass, the core becomes air");
    assert_eq!(s.block([1, 65, 1]), b::AIR);
    assert_eq!(s.run(&d, "fill 0 64 0 2 66 2 stone outline"), Ok(26));
    assert_eq!(s.block([1, 65, 1]), b::AIR, "outline leaves the inside");
    assert_eq!(s.run(&d, "fill 0 64 0 2 66 2 dirt replace stone"), Ok(26));
    assert_eq!(s.run(&d, "fill 0 64 0 2 66 2 sand keep"), Ok(1), "only the air block");
    assert_eq!(s.run(&d, "fill 0 64 0 2 66 2 glass replace #minecraft:sand strict"), Ok(1));
    assert_eq!(s.run(&d, "fill 5 62 5 5 64 5 air destroy"), Ok(2), "two destroyed stones; placing air in air fails");
    s.rules.insert("minecraft:max_block_modifications".into(), GameRuleValue::Int(10));
    let e = s.run(&d, "fill 0 64 0 2 66 2 dirt").unwrap_err();
    assert_eq!(e.key(), Some("commands.fill.toobig"));
    assert_eq!(e.args(), &[crate::text::Arg::Int(10), crate::text::Arg::Long(27)]);
}

#[test]
fn clone_modes() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    s.run(&d, "fill 0 64 0 1 64 1 gold_block").unwrap();
    assert_eq!(s.run(&d, "clone 0 64 0 1 65 1 10 64 10"), Ok(8), "air counts too");
    assert_eq!(s.block([11, 64, 11]), b::GOLD_BLOCK);
    assert_eq!(s.feedback.pop().unwrap(), ("commands.clone.success[8]".into(), true));
    assert_eq!(err_key(s.run(&d, "clone 0 64 0 1 65 1 1 64 1")), "commands.clone.overlap");
    assert_eq!(s.run(&d, "clone 0 64 0 1 65 1 20 64 20 masked"), Ok(4), "air skipped");
    assert_eq!(s.run(&d, "clone 0 64 0 1 64 1 1 64 0 replace force"), Ok(4));
    assert_eq!(s.run(&d, "clone 10 64 10 11 64 11 30 64 30 filtered minecraft:gold_block move"), Ok(4));
    assert_eq!(s.block([10, 64, 10]), b::AIR, "moved away");
    assert_eq!(s.block([31, 64, 31]), b::GOLD_BLOCK);
    assert_eq!(err_key(s.run(&d, "clone 40 70 40 41 70 41 50 70 50 masked")), "commands.clone.failed");
    assert_eq!(s.run(&d, "clone 0 64 0 0 64 0 to minecraft:the_nether 0 80 0"), Ok(1));
    assert_eq!(err_key(s.run(&d, "clone from minecraft:the_end 0 64 0 0 64 0 5 64 5")), "argument.dimension.invalid");
    assert_eq!(err_key(s.run(&d, "clone 0 64 0 0 64 0 200 64 200")), "argument.pos.unloaded");
    s.rules.insert("minecraft:max_block_modifications".into(), GameRuleValue::Int(3));
    assert_eq!(err_key(s.run(&d, "clone 0 64 0 1 65 1 10 64 10")), "commands.clone.toobig");
}

#[test]
fn execute_conditions() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    assert_eq!(s.run(&d, "execute if block 0 63 0 stone"), Ok(1));
    assert_eq!(s.feedback.pop().unwrap(), ("commands.execute.conditional.pass".into(), false));
    assert_eq!(err_key(s.run(&d, "execute if block 0 64 0 stone")), "commands.execute.conditional.fail");
    assert_eq!(s.run(&d, "execute unless block 0 64 0 stone"), Ok(1));
    assert_eq!(s.run(&d, "execute if block 0 63 0 #minecraft:base_stone_overworld"), Ok(1));
    assert_eq!(err_key(s.run(&d, "execute if block 1000 63 0 stone")), "argument.pos.unloaded");
    assert_eq!(s.run(&d, "execute if entity @e[type=zombie]"), Ok(1));
    assert_eq!(s.feedback.pop().unwrap().0, "commands.execute.conditional.pass_count[1]");
    assert_eq!(s.run(&d, "execute if entity @a"), Ok(3));
    let e = s.run(&d, "execute unless entity @a").unwrap_err();
    assert_eq!((e.key(), e.args()), (Some("commands.execute.conditional.fail_count"), &[crate::text::Arg::Int(3)][..]));
    assert_eq!(s.run(&d, "execute if blocks 0 60 0 1 61 1 5 60 5 all"), Ok(8));
    s.run(&d, "setblock 5 60 5 dirt").unwrap();
    assert_eq!(err_key(s.run(&d, "execute if blocks 0 60 0 1 61 1 5 60 5 all")), "commands.execute.conditional.fail");
    assert_eq!(s.run(&d, "execute unless blocks 0 60 0 1 61 1 5 60 5 all"), Ok(1));
    s.run(&d, "setblock 0 70 0 dirt").unwrap();
    assert_eq!(s.run(&d, "execute if blocks 0 70 0 0 71 0 5 60 5 masked"), Ok(1), "air in the source is skipped");
    assert_eq!(err_key(s.run(&d, "execute if blocks 0 0 0 40 40 40 0 0 0 all")), "commands.execute.blocks.toobig");
    assert_eq!(s.run(&d, "execute if dimension overworld"), Ok(1));
    assert_eq!(s.run(&d, "execute if loaded 0 0 0"), Ok(1));
    assert_eq!(err_key(s.run(&d, "execute if loaded 1000 0 0")), "commands.execute.conditional.fail");
    assert_eq!(s.run(&d, "execute if biome 0 64 0 plains"), Ok(1));
    assert_eq!(s.run(&d, "execute if biome 0 64 0 #minecraft:is_overworld"), Ok(1));
    assert_eq!(err_key(s.run(&d, "execute if biome 0 64 0 #minecraft:is_nether")), "commands.execute.conditional.fail");
    assert_eq!(err_key(s.run(&d, "execute if data block 0 63 0 x")), "commands.data.block.invalid");
    assert_eq!(err_key(s.run(&d, "execute if data storage kiln:x a")), "commands.execute.conditional.fail");
    assert_eq!(err_key(s.run(&d, "execute if predicate kiln:p")), "argument.resource_or_id.no_such_element");
    assert_eq!(err_key(s.run(&d, "execute if stopwatch kiln:w 1..")), "commands.stopwatch.does_not_exist");
    assert_eq!(err_key(s.run(&d, "execute if function kiln:f")), "command.unknown.command", "no executes");
    // After a fork, the function lookup error is silent.
    assert_eq!(s.run(&d, "execute if function kiln:f run say hi"), Ok(0));
}

#[test]
fn execute_modifiers() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    assert_eq!(s.run(&d, "execute as @a run kill @s"), Ok(3));
    assert_eq!(s.effects, ["kill Alice", "kill Bob", "kill Carol"]);
    assert!(s.stack().entity.is_none(), "the console stack is restored");
    // `at` moves to each entity; Bob stands at 10.5 64 0.5 in the overworld.
    assert_eq!(s.run(&d, "execute at Bob run setblock ~ ~ ~ gold_block"), Ok(1));
    assert_eq!(s.block([10, 64, 0]), b::GOLD_BLOCK);
    assert_eq!(s.run(&d, "execute positioned 5 70 5 run setblock ~ ~ ~ glass"), Ok(1));
    assert_eq!(s.block([5, 70, 5]), b::GLASS);
    assert_eq!(s.run(&d, "execute positioned 5.7 70.2 5.9 align xz run setblock ~ ~1 ~ glass"), Ok(1));
    assert_eq!(s.block([5, 71, 5]), b::GLASS);
    assert_eq!(s.run(&d, "execute positioned 3 100 3 positioned over world_surface run setblock ~ ~ ~ sand"), Ok(1));
    assert_eq!(s.block([3, 64, 3]), b::SAND, "on top of the stone");
    // Facing +x, ^ ^ ^2 is two blocks east.
    assert_eq!(s.run(&d, "execute positioned 0 70 0 facing 10 70 0 run setblock ^ ^ ^2 stone"), Ok(1));
    assert_eq!(s.block([2, 70, 0]), b::STONE);
    assert_eq!(s.run(&d, "execute positioned 0.5 72 0.5 rotated -90 0 run setblock ^ ^ ^3 stone"), Ok(1));
    assert_eq!(s.block([3, 72, 0]), b::STONE);
    // `in` scales x and z into the nether.
    assert_eq!(s.run(&d, "execute positioned 16 70 16 in the_nether run setblock ~ ~ ~ stone"), Ok(1));
    assert_eq!(s.block([2, 70, 2]), b::STONE);
    assert_eq!(err_key(s.run(&d, "execute in the_end run say x")), "argument.dimension.invalid");
    // Anchored eyes: local coordinates start at the eyes (Alice: 64 + 1.62).
    let p = &mut Mock::new(2);
    assert_eq!(p.run(&d, "execute anchored eyes run setblock ^ ^ ^ stone"), Ok(1));
    assert_eq!(p.block([0, 65, 0]), b::STONE);
    assert_eq!(err_key(s.run(&d, "execute summon zombie run say hi")), "commands.summon.failed");
    assert_eq!(s.run(&d, "execute as @a on vehicle run kill @s"), Ok(0), "players ride nothing");
    assert_eq!(err_key(s.run(&d, "execute run")), "command.unknown.command");
}

#[test]
fn execute_errors_after_forks_are_silent() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    s.rules.insert("minecraft:max_block_modifications".into(), GameRuleValue::Int(2));
    assert_eq!(err_key(s.run(&d, "execute positioned 0 70 0 run fill ~ ~ ~ ~5 ~ ~ stone")), "commands.fill.toobig");
    assert_eq!(s.run(&d, "execute as @a run fill 0 70 0 5 70 0 stone"), Ok(0));
    assert_eq!(s.run(&d, "execute if block 0 63 0 stone run fill 0 70 0 5 70 0 stone"), Ok(0));
    // A zero fork limit stops every modifier; only unforked stages report it.
    s.rules.insert("minecraft:max_command_forks".into(), GameRuleValue::Int(1));
    assert_eq!(err_key(s.run(&d, "execute positioned 0 0 0 run say hi")), "command.forkLimit");
    assert_eq!(s.run(&d, "execute as @a run say hi"), Ok(0));
    s.rules.remove("minecraft:max_command_forks");
    s.rules.insert("minecraft:max_command_sequence_length".into(), GameRuleValue::Int(3));
    s.effects.clear();
    assert_eq!(s.run(&d, "execute as @a run kill @s"), Ok(2), "1 stage + 2 executions");
    assert_eq!(s.effects, ["kill Alice", "kill Bob"]);
}

#[test]
fn execute_store_and_scores() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    assert_eq!(err_key(s.run(&d, "execute store result score @a obj run seed")), "arguments.objective.notFound");
    s.scoreboard.add_objective(objective("obj"));
    s.scoreboard.add_objective(objective("other"));
    s.run(&d, "execute store result score Alice obj run fill 0 70 0 2 70 0 stone").unwrap();
    assert_eq!(s.scoreboard.score("Alice", "obj"), Some(3));
    s.run(&d, "execute store success score #fake obj run fill 0 70 0 2 70 0 stone").unwrap_err();
    assert_eq!(s.scoreboard.score("#fake", "obj"), Some(0), "failures store 0");
    s.run(&d, "execute store success score #fake obj run setblock 0 71 0 stone").unwrap();
    assert_eq!(s.scoreboard.score("#fake", "obj"), Some(1));
    // Per execution: each player stores its own result.
    s.run(&d, "execute as @a store result score @s other run list").unwrap();
    assert_eq!(s.scoreboard.score("Bob", "other"), Some(3));
    assert_eq!(s.run(&d, "execute if score Alice obj matches 3"), Ok(1));
    assert_eq!(err_key(s.run(&d, "execute if score Alice obj matches 4..")), "commands.execute.conditional.fail");
    assert_eq!(s.run(&d, "execute if score Alice obj > #fake obj"), Ok(1));
    assert_eq!(s.run(&d, "execute if score Alice obj = Alice other"), Ok(1));
    assert_eq!(err_key(s.run(&d, "execute if score Nobody obj < Alice obj")), "commands.execute.conditional.fail");
    assert_eq!(s.run(&d, "execute if entity @a[scores={obj=3}]"), Ok(1), "selectors read the scoreboard");
    // `*` in store stands for every tracked holder.
    s.run(&d, "execute store result score * obj run seed").unwrap();
    assert_eq!(s.scoreboard.score("Bob", "obj"), Some(-1234567890123i64 as i32));
    assert_eq!(err_key(s.run(&d, "execute store result bossbar kiln:b value run seed")), "commands.bossbar.unknown");
    // Storage: Java casts of value * scale, parents created on the way.
    s.run(&d, "execute store result storage kiln:s a.b byte 0.5 run fill 0 72 0 4 72 0 stone").unwrap();
    assert_eq!(s.storage.get("kiln:s"), Tag::Compound(vec![("a".into(), Tag::Compound(vec![("b".into(), Tag::Byte(2))]))]));
    s.run(&d, "execute store result storage kiln:s big int 1 run seed").unwrap();
    assert_eq!(s.run(&d, "execute if data storage kiln:s big"), Ok(1));
    assert_eq!(s.run(&d, "execute if data storage kiln:s a.b"), Ok(1));
    assert_eq!(err_key(s.run(&d, "execute if data storage kiln:other a")), "commands.execute.conditional.fail");
    s.run(&d, "execute store success storage kiln:s a.b.c double 1 run seed").unwrap();
    assert_eq!(err_key(s.run(&d, "execute if data storage kiln:s a.b.c")), "commands.execute.conditional.fail", "no parent");
    // Player data writes are dropped silently, as vanilla drops them.
    assert_eq!(Mock::new(2).run(&d, "execute store result entity @s Health float 1 run seed").map(|_| ()), Ok(()));
}

#[test]
fn tellraw_resolves_per_recipient() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    s.scoreboard.add_objective(objective("obj"));
    s.scoreboard.set_score("Bob", "obj", 7);
    assert_eq!(s.run(&d, "tellraw @a[distance=..20] {text:'hi ',extra:[{selector:'@s'}]}"), Ok(2));
    assert_eq!(s.chat, ["to Alice: hi Alice", "to Bob: hi Bob"]);
    s.chat.clear();
    s.run(&d, "tellraw Bob [\"score: \",{score:{name:'*',objective:obj}}]").unwrap();
    assert_eq!(s.chat, ["to Bob: score: 7"]);
    assert_eq!(err_key(s.run(&d, "tellraw @a {bold:1b}")), "argument.component.invalid");
    assert_eq!(err_key(s.run(&d, "tellraw Nobody \"x\"")), "argument.entity.notfound.player");
}

fn load(s: &mut Mock, id: &str, lines: &[&str]) {
    let f = crate::functions::CommandFunction::from_lines(crate::Identifier::parse(id).unwrap(), lines).unwrap();
    s.functions.insert(f);
}

#[test]
fn functions_return_and_schedule() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    s.scoreboard.add_objective(objective("o"));
    load(s, "k:ret", &["setblock 1 64 1 gold_block", "return 7", "setblock 2 64 2 gold_block"]);
    load(s, "k:none", &["scoreboard players add #n o 1"]);
    load(s, "k:outer", &["function k:ret", "scoreboard players add #n o 10", "return run function k:none"]);
    load(s, "k:m", &["$scoreboard players set #m o $(v)"]);
    s.functions.set_tag(crate::Identifier::parse("k:t").unwrap(), vec![
        crate::Identifier::parse("k:ret").unwrap(),
        crate::Identifier::parse("k:none").unwrap(),
    ]);
    assert_eq!(s.run(&d, "function k:ret"), Ok(0));
    // Feedback from inside the body is suppressed; the call and its result are not.
    assert_eq!(s.feedback_keys(), ["commands.function.scheduled.single[k:ret]", "commands.function.result[k:ret, 7]"]);
    assert_eq!(s.block([1, 64, 1]), b::GOLD_BLOCK);
    assert_eq!(s.block([2, 64, 2]), b::AIR, "return discards the rest");
    s.feedback.clear();
    s.run(&d, "function k:outer").unwrap();
    assert_eq!(s.scoreboard.score("#n", "o"), Some(11), "a callee's return does not end the caller");
    // `return run function k:none`: k:none never returns, so k:outer returns failure (0).
    assert_eq!(s.feedback.pop().unwrap().0, "commands.function.result[k:outer, 0]");
    s.run(&d, "execute store result score #x o run function #k:t").unwrap();
    assert_eq!(s.scoreboard.score("#x", "o"), Some(7), "the returned values add up");
    s.run(&d, "execute store result score #y o run function k:none").unwrap();
    assert_eq!(s.scoreboard.score("#y", "o"), None, "no return, nothing stored");
    s.run(&d, "function k:m {v:5}").unwrap();
    assert_eq!(s.scoreboard.score("#m", "o"), Some(5));
    assert_eq!(err_key(s.run(&d, "function k:m")), "commands.function.instantiationFailure");
    assert_eq!(err_key(s.run(&d, "function k:nope")), "arguments.function.unknown");
    assert_eq!(err_key(s.run(&d, "function #k:nope")), "commands.function.scheduled.no_functions");
    assert_eq!(s.run(&d, "execute if function k:ret run seed").map(|_| ()), Ok(()));
    assert_eq!(s.run(&d, "execute if function k:none run seed"), Ok(0), "no source passes, silently");
    assert_eq!(s.run(&d, "schedule function k:none 5t"), Ok(105));
    assert_eq!(err_key(s.run(&d, "schedule function k:m 5t")), "commands.schedule.macro");
    assert_eq!(err_key(s.run(&d, "schedule function k:none 0")), "commands.schedule.same_tick");
    assert_eq!(s.run(&d, "schedule clear k:none"), Ok(1));
    assert_eq!(err_key(s.run(&d, "schedule clear k:none")), "commands.schedule.cleared.failure");
    // The command quota covers nested functions.
    load(s, "k:loop", &["scoreboard players add #l o 1", "function k:loop"]);
    s.rules.insert("minecraft:max_command_sequence_length".into(), crate::host::GameRuleValue::Int(20));
    s.run(&d, "function k:loop").unwrap();
    let loops = s.scoreboard.score("#l", "o").unwrap();
    assert!(loops > 1 && loops < 20, "{loops}");
}

#[test]
fn teams_bossbars_titles_and_triggers() {
    let d = dispatcher();
    let s = &mut Mock::console(4);
    assert_eq!(s.run(&d, "team add red"), Ok(1));
    assert_eq!(err_key(s.run(&d, "team add red")), "commands.team.add.duplicate");
    assert_eq!(s.run(&d, "team join red @a"), Ok(3), "Alice, Bob and Carol");
    s.run(&d, "team modify red prefix \"[R] \"").unwrap();
    assert_eq!(s.scoreboard.player_display_name("Bob").to_plain(), "[R] Bob");
    assert_eq!(err_key(s.run(&d, "team modify red color reset")), "commands.team.option.color.unchanged");
    assert_eq!(err_key(s.run(&d, "team modify nope color red")), "team.notFound");
    assert_eq!(s.run(&d, "team leave Bob"), Ok(1));
    assert_eq!(s.run(&d, "team empty red"), Ok(2));
    assert_eq!(s.feedback.pop().unwrap().0, "commands.team.empty.success[2, [red]]");
    // Add, change (display name), 3 joins, change (prefix), leave, 2 leaves.
    assert_eq!(s.scoreboard.take_packets().len(), 9);

    assert_eq!(s.run(&d, "bossbar add kiln:b \"B\""), Ok(1));
    assert_eq!(s.run(&d, "bossbar set kiln:b max 10"), Ok(10));
    s.run(&d, "execute store result bossbar kiln:b value run bossbar get kiln:b max").unwrap();
    assert_eq!(s.bossbars.get(&crate::Identifier::parse("kiln:b").unwrap()).unwrap().progress, 1.0);
    assert_eq!(err_key(s.run(&d, "bossbar set kiln:b value 10")), "commands.bossbar.set.value.unchanged");
    assert_eq!(s.run(&d, "bossbar set kiln:b players @a[name=!Carol]"), Ok(2));
    assert_eq!(s.bossbars.take_packets().len(), 2, "Alice and Bob see it");

    assert_eq!(s.run(&d, "title @a[name=!Carol] times 1 2 3"), Ok(2));
    assert_eq!(s.packets.len(), 2);
    assert_eq!(s.feedback.pop().unwrap().0, "commands.title.times.multiple[2]");

    s.run(&d, "scoreboard objectives add t trigger").unwrap();
    assert_eq!(err_key(s.run(&d, "trigger t")), "permissions.requires.player");
    let p = &mut Mock::new(0);
    p.scoreboard = s.scoreboard.clone();
    assert_eq!(err_key(p.run(&d, "trigger t")), "commands.trigger.failed.unprimed");
    let mut a = p.scoreboard.access("Alice", "t");
    p.scoreboard.set_locked(&mut a, false);
    assert_eq!(p.run(&d, "trigger t add 4"), Ok(4));
    assert_eq!(err_key(p.run(&d, "trigger t")), "commands.trigger.failed.unprimed", "locked again");
}
