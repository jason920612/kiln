//! wp85: the hoe, the axe and the shovel do what vanilla's own `block_transformer` files say. Kiln's built-in rules (`tools.rs`) stood in for
//! them; now the files are read, and this test checks that the two agree: every block, every tool and the two faces that matter (up, and
//! the side), the block that results and the items that pop out. Skipped without the vanilla data (`KILN_DATAPACK`).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn vanilla() -> bool {
    std::env::var_os("KILN_DATAPACK").is_some_and(|d| std::path::Path::new(&d).join("data/minecraft/block_transformer/hoe.json").is_file())
}

struct World {
    sim: Sim,
    client: Client,
    sequence: i32,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative User".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut w = World { sim, client: Client::new(1, stats), sequence: 0 };
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn hold(&mut self, item: &str) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    fn click(&mut self, pos: [i32; 3], face: i32) {
        self.sequence += 1;
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face, cursor: [0.5, 0.5, 0.5], inside: false, sequence: self.sequence };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }
}

/// What the three tools do to every block, on two faces: `(tool, block, face) -> resulting state and the number of item entities`.
fn outcomes() -> Vec<String> {
    let mut w = World::new();
    let p = w.client.pos;
    let at = [p[0].floor() as i32 + 3, p[1].floor() as i32, p[2].floor() as i32];
    let mut out = Vec::new();
    for tool in ["minecraft:iron_hoe", "minecraft:iron_axe", "minecraft:iron_shovel"] {
        w.hold(tool);
        for block in kiln_data::blocks::BLOCKS.iter().filter(|b| !b.name.contains("command") && !b.name.contains("jigsaw") && !b.name.contains("structure")) {
            for state in block.first..=block.last {
                let name = kiln_blocks::state::state_string(state);
                for face in [1, 2] {
                    w.sim.console_now("kill @e[type=minecraft:item]");
                    w.sim.console_now(&format!("fill {} {} {} {} {} {} minecraft:air", at[0] - 1, at[1] - 1, at[2] - 1, at[0] + 1, at[1] + 2, at[2] + 1));
                    // The block with a solid floor under it (so it stays) and air above.
                    w.sim.console_now(&format!("setblock {} {} {} minecraft:stone", at[0], at[1] - 1, at[2]));
                    w.sim.console_now(&format!("setblock {} {} {} {name}", at[0], at[1], at[2]));
                    let before = w.sim.block_at(at[0], at[1], at[2]);
                    w.click(at, face);
                    let after = w.sim.block_at(at[0], at[1], at[2]);
                    if before != after {
                        let items = w.sim.entities().iter().filter(|(k, _)| *k == "minecraft:item").count();
                        out.push(format!(
                            "{tool} {name} face {face}: {} -> {}, {items} items",
                            before.map_or("?".to_owned(), kiln_blocks::state::state_string),
                            after.map_or("?".to_owned(), kiln_blocks::state::state_string)
                        ));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn vanillas_block_transformer_files_do_what_the_built_in_rules_do() {
    if !vanilla() {
        eprintln!("skipped: no vanilla data (KILN_DATAPACK)");
        return;
    }
    kiln_sim::testing::use_builtin_block_transformers(false);
    let from_files = outcomes();
    kiln_sim::testing::use_builtin_block_transformers(true);
    let built_in = outcomes();
    kiln_sim::testing::use_builtin_block_transformers(false);
    eprintln!("{} blocks changed by a tool", from_files.len());
    assert!(from_files.len() > 100, "the tools change many blocks: {}", from_files.len());
    let diff: Vec<_> = from_files.iter().zip(&built_in).filter(|(a, b)| a != b).take(10).collect();
    assert!(diff.is_empty() && from_files.len() == built_in.len(), "files {} built-in {}: {diff:#?}", from_files.len(), built_in.len());
}
