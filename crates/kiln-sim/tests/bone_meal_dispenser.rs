//! wp80: bone meal reaches the plants that grow without a tree or a crop table (found with a dispenser against 40 plants of each kind: moss, bamboo, dripleaves,
//! bushes... did nothing in Kiln), by a dispenser and by a player's use.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn stack(sim: &mut Sim, client: &mut Client, cmds: &[String], ticks: usize) {
    assert!(sim.step(cmds.iter().map(|c| ToSim::Console(c.clone()))));
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

#[test]
fn a_dispenser_with_bone_meal_grows_a_bamboo_sapling_and_water_plants() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Keeper", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let p = client.pos;
    let (x, y, z) = (p[0].floor() as i32 + 4, p[1].floor() as i32 + 10, p[2].floor() as i32);
    let dispenser = format!("setblock {} {y} {z} minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:8}}]}}", x - 1);
    stack(
        &mut sim,
        &mut client,
        &[
            format!("fill {} {} {} {} {} {} minecraft:air", x - 3, y - 3, z - 3, x + 3, y + 8, z + 3),
            format!("setblock {x} {} {z} minecraft:grass_block", y - 1),
            format!("setblock {x} {y} {z} minecraft:bamboo_sapling"),
            dispenser,
            format!("setblock {} {} {z} minecraft:redstone_block", x - 1, y + 1),
        ],
        10,
    );
    let above = sim.block_at(x, y + 1, z).expect("loaded");
    assert_eq!(kiln_blocks::BlockId::of(above).name(), "minecraft:bamboo", "bone meal put a bamboo on top of the sapling");
    // Water beside a solid face: the dispenser aimed at water scatters seagrass.
    let (wx, wy, wz) = (x, y + 20, z + 12);
    stack(
        &mut sim,
        &mut client,
        &[
            format!("fill {} {} {} {} {} {} minecraft:water", wx - 6, wy, wz - 6, wx + 6, wy + 8, wz + 6),
            format!("fill {} {} {} {} {} {} minecraft:stone", wx - 6, wy - 1, wz - 6, wx + 6, wy - 1, wz + 6),
            format!("setblock {} {wy} {wz} minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:8}}]}}", wx - 1),
            format!("setblock {} {} {wz} minecraft:redstone_block", wx - 1, wy + 1),
        ],
        10,
    );
    let mut seagrass = 0;
    for dx in -6..=6 {
        for dz in -6..=6 {
            for dy in 0..=8 {
                if let Some(s) = sim.block_at(wx + dx, wy + dy, wz + dz)
                    && kiln_blocks::BlockId::of(s).name().ends_with("seagrass")
                {
                    seagrass += 1;
                }
            }
        }
    }
    assert!(seagrass >= 3, "seagrass was scattered: {seagrass}");
}
