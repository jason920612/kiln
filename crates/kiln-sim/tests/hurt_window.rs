//! The hurt window (`LivingEntity.hurtServer`, wp79 item 5): after a hit the target is invulnerable for 20 ticks, and while `invulnerableTime > 10` a second hit
//! of `amount <= lastHurt` is ignored and a bigger one costs only `amount - lastHurt`. So `damage W 2` right after `damage W 1` costs 1 for the next ten ticks
//! and the full 2 after that. The `data get entity Diff0 Health` 19-against-18 flake of command_diff was this boundary (an earlier hit of 1 from the landing
//! at the start of the section, a second one on the edge of the window), against vanilla's own boundary measured at 500 ms: `tools/wp79/flake79.py`.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn health_after(wait_ticks: usize) -> f32 {
    let mut sim = Sim::new(SimConfig::new(2, 4, None));
    let (msg, stats) = join(1, "Hurt", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode survival Hurt".into()), ToSim::Console("gamerule natural_health_regeneration false".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, 40);
    assert!(sim.step([ToSim::Console("damage Hurt 1".into())]));
    tick(&mut sim, wait_ticks);
    assert!(sim.step([ToSim::Console("damage Hurt 2".into())]));
    sim.health(1).unwrap().0
}

#[test]
fn a_second_hit_in_the_window_costs_the_difference_and_after_it_the_full_amount() {
    // (The ticks between the two commands: the first command's own tick counts one, so 8 waits are 9 ticks after the hit, 10 are 11.)
    assert_eq!(health_after(0), 18.0, "the same tick");
    assert_eq!(health_after(8), 18.0, "9 ticks after: still inside (invulnerableTime 11)");
    assert_eq!(health_after(10), 17.0, "11 ticks after: the window is over (invulnerableTime 9)");
    assert_eq!(health_after(30), 17.0, "well after");
}
