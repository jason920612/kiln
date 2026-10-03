//! Riding stacks persist (`Entity.saveWithoutId`'s `Passengers`, `EntityType.loadEntityRecursive`):
//! a vehicle with its riders leaves with its chunk and comes back as it was, survives a restart,
//! and `/summon` and `/data get entity` speak the same format.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;

fn settle(sim: &mut Sim, client: &mut Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

fn world(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn console(sim: &mut Sim, cmd: &str) {
    assert!(sim.step([ToSim::Console(cmd.into())]));
}

/// A creative player on a stone floor of an empty world; nothing spawns but what a test summons.
fn start(dir: &std::path::Path) -> (Sim, Client, [i32; 3]) {
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.to_owned())));
    let (msg, stats) = join(1, "Rider", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    for c in ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:advance_weather false", "gamemode creative Rider"] {
        console(&mut sim, c);
    }
    settle(&mut sim, &mut client, 5);
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    console(&mut sim, &format!("fill {} {} {} {} {} {} stone", x - 6, y - 1, z - 6, x + 6, y - 1, z + 6));
    settle(&mut sim, &mut client, 2);
    (sim, client, [x, y, z])
}

const STACK: &str = r#"{Tags:["root"],Passengers:[{id:"minecraft:skeleton",Tags:["a"],Passengers:[{id:"minecraft:parrot",Tags:["b"]}]},{id:"minecraft:zombie",Tags:["c"],IsBaby:1b}]}"#;

fn summon_stack(sim: &mut Sim, client: &mut Client, [x, y, z]: [i32; 3]) {
    // A horse whose riders ride it: a skeleton carrying a parrot, and a baby zombie.
    console(sim, &format!("summon minecraft:horse {} {y} {} {STACK}", x + 3, z + 3));
    settle(sim, client, 3);
}

/// (type, uuid, vehicle's uuid) of every riding link, by type for stable comparisons.
fn links(sim: &Sim) -> Vec<(String, Tag, Option<Tag>)> {
    let nbt = sim.entity_nbt();
    let ids: Vec<(i32, Option<i32>, Vec<i32>)> = sim.riding();
    let mut all: Vec<(i32, Tag, String)> = Vec::new();
    // `entity_nbt` and `riding` list entities in the same id order.
    let live: Vec<&Tag> = nbt.iter().collect();
    assert_eq!(live.len(), ids.len());
    for (t, (id, _, _)) in live.iter().zip(&ids) {
        all.push((*id, t.get("UUID").cloned().unwrap(), t.get("id").and_then(Tag::as_str).unwrap().to_owned()));
    }
    let mut out = Vec::new();
    for ((id, vehicle, _), (_, uuid, ty)) in ids.iter().zip(&all) {
        let _ = id;
        let v = vehicle.and_then(|v| all.iter().find(|a| a.0 == v)).map(|a| a.1.clone());
        out.push((ty.clone(), uuid.clone(), v));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn tags_of(t: &Tag) -> Vec<String> {
    t.get("Tags").and_then(Tag::as_list).unwrap_or(&[]).iter().filter_map(|t| t.as_str().map(str::to_owned)).collect()
}

/// The saved stack of the horse: its `Passengers` nested as vanilla writes them.
fn check_saved(stacks: &[Tag]) {
    assert_eq!(stacks.len(), 1, "only the root is a chunk entity: {stacks:?}");
    let horse = &stacks[0];
    assert_eq!(horse.get("id").and_then(Tag::as_str), Some("minecraft:horse"));
    let riders = horse.get("Passengers").and_then(Tag::as_list).expect("Passengers");
    let ids: Vec<&str> = riders.iter().map(|r| r.get("id").and_then(Tag::as_str).unwrap()).collect();
    assert_eq!(ids, ["minecraft:skeleton", "minecraft:zombie"]);
    let parrot = riders[0].get("Passengers").and_then(Tag::as_list).expect("the skeleton's passenger");
    assert_eq!(parrot.len(), 1);
    assert_eq!(parrot[0].get("id").and_then(Tag::as_str), Some("minecraft:parrot"));
    assert_eq!(tags_of(&parrot[0]), ["b"]);
    // A rider's Pos is its vehicle's x and z (`saveWithoutId`).
    let pos = |t: &Tag| -> Vec<f64> { t.get("Pos").and_then(Tag::as_list).unwrap().iter().map(|v| v.as_f64().unwrap()).collect() };
    let hp = pos(horse);
    for r in riders {
        let rp = pos(r);
        assert_eq!((rp[0], rp[2]), (hp[0], hp[2]));
    }
}

#[test]
fn a_summoned_stack_is_riding_and_saved_by_its_root() {
    let dir = world("passengers-summon");
    let (mut sim, mut client, at) = start(&dir);
    summon_stack(&mut sim, &mut client, at);
    let riding = sim.riding();
    assert_eq!(riding.len(), 4, "{riding:?}");
    let by_type = |name: &str| sim.entity_ids_of(name)[0];
    let (horse, skeleton, parrot, zombie) = (by_type("minecraft:horse"), by_type("minecraft:skeleton"), by_type("minecraft:parrot"), by_type("minecraft:zombie"));
    let find = |id: i32| riding.iter().find(|r| r.0 == id).unwrap().clone();
    assert_eq!(find(horse).2, [skeleton, zombie]);
    assert_eq!(find(skeleton).1, Some(horse));
    assert_eq!(find(skeleton).2, [parrot]);
    assert_eq!(find(parrot).1, Some(skeleton));
    assert_eq!(find(zombie).1, Some(horse));
    check_saved(&sim.entity_stacks_nbt());
    // Riders sit on their vehicle, not at the origin.
    for (ty, p) in sim.entities() {
        assert!((p[0] - (at[0] + 3) as f64).abs() < 2.0 && (p[2] - (at[2] + 3) as f64).abs() < 2.0, "{ty} at {p:?}");
    }
}

#[test]
fn a_stack_leaves_and_comes_back_with_its_chunk() {
    let dir = world("passengers-unload");
    let (mut sim, mut client, at) = start(&dir);
    summon_stack(&mut sim, &mut client, at);
    let before = links(&sim);
    assert_eq!(before.len(), 4);
    console(&mut sim, &format!("tp Rider {} {} {}", at[0] + 5000, at[1], at[2]));
    settle(&mut sim, &mut client, 60);
    assert!(sim.riding().is_empty(), "the stack unloads with its chunk: {:?}", sim.riding());
    console(&mut sim, &format!("tp Rider {} {} {}", at[0], at[1], at[2]));
    settle(&mut sim, &mut client, 3);
    assert_eq!(links(&sim), before, "same entities, same riding");
    check_saved(&sim.entity_stacks_nbt());
}

#[test]
fn a_stack_survives_a_restart() {
    let dir = world("passengers-restart");
    let (mut sim, mut client, at) = start(&dir);
    summon_stack(&mut sim, &mut client, at);
    settle(&mut sim, &mut client, 20);
    let before = links(&sim);
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!sim.step([ToSim::Shutdown { done }]));
    drop(sim);

    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.clone())));
    let (msg, stats) = join(1, "Rider", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 3);
    assert_eq!(links(&sim), before, "same entities, same riding");
    check_saved(&sim.entity_stacks_nbt());
    // They keep riding on: the riders stay with their horse as it ticks.
    settle(&mut sim, &mut client, 20);
    assert_eq!(links(&sim), before);
}

#[test]
fn data_get_entity_has_the_passengers() {
    let dir = world("passengers-data");
    let (mut sim, mut client, at) = start(&dir);
    summon_stack(&mut sim, &mut client, at);
    // The horse's data has its riders (and theirs), the skeleton's its parrot.
    console(&mut sim, "execute if data entity @e[type=minecraft:horse,limit=1] Passengers[{id:\"minecraft:skeleton\"}].Passengers[{id:\"minecraft:parrot\"}] run tag @e[type=minecraft:horse] add found");
    console(&mut sim, "execute if data entity @e[type=minecraft:skeleton,limit=1] Passengers[{id:\"minecraft:parrot\"}] run tag @e[type=minecraft:skeleton] add carrier");
    console(&mut sim, "execute if data entity @e[type=minecraft:parrot,limit=1] Passengers run tag @e[type=minecraft:parrot] add wrong");
    let count = |tag: &str| sim.entity_nbt().into_iter().filter(|t| tags_of(t).contains(&tag.to_owned())).count();
    assert_eq!((count("found"), count("carrier"), count("wrong")), (1, 1, 0));
}

/// A player riding a horse that carries a skeleton leaves: the stack goes with them (it is in
/// their data as `RootVehicle`), and comes back under them when they join again.
#[test]
fn a_riding_players_stack_leaves_and_returns_with_them() {
    let dir = world("passengers-root-vehicle");
    let (mut sim, mut client, at) = start(&dir);
    console(&mut sim, &format!("summon minecraft:horse {} {} {} {{Tags:[\"mount\"],Passengers:[{{id:\"minecraft:skeleton\",Tags:[\"pillion\"]}}]}}", at[0] + 1, at[1], at[2] + 1));
    settle(&mut sim, &mut client, 3);
    console(&mut sim, "ride Rider mount @e[type=minecraft:horse,limit=1]");
    settle(&mut sim, &mut client, 5);
    let horse = sim.entity_ids_of("minecraft:horse")[0];
    assert_eq!(sim.vehicle_of(1), Some(horse));
    let uuids = |sim: &Sim| -> Vec<(String, Tag)> {
        let mut v: Vec<_> = sim.entity_nbt().into_iter().map(|t| (t.get("id").and_then(Tag::as_str).unwrap().to_owned(), t.get("UUID").cloned().unwrap())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let before = uuids(&sim);
    assert_eq!(before.len(), 2);
    let skeleton = sim.entity_ids_of("minecraft:skeleton")[0];

    // Leaving: the horse and its skeleton go with the player.
    assert!(sim.step([ToSim::Leave(1)]));
    assert!(sim.step([]));
    assert!(sim.riding().is_empty(), "the stack left the level with its player: {:?}", sim.riding());

    // Joining again: the stack is back, the player on it, in front of the skeleton.
    let (msg, stats) = join(1, "Rider", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    assert_eq!(uuids(&sim), before, "the same horse and skeleton");
    let horse = sim.entity_ids_of("minecraft:horse")[0];
    assert_eq!(sim.vehicle_of(1), Some(horse));
    let skeleton_back = sim.entity_ids_of("minecraft:skeleton")[0];
    let riding = sim.riding();
    let h = riding.iter().find(|r| r.0 == horse).unwrap();
    assert_eq!(h.2.len(), 2, "{h:?}");
    assert_eq!(h.2[1], skeleton_back, "the player first, as the controlling passenger: {h:?}");
    let s = riding.iter().find(|r| r.0 == skeleton_back).unwrap();
    assert_eq!(s.1, Some(horse));
    let _ = skeleton;
}
