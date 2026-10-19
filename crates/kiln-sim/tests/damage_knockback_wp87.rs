//! wp87: a hit from outside the level's tick reaches the client unstepped. Vanilla runs console and chat commands (between ticks), the tick
//! functions and the scheduled block ticks (command blocks) before `ServerEntity.sendChanges`, and the player's own `travel` only after the
//! entities (`ServerGamePacketListenerImpl.tick` -> `doTick`): a `/damage` knockback is sent as it was left, 0.4 sideways and
//! min(0.4, -0.0784 / 2 + 0.4) = 0.3608 up, where a zombie's hit of the same tick reaches the client after one step (0.2184 / 0.2752, wp84).

use bytes::Bytes;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    stats: std::sync::Arc<kiln_sim::testing::SinkStats>,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Farmer", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        let keep = stats.clone();
        assert!(sim.step([
            msg,
            ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
            ToSim::Console("gamemode survival Farmer".into()),
            ToSim::Console("difficulty normal".into()),
        ]));
        let mut client = Client::new(1, stats);
        for _ in 0..40 {
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

    /// The velocity in a motion packet of the player.
    fn velocity(p: &Bytes) -> [f64; 3] {
        let mut r = kiln_proto::codec::Reader::new(p);
        r.varint().unwrap();
        r.varint().unwrap();
        kiln_proto::packets::entity::read_lp_vec3(&mut r).unwrap()
    }

    /// The motion packets the player was sent for itself since the last call.
    fn motions(&mut self) -> Vec<Bytes> {
        let id = self.sim.entity_id(1).expect("the player's entity id");
        let want = kiln_data::packets::play::clientbound::SET_ENTITY_MOTION;
        let mut log = self.stats.log.lock().unwrap();
        let all = log.take().unwrap_or_default();
        *log = Some(Vec::new());
        all.into_iter()
            .filter(|p| {
                let mut r = kiln_proto::codec::Reader::new(p);
                r.varint().ok() == Some(want) && r.varint().ok() == Some(id)
            })
            .collect()
    }
}


/// The first motion packet of the player since the last look, within `ticks` ticks, with the tick it came in (0: already there).
fn first_motion(w: &mut World, ticks: usize) -> Option<(usize, [f64; 3])> {
    for t in 0..ticks {
        if t > 0 {
            w.ticks(1);
        }
        if let Some(p) = w.motions().first() {
            return Some((t, World::velocity(p)));
        }
    }
    None
}

fn horizontal(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[2] * v[2]).sqrt()
}

fn assert_raw(v: [f64; 3], what: &str) {
    let y = 0.4f64.min(-0.0784 / 2.0 + 0.4000000059604645);
    assert!((horizontal(v) - 0.4).abs() < 3e-3 && (v[1] - y).abs() < 3e-3, "{what}: sent horizontal {:.4} / up {:.4}, vanilla 0.4000 / {y:.4}", horizontal(v), v[1]);
}

#[test]
fn a_damage_command_tells_the_client_the_unstepped_knockback() {
    let mut w = World::new();
    let p = w.client.pos;
    let _ = w.motions();
    w.run(&format!("damage Farmer 2 minecraft:mob_attack at {} {} {}", p[0] + 2.0, p[1], p[2]));
    let (t, v) = first_motion(&mut w, 4).expect("a motion packet");
    assert_eq!(t, 0, "the packet is sent in the tick of the command");
    assert_raw(v, "damage command");
    // Pushed away from the source position (it was to the east).
    assert!(v[0] < 0.0, "pushed west: {v:?}");
}

#[test]
fn a_command_block_damage_is_unstepped_too() {
    let mut w = World::new();
    let p = w.client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32);
    let _ = w.motions();
    w.run(&format!("setblock {x} {y} {z} minecraft:command_block{{auto:1b,Command:\"damage Farmer 2 minecraft:mob_attack at {x} {y} {z}\"}}"));
    let (_, v) = first_motion(&mut w, 10).expect("the command block hurt the player");
    assert_raw(v, "command block");
}

#[test]
fn a_hit_of_the_entities_turn_still_waits_for_the_step() {
    // (The zombie of wp84, here as `/execute as` a summoned mob so that the hit is in the entity phase: a mob attack.)
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!("summon minecraft:zombie {} {} {} {{PersistenceRequired:1b}}", p[0] + 1.2, p[1], p[2]));
    let _ = w.motions();
    let (_, v) = first_motion(&mut w, 200).expect("the zombie hit");
    assert!((horizontal(v) - 0.2184).abs() < 3e-3, "stepped horizontal {:.4}", horizontal(v));
}
