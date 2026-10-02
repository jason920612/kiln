//! Allays in the running simulation (wp28): hearing a note block (a game event that reaches
//! them after the travel time), liking it for 600 ticks and bringing the items they carry to
//! the block. Their brain against vanilla is checked by kiln-entity's `mob_parity`.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Listener", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative Listener".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Self { sim, client, ground }
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    fn setblock(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn allays(&self) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == "minecraft:allay").map(|m| (m.0, m.2, m.3)).collect()
    }
}

#[test]
fn allays_bring_their_items_to_the_note_block_they_heard() {
    let mut w = World::new();
    let p = w.client.pos;
    // Holding cobblestone, two more in the inventory, and nobody it likes.
    w.run(&format!(
        "summon minecraft:allay {} {} {} {{equipment:{{mainhand:{{id:\"minecraft:cobblestone\",count:1}}}},Inventory:[{{id:\"minecraft:cobblestone\",count:2}}],PersistenceRequired:1b}}",
        p[0] + 2.0,
        p[1] + 1.0,
        p[2] - 2.0
    ));
    w.ticks(40);
    assert_eq!(w.allays().len(), 1);
    // Nothing to bring it anywhere yet: no item lies near the note block.
    let (block, power) = (w.at(9, 1, 0), w.at(10, 1, 0));
    w.setblock(block, "minecraft:note_block");
    w.ticks(2);
    let items_near = |w: &World| w.sim.entities().into_iter().filter(|e| e.0 == "minecraft:item").filter(|e| (e.1[0] - block[0] as f64 - 0.5).abs() < 4.0 && (e.1[2] - block[2] as f64 - 0.5).abs() < 4.0).count();
    assert_eq!(items_near(&w), 0);
    // Powering it plays the note; the vibration reaches the allay within its travel time.
    w.setblock(power, "minecraft:redstone_block");
    let mut thrown = None;
    for t in 0..400 {
        w.ticks(1);
        if items_near(&w) > 0 {
            thrown = Some(t);
            break;
        }
    }
    let t = thrown.expect("the allay brought an item to the note block");
    assert!(t > 5, "it had to fly there (tick {t})");
    let (_, pos, _) = w.allays()[0];
    let d = ((pos[0] - block[0] as f64 - 0.5).powi(2) + (pos[2] - block[2] as f64 - 0.5).powi(2)).sqrt();
    assert!(d < 6.0, "it hovers near the block ({d})");
}

#[test]
fn allays_ignore_note_blocks_beyond_earshot() {
    let mut w = World::new();
    let p = w.client.pos;
    w.run(&format!(
        "summon minecraft:allay {} {} {} {{equipment:{{mainhand:{{id:\"minecraft:cobblestone\",count:1}}}},Inventory:[{{id:\"minecraft:cobblestone\",count:2}}],PersistenceRequired:1b,NoGravity:0b}}",
        p[0] - 2.0,
        p[1] + 1.0,
        p[2]
    ));
    w.ticks(40);
    // 40 blocks away: past the 16 block radius.
    let (block, power) = (w.at(40, 1, 0), w.at(41, 1, 0));
    w.setblock(block, "minecraft:note_block");
    w.ticks(2);
    w.setblock(power, "minecraft:redstone_block");
    w.ticks(300);
    let items = w.sim.entities().into_iter().filter(|e| e.0 == "minecraft:item").count();
    assert_eq!(items, 0, "nothing was thrown");
}
