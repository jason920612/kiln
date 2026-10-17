//! wp76: a region's entity turns are speculated with villagers, copper golems and vibration listeners in it (they used to
//! keep the whole region's turns in order): the villagers run in place and take points of interest, which the turns that
//! read them log; turns that would tell a listener anything run in place. Every tick's state hash and every player's
//! packet stream must be what one worker (no speculation) gives.

use kiln_sim::testing::{Client, Walker, join, stream_digest};
use kiln_sim::{Sim, SimConfig};
use std::sync::atomic::Ordering::Relaxed;

const SURFACE_Y: i32 = -60;

fn run(workers: usize, chaos: Option<u64>) -> (Vec<u64>, u64, u64) {
    kiln_sim::testing::hash_packets();
    let mut config = SimConfig::new(4, 4, None);
    config.keep_alive = false;
    config.pool.workers = workers;
    config.pool.chaos = chaos;
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "P0", 4);
    let mut inbox = vec![msg, kiln_link::ToSim::Console(format!("tp P0 8.5 {SURFACE_Y} 8.5"))];
    let mut walkers = vec![Walker::new(Client::new(1, stats), [8.5, 8.5], 1)];
    let kept0 = kiln_sim::testing::SPEC_KEPT.load(Relaxed);
    let mut hashes = Vec::new();
    let mut built = false;
    for tick in 0..500 {
        for w in &mut walkers {
            w.tick(3.0, true, &mut inbox);
        }
        if !built && walkers.iter().all(|w| w.client.settled()) {
            built = true;
            let y = SURFACE_Y;
            let mut cmds = vec!["gamerule minecraft:spawn_mobs false".to_string(), "time set 23000".to_string()];
            // A village corner: beds, workstations and a bell; villagers and an iron golem.
            for (i, block) in ["white_bed[part=foot,facing=east]", "composter", "lectern", "barrel", "smoker", "bell[attachment=floor]"].iter().enumerate() {
                cmds.push(format!("setblock {} {y} 20 minecraft:{block}", 16 + 3 * i as i32));
            }
            cmds.push(format!("setblock 17 {y} 20 minecraft:white_bed[part=head,facing=east]"));
            for i in 0..6 {
                cmds.push(format!("summon minecraft:villager {} {y} 16.5", 15.5 + 2.0 * i as f64));
            }
            cmds.push(format!("summon minecraft:iron_golem 24.5 {y} 12.5"));
            cmds.push(format!("summon minecraft:copper_golem 10.5 {y} 22.5"));
            // A sculk sensor listening in the middle of it all.
            cmds.push(format!("setblock 12 {y} 12 minecraft:sculk_sensor"));
            // Drops lying about and falling: some that merge, some that cannot.
            for i in 0..240 {
                let (a, r) = (i as f64 * 2.399, 1.0 + (i % 11) as f64);
                let item = ["rotten_flesh", "bone", "arrow", "wheat", "bread"][i % 5];
                let count = if i % 3 == 0 { 64 } else { 1 + i % 20 };
                cmds.push(format!(
                    "summon minecraft:item {} {} {} {{Item:{{id:\"minecraft:{item}\",count:{count}}}}}",
                    4.5 + r * a.cos(),
                    y as f64 + 0.5 + (i % 4) as f64,
                    4.5 + r * a.sin()
                ));
            }
            inbox.extend(cmds.into_iter().map(kiln_link::ToSim::Console));
        }
        assert!(sim.step(inbox.drain(..)));
        if built {
            hashes.push(sim.state_hash());
        }
        assert!(tick < 499 || built, "the player did not settle");
    }
    let kept = kiln_sim::testing::SPEC_KEPT.load(Relaxed) - kept0;
    (hashes, stream_digest(&walkers, &[]), kept)
}

#[test]
fn villages_and_listeners_speculate_and_match_serial() {
    let reference = run(1, None);
    assert!(reference.0.len() > 300, "the village ticked");
    for (workers, chaos) in [(4, Some(7)), (7, None)] {
        let r = run(workers, chaos);
        assert!(r.2 > 0, "turns were speculated and kept ({workers} workers)");
        assert_eq!(r.0, reference.0, "state hashes, {workers} workers, chaos {chaos:?}");
        if std::env::var_os("KILN_DATAPACK").is_none() {
            assert_eq!(r.1, reference.1, "packet streams, {workers} workers, chaos {chaos:?}");
        }
    }
}
