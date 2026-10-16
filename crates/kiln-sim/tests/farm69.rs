//! wp69: the iron golem farm of wp66 (`tools/wp58/rt_golem.py`) in the simulation itself, without a server or a console
//! driver: pens of three villagers with beds, a zombie behind a fence by day, every new golem logged (game time, position) and
//! killed. `cargo test --release -p kiln-sim --test farm69 -- --ignored --nocapture` prints the golems of
//! `FARM_DAYS` days (default 3) in `FARM_PENS` pens (default 4).

use kiln_link::ToSim;
use kiln_sim::{Sim, SimConfig};

const F: i32 = 64;
const PEN_DZ: i32 = 40;

fn console(sim: &mut Sim, cmds: impl IntoIterator<Item = String>) {
    assert!(sim.step(cmds.into_iter().map(ToSim::Console)));
}

/// `FARM_X0`: where the pens' west side is (0: on a chunk boundary, as the console driver's 4000 is; 4: well inside a chunk).
fn x0() -> i32 {
    std::env::var("FARM_X0").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn at(pen: i32, dx: i32, dy: i32, dz: i32) -> String {
    format!("{} {} {}", dx + x0(), F + dy, pen * PEN_DZ + dz)
}

fn build(sim: &mut Sim, pens: i32) {
    console(
        sim,
        [
            "gamerule minecraft:spawn_mobs false".into(),
            "gamerule minecraft:advance_weather false".into(),
            "gamerule minecraft:mob_drops false".into(),
            "difficulty normal".into(),
            "weather clear 1000000".into(),
            format!("forceload add {} -16 {} {}", x0() - 16, x0() + 32, pens * PEN_DZ + 24),
        ],
    );
    // Chunks load over a few ticks.
    for _ in 0..200 {
        assert!(sim.step([]));
    }
    for pen in 0..pens {
        let mut cmds = vec![
            format!("fill {} {} minecraft:stone", at(pen, -12, 0, -12), at(pen, 24, 0, 20)),
            format!("fill {} {} minecraft:air", at(pen, -12, 1, -12), at(pen, 24, 8, 20)),
            format!("fill {} {} minecraft:stone_bricks", at(pen, -1, 1, -1), at(pen, 12, 3, 9)),
            format!("fill {} {} minecraft:air", at(pen, 0, 1, 0), at(pen, 8, 2, 8)),
            format!("fill {} {} minecraft:air", at(pen, 10, 1, 0), at(pen, 11, 2, 8)),
        ];
        for z in 0..9 {
            cmds.push(format!("setblock {} minecraft:oak_fence", at(pen, 9, 1, z)));
            cmds.push(format!("setblock {} minecraft:air", at(pen, 9, 2, z)));
        }
        for z in [2, 4, 6] {
            cmds.push(format!("setblock {} minecraft:red_bed[facing=east,part=head]", at(pen, 3, 1, z)));
            cmds.push(format!("setblock {} minecraft:red_bed[facing=east,part=foot]", at(pen, 4, 1, z)));
        }
        console(sim, cmds);
    }
    console(sim, ["kill @e[type=!player]".to_string()]);
    let mut cmds = Vec::new();
    for pen in 0..pens {
        for i in 0..3 {
            cmds.push(format!(
                "summon minecraft:villager {}.5 {} {}.5 {{VillagerData:{{type:\"minecraft:plains\",profession:\"minecraft:none\",level:1}},PersistenceRequired:1b}}",
                x0() + 6,
                F + 1,
                pen * PEN_DZ + 3 + i * 2
            ));
        }
    }
    console(sim, cmds);
}

fn zombies(sim: &mut Sim, pens: i32, on: bool) {
    if std::env::var_os("FARM_IDLE").is_some() {
        return;
    }
    if !on {
        console(sim, ["kill @e[type=minecraft:zombie]".to_string()]);
        return;
    }
    console(
        sim,
        (0..pens).map(|pen| format!("summon minecraft:zombie {}.5 {} {}.5 {{PersistenceRequired:1b}}", x0() + 10, F + 1, pen * PEN_DZ + 4)).collect::<Vec<_>>(),
    );
}

/// `FARM_PANIC`: how often the villagers panic (as the vanilla dawn scenarios of `mob_parity` count: dt 300..1500 only when `FARM_PANIC=dawn`,
/// else dt 300..11000) and in how many of the opportunities (`game time % 100 == 0`) somebody of the pen panics.
#[derive(Default)]
struct Panic {
    ticks: u64,
    panicking: u64,
    opps: u64,
    opps_any: u64,
    by_phase: [(u64, u64); 20],
    /// Per 100 ticks after dt 300: (villager-ticks, panicking ones).
    bins: [(u64, u64); 20],
}

fn count_panic(sim: &Sim, panic: &mut Panic, dawn: bool) {
    let dt = sim.day_time() % 24000;
    let in_bins = (300..2300).contains(&dt);
    if !(300..if dawn { 1500 } else { 11000 }).contains(&dt) && !in_bins {
        return;
    }
    let mut pens: std::collections::BTreeMap<i32, bool> = std::collections::BTreeMap::new();
    for (id, kind, pos, _) in sim.mobs() {
        if kind != "minecraft:villager" {
            continue;
        }
        let p = sim.mob_brain(id).is_some_and(|b| b.iter().any(|t| t == "act:panic"));
        if in_bins {
            let b = &mut panic.bins[((dt - 300) / 100) as usize];
            b.0 += 1;
            b.1 += p as u64;
        }
        panic.ticks += 1;
        panic.panicking += p as u64;
        let ph = (sim.game_time() % 100 / 5) as usize;
        panic.by_phase[ph].0 += 1;
        panic.by_phase[ph].1 += p as u64;
        *pens.entry(((pos[2] + 20.0) / 40.0).floor() as i32).or_default() |= p;
    }
    if sim.game_time() % 100 == 0 {
        panic.opps += pens.len() as u64;
        panic.opps_any += pens.values().filter(|&&b| b).count() as u64;
    }
}

/// `FARM_ARRIVAL`: where the villagers are against the zombie of their pen in the 200 ticks after it comes (the counters of `MobVectors.arrivalSample`):
/// per 10 ticks (villager-ticks, within 8 blocks of the zombie, moving fast (> 0.15 blocks a tick), panicking).
static ARRIVAL: std::sync::Mutex<([[u64; 4]; 20], Vec<(i32, [f64; 2])>, Option<i64>)> = std::sync::Mutex::new(([[0; 4]; 20], Vec::new(), None));

fn arrival_sample(sim: &Sim) {
    let mut a = ARRIVAL.lock().unwrap();
    let Some(from) = a.2 else { return };
    let t = sim.game_time() - from;
    if !(0..200).contains(&t) {
        if t >= 200 {
            a.2 = None;
            a.1.clear();
        }
        return;
    }
    let mobs = sim.mobs();
    for pen in 0..64 {
        let zone = |z: f64| ((z + 20.0) / 40.0).floor() as i32 == pen;
        let Some(zombie) = mobs.iter().find(|m| m.1 == "minecraft:zombie" && zone(m.2[2])) else { continue };
        for m in mobs.iter().filter(|m| m.1 == "minecraft:villager" && zone(m.2[2])) {
            let panic = sim.mob_brain(m.0).is_some_and(|b| b.iter().any(|x| x == "act:panic"));
            let d2 = (m.2[0] - zombie.2[0]).powi(2) + (m.2[1] - zombie.2[1]).powi(2) + (m.2[2] - zombie.2[2]).powi(2);
            let fast = a.1.iter().find(|p| p.0 == m.0).is_some_and(|p| ((m.2[0] - p.1[0]).powi(2) + (m.2[2] - p.1[1]).powi(2)).sqrt() > 0.15);
            let b = &mut a.0[(t / 10) as usize];
            b[0] += 1;
            b[1] += (d2 <= 64.0) as u64;
            b[2] += fast as u64;
            b[3] += panic as u64;
        }
    }
    let prev: Vec<(i32, [f64; 2])> = mobs.iter().filter(|m| m.1 == "minecraft:villager").map(|m| (m.0, [m.2[0], m.2[2]])).collect();
    a.1.clear();
    a.1.extend(prev);
}

/// `FARM_IDLE`: no zombies; the villagers counted by x offset (bins of one block from the pen's west side), every 20 ticks of the day between dt 300 and 11000.
static IDLE: std::sync::Mutex<([u64; 12], u64)> = std::sync::Mutex::new(([0; 12], 0));

fn idle_sample(sim: &Sim) {
    let dt = sim.day_time() % 24000;
    if !(300..11000).contains(&dt) || sim.game_time() % 20 != 0 {
        return;
    }
    let mut a = IDLE.lock().unwrap();
    for (_, kind, pos, _) in sim.mobs() {
        if kind == "minecraft:villager" {
            let b = ((pos[0] - x0() as f64).floor() as i32 + 1).clamp(0, 11) as usize;
            a.0[b] += 1;
            a.1 += 1;
        }
    }
}

/// Runs `n` ticks, logging new golems (and killing them in the tick after).
fn advance(sim: &mut Sim, n: i64, seen: &mut std::collections::HashSet<i32>, log: &mut Vec<(i64, i64, [f64; 3])>, panic: &mut Panic) {
    let mut kill = false;
    for _ in 0..n {
        if kill {
            console(sim, ["kill @e[type=minecraft:iron_golem]".to_string()]);
            kill = false;
        } else {
            assert!(sim.step([]));
        }
        if std::env::var_os("FARM_ARRIVAL").is_some() {
            arrival_sample(sim);
        }
        if std::env::var_os("FARM_IDLE").is_some() {
            idle_sample(sim);
        }
        if let Ok(v) = std::env::var("FARM_PANIC") {
            count_panic(sim, panic, v == "dawn");
        }
        // `FARM_TRACE=from,to`: where the villagers of the first pens are, every 20 ticks of the day between.
        if let Some((from, to)) = std::env::var("FARM_TRACE").ok().and_then(|v| v.split_once(',').map(|(a, b)| (a.parse::<i64>().unwrap(), b.parse::<i64>().unwrap()))) {
            let dt = sim.day_time() % 24000;
            if (from..to).contains(&dt) && dt % 20 == 0 {
                for (id, kind, pos, _) in sim.mobs() {
                    if (kind == "minecraft:villager" || kind == "minecraft:zombie") && pos[2] < 12.0 {
                        let brain = sim.mob_brain(id).map(|b| b.into_iter().filter(|t| t.starts_with("act:") || t.starts_with("m:minecraft:nearest_hostile") || t.starts_with("m:minecraft:golem") || t.starts_with("m:minecraft:home") || t.starts_with("m:minecraft:last_slept")).collect::<Vec<_>>().join(",")).unwrap_or_default();
                        println!("TRACE {} {dt} {id} {} {:.2} {:.2} {:.2} {brain}", sim.game_time(), kind.trim_start_matches("minecraft:"), pos[0], pos[1], pos[2]);
                    }
                }
            }
        }
        for (id, kind, pos, _) in sim.mobs() {
            if kind == "minecraft:iron_golem" && seen.insert(id) {
                log.push((sim.game_time(), sim.day_time(), pos));
                kill = true;
            }
        }
    }
}

#[test]
#[ignore]
fn the_farm_in_the_simulation() {
    let days: i64 = std::env::var("FARM_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    // `FARM_ARRIVE`: the day time at which the zombies come (300 as in wp66).
    let arrive: i64 = std::env::var("FARM_ARRIVE").ok().and_then(|v| v.parse().ok()).unwrap_or(299);
    let pens: i32 = std::env::var("FARM_PENS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("farm69");
    let _ = std::fs::remove_dir_all(&dir);
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.clone())));
    build(&mut sim, pens);
    // The day time and the game time are made to agree modulo 100: the opportunities of `VillagerPanicTrigger` (`game time % 100 == 0`) then
    // fall on whole hundreds of the day, as they do in the scenarios `tools/MobVectors.java` records (`farm69_days_*`).
    let mut want = 11500;
    for _ in 0..5 {
        console(&mut sim, [format!("time set {want}")]);
        let d = (sim.day_time() - sim.game_time()).rem_euclid(100);
        if d == 0 {
            break;
        }
        // (the tick that applies the command also takes the game time one on)
        want = want - d + 1;
    }
    assert_eq!((sim.day_time() - sim.game_time()).rem_euclid(100), 0);
    let mut seen = std::collections::HashSet::new();
    let mut log = Vec::new();
    let mut panic = Panic::default();
    let t0 = std::time::Instant::now();
    for day in 0..=days {
        let dt = sim.day_time() % 24000;
        if dt >= 11000 || day == 0 {
            zombies(&mut sim, pens, false);
            advance(&mut sim, (24000 - dt + arrive) % 24000, &mut seen, &mut log, &mut panic);
        }
        if day == days {
            break;
        }
        zombies(&mut sim, pens, true);
        ARRIVAL.lock().unwrap().2 = Some(sim.game_time());
        let dt = sim.day_time() % 24000;
        advance(&mut sim, 11000 - dt, &mut seen, &mut log, &mut panic);
        eprintln!("day {day}: {} golems so far [{:.0}s]", log.len(), t0.elapsed().as_secs_f64());
    }
    // `FARM_VILLAGE`: how far from a village the pens' rooms are (`sections_to_village`: 0 is a village).
    if std::env::var_os("FARM_VILLAGE").is_some() {
        for pen in 0..pens {
            println!("VILLAGE pen {pen}: {} (room), {} (zombie cell)", sim.sections_to_village("minecraft:overworld", [4, F + 1, pen * PEN_DZ + 4]), sim.sections_to_village("minecraft:overworld", [10, F + 1, pen * PEN_DZ + 4]));
        }
    }
    for (g, dt, p) in &log {
        println!("GOLEM {g} {} {:.1} {:.1} {:.1}", dt % 24000, p[0], p[1], p[2]);
    }
    println!("PANIC villager-ticks {} panicking {:.4} opps {} any {:.4}", panic.ticks, panic.panicking as f64 / panic.ticks.max(1) as f64, panic.opps, panic.opps_any as f64 / panic.opps.max(1) as f64);
    println!("BINS {:?}", panic.bins.iter().map(|&(a, b)| (b as f64 / a.max(1) as f64 * 1000.0).round() / 1000.0).collect::<Vec<_>>());
    println!("PHASE {:?}", panic.by_phase.iter().map(|&(a, b)| (b as f64 / a.max(1) as f64 * 1000.0).round() / 1000.0).collect::<Vec<_>>());
    if std::env::var_os("FARM_ARRIVAL").is_some() {
        let a = ARRIVAL.lock().unwrap();
        println!("ARRIVAL {:?}", a.0);
    }
    if std::env::var_os("FARM_IDLE").is_some() {
        let a = IDLE.lock().unwrap();
        println!("IDLE samples {} x 0..8: {:?}", a.1, (1..10).map(|i| (a.0[i] as f64 / a.1 as f64 * 1000.0).round() / 1000.0).collect::<Vec<_>>());
    }
    println!("TOTAL {} golems in {days} days, {pens} pens", log.len());
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}
