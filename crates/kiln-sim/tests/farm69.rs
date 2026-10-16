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

fn at(pen: i32, dx: i32, dy: i32, dz: i32) -> String {
    format!("{} {} {}", dx, F + dy, pen * PEN_DZ + dz)
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
            format!("forceload add -16 -16 32 {}", pens * PEN_DZ + 24),
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
                "summon minecraft:villager 6.5 {} {}.5 {{VillagerData:{{type:\"minecraft:plains\",profession:\"minecraft:none\",level:1}},PersistenceRequired:1b}}",
                F + 1,
                pen * PEN_DZ + 3 + i * 2
            ));
        }
    }
    console(sim, cmds);
}

fn zombies(sim: &mut Sim, pens: i32, on: bool) {
    if !on {
        console(sim, ["kill @e[type=minecraft:zombie]".to_string()]);
        return;
    }
    console(
        sim,
        (0..pens).map(|pen| format!("summon minecraft:zombie 10.5 {} {}.5 {{PersistenceRequired:1b}}", F + 1, pen * PEN_DZ + 4)).collect::<Vec<_>>(),
    );
}

/// Runs `n` ticks, logging new golems (and killing them in the tick after).
fn advance(sim: &mut Sim, n: i64, seen: &mut std::collections::HashSet<i32>, log: &mut Vec<(i64, i64, [f64; 3])>) {
    let mut kill = false;
    for _ in 0..n {
        if kill {
            console(sim, ["kill @e[type=minecraft:iron_golem]".to_string()]);
            kill = false;
        } else {
            assert!(sim.step([]));
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
    let pens: i32 = std::env::var("FARM_PENS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("farm69");
    let _ = std::fs::remove_dir_all(&dir);
    let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.clone())));
    build(&mut sim, pens);
    console(&mut sim, ["time set 11500".to_string()]);
    let mut seen = std::collections::HashSet::new();
    let mut log = Vec::new();
    let t0 = std::time::Instant::now();
    for day in 0..=days {
        let dt = sim.day_time() % 24000;
        if dt >= 11000 || day == 0 {
            zombies(&mut sim, pens, false);
            advance(&mut sim, (24000 - dt + 300) % 24000, &mut seen, &mut log);
        }
        if day == days {
            break;
        }
        zombies(&mut sim, pens, true);
        let dt = sim.day_time() % 24000;
        advance(&mut sim, 11000 - dt, &mut seen, &mut log);
        eprintln!("day {day}: {} golems so far [{:.0}s]", log.len(), t0.elapsed().as_secs_f64());
    }
    for (g, dt, p) in &log {
        println!("GOLEM {g} {} {:.1} {:.1} {:.1}", dt % 24000, p[0], p[1], p[2]);
    }
    println!("TOTAL {} golems in {days} days, {pens} pens", log.len());
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}
