//! wp84 (found by a real client against vanilla 26.3 side by side): a grown happy ghast is not fed (`canFallInLove` is false and only a baby
//! eats to grow up: vanilla keeps the snowball and sends no love hearts), and a ghastling that comes from saved data / `/summon … {Age:-24000}`
//! is set up as a baby (`AgeableMob.setAge` crosses zero while loading: `babyGhastSetup`, the brain follows the player holding a snowball
//! up to 3 blocks), not as a grown one that stops 7 blocks short.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/recipe").is_dir();
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        let found = dir.join("data/minecraft/recipe").is_dir();
        if found {
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
        found
    })
}

struct World {
    sim: Sim,
    client: Client,
    stats: std::sync::Arc<kiln_sim::testing::SinkStats>,
}

impl World {
    fn new() -> Self {
        have_datapack();
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Farmer", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        let keep = stats.clone();
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode survival Farmer".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        World { sim, client, stats: keep }
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn ghast(&self) -> Tag {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:happy_ghast")).expect("the happy ghast")
    }

    fn snowballs(&mut self) -> i64 {
        self.sim.saved_player_nbt(1).and_then(|t| {
            let Some(Tag::List(inv)) = t.get("Inventory").cloned() else { return Some(0) };
            Some(inv.iter().filter(|i| i.get("id").and_then(Tag::as_str) == Some("minecraft:snowball")).map(|i| i.get("count").and_then(Tag::as_i64).unwrap_or(0)).sum())
        }).unwrap_or(0)
    }
}

#[test]
fn a_grown_happy_ghast_does_not_take_the_snowball() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run("give Farmer minecraft:snowball 2");
    w.run(&format!("summon minecraft:happy_ghast {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] + 1.0, p[1] + 1.0, p[2] + 6.0));
    w.ticks(3);
    let id = w.sim.entity_ids_of("minecraft:happy_ghast")[0];
    let before = w.snowballs();
    assert_eq!(before, 2, "two snowballs to start with");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 1.0, 0.0], sneaking: false })]));
    w.ticks(2);
    let g = w.ghast();
    assert!(g.get("InLove").and_then(Tag::as_i64).unwrap_or(0) == 0, "a grown happy ghast never falls in love: {g:?}");
    assert_eq!(w.snowballs(), 2, "the snowball was not used up");
}

#[test]
fn a_ghastling_from_saved_data_follows_a_snowball_like_a_baby() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run("give Farmer minecraft:snowball 4");
    // 9 blocks away, 1.5 above: the grown ghast's tempt goal would stop 7 blocks short; the ghastling's brain comes to 3.
    w.run(&format!("summon minecraft:happy_ghast {} {} {} {{Age:-24000,PersistenceRequired:1b}}", p[0], p[1] + 1.5, p[2] + 9.0));
    w.ticks(160);
    let g = w.ghast();
    let Some(Tag::List(at)) = g.get("Pos") else { panic!("Pos") };
    let (dx, dz) = (at[0].as_f64().unwrap() - p[0], at[2].as_f64().unwrap() - p[2]);
    let d = (dx * dx + dz * dz).sqrt();
    assert!(d < 4.5, "the ghastling came to the player holding a snowball: {d:.2} blocks away ({at:?} / player {p:?})");
}

/// The sounds (by registry name) among the packets the player was sent.
fn sounds_heard(w: &World) -> Vec<i32> {
    let log = w.stats.log.lock().unwrap().clone().unwrap();
    let mut out = Vec::new();
    for p in &log {
        let mut r = kiln_proto::codec::Reader::new(p);
        if r.varint().ok() == Some(kiln_data::packets::play::clientbound::SOUND) {
            if let Ok(holder) = r.varint() {
                out.push(holder - 1);
            }
        }
    }
    out
}

#[test]
fn a_harnessed_happy_ghast_plays_its_goggles_when_the_first_rider_boards_and_the_last_leaves() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!(
        "summon minecraft:happy_ghast {} {} {} {{NoAI:1b,PersistenceRequired:1b,equipment:{{body:{{id:\"minecraft:blue_harness\",count:1}}}}}}",
        p[0] + 1.0,
        p[1] + 1.0,
        p[2] + 6.0
    ));
    w.ticks(3);
    let down = kiln_data::builtin_id("minecraft:sound_event", "minecraft:entity.happy_ghast.harness_goggles_down").expect("sound");
    let up = kiln_data::builtin_id("minecraft:sound_event", "minecraft:entity.happy_ghast.harness_goggles_up").expect("sound");
    assert!(!sounds_heard(&w).contains(&down), "nothing yet");
    w.run("ride Farmer mount @e[type=happy_ghast,limit=1]");
    w.ticks(3);
    assert_eq!(sounds_heard(&w).iter().filter(|s| **s == down).count(), 1, "goggles down once the first rider boards");
    w.run("ride Farmer dismount");
    w.ticks(3);
    assert_eq!(sounds_heard(&w).iter().filter(|s| **s == up).count(), 1, "goggles up once the last rider left");
}

#[test]
fn a_leashed_happy_ghast_keeps_its_lead_out_to_16_blocks_not_12() {
    let mut w = World::new();
    let p = w.client.pos;
    let uuid = match w.sim.saved_player_nbt(1).and_then(|t| t.get("UUID").cloned()) {
        Some(Tag::IntArray(v)) if v.len() == 4 => v,
        other => panic!("the player's UUID: {other:?}"),
    };
    // 14 blocks away (centre to centre): past the 12 of an ordinary lead (it would snap), inside the 16 of `HappyGhast.leashSnapDistance`.
    w.run(&format!(
        "summon minecraft:happy_ghast {} {} {} {{NoAI:1b,PersistenceRequired:1b,leash:{{UUID:[I;{},{},{},{}]}}}}",
        p[0] + 0.0,
        p[1] + 0.0,
        p[2] + 14.0 - 0.0,
        uuid[0],
        uuid[1],
        uuid[2],
        uuid[3]
    ));
    w.ticks(10);
    let g = w.ghast();
    assert!(g.get("leash").is_some(), "the lead holds at 14 blocks: {g:?}");
    // ... and it snaps past 16.
    w.run("execute as @e[type=happy_ghast,limit=1] at @s run tp @s ~ ~ ~4");
    w.ticks(5);
    let g = w.ghast();
    assert!(g.get("leash").is_none(), "the lead snapped farther than 16 blocks: {g:?}");
}
