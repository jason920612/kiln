//! Wooden shelves (wp68): a click on one of the three slots swaps the held stack with it, and a powered row of up to three chained shelves
//! swaps the whole hotbar. Kiln had the blocks but neither their block entity nor their use: found with two real clients beside a
//! vanilla server, where the item went onto the shelf and in Kiln the click did nothing.

use kiln_blocks::state;
use kiln_link::{PlayIn, ToSim};
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
    sequence: i32,
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Keeper")), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Self { sim, client, ground, sequence: 0 }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
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

    /// A right click on the front (north) face of the shelf at `p`, at `x` across the block.
    fn click_front(&mut self, p: [i32; 3], x: f32) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos: p, face: 2, cursor: [x, 0.5, 0.0], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn items(&self, p: [i32; 3]) -> Vec<(usize, &'static str, i32)> {
        self.sim.container_at(p).expect("a shelf").0
    }

    /// The hotbar as (item id, count) per slot.
    fn hotbar(&self) -> Vec<Option<(i32, i32)>> {
        self.sim.inventory(1).unwrap()[36..45].to_vec()
    }

    fn id(item: &str) -> i32 {
        kiln_data::builtin_id("minecraft:item", item).unwrap()
    }
}

#[test]
fn a_click_on_a_slot_swaps_the_held_stack_with_it() {
    let mut w = World::new("survival");
    let shelf = w.at(2, 1, 0);
    w.setblock(shelf, "minecraft:oak_shelf[facing=north]");
    w.run("give Keeper minecraft:diamond 3");
    // Facing north the left of the front is east: an x of 0.5 is the middle slot, 0.9 the first (x 0.1 the last).
    w.click_front(shelf, 0.5);
    assert_eq!(w.items(shelf), vec![(1, "minecraft:diamond", 3)], "the whole stack went in");
    assert_eq!(w.hotbar()[0], None, "and the hand is empty");
    // The empty hand takes it back, from the slot it clicks only.
    w.click_front(shelf, 0.9);
    assert_eq!(w.items(shelf), vec![(1, "minecraft:diamond", 3)], "another slot: nothing there to take");
    w.click_front(shelf, 0.5);
    assert!(w.items(shelf).is_empty());
    assert_eq!(w.hotbar()[0], Some((World::id("minecraft:diamond"), 3)));
    // A held stack against a slot that has one: they swap.
    w.click_front(shelf, 0.1);
    assert_eq!(w.items(shelf), vec![(2, "minecraft:diamond", 3)]);
    w.run("give Keeper minecraft:emerald 5");
    assert_eq!(w.hotbar()[0], Some((World::id("minecraft:emerald"), 5)));
    w.click_front(shelf, 0.1);
    assert_eq!(w.items(shelf), vec![(2, "minecraft:emerald", 5)]);
    assert_eq!(w.hotbar()[0], Some((World::id("minecraft:diamond"), 3)));
    // A broken shelf drops what it holds.
    w.run(&format!("setblock {} {} {} minecraft:air destroy", shelf[0], shelf[1], shelf[2]));
    let drops: Vec<_> = w.sim.item_stacks().iter().map(|s| s.item_name().to_owned()).collect();
    assert!(drops.iter().any(|n| n == "minecraft:emerald"), "{drops:?}");
    assert!(drops.iter().any(|n| n == "minecraft:oak_shelf"), "{drops:?}");
}

#[test]
fn a_powered_row_swaps_the_hotbar_with_all_of_its_shelves() {
    let mut w = World::new("creative");
    // Three shelves facing north, each with a redstone block behind it.
    let row = [w.at(2, 1, 0), w.at(3, 1, 0), w.at(4, 1, 0)];
    for p in row {
        w.setblock(p, "minecraft:oak_shelf[facing=north]");
    }
    let items = ["diamond", "emerald", "gold_ingot", "iron_ingot", "coal", "apple", "stick", "bone", "arrow"];
    for (n, item) in items.iter().enumerate() {
        w.run(&format!("item replace entity Keeper hotbar.{n} with minecraft:{item} {}", n + 1));
    }
    // Powering them one after the other chains them: the last one powered is the left end (the east end, seen from the front).
    for p in row {
        w.setblock([p[0], p[1], p[2] + 1], "minecraft:redstone_block");
    }
    let part = |w: &World, p: [i32; 3]| state::get(w.block(p), "side_chain").unwrap();
    assert_eq!([part(&w, row[0]), part(&w, row[1]), part(&w, row[2])], ["right", "center", "left"]);
    assert!(row.iter().all(|&p| state::get_bool(w.block(p), "powered")));
    // A click on the middle one: the list is that shelf, then the one on its left, then the one on its right.
    w.click_front(row[1], 0.5);
    let want = |a: [&'static str; 3], n0: i32| (0..3).map(|j| (j, a[j], n0 + j as i32)).collect::<Vec<_>>();
    let mid: Vec<_> = want(["minecraft:diamond", "minecraft:emerald", "minecraft:gold_ingot"], 1);
    let left: Vec<_> = want(["minecraft:iron_ingot", "minecraft:coal", "minecraft:apple"], 4);
    let right: Vec<_> = want(["minecraft:stick", "minecraft:bone", "minecraft:arrow"], 7);
    assert_eq!(w.items(row[1]), mid, "the clicked shelf takes hotbar slots 0 to 2");
    assert_eq!(w.items(row[2]), left, "the one on its left takes 3 to 5");
    assert_eq!(w.items(row[0]), right, "the one on its right takes 6 to 8");
    assert!(w.hotbar().iter().all(Option::is_none), "{:?}", w.hotbar());
    // Again: the hotbar gets everything back.
    w.click_front(row[1], 0.5);
    assert_eq!(w.hotbar()[0], Some((World::id("minecraft:diamond"), 1)));
    assert_eq!(w.hotbar()[8], Some((World::id("minecraft:arrow"), 9)));
    assert!(row.iter().all(|&p| w.items(p).is_empty()));
    // Taking the power away unchains them.
    w.setblock([row[1][0], row[1][1], row[1][2] + 1], "minecraft:air");
    w.ticks(2);
    assert!(!state::get_bool(w.block(row[1]), "powered"));
    assert_eq!(part(&w, row[1]), "unconnected");
    assert_eq!([part(&w, row[0]), part(&w, row[2])], ["unconnected", "unconnected"]);
}
