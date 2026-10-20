//! wp87: the mace's smash attack from a real fall. `tools/wp87/MaceFall.java` runs it on vanilla 26.3 (a mock player falls with the client's own
//! physics, one position packet a tick, and attacks a mob with a mace: Density, Breach, Wind Burst) and dumps, per scenario, the packets' path and what
//! came out. Here Kiln gets the same packets (the same positions, the same tick of the attack) and has to give the same damage, the same motion of the
//! mob, the same wear of the mace, the same fall distance left and the same explosion packet.
//!
//! Skipped without the dump (`$KILN_MACE_VECTORS`, default `<work>/wp87/mace.jsonl`; `tools/wp87/mace87.sh` writes it).

use bytes::Bytes;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::Reader;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use serde_json::Value;
use std::path::{Path, PathBuf};

mod support;

fn vectors() -> Option<Vec<Value>> {
    let file = std::env::var_os("KILN_MACE_VECTORS").map(PathBuf::from).unwrap_or_else(|| {
        match std::env::var_os("KILN_WORK") {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
        }
        .join("wp87/mace.jsonl")
    });
    let text = std::fs::read_to_string(&file).ok()?;
    Some(text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("a JSON line")).collect())
}

struct World {
    sim: Sim,
    client: Client,
    stats: std::sync::Arc<kiln_sim::testing::SinkStats>,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Faller", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        let keep = stats.clone();
        assert!(sim.step([
            msg,
            ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
            ToSim::Console("difficulty normal".into()),
            ToSim::Console("gamemode survival Faller".into()),
            ToSim::Console("time set midnight".into()),
            ToSim::Console("gamerule minecraft:advance_time false".into()),
        ]));
        let mut w = World { sim, client: Client::new(1, stats), stats: keep };
        w.ticks(40);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        self.sim.console_now(command);
    }

    /// The packets the player was sent since the last call.
    fn packets(&mut self) -> Vec<Bytes> {
        let mut log = self.stats.log.lock().unwrap();
        let all = log.take().unwrap_or_default();
        *log = Some(Vec::new());
        all
    }
}

/// `"motion <id> x y z"` / `"explode ... "` lines of the packets that matter, as the dump writes them.
fn describe(w: &World, packets: &[Bytes], player: i32) -> Vec<String> {
    let _ = w;
    use kiln_data::packets::play::clientbound as cb;
    let mut out = Vec::new();
    for p in packets {
        let mut r = Reader::new(p);
        let id = r.varint().unwrap();
        if id == cb::SET_ENTITY_MOTION {
            let entity = r.varint().unwrap();
            if entity == player {
                let v = kiln_proto::packets::entity::read_lp_vec3(&mut r).unwrap();
                out.push(format!("motion {entity} {} {} {}", v[0], v[1], v[2]));
            }
        } else if id == cb::EXPLODE {
            out.push("explode".to_owned());
        }
    }
    out
}

#[test]
fn a_smash_attack_from_a_fall_does_what_vanilla_does() {
    let Some(vectors) = vectors() else {
        eprintln!("skipped: no mace vectors (tools/wp87/mace87.sh on the VM)");
        return;
    };
    assert!(!vectors.is_empty());
    if !support::vanilla_data() {
        eprintln!("skipped: needs the vanilla data pack (the enchantments' definitions)");
        return;
    }
    let _x = kiln_data::live::exclusive();
    let mut failures = Vec::new();
    for v in &vectors {
        let name = v["name"].as_str().unwrap().to_owned();
        if v.get("error").is_some() {
            failures.push(format!("{name}: the vanilla harness failed: {}", v["error"]));
            continue;
        }
        let mut w = World::new();
        let p = w.client.pos;
        let ground = p[1].floor() as i32 - 1;
        let floor = ground as f64 + 1.0;
        let (x, z) = (p[0], p[2]);
        let dy = floor - 100.0;
        w.run(&format!("fill {} {ground} {} {} {ground} {} minecraft:stone", x.floor() as i32 - 3, z.floor() as i32 - 3, x.floor() as i32 + 3, z.floor() as i32 + 3));
        let armor = if v["victim_armored"].as_bool().unwrap_or(false) {
            ",equipment:{head:{id:\"minecraft:diamond_helmet\"},chest:{id:\"minecraft:diamond_chestplate\"},legs:{id:\"minecraft:diamond_leggings\"},feet:{id:\"minecraft:diamond_boots\"}}"
        } else {
            ""
        };
        w.run(&format!("summon minecraft:zombie {} {floor} {z} {{NoAI:1b,PersistenceRequired:1b,Health:1000f,attributes:[{{id:\"minecraft:max_health\",base:1000d}}]{armor}}}", x - 0.7));
        let enchants: Vec<String> = v["enchants"].as_object().unwrap().iter().map(|(k, l)| format!("\"{k}\":{l}")).collect();
        w.run(&format!("item replace entity Faller weapon.mainhand with minecraft:mace[enchantments={{{}}}]", enchants.join(",")));
        let path: Vec<f64> = v["path"].as_array().unwrap().iter().map(|y| y.as_f64().unwrap() + dy).collect();
        // The vanilla player began one fall step above the first position of its path.
        let start_y = path[0] + 0.0784;
        // (The mace is swung at full strength: its cooldown ran out.)
        w.ticks(45);
        let mob = *w.sim.entity_ids_of("minecraft:zombie").last().expect("the zombie");
        let health_before = w.sim.mob_state(mob).unwrap().health;
        let attack_at = v["attack_at"].as_u64().unwrap() as usize;
        w.run(&format!("tp Faller {x} {start_y} {z}"));
        w.ticks(2);
        let _ = w.packets();
        for (tick, &y) in path.iter().enumerate() {
            let mut inbox = Vec::new();
            inbox.push(ToSim::Packet(1, PlayIn::Move { pos: Some([x, y, z]), rot: Some([0.0, 90.0]), on_ground: false, horizontal_collision: false }));
            if tick == attack_at {
                inbox.push(ToSim::Packet(1, PlayIn::Attack { entity_id: mob }));
            }
            inbox.push(ToSim::Packet(1, PlayIn::ClientTickEnd));
            assert!(w.sim.step(inbox));
            if tick == attack_at {
                break;
            }
        }
        let player = w.sim.entity_id(1).unwrap();
        let packets = w.packets();
        let lines = describe(&w, &packets, player);
        let state = w.sim.mob_state(mob).unwrap();
        let r = &v["result"];
        let mut diffs = Vec::new();
        let mut check = |what: &str, ours: String, theirs: String| {
            if ours != theirs {
                diffs.push(format!("{what}: kiln {ours} vanilla {theirs}"));
            }
        };
        check("victim health before", format!("{health_before}"), format!("{}", r["victim_health_before"].as_f64().unwrap() as f32));
        check("victim health", format!("{}", state.health), format!("{}", r["victim_health"].as_f64().unwrap() as f32));
        let theirs: Vec<f64> = r["victim_delta"].as_array().unwrap().iter().map(|d| d.as_f64().unwrap()).collect();
        check("victim motion", format!("{:?}", state.delta), format!("{theirs:?}"));
        check("attacker fall distance", format!("{:.4}", w.sim.fall_distance(1).unwrap()), format!("{:.4}", r["attacker_fall_distance"].as_f64().unwrap()));
        check("mace damage", format!("{}", w.sim.item_damage(1, 36).unwrap_or(0)), format!("{}", r["mace_damage"]));
        let explosions_theirs = r["packets"].as_array().unwrap().iter().filter(|p| p.as_str().unwrap().starts_with("explode")).count();
        check("explosion packets", format!("{}", lines.iter().filter(|l| l.starts_with("explode")).count()), format!("{explosions_theirs}"));
        if !diffs.is_empty() {
            failures.push(format!("{name}: {}", diffs.join("; ")));
        }
    }
    for f in &failures {
        eprintln!("  {f}");
    }
    eprintln!("{} scenarios, {} differ", vectors.len(), failures.len());
    assert!(failures.is_empty(), "{} of {} smash scenarios differ from vanilla", failures.len(), vectors.len());
}
