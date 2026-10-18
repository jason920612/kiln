//! wp80 (found by comparing 1,600 eyeblossoms in both servers across the day: vanilla's all closed at noon and open at midnight within seconds, Kiln's never
//! changed): `EyeblossomBlock` follows the overworld's `gameplay/eyeblossom_open` attribute (open from tick 12600 to 23401) on random ticks, and the ones that
//! change wake their neighbours of the same kind with scheduled ticks, nearest first.

use kiln_blocks::BlockPos;
use kiln_data::blocks::default_state as d;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn count_after(time: &str, start: u16, ticks: usize) -> (usize, usize) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Gardener", 2);
    assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamerule minecraft:advance_time false".into())]));
    let mut client = Client::new(1, stats);
    let mut tick = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    tick(&mut sim, 5);
    let (x, y, z) = (client.pos[0].floor() as i32 + 2, client.pos[1].floor() as i32, client.pos[2].floor() as i32);
    let name = if start == d::OPEN_EYEBLOSSOM { "open_eyeblossom" } else { "closed_eyeblossom" };
    assert!(sim.step([
        ToSim::Console(format!("fill {x} {} {z} {} {} {} minecraft:grass_block", y - 1, x + 5, y - 1, z + 5)),
        ToSim::Console(format!("fill {x} {y} {z} {} {y} {} minecraft:{name}", x + 5, z + 5)),
        ToSim::Console(format!("time set {time}")),
        ToSim::Console("gamerule minecraft:random_tick_speed 4096".into()),
    ]));
    tick(&mut sim, ticks);
    let (mut open, mut closed) = (0, 0);
    for dx in 0..=5 {
        for dz in 0..=5 {
            match sim.block_at(x + dx, y, z + dz) {
                Some(s) if s == d::OPEN_EYEBLOSSOM => open += 1,
                Some(s) if s == d::CLOSED_EYEBLOSSOM => closed += 1,
                _ => {}
            }
        }
    }
    let _ = BlockPos::new(0, 0, 0);
    (open, closed)
}

#[test]
fn closed_eyeblossoms_open_at_night_and_open_ones_close_by_day() {
    assert_eq!(count_after("18000", d::CLOSED_EYEBLOSSOM, 200), (36, 0), "midnight: every closed one opens");
    assert_eq!(count_after("6000", d::OPEN_EYEBLOSSOM, 200), (0, 36), "noon: every open one closes");
    assert_eq!(count_after("6000", d::CLOSED_EYEBLOSSOM, 200), (0, 36), "noon: closed ones stay closed");
}
