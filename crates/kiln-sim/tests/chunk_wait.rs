//! A player whose chunk is not loaded yet (joined, or teleported into terrain nobody generated)
//! is in the level and ticks like anyone, as in vanilla: `ServerGamePacketListenerImpl.tick`
//! calls `ServerPlayer.doTick` every tick whether or not the chunk is there, so effects, hunger
//! and the other timers advance while the client shows "Loading terrain". Meanwhile the chunk is
//! generated on the side and no tick holds up. Needs the vanilla datapack (`KILN_DATAPACK`, else
//! `$KILN_WORK/generated`, else `work/generated`); skips without it.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{NoiseConfig, Sim, SimConfig};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn datapack() -> Option<PathBuf> {
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let work = match std::env::var_os("KILN_WORK") {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
            };
            work.join("generated")
        }
    };
    if dir.join("reports/biome_parameters").is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: no datapack at {} (KILN_DATAPACK or `cargo xtask data`)", dir.display());
        None
    }
}

fn config(datapack: PathBuf, world: Option<PathBuf>) -> SimConfig {
    let mut config = SimConfig::new(4, 3, world);
    config.noise = Some(NoiseConfig { seed: 777, datapack, threads: 2 });
    config
}

/// One tick with the clients' answers; the time it took.
fn tick(sim: &mut Sim, clients: &mut [Client], extra: Vec<ToSim>) -> Duration {
    let mut inbox = extra;
    for c in clients.iter_mut() {
        c.tick(None, &mut inbox);
    }
    let started = Instant::now();
    assert!(sim.step(inbox));
    started.elapsed()
}

fn console(sim: &mut Sim, clients: &mut [Client], command: &str) -> Duration {
    tick(sim, clients, vec![ToSim::Console(command.into())])
}

/// Ticks until nobody waits for a chunk any more.
fn settle(sim: &mut Sim, clients: &mut [Client], conns: &[u64]) {
    for _ in 0..20_000 {
        if conns.iter().all(|&c| sim.waiting_for_chunk(c) == Some(false)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
        tick(sim, clients, Vec::new());
    }
    panic!("the players never got their chunks");
}

/// What a player's tick advances without moving: effects and hunger.
fn state(sim: &Sim, conn: u64) -> (Vec<(&'static str, i32, i32)>, (i32, f32)) {
    (sim.effects(conn).unwrap(), sim.food(conn).unwrap())
}

#[test]
fn a_teleported_player_waiting_for_its_chunk_ticks_like_one_in_loaded_terrain() {
    let Some(datapack) = datapack() else { return };
    let mut sim = Sim::new(config(datapack, None));
    let (alice, a_stats) = join(1, "Alice", 2);
    let (bob, b_stats) = join(2, "Bob", 2);
    assert!(sim.step([alice, bob]));
    let mut clients = [Client::new(1, a_stats), Client::new(2, b_stats)];
    settle(&mut sim, &mut clients, &[1, 2]);
    for _ in 0..5 {
        tick(&mut sim, &mut clients, Vec::new());
    }
    // The same effects on both: hunger drains the food bar (0.005 exhaustion a tick per level,
    // 256 levels), speed just counts down.
    let both = ["Alice", "Bob"].into_iter().flat_map(|who| {
        [format!("gamemode survival {who}"), format!("effect give {who} minecraft:hunger 100 255"), format!("effect give {who} minecraft:speed 100 0")].map(ToSim::Console)
    });
    tick(&mut sim, &mut clients, both.collect());
    assert_eq!(state(&sim, 1), state(&sim, 2), "the same start");
    // Bob goes to terrain nobody generated; Alice stays.
    let mut longest = console(&mut sim, &mut clients, "tp Bob 41000 120 41000");
    assert_eq!(sim.waiting_for_chunk(2), Some(true), "Bob waits for his chunk");
    assert_eq!(sim.waiting_for_chunk(1), Some(false));
    let mut waited = 0;
    let started = Instant::now();
    while sim.waiting_for_chunk(2) == Some(true) {
        assert!(started.elapsed() < Duration::from_secs(60), "the chunk never came");
        std::thread::sleep(Duration::from_millis(1));
        longest = longest.max(tick(&mut sim, &mut clients, Vec::new()));
        if sim.waiting_for_chunk(2) == Some(true) {
            waited += 1;
            // Tick by tick, effects and food of the waiting player are the standing one's.
            assert_eq!(state(&sim, 1), state(&sim, 2), "tick {waited} of the wait");
        }
    }
    assert!(waited >= 30, "the chunk was there after {waited} ticks: too soon to tell");
    let (effects, (food, _)) = state(&sim, 2);
    assert!(food < 20, "hunger ran while waiting: food {food}");
    assert!(effects.iter().all(|&(_, _, left)| left < 2000 - 30), "the effects counted down: {effects:?}");
    // And on, in his region.
    for n in 0..20 {
        tick(&mut sim, &mut clients, Vec::new());
        assert_eq!(state(&sim, 1).0, state(&sim, 2).0, "tick {n} after");
    }
    // Generating the chunk did not stall the tick (it takes seconds on a fresh world when the
    // tick thread makes it).
    eprintln!("waited {waited} ticks, longest tick {longest:?}");
    assert!(longest < Duration::from_millis(500), "a tick took {longest:?} during the wait of {waited} ticks");
}

#[test]
fn a_player_joining_into_new_terrain_is_in_the_level_at_once_and_ticks_while_the_chunk_is_made() {
    let Some(datapack) = datapack() else { return };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("chunk-wait-join");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A first run leaves Carol standing in terrain it never generated: she goes there and
    // quits in the same breath, with a hunger effect on.
    {
        let mut sim = Sim::new(config(datapack.clone(), Some(dir.clone())));
        let (carol, stats) = join(1, "Carol", 2);
        assert!(sim.step([carol]));
        let mut clients = [Client::new(1, stats)];
        settle(&mut sim, &mut clients, &[1]);
        console(&mut sim, &mut clients, "gamemode survival Carol");
        console(&mut sim, &mut clients, "effect give Carol minecraft:hunger 100 255");
        console(&mut sim, &mut clients, "effect give Carol minecraft:speed 100 0");
        console(&mut sim, &mut clients, "tp Carol 52000 120 52000");
        assert_eq!(sim.waiting_for_chunk(1), Some(true));
        assert!(sim.step([ToSim::Leave(1)]));
        assert_eq!(sim.player_count(), 0);
    }
    // The second run: Carol joins into that terrain.
    let mut sim = Sim::new(config(datapack, Some(dir)));
    let (carol, stats) = join(1, "Carol", 2);
    let packets_before = stats.packets.load(std::sync::atomic::Ordering::Relaxed);
    let mut clients = [Client::new(1, stats.clone())];
    let mut longest = tick(&mut sim, &mut clients, vec![carol]);
    assert_eq!(sim.player_count(), 1, "in the level at once, not waiting on a login screen");
    assert!(stats.packets.load(std::sync::atomic::Ordering::Relaxed) > packets_before, "the login went out");
    assert_eq!(sim.waiting_for_chunk(1), Some(true), "her chunk is not there");
    let (start_effects, (start_food, _)) = state(&sim, 1);
    assert_eq!(start_effects.len(), 2, "the saved effects: {start_effects:?}");
    let mut waited = 0;
    let started = Instant::now();
    while sim.waiting_for_chunk(1) == Some(true) {
        assert!(started.elapsed() < Duration::from_secs(60), "the chunk never came");
        std::thread::sleep(Duration::from_millis(1));
        longest = longest.max(tick(&mut sim, &mut clients, Vec::new()));
        if sim.waiting_for_chunk(1) == Some(true) {
            waited += 1;
            // One tick of every effect per server tick.
            let (effects, _) = state(&sim, 1);
            for (now, then) in effects.iter().zip(&start_effects) {
                assert_eq!(now.2, then.2 - waited, "{} after {waited} ticks", now.0);
            }
        }
    }
    assert!(waited >= 30, "the chunk was there after {waited} ticks: too soon to tell");
    assert!(state(&sim, 1).1.0 < start_food, "hunger ran while waiting: {:?} -> {:?} in {waited} ticks", start_effects, state(&sim, 1));
    eprintln!("waited {waited} ticks, longest tick {longest:?}");
    assert!(longest < Duration::from_millis(500), "a tick took {longest:?} during the wait of {waited} ticks");
}
