//! wp84 (a zombie hit a standing survival player on both servers, the client's own motion packets compared): vanilla's `ServerPlayer` delta is
//! its body's - resting at -0.0784 on the ground - and the player's own `travel` moves it one step before `ServerEntity.sendChanges` tells the
//! client, so a knockback reaches the client as 0.4 * 0.546 sideways and (min(0.4, -0.0784 / 2 + 0.4) - 0.08) * 0.98 = 0.2752 up
//! (the velocity Kiln sent was the raw 0.4 / 0.4, and what was left of it added to the next hit). A lunge starts from the resting delta too
//! (the packet carries y = -0.0784).

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

/// Waits (up to 200 ticks) for the zombie to hit and returns the motion packet the hit brought.
fn hit_by_a_zombie(w: &mut World) -> [f64; 3] {
    let _ = w.motions();
    for _ in 0..200 {
        w.ticks(1);
        let got = w.motions();
        if let Some(p) = got.first() {
            assert_eq!(got.len(), 1, "one motion packet for the hit: {got:?}");
            return World::velocity(p);
        }
    }
    panic!("the zombie did not hit");
}

#[test]
fn a_zombie_hit_reaches_the_client_after_one_step_of_travel() {
    let mut w = World::new();
    let p = w.client.pos;
    // (Entities act first in a tick, the player's own `travel` moves its delta after them: the hit is told stepped.)
    w.run(&format!("summon minecraft:zombie {} {} {} {{PersistenceRequired:1b,Tags:[\"z\"]}}", p[0] + 1.2, p[1], p[2]));
    let v = hit_by_a_zombie(&mut w);
    // `knockback(0.4, dx, dz)`: horizontal 0.4 away from the zombie, y = min(0.4, -0.0784 / 2 + 0.4); then `travelInAir` on the ground:
    // x and z * (0.6f * 0.91f), (y - 0.08) * 0.98f.
    let strength = 0.4000000059604645f64;
    let rest = (0.0 - 0.08) * 0.98f32 as f64;
    let y = 0.4f64.min(rest / 2.0 + strength);
    let friction = (0.6f32 * 0.91f32) as f64;
    let (wh, wy) = (strength * friction, (y - 0.08) * 0.98f32 as f64);
    let h = (v[0] * v[0] + v[2] * v[2]).sqrt();
    assert!((h - wh).abs() < 3e-3 && (v[1] - wy).abs() < 3e-3, "sent horizontal {h:.4} / up {:.4}, vanilla {wh:.4} / {wy:.4}", v[1]);
    // The velocity faded: a second hit later is the same push again, not a heavier one.
    w.ticks(40);
    let v2 = hit_by_a_zombie(&mut w);
    let h2 = (v2[0] * v2[0] + v2[2] * v2[2]).sqrt();
    assert!((h2 - wh).abs() < 3e-3 && (v2[1] - wy).abs() < 3e-3, "second hit: horizontal {h2:.4} / up {:.4}", v2[1]);
}

#[test]
fn a_standing_player_rests_at_minus_0_0784() {
    use kiln_proto::nbt::Tag;
    let w = World::new();
    let saved = w.sim.saved_player_nbt(1).expect("the player");
    let Some(Tag::List(m)) = saved.get("Motion") else { panic!("Motion: {saved:?}") };
    let y = m[1].as_f64().unwrap();
    assert!((y + 0.0784).abs() < 1e-4, "Motion y of a standing player: {y}");
}
