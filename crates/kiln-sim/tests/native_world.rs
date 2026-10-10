//! A world in Kiln's native format (design §9.4) through the simulation: blocks, entities and
//! plugin cell data go into the dimension's cell files (no region files, no plugin sidecar
//! files), come back after a restart, and convert to an Anvil world with its sidecars.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use kiln_storage::WorldFormat;
use std::path::{Path, PathBuf};

const OW: &str = "dimensions/minecraft/overworld";

fn settle(sim: &mut Sim, client: &mut Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

fn item_count(sim: &Sim) -> usize {
    sim.entity_nbt().iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item")).count()
}

fn config(dir: &Path) -> SimConfig {
    let mut c = SimConfig::new(8, 4, Some(dir.to_owned()));
    let mut plugins = kiln_sim::PluginSettings::new(kiln_plugin_host::examples::custom_dir("native-world", &["chat-format", "counter", "spawn-protection"], &[]).expect("example plugins"));
    // The default budget (500 us a call) is for a running server: the first call into a plugin that was just compiled can take longer on a loaded
    // machine, which fails closed (a timeout denies and records nothing), and the claim this test looks for would never be written.
    plugins.call_budget = std::time::Duration::from_millis(200);
    c.plugins = Some(plugins);
    c
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_type().unwrap().is_dir() {
            files(&e.path(), out);
        } else {
            out.push(e.path());
        }
    }
}

#[test]
fn native_world_keeps_blocks_entities_and_plugin_cell_data() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native-world");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = config(&dir);
    c.world_format = WorldFormat::Native;
    let mut sim = Sim::new(c);
    let (msg, stats) = join(1, "Builder", 2);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Builder".into())]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    let [x, y, z] = client.pos.map(|c| c.floor() as i32);
    assert!(sim.step([ToSim::Console(format!("fill {} {} {} {} {} {} stone", x - 6, y - 1, z - 6, x + 6, y - 1, z + 6))]));
    let diamond = kiln_data::builtin_id("minecraft:item", "minecraft:diamond").unwrap();
    assert!(sim.step([
        ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(ItemStack { item: diamond, count: 5, added: Vec::new(), removed: Vec::new() }) }),
        ToSim::Packet(1, PlayIn::PlayerAction { action: 4, pos: [0, 0, 0], face: 0, sequence: 0 }),
    ]));
    settle(&mut sim, &mut client, 30);
    assert_eq!(item_count(&sim), 1);
    // Breaking the floor at spawn: spawn protection refuses and records its claim and the
    // denial in the cell.
    let floor = [x + 2, y - 1, z];
    let stone = sim.block_at(floor[0], floor[1], floor[2]).unwrap();
    assert!(sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 0, pos: floor, face: 1, sequence: 1 })]));
    settle(&mut sim, &mut client, 2);
    assert_eq!(sim.block_at(floor[0], floor[1], floor[2]), Some(stone), "protected");
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!sim.step([ToSim::Shutdown { done }]));
    drop(sim);

    assert_eq!(WorldFormat::of(&dir), WorldFormat::Native);
    let mut all = Vec::new();
    files(&dir, &mut all);
    assert!(all.iter().any(|f| f.extension().is_some_and(|e| e == "kcell")), "cell files written");
    assert!(!all.iter().any(|f| f.extension().is_some_and(|e| e == "mca")), "no region files: {all:?}");
    assert!(!all.iter().any(|f| f.starts_with(dir.join("kiln/plugins/cells"))), "no sidecar files: {all:?}");
    let mut store = kiln_storage::NativeStore::open(dir.join(OW).join("native"));
    assert!(store.read_sidecar(x.div_euclid(512), z.div_euclid(512)).is_some(), "plugin cell data in the cell file");
    drop(store);

    // After a restart (a native world stays native), everything is back.
    let mut sim = Sim::new(config(&dir));
    let (msg, stats) = join(1, "Builder", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    settle(&mut sim, &mut client, 5);
    assert_eq!(sim.block_at(floor[0], floor[1], floor[2]), Some(stone));
    assert_eq!(item_count(&sim), 1, "the dropped diamonds came back");
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!sim.step([ToSim::Shutdown { done }]));
    drop(sim);

    // As an Anvil world: region files, entity chunks and the plugin sidecar.
    let anvil = dir.with_file_name("native-world-anvil");
    let _ = std::fs::remove_dir_all(&anvil);
    let r = kiln_storage::native::convert::convert_world(&dir, &anvil, WorldFormat::Anvil, 2).unwrap();
    assert!(r.native_chunks > 0 && r.entity_chunks == 1 && r.plugin_sidecars == 1, "{r}");
    let sidecar = anvil.join(format!("kiln/plugins/cells/minecraft/overworld/r.{}.{}.bin", x.div_euclid(512), z.div_euclid(512)));
    assert!(sidecar.exists());
    let mut src = kiln_storage::AnvilSource::new(anvil.join(OW).join("region"));
    let chunk = kiln_world::ChunkSource::load(&mut src, kiln_world::ChunkPos::of_block(floor[0], floor[2]), kiln_world::OVERWORLD).unwrap();
    assert_eq!(chunk.get((floor[0] & 15) as usize, floor[1], (floor[2] & 15) as usize), stone);
}
