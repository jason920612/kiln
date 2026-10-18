//! A tame cat lies by its sleeping owner and, when the owner wakes in the morning after sleeping at least 100 ticks, may leave a gift
//! (`Cat.CatRelaxOnOwnerGoal`, `minecraft:gameplay/cat_morning_gift`, chance 0.7 from the end of the night). Kiln's goal never started (wp77).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn gifts(sim: &Sim) -> Vec<String> {
    const KINDS: [&str; 7] = ["rabbit_hide", "rabbit_foot", "chicken", "feather", "rotten_flesh", "string", "phantom_membrane"];
    sim.entity_nbt()
        .into_iter()
        .filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item"))
        .filter_map(|t| t.get("Item").and_then(|i| i.get("id")).and_then(Tag::as_str).map(str::to_owned))
        .filter(|id| KINDS.iter().any(|k| id == &format!("minecraft:{k}")))
        .collect()
}

#[test]
fn a_cat_leaves_a_gift_when_its_owner_wakes() {
    // The gift is a loot table of the vanilla datapack (`KILN_DATAPACK`, else `$KILN_WORK/generated`, else `work/generated`): skips without it.
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => std::path::PathBuf::from(d),
        None => std::env::var_os("KILN_WORK").filter(|d| !d.is_empty()).map(std::path::PathBuf::from).unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work")).join("generated"),
    };
    if !dir.join("data").is_dir() {
        return;
    }
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode survival Keeper".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut client = Client::new(1, stats);
    let mut ticks = |sim: &mut Sim, client: &mut Client, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, &mut client, 5);
    let p = client.pos;
    let [x, y, z] = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
    let run = |sim: &mut Sim, c: String| assert!(sim.step([ToSim::Console(c)]));
    // A bed two blocks east (head further east), a cat that belongs to the player.
    run(&mut sim, format!("setblock {} {y} {z} minecraft:red_bed[facing=east,part=foot]", x + 2));
    run(&mut sim, format!("setblock {} {y} {z} minecraft:red_bed[facing=east,part=head]", x + 3));
    run(&mut sim, format!("summon minecraft:cat {} {y} {} {{Owner:[I;0,1802071150,0,1],Sitting:0b}}", x + 1, z + 1));
    let mut found = Vec::new();
    for night in 0..8 {
        run(&mut sim, "time set 13000".into());
        ticks(&mut sim, &mut client, 3);
        let pkt = PlayIn::UseItemOn { hand: 0, pos: [x + 3, y, z], face: 1, cursor: [0.5, 0.5, 0.5], inside: false, sequence: night + 1 };
        assert!(sim.step([ToSim::Packet(1, pkt)]));
        // asleep for 100 ticks, the night is skipped, the owner wakes and the cat has its say
        ticks(&mut sim, &mut client, 200);
        found = gifts(&sim);
        if !found.is_empty() {
            break;
        }
    }
    assert!(!found.is_empty(), "no gift in eight mornings (the chance is 0.7 each)");
}
