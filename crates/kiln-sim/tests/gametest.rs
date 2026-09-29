//! `/test`: the game test framework. Test instances come from a data pack in the world's
//! `datapacks`, and their structures from templates in the same pack (the built-in
//! `minecraft:always_pass` and `minecraft:empty` come with the vanilla data, found through
//! `KILN_DATAPACK` or `work/generated`).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::io::Write;
use std::path::{Path, PathBuf};

fn vanilla_data() -> bool {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let work = std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        if !dir.join("data/minecraft/test_instance/always_pass.json").exists() {
            eprintln!("skipped: no vanilla data (KILN_DATAPACK)");
            return false;
        }
        unsafe { std::env::set_var("KILN_DATAPACK", dir) };
    }
    true
}

fn list(items: Vec<Tag>) -> Tag {
    Tag::List(items)
}

fn compound(fields: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// A structure template of `size` with `blocks`: (position, block id, properties, block entity
/// data).
fn template(size: [i32; 3], blocks: &[([i32; 3], &str, &[(&str, &str)], Option<Tag>)]) -> Tag {
    let mut palette: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut out = Vec::new();
    for (pos, name, props, nbt) in blocks {
        let key = (name.to_string(), props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>());
        let state = palette.iter().position(|p| *p == key).unwrap_or_else(|| {
            palette.push(key);
            palette.len() - 1
        });
        let mut b = vec![("pos", list(pos.iter().map(|&v| Tag::Int(v)).collect())), ("state", Tag::Int(state as i32))];
        if let Some(n) = nbt {
            b.push(("nbt", n.clone()));
        }
        out.push(compound(b));
    }
    let palette = palette
        .into_iter()
        .map(|(name, props)| {
            let mut f = vec![("Name", Tag::String(name))];
            if !props.is_empty() {
                f.push(("Properties", Tag::Compound(props.into_iter().map(|(k, v)| (k, Tag::String(v))).collect())));
            }
            compound(f)
        })
        .collect();
    compound(vec![
        ("size", list(size.iter().map(|&v| Tag::Int(v)).collect())),
        ("palette", list(palette)),
        ("blocks", list(out)),
        ("entities", list(Vec::new())),
        ("DataVersion", Tag::Int(5023)),
    ])
}

fn write_structure(pack: &Path, id: &str, tag: &Tag) {
    let path = pack.join(format!("data/kilntest/structure/{id}.nbt"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut buf = bytes::BytesMut::new();
    tag.write_named("", &mut buf);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&buf).unwrap();
    std::fs::write(path, gz.finish().unwrap()).unwrap();
}

struct Game {
    sim: Sim,
    client: Client,
}

impl Game {
    /// A world whose pack `kilntest` defines `tests` (id, JSON), `environments` and
    /// `structures`.
    fn new(name: &str, tests: &[(&str, &str)], environments: &[(&str, &str)], structures: &[(&str, Tag)]) -> Option<Game> {
        if !vanilla_data() {
            return None;
        }
        let world = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("gametest-{name}"));
        let _ = std::fs::remove_dir_all(&world);
        let pack = world.join("datapacks/kilntest");
        std::fs::create_dir_all(pack.join("data/kilntest/test_instance")).unwrap();
        std::fs::create_dir_all(pack.join("data/kilntest/test_environment")).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"tests","min_format":121,"max_format":121}}"#).unwrap();
        for (id, json) in tests {
            std::fs::write(pack.join(format!("data/kilntest/test_instance/{id}.json")), json).unwrap();
        }
        for (id, json) in environments {
            std::fs::write(pack.join(format!("data/kilntest/test_environment/{id}.json")), json).unwrap();
        }
        for (id, tag) in structures {
            write_structure(&pack, id, tag);
        }
        let mut sim = Sim::new(SimConfig::new(4, 4, Some(world)));
        sim.capture_console();
        let (msg, stats) = join(1, "Tester", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative Tester".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut g = Game { sim, client: Client::new(1, stats) };
        g.ticks(5);
        g.sim.take_console();
        Some(g)
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    /// Runs a console command in one tick and returns what the console printed.
    fn run(&mut self, command: &str) -> Vec<String> {
        let mut inbox = Vec::new();
        self.client.tick(None, &mut inbox);
        inbox.push(ToSim::Console(command.into()));
        assert!(self.sim.step(inbox));
        self.sim.take_console()
    }

    /// Ticks until the console printed `until` or `limit` ticks passed; everything printed.
    fn until(&mut self, until: &str, limit: usize) -> Vec<String> {
        let mut all = Vec::new();
        for _ in 0..limit {
            self.ticks(1);
            all.extend(self.sim.take_console());
            if all.iter().any(|l| l.contains(until)) {
                break;
            }
        }
        all
    }
}

const PASS: &str = r#"{"type":"minecraft:function","function":"minecraft:always_pass","environment":"minecraft:default","structure":"minecraft:empty","max_ticks":1,"setup_ticks":1}"#;

fn block_test(structure: &str, extra: &str) -> String {
    format!(r#"{{"type":"minecraft:block_based","environment":"minecraft:default","structure":"kilntest:{structure}","max_ticks":20{extra}}}"#)
}

fn start_accept() -> Tag {
    template(
        [3, 2, 1],
        &[
            ([0, 0, 0], "minecraft:stone", &[], None),
            ([1, 0, 0], "minecraft:stone", &[], None),
            ([2, 0, 0], "minecraft:stone", &[], None),
            ([0, 1, 0], "minecraft:test_block", &[("mode", "start")], None),
            ([1, 1, 0], "minecraft:test_block", &[("mode", "accept")], None),
        ],
    )
}

fn start_fail() -> Tag {
    template(
        [3, 2, 1],
        &[
            ([0, 0, 0], "minecraft:stone", &[], None),
            ([0, 1, 0], "minecraft:test_block", &[("mode", "start")], None),
            ([1, 1, 0], "minecraft:test_block", &[("mode", "fail")], Some(compound(vec![("message", Tag::String("boom".into()))]))),
            ([2, 1, 0], "minecraft:test_block", &[("mode", "accept")], None),
        ],
    )
}

fn start_only() -> Tag {
    template([3, 2, 1], &[([0, 1, 0], "minecraft:test_block", &[("mode", "start")], None), ([2, 1, 0], "minecraft:test_block", &[("mode", "accept")], None)])
}

fn no_start() -> Tag {
    template([1, 1, 1], &[([0, 0, 0], "minecraft:test_block", &[("mode", "accept")], None)])
}

#[test]
fn always_pass_runs_and_reports_after_its_setup_ticks() {
    let Some(mut g) = Game::new("always-pass", &[], &[], &[]) else { return };
    let first = g.run("test run minecraft:always_pass");
    assert_eq!(first, ["commands.test.run.running[1]", "commands.test.batch.starting[minecraft:default, 0]"]);
    // Two setup ticks, then the test starts and passes in the third tick after this one; the
    // console has no player, so the summary says where the tests were put.
    let rest = g.until("test.run.coordinates", 10);
    assert_eq!(rest.len(), 3, "{rest:?}");
    assert_eq!(rest[0], "commands.test.summary[1]");
    assert_eq!(rest[1], "commands.test.summary.all_required_passed");
    assert!(rest[2].starts_with("[test.run.coordinates["), "{rest:?}");
    // The test instance block is left behind, finished.
    let near = g.run("test locate minecraft:always_pass");
    assert_eq!(near[0], "commands.test.locate.started");
    assert!(near[1].starts_with("commands.test.locate.found["), "{near:?}");
    assert_eq!(*near.last().unwrap(), "commands.test.locate.done[1]");
}

#[test]
fn block_based_tests_pass_fail_and_time_out() {
    let tests = [
        ("accepts", block_test("start_accept", "")),
        ("fails", block_test("start_fail", "")),
        ("times_out", block_test("start_only", r#","required":false"#)),
        ("lacks_start", block_test("no_start", "")),
    ];
    let tests: Vec<(&str, &str)> = tests.iter().map(|(i, j)| (*i, j.as_str())).collect();
    let structures = [("start_accept", start_accept()), ("start_fail", start_fail()), ("start_only", start_only()), ("no_start", no_start())];
    let Some(mut g) = Game::new("block-based", &tests, &[], &structures) else { return };
    // Passing: the start block powers the accept block next to it.
    g.run("test run kilntest:accepts");
    let out = g.until("commands.test.summary", 40);
    assert!(out.contains(&"commands.test.summary.all_required_passed".to_owned()), "{out:?}");
    // Failing: the fail block fires with its message and the accept block does not.
    g.run("test run kilntest:fails");
    let out = g.until("commands.test.summary", 40);
    assert!(out.iter().any(|l| l == "commands.test.summary.failed[1]"), "{out:?}");
    // An optional test that does not finish in time.
    g.run("test run kilntest:times_out");
    let out = g.until("commands.test.summary.optional_failed", 60);
    assert!(out.iter().any(|l| l == "commands.test.summary.all_required_passed"), "{out:?}");
    assert!(out.iter().any(|l| l == "commands.test.summary.optional_failed[1]"), "{out:?}");
    // No start block at all.
    g.run("test run kilntest:lacks_start");
    let out = g.until("commands.test.summary", 40);
    assert!(out.iter().any(|l| l == "commands.test.summary.failed[1]"), "{out:?}");
}

#[test]
fn selectors_create_clear_reset_and_locate() {
    let Some(mut g) = Game::new("selectors", &[("pass", PASS)], &[], &[]) else { return };
    assert_eq!(g.run("test run nothing:here")[0], "argument.resource_selector.not_found[nothing:here, minecraft:test_instance]");
    // create makes a block; a test that is not defined cannot be found or run.
    assert_eq!(g.run("test create foo 3"), ["commands.test.create.success[minecraft:foo]"]);
    // (The world here is empty and the spawn far above it: only `runthese` reaches the block.)
    let out = g.run("test runthese");
    assert_eq!(out[0], "commands.test.error.non_existant_test[minecraft:foo]");
    assert_eq!(g.run("test clearall"), ["commands.test.clear.success[1]"]);
    assert_eq!(g.run("test clearall"), ["commands.test.clear.error.no_tests"]);
    // A defined test made by hand runs from its block, and resets and clears.
    assert_eq!(g.run("test create kilntest:pass 2"), ["commands.test.create.success[kilntest:pass]"]);
    let out = g.run("test runthese");
    assert_eq!(out[0], "commands.test.run.running[1]");
    let out = g.until("commands.test.summary", 10);
    assert_eq!(out[0], "commands.test.summary[1]");
    assert_eq!(g.run("test resetthese").len(), 2);
    assert_eq!(g.run("test clearthese"), ["commands.test.clear.success[1]"]);
    assert_eq!(g.run("test runfailed"), ["commands.test.no_tests"]);
    assert_eq!(g.run("test stop"), Vec::<String>::new());
    // A console has no view to look through.
    assert_eq!(g.run("test runthat"), ["command.failed"]);
}

#[test]
fn environments_hold_during_a_batch_and_are_undone_after() {
    let env = r#"{"type":"minecraft:game_rules","rules":{"minecraft:spawn_mobs":true,"minecraft:max_command_sequence_length":1234}}"#;
    let test = r#"{"type":"minecraft:function","function":"minecraft:always_pass","environment":"kilntest:rules","structure":"minecraft:empty","max_ticks":1,"setup_ticks":4}"#;
    let Some(mut g) = Game::new("environment", &[("env_test", test)], &[("rules", env)], &[]) else { return };
    assert_eq!(g.run("gamerule minecraft:max_command_sequence_length"), ["commands.gamerule.query[max_command_sequence_length, 65536]"]);
    g.run("test run kilntest:env_test");
    assert_eq!(g.run("gamerule minecraft:max_command_sequence_length"), ["commands.gamerule.query[max_command_sequence_length, 1234]"]);
    g.until("commands.test.summary", 20);
    assert_eq!(g.run("gamerule minecraft:max_command_sequence_length"), ["commands.gamerule.query[max_command_sequence_length, 65536]"]);
}

#[test]
fn retries_run_the_test_again_and_count_every_run() {
    let Some(mut g) = Game::new("retries", &[], &[], &[]) else { return };
    let out = g.run("test run minecraft:always_pass 3");
    assert_eq!(out[0], "commands.test.run.running[1]");
    let out = g.until("commands.test.summary", 60);
    // Each run is its own batch.
    assert!(out.iter().any(|l| l == "commands.test.summary[3]"), "three runs were made: {out:?}");
}

#[test]
fn verify_runs_every_rotation() {
    let Some(mut g) = Game::new("verify", &[], &[], &[]) else { return };
    let out = g.run("test verify minecraft:always_pass");
    assert_eq!(out, ["commands.test.batch.starting[minecraft:default, 0]"]);
    let out = g.until("commands.test.summary", 200);
    assert!(out.iter().any(|l| l.contains("commands.test.batch.starting[minecraft:default, 3]")), "{out:?}");
    assert!(out.iter().any(|l| l == "commands.test.summary[400]"), "{out:?}");
}
