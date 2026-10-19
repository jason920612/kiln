//! wp82: bone meal on the `BonemealableBlock`s wp80 left out or got wrong (a dispenser aimed at the plant, one bone meal each): carrots and potatoes (their
//! own `CarrotBlock`/`PotatoBlock` classes were missing from the crop arms: bone meal did nothing), shelf mushrooms (age 0 → 1), glow lichen (the
//! `MultifaceSpreader`: another face of the lichen or a copy on the next surface).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct Lab {
    sim: Sim,
    client: Client,
    x: i32,
    y: i32,
    z: i32,
}

impl Lab {
    fn new() -> Lab {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
        let mut lab = Lab { sim, client: Client::new(1, stats), x: 0, y: 0, z: 0 };
        lab.ticks(5);
        let p = lab.client.pos;
        (lab.x, lab.y, lab.z) = (p[0].floor() as i32 + 4, p[1].floor() as i32 + 10, p[2].floor() as i32);
        lab
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, cmds: &[String]) {
        assert!(self.sim.step(cmds.iter().map(|c| ToSim::Console(c.clone()))));
    }

    /// An empty box around the lab's spot, `floor` below it, then `setup` (offsets from the spot), and a dispenser west of the spot with `n` bone meal.
    fn plant(&mut self, floor: &str, setup: &[(i32, i32, i32, &str)], n: u32) {
        let (x, y, z) = (self.x, self.y, self.z);
        let mut cmds = vec![format!("fill {} {} {} {} {} {} minecraft:air", x - 5, y - 3, z - 5, x + 5, y + 8, z + 5), format!("fill {} {} {} {} {} {} minecraft:{floor}", x - 5, y - 1, z - 5, x + 5, y - 1, z + 5)];
        for &(dx, dy, dz, block) in setup {
            cmds.push(format!("setblock {} {} {} {block}", x + dx, y + dy, z + dz));
        }
        cmds.push(format!("setblock {} {y} {z} minecraft:dispenser[facing=east]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:{n}}}]}}", x - 1));
        self.run(&cmds);
        self.ticks(3);
    }

    fn fire(&mut self) {
        let (x, y, z) = (self.x, self.y, self.z);
        // Toggle the power: the dispenser fires on a rising edge.
        self.run(&[format!("setblock {} {} {z} minecraft:redstone_block", x - 1, y + 1)]);
        self.ticks(8);
        self.run(&[format!("setblock {} {} {z} minecraft:air", x - 1, y + 1)]);
        self.ticks(4);
    }

    fn state(&self, dx: i32, dy: i32, dz: i32) -> String {
        let s = self.sim.block_at(self.x + dx, self.y + dy, self.z + dz).expect("loaded");
        kiln_blocks::BlockId::of(s).name().to_string()
    }

    fn age(&self, dx: i32, dy: i32, dz: i32) -> i32 {
        kiln_blocks::state::get_int(self.sim.block_at(self.x + dx, self.y + dy, self.z + dz).expect("loaded"), "age")
    }
}

#[test]
fn bone_meal_ages_carrots_and_potatoes_like_other_crops() {
    for (crop, name) in [("carrots", "minecraft:carrots"), ("potatoes", "minecraft:potatoes"), ("wheat", "minecraft:wheat")] {
        let mut lab = Lab::new();
        lab.plant("farmland[moisture=7]", &[(0, 0, 0, &format!("minecraft:{crop}[age=0]"))], 1);
        assert_eq!(lab.age(0, 0, 0), 0);
        lab.fire();
        assert_eq!(lab.state(0, 0, 0), name);
        assert!((2..=5).contains(&lab.age(0, 0, 0)), "{crop}: one bone meal adds 2 to 5 ages: {}", lab.age(0, 0, 0));
    }
}

#[test]
fn bone_meal_ripens_a_shelf_mushroom_once() {
    let mut lab = Lab::new();
    lab.plant("stone", &[(0, 0, 1, "minecraft:stone"), (0, 0, 0, "minecraft:shelf_mushroom[facing=north,age=0]")], 2);
    assert_eq!(lab.age(0, 0, 0), 0);
    lab.fire();
    assert_eq!(lab.age(0, 0, 0), 1, "ripe after one bone meal");
    lab.fire();
    assert_eq!(lab.age(0, 0, 0), 1, "and it stays ripe");
}

#[test]
fn bone_meal_spreads_glow_lichen_to_another_face() {
    // Lichen on the floor of a stone box: one bone meal adds a face on the lichen's block or a lichen beside it, on the walls or the floor.
    let mut lab = Lab::new();
    lab.plant("stone", &[(0, 0, 0, "minecraft:glow_lichen[down=true]")], 1);
    let count = |lab: &Lab| {
        let mut faces = 0;
        let mut blocks = 0;
        for dx in -2..=2 {
            for dy in -1..=2 {
                for dz in -2..=2 {
                    let s = lab.sim.block_at(lab.x + dx, lab.y + dy, lab.z + dz).unwrap();
                    if kiln_blocks::BlockId::of(s).name() == "minecraft:glow_lichen" {
                        blocks += 1;
                        for face in ["up", "down", "north", "south", "east", "west"] {
                            faces += i32::from(kiln_blocks::state::get_bool(s, face));
                        }
                    }
                }
            }
        }
        (blocks, faces)
    };
    assert_eq!(count(&lab), (1, 1));
    lab.fire();
    let (blocks, faces) = count(&lab);
    assert_eq!(faces, 2, "one more face: {blocks} blocks, {faces} faces");
    assert!(blocks <= 2);
}

#[test]
fn bone_meal_that_ripens_a_stem_lets_it_try_to_bear_fruit_at_once() {
    // `StemBlock.performBonemeal`: age + 2..5 (at most 7), and at 7 the stem's `randomTick` runs on the spot: it bears a melon beside it with about 1 in 7.
    let mut lab = Lab::new();
    let (x, y, z) = (lab.x, lab.y, lab.z);
    let mut cmds = vec![
        format!("fill {} {} {} {} {} {} minecraft:air", x - 3, y - 2, z - 3, x + 3 * 40, y + 4, z + 6),
        format!("fill {} {} {} {} {} {} minecraft:farmland[moisture=7]", x - 3, y - 1, z - 3, x + 3 * 40, y - 1, z + 6),
    ];
    for i in 0..40 {
        let sx = x + 3 * i;
        cmds.push(format!("setblock {sx} {y} {z} minecraft:melon_stem[age=6]"));
        cmds.push(format!("setblock {sx} {y} {} minecraft:dispenser[facing=north]{{Items:[{{Slot:0b,id:\"minecraft:bone_meal\",count:1}}]}}", z + 1));
    }
    lab.run(&cmds);
    lab.ticks(3);
    let fire: Vec<String> = (0..40).map(|i| format!("setblock {} {} {} minecraft:redstone_block", x + 3 * i, y + 1, z + 1)).collect();
    lab.run(&fire);
    lab.ticks(8);
    let mut fruit = 0;
    for i in 0..40 {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if lab.state(3 * i + dx, 0, dz) == "minecraft:melon" {
                fruit += 1;
            }
        }
    }
    assert!(fruit >= 1, "of 40 stems brought to age 7 by bone meal some bore a melon at once (1 in 7 each): {fruit}");
}
