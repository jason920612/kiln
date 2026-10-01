//! Creaking hearts in the running simulation: an awake heart in a pale oak trunk spawns its
//! creaking near a player at night and holds it, lets it crumble by day or when it is broken,
//! hurts (spreading resin) when the creaking is hit, and wakes from its logs. The creaking's own
//! brain and the heart's random draws are compared with vanilla tick by tick by kiln-entity's
//! `mob_parity` (`tools/mob_vectors.py --filter creaking`).

use kiln_blocks::state;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([
            msg,
            ToSim::Console(format!("gamemode {mode} Keeper")),
            ToSim::Console("difficulty normal".into()),
            ToSim::Console("gamerule minecraft:spawn_mobs true".into()),
            ToSim::Console("gamerule minecraft:spawn_monsters true".into()),
            ToSim::Console("time set 18000".into()),
        ]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Self { sim, client, ground }
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn setblock(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    fn creakings(&self) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == "minecraft:creaking").map(|m| (m.0, m.2, m.3)).collect()
    }

    /// A pale oak trunk along y with a heart in the middle at `heart`, in `state`.
    fn heart(&mut self, heart: [i32; 3], heart_state: &str) {
        self.setblock([heart[0], heart[1] - 1, heart[2]], "minecraft:pale_oak_log[axis=y]");
        self.setblock([heart[0], heart[1] + 1, heart[2]], "minecraft:pale_oak_log[axis=y]");
        self.setblock(heart, &format!("minecraft:creaking_heart[axis=y,creaking_heart_state={heart_state},natural=false]"));
    }

    fn heart_state(&self, heart: [i32; 3]) -> &'static str {
        state::get(self.block(heart), "creaking_heart_state").unwrap_or("?")
    }

    /// The UUID the heart holds (`creaking` in its data).
    fn holds(&self, heart: [i32; 3]) -> Option<Vec<i32>> {
        match self.sim.block_entity_nbt(heart[0], heart[1], heart[2])?.get("creaking") {
            Some(Tag::IntArray(a)) => Some(a.clone()),
            _ => None,
        }
    }

    /// Level particle packets received since counting began.
    fn particles(&self) -> u64 {
        self.client.stats.count_ids.store(true, std::sync::atomic::Ordering::Relaxed);
        self.client.stats.by_id.lock().unwrap().get(&kiln_data::packets::play::clientbound::LEVEL_PARTICLES).map_or(0, |e| e.0)
    }

    fn resin_around(&self, heart: [i32; 3]) -> usize {
        let mut n = 0;
        for dx in -3..=3 {
            for dy in -3..=3 {
                for dz in -3..=3 {
                    let s = self.block([heart[0] + dx, heart[1] + dy, heart[2] + dz]);
                    if kiln_blocks::BlockId::of(s).name() == "minecraft:resin_clump" {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Waits for the heart's first creaking.
    fn wait_for_creaking(&mut self, limit: usize) -> Option<usize> {
        for t in 0..limit {
            self.ticks(1);
            if !self.creakings().is_empty() {
                return Some(t);
            }
        }
        None
    }
}

#[test]
fn an_awake_heart_spawns_its_creaking_at_night_and_holds_it() {
    let mut w = World::new("survival");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    let at = w.wait_for_creaking(200);
    assert!(at.is_some(), "the heart spawned a creaking");
    let list = w.creakings();
    assert_eq!(list.len(), 1);
    let (_, pos, health) = list[0];
    assert_eq!(health, 1.0);
    // Within 16 blocks sideways of the heart (`trySpawnMob`'s range), standing on the ground.
    assert!((pos[0] - heart[0] as f64 - 0.5).abs() <= 16.0 && (pos[2] - heart[2] as f64 - 0.5).abs() <= 16.0, "near the heart: {pos:?}");
    // The heart holds it by UUID, and keeps holding it (no second creaking) for a while.
    let held = w.holds(heart).expect("the heart saves its creaking's UUID");
    assert_eq!(held.len(), 4);
    w.ticks(300);
    assert_eq!(w.creakings().len(), 1, "one creaking per heart");
    assert_eq!(w.holds(heart).as_ref(), Some(&held));
    assert_eq!(w.heart_state(heart), "awake");
}

#[test]
fn a_dormant_heart_a_peaceful_world_or_no_player_near_spawn_nothing() {
    // By day the heart is dormant.
    let mut w = World::new("survival");
    w.run("time set 6000");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    w.ticks(120);
    assert!(w.creakings().is_empty(), "no creaking by day");
    assert_eq!(w.heart_state(heart), "dormant", "the heart sleeps by day");
    // At night on peaceful difficulty, nothing either.
    let mut w = World::new("survival");
    w.run("difficulty peaceful");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    w.ticks(120);
    assert!(w.creakings().is_empty(), "no creaking in peaceful");
    // Nor when only a spectator is near.
    let mut w = World::new("spectator");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    w.ticks(120);
    assert!(w.creakings().is_empty(), "spectators do not count");
}

#[test]
fn a_heart_lets_its_creaking_crumble_by_day() {
    let mut w = World::new("survival");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    assert!(w.wait_for_creaking(200).is_some());
    // Morning: within the heart's next check (20 to 24 ticks) the creaking is torn down and
    // the heart falls dormant, holding nothing.
    w.run("time set 6000");
    w.ticks(40);
    assert!(w.creakings().is_empty(), "the creaking crumbled");
    assert_eq!(w.heart_state(heart), "dormant");
    assert_eq!(w.holds(heart), None);
}

#[test]
fn breaking_the_heart_takes_its_creaking() {
    let mut w = World::new("survival");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    assert!(w.wait_for_creaking(200).is_some());
    let before = w.particles();
    w.setblock(heart, "minecraft:air destroy");
    w.ticks(3);
    assert!(w.creakings().is_empty(), "no heart, no creaking");
    // `tearDown`: the pale oak wood crumble (100) and the awake heart crumble (10).
    assert!(w.particles() >= before + 2, "the creaking crumbled in front of the player");
}

#[test]
fn a_creaking_far_from_its_heart_is_let_go() {
    let mut w = World::new("survival");
    // No spawning: the creaking is summoned by hand, held by the heart through its UUID.
    w.run("gamerule minecraft:spawn_monsters false");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    w.run(&format!("data merge block {} {} {} {{creaking:[I;1,2,3,4]}}", heart[0], heart[1], heart[2]));
    assert_eq!(w.holds(heart), Some(vec![1, 2, 3, 4]));
    let (x, y, z) = (heart[0] as f64 + 0.5, heart[1] as f64, heart[2] as f64 + 40.5);
    w.run(&format!("summon minecraft:creaking {x} {y} {z} {{UUID:[I;1,2,3,4],home_pos:[I;{},{},{}]}}", heart[0], heart[1], heart[2]));
    w.ticks(2);
    assert_eq!(w.creakings().len(), 1, "the heart holds it (else it would have died as its own heart's stray)");
    // 40 blocks away: the heart drops it at its next check (20 to 24 ticks).
    w.ticks(30);
    assert!(w.creakings().is_empty(), "the heart let it go");
    assert_eq!(w.holds(heart), None);
}

#[test]
fn a_player_breaking_the_heart_makes_its_creaking_twitch_and_die() {
    let mut w = World::new("creative");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    assert!(w.wait_for_creaking(200).is_some());
    // Start destroying (instant in creative).
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 0, pos: heart, face: 1, sequence: 1 })]));
    w.ticks(3);
    assert_eq!(kiln_blocks::BlockId::of(w.block(heart)).name(), "minecraft:air", "the heart is gone");
    // The creaking died (health 0) and twitches for 45 ticks before it crumbles.
    let dying = w.creakings();
    assert_eq!(dying.len(), 1, "still twitching");
    assert_eq!(dying[0].2, 0.0);
    w.ticks(30);
    assert_eq!(w.creakings().len(), 1, "twitching lasts 45 ticks");
    w.ticks(30);
    assert!(w.creakings().is_empty(), "then it crumbles");
}

#[test]
fn a_hurt_creaking_sways_and_its_heart_spreads_resin() {
    let mut w = World::new("survival");
    let heart = w.at(6, 1, 0);
    w.heart(heart, "awake");
    assert!(w.wait_for_creaking(200).is_some());
    assert_eq!(w.resin_around(heart), 0);
    // The player steps up to it and hits it: no damage, the heart hurts.
    let (id, pos, _) = w.creakings()[0];
    w.run(&format!("tp Keeper {} {} {}", pos[0] - 1.5, pos[1], pos[2]));
    w.ticks(2);
    let (_, pos2, _) = w.creakings()[0];
    let before = w.particles();
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: id }), ToSim::Packet(1, PlayIn::Punch)]));
    w.ticks(2);
    // `creakingHurt` sends 20 trail particles at once; the emitter then sends two a tick.
    assert!(w.particles() >= before + 20 + 2, "the hurt heart's trails reached the player ({} -> {})", before, w.particles());
    let list = w.creakings();
    assert_eq!(list.len(), 1, "the creaking sways instead of dying (at {pos2:?})");
    assert_eq!(list[0].2, 1.0, "no damage while it is heart-bound");
    assert!(w.resin_around(heart) > 0, "the awake heart spread resin over its logs");
}

#[test]
fn hearts_wake_from_their_logs() {
    // Uprooted, with logs on both sides along its axis: night wakes it.
    let mut w = World::new("survival");
    let heart = w.at(6, 1, 0);
    w.setblock([heart[0], heart[1] - 1, heart[2]], "minecraft:pale_oak_log[axis=y]");
    w.setblock([heart[0], heart[1] + 1, heart[2]], "minecraft:pale_oak_log[axis=y]");
    w.setblock(heart, "minecraft:creaking_heart[axis=y,creaking_heart_state=uprooted]");
    w.ticks(3);
    assert_eq!(w.heart_state(heart), "awake");
    // Without a log on one side it stays uprooted.
    let bare = w.at(10, 1, 0);
    w.setblock([bare[0], bare[1] - 1, bare[2]], "minecraft:pale_oak_log[axis=y]");
    w.setblock(bare, "minecraft:creaking_heart[axis=y,creaking_heart_state=uprooted]");
    w.ticks(3);
    assert_eq!(w.heart_state(bare), "uprooted");
    // A log placed later wakes it (the shape update schedules the check).
    w.setblock([bare[0], bare[1] + 1, bare[2]], "minecraft:pale_oak_log[axis=y]");
    w.ticks(3);
    assert_eq!(w.heart_state(bare), "awake");
    // By day a heart wakes dormant.
    w.run("time set 6000");
    let day = w.at(14, 1, 0);
    w.setblock([day[0], day[1] - 1, day[2]], "minecraft:pale_oak_log[axis=y]");
    w.setblock([day[0], day[1] + 1, day[2]], "minecraft:pale_oak_log[axis=y]");
    w.setblock(day, "minecraft:creaking_heart[axis=y,creaking_heart_state=uprooted]");
    w.ticks(3);
    assert_eq!(w.heart_state(day), "dormant");
}
