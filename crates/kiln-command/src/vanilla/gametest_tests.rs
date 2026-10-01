//! `test`, `fetchprofile`, and the vanilla commands a dedicated server does not have.

use super::tests::{Mock, dispatcher, err_key};
use crate::host::{ProfileProperty, ProfileQuery, ResolvedProfile, TestCommand, TestSelection};
use crate::types::Identifier;
use uuid::Uuid;

fn last(s: &Mock) -> String {
    s.effects.last().cloned().unwrap_or_default()
}

#[test]
fn test_runs_parse_selectors_and_options() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    let run = |select: TestSelection, copies, tries, halt, steps, per_row| {
        format!("test {:?}", TestCommand::Run { select, copies, tries, halt_on_failure: halt, rotation_steps: steps, per_row })
    };
    let ids = |v: &[&str]| TestSelection::Ids(v.iter().map(|s| s.to_string()).collect());
    assert_eq!(s.run(&d, "test run minecraft:always_pass"), Ok(1));
    assert_eq!(last(s), run(ids(&["minecraft:always_pass"]), 1, 1, true, 0, 8));
    // `*` is `minecraft:*`; a namespace and a wildcard select by pattern.
    s.run(&d, "test run *").unwrap();
    assert_eq!(last(s), run(ids(&["minecraft:always_pass"]), 1, 1, true, 0, 8));
    s.run(&d, "test run kiln:slow/*").unwrap();
    assert_eq!(last(s), run(ids(&["kiln:slow/one", "kiln:slow/two"]), 1, 1, true, 0, 8));
    s.run(&d, "test run kiln:*").unwrap();
    assert_eq!(last(s), run(ids(&["kiln:pass", "kiln:slow/one", "kiln:slow/two"]), 1, 1, true, 0, 8));
    s.run(&d, "test run kiln:sl?w/one 3").unwrap();
    assert_eq!(last(s), run(ids(&["kiln:slow/one"]), 1, 3, false, 0, 8));
    s.run(&d, "test run kiln:pass 0 true 2 5").unwrap();
    assert_eq!(last(s), run(ids(&["kiln:pass"]), 1, 0, true, 2, 5));
    s.run(&d, "test runmultiple kiln:pass 4").unwrap();
    assert_eq!(last(s), run(ids(&["kiln:pass"]), 4, 1, true, 0, 8));
    s.run(&d, "test runthese 2 false").unwrap();
    assert_eq!(last(s), run(TestSelection::Nearby, 1, 2, false, 0, 8));
    s.run(&d, "test runclosest").unwrap();
    assert_eq!(last(s), run(TestSelection::Nearest, 1, 1, true, 0, 8));
    s.run(&d, "test runthat 5").unwrap();
    assert_eq!(last(s), run(TestSelection::LookedAt, 1, 5, false, 0, 8));
    s.run(&d, "test runfailed").unwrap();
    assert_eq!(last(s), run(TestSelection::Failed { only_required: false }, 1, 1, true, 0, 8));
    s.run(&d, "test runfailed true 2 true 1 4").unwrap();
    assert_eq!(last(s), run(TestSelection::Failed { only_required: true }, 1, 2, true, 1, 4));
    s.run(&d, "test verify kiln:pass").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Verify { ids: vec!["kiln:pass".into()] }));
    s.run(&d, "test locate kiln:*").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Locate { ids: ["kiln:pass", "kiln:slow/one", "kiln:slow/two"].map(String::from).to_vec() }));
}

#[test]
fn test_other_subcommands() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    s.run(&d, "test clearall").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Clear(TestSelection::Radius(250))));
    s.run(&d, "test clearall 5000").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Clear(TestSelection::Radius(1024))));
    s.run(&d, "test clearall -3").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Clear(TestSelection::Radius(0))));
    s.run(&d, "test resetthese").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Reset(TestSelection::Nearby)));
    s.run(&d, "test create kiln:box").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Create { id: Identifier::parse("kiln:box").unwrap(), size: [5, 5, 5] }));
    s.run(&d, "test create box 3").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Create { id: Identifier::parse("box").unwrap(), size: [3, 3, 3] }));
    s.run(&d, "test create box 3 4 5").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Create { id: Identifier::parse("box").unwrap(), size: [3, 4, 5] }));
    s.run(&d, "test pos x").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Pos("x".into())));
    s.run(&d, "test stop").unwrap();
    assert_eq!(last(s), format!("test {:?}", TestCommand::Stop));
}

#[test]
fn test_argument_errors() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    assert_eq!(err_key(s.run(&d, "test run kiln:nope")), "argument.resource_selector.not_found");
    assert_eq!(err_key(s.run(&d, "test run nope*")), "argument.resource_selector.not_found");
    assert_eq!(err_key(s.run(&d, "test run kiln:pass -1")), "argument.integer.low");
    assert_eq!(err_key(s.run(&d, "test")), "command.unknown.command");
    assert_eq!(err_key(s.run(&d, "test bogus")), "command.unknown.argument");
    assert_eq!(err_key(s.run(&d, "test locate")), "command.unknown.command");
    // Game masters only.
    let s = &mut Mock::new(1);
    assert_eq!(err_key(s.run(&d, "test stop")), "command.unknown.command");
}

#[test]
fn publish_and_unpublish_do_not_exist_on_a_dedicated_server() {
    // `PublishCommand` and `UnpublishCommand` are registered for integrated servers only.
    let d = dispatcher();
    for command in ["publish", "publish true", "publish true 25565", "unpublish"] {
        for level in [0, 2, 4] {
            let s = &mut Mock::new(level);
            assert_eq!(err_key(s.run(&d, command)), "command.unknown.command", "{command} at level {level}");
        }
    }
    assert!(!COMMANDS_LIST.contains(&"publish") && !COMMANDS_LIST.contains(&"unpublish"));
}

const COMMANDS_LIST: &[&str] = super::COMMANDS;

/// Every command of vanilla's tree (the data generator registers all of them) is registered,
/// except the two that only integrated servers have.
#[test]
fn every_vanilla_command_but_the_integrated_ones_is_registered() {
    let work = std::env::var_os("KILN_WORK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let Ok(text) = std::fs::read_to_string(work.join("generated/reports/commands.json")) else {
        eprintln!("skipped: commands.json not found");
        return;
    };
    let vanilla: serde_json::Value = serde_json::from_str(&text).unwrap();
    let missing: Vec<&String> = vanilla["children"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|name| !COMMANDS_LIST.contains(&name.as_str()) && !["publish", "unpublish"].contains(&name.as_str()))
        .collect();
    assert!(missing.is_empty(), "not registered: {missing:?}");
}

fn profile() -> ResolvedProfile {
    ResolvedProfile {
        id: Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap(),
        name: "Notch".into(),
        properties: vec![ProfileProperty { name: "textures".into(), value: "dGV4".into(), signature: Some("c2ln".into()) }],
    }
}

#[test]
fn profile_nbt_is_the_full_resolvable_profile_codec() {
    use crate::nbt_text::snbt;
    let tag = super::profile::profile_nbt(&profile());
    assert_eq!(
        snbt(&tag),
        r#"{id:[I;110787060,1156138790,-1514210135,238594805],name:"Notch",properties:[{name:"textures",signature:"c2ln",value:"dGV4"}]}"#
    );
    let bare = ResolvedProfile { properties: Vec::new(), ..profile() };
    assert_eq!(snbt(&super::profile::profile_nbt(&bare)), r#"{id:[I;110787060,1156138790,-1514210135,238594805],name:"Notch"}"#);
}

#[test]
fn profile_reports_carry_four_buttons() {
    let text = super::profile::lookup_success_text(&ProfileQuery::Name("Notch".into()), &profile());
    assert_eq!(text.key(), Some("commands.fetchprofile.name.success"));
    assert_eq!(
        text.to_plain(),
        "commands.fetchprofile.name.success[Notch, [commands.fetchprofile.copy_component] [commands.fetchprofile.give_item] [commands.fetchprofile.summon_mannequin] [commands.fetchprofile.copy_text[[Notch head]]]]"
    );
    let failure = super::profile::failure_text(&ProfileQuery::Id(profile().id));
    assert_eq!(failure.to_plain(), "commands.fetchprofile.id.failure[069a79f4-44e9-4726-a5be-fca90e38aaf5]");
}

#[test]
fn fetchprofile_arguments() {
    let d = dispatcher();
    let s = &mut Mock::new(2);
    // The mock cannot look profiles up: the default host reports the failure at once.
    assert_eq!(s.run(&d, "fetchprofile name Some Name With Spaces"), Ok(1));
    assert_eq!(s.feedback.last().unwrap().0, "commands.fetchprofile.name.failure[Some Name With Spaces]");
    assert_eq!(s.run(&d, "fetchprofile id 069a79f4-44e9-4726-a5be-fca90e38aaf5"), Ok(1));
    assert_eq!(s.feedback.last().unwrap().0, "commands.fetchprofile.id.failure[069a79f4-44e9-4726-a5be-fca90e38aaf5]");
    assert_eq!(err_key(s.run(&d, "fetchprofile id abc")), "argument.uuid.invalid");
    assert_eq!(err_key(s.run(&d, "fetchprofile")), "command.unknown.command");
    assert_eq!(err_key(s.run(&d, "fetchprofile name")), "command.unknown.command");
    // An entity without a profile.
    assert_eq!(err_key(s.run(&d, "fetchprofile entity @e[type=minecraft:zombie,limit=1]")), "commands.fetchprofile.no_profile");
    assert_eq!(err_key(s.run(&d, "fetchprofile entity @e")), "argument.entity.toomany");
}
