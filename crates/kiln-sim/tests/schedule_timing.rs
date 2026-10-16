//! How many ticks a scheduled block tick that a console command starts takes (wp72). Vanilla runs a console command between two ticks at the
//! game time `N` of the last one and schedules at `N + delay`; the tick after the command runs at `N + 1`, so a delay of `d` shows in the
//! `d`-th tick that follows the command. A real-client run with `tick freeze` / `tick step 1` on both servers had Kiln's lamp (delay 4),
//! observer (2) and first repeater (4) one tick early.

use kiln_blocks::state;
use kiln_link::ToSim;
use kiln_sim::{Sim, SimConfig};

fn tick(sim: &mut Sim, n: usize) {
    for _ in 0..n {
        assert!(sim.step([]));
    }
}

fn state_at(sim: &Sim, p: [i32; 3]) -> String {
    state::state_string(sim.block_at(p[0], p[1], p[2]).expect("loaded"))
}

/// Runs `command` as one tick's console input and returns on which of the following ticks (1 = the one that ran the command) `watch` first
/// differs from what it was before.
fn ticks_until_change(sim: &mut Sim, command: &str, watch: [i32; 3]) -> usize {
    let before = state_at(sim, watch);
    assert!(sim.step([ToSim::Console(command.into())]));
    for k in 1..=40 {
        if state_at(sim, watch) != before {
            return k;
        }
        tick(sim, 1);
    }
    panic!("{watch:?} never changed from {before}");
}

fn world() -> Sim {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    assert!(sim.step([
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console("gamerule minecraft:random_tick_speed 0".into()),
        ToSim::Console("forceload add 0 0 31 31".into()),
    ]));
    tick(&mut sim, 20);
    let (y0, y1) = (100, 104);
    for c in [format!("fill 0 {y0} 0 31 {y1} 31 minecraft:air"), format!("fill 0 {} 0 31 {} 31 minecraft:stone", y0 - 1, y0 - 1)] {
        assert!(sim.step([ToSim::Console(c)]));
    }
    tick(&mut sim, 10);
    sim
}

#[test]
fn a_lamp_goes_dark_four_ticks_after_its_power_is_removed() {
    let mut sim = world();
    for c in ["setblock 5 100 5 minecraft:redstone_lamp", "setblock 6 100 5 minecraft:redstone_block"] {
        assert!(sim.step([ToSim::Console(c.into())]));
    }
    tick(&mut sim, 30);
    assert!(state_at(&sim, [5, 100, 5]).contains("lit=true"));
    // the command runs in tick 1; the lamp's scheduled tick (delay 4) runs at game time N + 4: tick 4
    assert_eq!(ticks_until_change(&mut sim, "setblock 6 100 5 minecraft:air", [5, 100, 5]), 4);
}

#[test]
fn an_observer_pulses_two_ticks_after_the_block_it_watches_changes() {
    let mut sim = world();
    assert!(sim.step([ToSim::Console("setblock 5 100 5 minecraft:observer[facing=west]".into())]));
    tick(&mut sim, 30);
    assert!(state_at(&sim, [5, 100, 5]).contains("powered=false"));
    assert_eq!(ticks_until_change(&mut sim, "setblock 4 100 5 minecraft:stone", [5, 100, 5]), 2);
}

#[test]
fn a_repeater_turns_on_after_its_delay() {
    let mut sim = world();
    assert!(sim.step([ToSim::Console("setblock 5 100 5 minecraft:repeater[facing=west,delay=2]".into())]));
    tick(&mut sim, 30);
    assert_eq!(ticks_until_change(&mut sim, "setblock 4 100 5 minecraft:redstone_block", [5, 100, 5]), 4);
}
