//! `execute on controller` (`Entity.getControllingPassenger`): boats take a player first rider,
//! a mob with AI is steered by a mob rider, nothing else has a controller; the stack of a horse
//! and a pig on top of it stays put and keeps answering as the horse's own AI runs.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn console(sim: &mut Sim, cmd: &str) {
    assert!(sim.step([ToSim::Console(cmd.into())]));
}

fn tick(sim: &mut Sim, client: &mut Client) {
    let mut inbox = Vec::new();
    client.tick(None, &mut inbox);
    assert!(sim.step(inbox));
}

fn tags_of(sim: &Sim, ty: &str) -> Vec<String> {
    sim.entity_nbt()
        .iter()
        .filter(|t| t.get("id").and_then(|i| i.as_str()) == Some(ty))
        .flat_map(|t| match t.get("Tags") {
            Some(Tag::List(l)) => l.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn a_mob_rider_controls_a_horse_with_ai_all_along() {
    let mut sim = Sim::new(SimConfig::new(8, 4, None));
    let (msg, stats) = join(1, "Rider", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    for c in ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:advance_weather false", "gamemode creative Rider"] {
        console(&mut sim, c);
    }
    for _ in 0..5 {
        tick(&mut sim, &mut client);
    }
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    console(&mut sim, &format!("fill {} {} {} {} {} {} stone", x - 12, y - 1, z - 12, x + 12, y - 1, z + 12));
    console(&mut sim, &format!("summon minecraft:horse {} {y} {} {{Silent:1b,Tags:[\"c0\"],Passengers:[{{id:\"minecraft:pig\",NoAI:1b,Silent:1b,Tags:[\"c1\"]}}]}}", x + 3, z + 3));
    for t in 0..400 {
        tick(&mut sim, &mut client);
        console(&mut sim, "execute as @e[tag=c1] on vehicle on controller run tag @s add seen");
        console(&mut sim, "execute as @e[tag=c0] on controller run tag @s add seen0");
        let (pigs, seen) = (tags_of(&sim, "minecraft:pig"), tags_of(&sim, "minecraft:pig"));
        assert!(pigs.iter().any(|t| t == "c1"), "tick {t}: the pig is gone: {pigs:?}");
        assert!(seen.iter().any(|t| t == "seen"), "tick {t}: no controller through the pig's vehicle: {seen:?}");
        assert!(seen.iter().any(|t| t == "seen0"), "tick {t}: the horse has no controller: {seen:?}");
        console(&mut sim, "tag @e[tag=c1] remove seen");
        console(&mut sim, "tag @e[tag=c1] remove seen0");
    }
}

#[test]
fn a_boat_is_steered_by_its_first_rider_if_that_is_a_player() {
    let mut sim = Sim::new(SimConfig::new(8, 4, None));
    let (msg, stats) = join(1, "Rider", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    for c in ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:advance_weather false", "gamemode creative Rider"] {
        console(&mut sim, c);
    }
    for _ in 0..5 {
        tick(&mut sim, &mut client);
    }
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    console(&mut sim, &format!("fill {} {} {} {} {} {} stone", x - 12, y - 1, z - 12, x + 12, y - 1, z + 12));
    console(&mut sim, &format!("summon minecraft:oak_boat {} {y} {} {{Tags:[\"boat\"],Passengers:[{{id:\"minecraft:pig\",NoAI:1b,Tags:[\"pig\"]}}]}}", x + 3, z + 3));
    console(&mut sim, &format!("summon minecraft:minecart {} {y} {} {{Tags:[\"cart\"],Passengers:[{{id:\"minecraft:zombie\",NoAI:1b,Tags:[\"zombie\"]}}]}}", x - 3, z - 3));
    tick(&mut sim, &mut client);
    // Nobody steers a boat with a pig in it, nor a minecart with a zombie.
    console(&mut sim, "execute as @e[tag=boat] on controller run tag @s add steering");
    console(&mut sim, "execute as @e[tag=cart] on controller run tag @s add steering");
    assert!(tags_of(&sim, "minecraft:pig").iter().all(|t| t != "steering"));
    assert!(tags_of(&sim, "minecraft:zombie").iter().all(|t| t != "steering"));
    // A player in the boat steers it; the player is seen as the controller from the boat.
    console(&mut sim, "ride Rider mount @e[tag=boat,limit=1]");
    tick(&mut sim, &mut client);
    console(&mut sim, "execute as @e[tag=boat] on controller run say steered");
    console(&mut sim, "execute as @e[tag=boat] on controller if entity @s[name=Rider] run tag @e[tag=pig] add by_rider");
    assert!(tags_of(&sim, "minecraft:pig").iter().any(|t| t == "by_rider"), "the player in the boat is its controller");
}
