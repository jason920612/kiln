//! The anger of neutral mobs (`NeutralMob`): `/summon` with `anger_end_time` and `angry_at`, or the old `AngerTime` (ticks left, counted from
//! the first update), and what the mob saves again.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn world() -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..6 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    (sim, client)
}

fn saved(sim: &Sim, kind: &str) -> Tag {
    sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(kind)).unwrap_or_else(|| panic!("a {kind}"))
}

#[test]
fn neutral_mobs_keep_the_anger_they_are_given() {
    let (mut sim, mut client) = world();
    let p = client.pos;
    for kind in ["wolf", "iron_golem", "polar_bear", "bee", "enderman", "zombified_piglin"] {
        let mut inbox = vec![ToSim::Console(format!(
            "summon minecraft:{kind} {} {} {} {{NoAI:1b,anger_end_time:99999999L,angry_at:[I;1,2,3,4]}}",
            p[0] + 3.0,
            p[1],
            p[2]
        ))];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let t = saved(&sim, &format!("minecraft:{kind}"));
        assert_eq!(t.get("anger_end_time").and_then(Tag::as_i64), Some(99999999), "{kind}");
        assert_eq!(t.get("angry_at"), Some(&Tag::IntArray(vec![1, 2, 3, 4])), "{kind}: the target is saved by its UUID");
        let mut inbox = vec![ToSim::Console(format!("kill @e[type=minecraft:{kind}]"))];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

#[test]
fn the_old_anger_time_counts_from_the_first_update() {
    let (mut sim, mut client) = world();
    let p = client.pos;
    for kind in ["wolf", "iron_golem", "zombified_piglin", "enderman"] {
        // (Let some time pass first: the end time is the game time plus the ticks left, not the ticks.)
        for _ in 0..30 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let mut inbox = vec![ToSim::Console(format!("summon minecraft:{kind} {} {} {} {{NoAI:0b,AngerTime:5000}}", p[0] + 3.0, p[1], p[2]))];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let t = saved(&sim, &format!("minecraft:{kind}"));
        let end = t.get("anger_end_time").and_then(Tag::as_i64).unwrap_or(-1);
        let now = sim.game_time();
        assert!((now + 4990..=now + 5010).contains(&end), "{kind}: ends at {end} when it is {now}");
        let mut inbox = vec![ToSim::Console(format!("kill @e[type=minecraft:{kind}]"))];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}
