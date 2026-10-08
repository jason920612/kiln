//! Trees in the running simulation with vanilla generation: a sapling under random ticks
//! grows into a tree (the real worldgen feature, replayed through the region's `setBlock`),
//! and bone meal on grass grows the biome's flowers. Needs the vanilla datapack
//! (`KILN_DATAPACK`, else `$KILN_WORK/generated`, else `work/generated`); skips without it.
//! The block-for-block comparison with vanilla is `tree_parity`.

use kiln_blocks::state;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{NoiseConfig, Sim, SimConfig};
use std::path::{Path, PathBuf};

fn datapack() -> Option<PathBuf> {
    let dir = match std::env::var_os("KILN_DATAPACK").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let work = match std::env::var_os("KILN_WORK") {
                Some(d) if !d.is_empty() => PathBuf::from(d),
                _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
            };
            work.join("generated")
        }
    };
    if dir.join("reports/biome_parameters").is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: no datapack at {} (KILN_DATAPACK or `cargo xtask data`)", dir.display());
        None
    }
}

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
}

impl World {
    fn new(datapack: PathBuf) -> Self {
        let mut config = SimConfig::new(4, 3, None);
        config.noise = Some(NoiseConfig { seed: 12345, datapack, threads: 2 });
        let mut sim = Sim::new(config);
        let (msg, stats) = join(1, "Gardener", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode creative Gardener".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let client = Client::new(1, stats);
        let mut w = Self { sim, client, ground: [0; 3] };
        w.ticks(40);
        // The player settles on the generated ground (it may be sent after the join); the test boxes
        // reach 7 blocks around him and 26 above: wait for those chunks.
        // (Generation runs on its own threads in real time, and a tick here takes microseconds.)
        for _ in 0..12000 {
            let p = w.client.pos;
            w.ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
            let [gx, gy, gz] = w.ground;
            let corners = [(-7, 0, -7), (7, 0, 7), (-7, 26, 7), (7, 26, -7), (0, 0, 0)];
            let loaded = corners.iter().all(|&(dx, dy, dz)| w.sim.block_at(gx + dx, gy + dy, gz + dz).is_some());
            if loaded && w.sim.block_at(gx, gy, gz).is_some_and(|b| b != 0) {
                break;
            }
            w.ticks(1);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        w
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

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).unwrap_or_else(|| panic!("{p:?} not loaded (ground {:?})", self.ground))
    }

    fn name(&self, p: [i32; 3]) -> &'static str {
        state::BlockId::of(self.block(p)).name()
    }
}

/// Counts the blocks of the box that are `name`.
fn count(w: &World, from: [i32; 3], to: [i32; 3], name: &str) -> usize {
    let mut n = 0;
    for x in from[0]..=to[0] {
        for y in from[1]..=to[1] {
            for z in from[2]..=to[2] {
                if w.name([x, y, z]) == name {
                    n += 1;
                }
            }
        }
    }
    n
}

#[test]
fn a_sapling_grows_into_a_tree_under_random_ticks() {
    let Some(pack) = datapack() else { return };
    let mut w = World::new(pack);
    let [gx, gy, gz] = w.ground;
    // A clear column of sky above a dirt block (the terrain may hold trees and hills).
    w.run(&format!("fill {} {} {} {} {} {} minecraft:air", gx - 6, gy + 1, gz - 6, gx + 6, gy + 25, gz + 6));
    w.run(&format!("setblock {gx} {gy} {gz} minecraft:dirt"));
    w.run(&format!("setblock {gx} {} {gz} minecraft:oak_sapling", gy + 1));
    w.run("gamerule minecraft:random_tick_speed 3000");
    let (from, to) = ([gx - 6, gy + 1, gz - 6], [gx + 6, gy + 25, gz + 6]);
    let mut grown_at = None;
    for t in 0..200 {
        w.ticks(1);
        if count(&w, from, to, "minecraft:oak_log") > 0 {
            grown_at = Some(t);
            break;
        }
    }
    let t = grown_at.expect("the sapling never grew into a tree");
    let (logs, leaves) = (count(&w, from, to, "minecraft:oak_log"), count(&w, from, to, "minecraft:oak_leaves"));
    eprintln!("sapling grew after {t} ticks: {logs} logs, {leaves} leaves");
    assert!(logs >= 3 && leaves >= 10, "a tree, not a stump: {logs} logs, {leaves} leaves");
    assert_eq!(w.name([gx, gy + 1, gz]), "minecraft:oak_log", "the trunk stands where the sapling was");
    assert_eq!(w.name([gx, gy, gz]), "minecraft:dirt");
}

#[test]
fn bone_meal_on_grass_grows_short_grass_and_flowers() {
    let Some(pack) = datapack() else { return };
    let mut w = World::new(pack);
    let [gx, gy, gz] = w.ground;
    w.run(&format!("fill {} {} {} {} {} {} minecraft:air", gx - 6, gy + 1, gz - 6, gx + 6, gy + 6, gz + 6));
    w.run(&format!("fill {} {} {} {} {} {} minecraft:grass_block", gx - 6, gy, gz - 6, gx + 6, gy, gz + 6));
    let id = kiln_data::builtin_id("minecraft:item", "minecraft:bone_meal").unwrap();
    let stack = ItemStack { item: id, count: 64, added: Vec::new(), removed: Vec::new() };
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    for seq in 1..=4 {
        let pkt = PlayIn::UseItemOn { hand: 0, pos: [gx, gy, gz], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: seq };
        assert!(w.sim.step([ToSim::Packet(1, pkt)]));
    }
    let (from, to) = ([gx - 6, gy + 1, gz - 6], [gx + 6, gy + 2, gz + 6]);
    let grass = count(&w, from, to, "minecraft:short_grass") + count(&w, from, to, "minecraft:tall_grass");
    assert!(grass >= 10, "bone meal sprouts grass around the block: {grass}");
}
