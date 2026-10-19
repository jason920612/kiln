//! wp85: dimension types and dimensions from data packs. The test packs `tools/wp85/packs/kiln85_dims` (a dimension type of
//! its own, a superflat dimension and a nether-noise dimension) and `kiln85_nether` (a replaced nether type) are put in a
//! world's `datapacks`; the levels the server runs, the dimension types the simulation reads, what the clients are told and
//! what the levels generate follow them. (Generation against vanilla is `kiln-worldgen`'s `packs` test and `wp85_vanilla`.)
//!
//! The levels and registries are process-wide: every test takes `live::exclusive()`.

mod support;

use bytes::Bytes;
use kiln_link::ToSim;
use kiln_proto::Reader;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;
use support::wp85_world;

const FLAT: &str = "kiln85:flatland";
const CAVERN: &str = "kiln85:cavern";

struct World {
    sim: Sim,
    client: Client,
    dir: PathBuf,
    sync: std::sync::Arc<kiln_link::DataSync>,
}

impl World {
    fn new(tag: &str, packs: &[&str]) -> World {
        let dir = wp85_world(tag, packs);
        World::in_dir(dir)
    }

    fn in_dir(dir: PathBuf) -> World {
        let config = SimConfig::new(4, 4, Some(dir.clone()));
        let sync = config.data_sync.clone();
        let mut sim = Sim::new(config);
        let (msg, stats) = join(1, "Traveller", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg, ToSim::Console("gamemode creative Traveller".into())]));
        let mut w = World { sim, client: Client::new(1, stats), dir, sync };
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

    fn run(&mut self, command: &str) {
        self.sim.console_now(command);
    }

    fn log(&mut self) -> Vec<Bytes> {
        std::mem::take(self.client.stats.log.lock().unwrap().as_mut().unwrap())
    }

    fn level(&self) -> (&'static str, [f64; 3]) {
        self.sim.player_level(1).expect("player")
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `(dimension keys, dimension type id, dimension)` of a Login packet.
fn login(log: &[Bytes]) -> Option<(Vec<String>, i32, String)> {
    for p in log {
        let mut r = Reader::new(p);
        if r.varint().ok()? != kiln_data::packets::play::clientbound::LOGIN {
            continue;
        }
        let _entity = r.i32().ok()?;
        let _hardcore = r.bool().ok()?;
        let dimensions = (0..r.varint().ok()?).map(|_| r.string(32767).map(str::to_owned)).collect::<Result<Vec<_>, _>>().ok()?;
        let (_max, _view, _sim, _reduced, _death, _limited) = (r.varint().ok()?, r.varint().ok()?, r.varint().ok()?, r.bool().ok()?, r.bool().ok()?, r.bool().ok()?);
        let dimension_type = r.varint().ok()?;
        let dimension = r.string(32767).ok()?.to_owned();
        return Some((dimensions, dimension_type, dimension));
    }
    None
}

/// `(dimension type id, dimension)` of the last Respawn packet.
fn respawn(log: &[Bytes]) -> Option<(i32, String)> {
    log.iter().rev().find_map(|p| {
        let mut r = Reader::new(p);
        (r.varint().ok()? == kiln_data::packets::play::clientbound::RESPAWN).then(|| Some((r.varint().ok()?, r.string(32767).ok()?.to_owned())))?
    })
}

#[test]
fn the_levels_are_the_three_vanilla_ones_and_the_packs() {
    let _x = kiln_data::live::exclusive();
    let w = World::new("levels", &["kiln85_dims"]);
    let levels: Vec<(&str, &str)> = kiln_data::levels::levels().iter().map(|l| (l.key, l.dimension_type)).collect();
    assert_eq!(
        levels,
        [
            ("minecraft:overworld", "minecraft:overworld"),
            ("minecraft:the_nether", "minecraft:the_nether"),
            ("minecraft:the_end", "minecraft:the_end"),
            (CAVERN, "kiln85:cavern_type"),
            (FLAT, "kiln85:flat_type"),
        ]
    );
    assert_eq!(w.sim.loaded_chunks().len(), 5, "a level per dimension");
    // The pack's superflat dimension fills chunks with its flat biome.
    assert_eq!(kiln_data::levels::levels()[4].biome, "minecraft:plains");
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn the_simulation_reads_the_dimension_types_of_the_packs() {
    let _x = kiln_data::live::exclusive();
    let w = World::new("types", &["kiln85_dims"]);
    let flat = kiln_data::dimension_type("kiln85:flat_type").expect("the type of the pack");
    assert_eq!((flat.min_y, flat.height, flat.logical_height), (-16, 64, 64));
    assert_eq!(flat.coordinate_scale, 2.0);
    assert!(flat.has_skylight && !flat.has_ceiling && !flat.has_fixed_time);
    assert_eq!(flat.default_clock, Some("minecraft:overworld"));
    assert!(flat.respawn_anchor_works);
    assert_eq!((flat.bed_can_sleep, flat.bed_can_set_spawn), (kiln_data::BedWhen::Always, kiln_data::BedWhen::Always));
    assert!(flat.has_timeline("minecraft:day") && flat.piglins_zombify && flat.can_start_raid);
    let cavern = kiln_data::dimension_type("kiln85:cavern_type").unwrap();
    assert_eq!((cavern.min_y, cavern.height, cavern.logical_height), (0, 256, 128));
    assert!(cavern.has_ceiling && cavern.has_fixed_time && !cavern.has_skylight);
    assert!(cavern.fast_lava && cavern.water_evaporates && cavern.respawn_anchor_works);
    assert!(cavern.bed_explodes && !cavern.bed_sets_spawn);
    assert!(!cavern.piglins_zombify && !cavern.can_start_raid);
    assert_eq!(cavern.default_clock, None);
    assert!(!cavern.has_timeline("minecraft:day") && cavern.has_timeline("minecraft:villager_schedule"));
    // The vanilla types are still there.
    assert_eq!(kiln_data::dimension_type("minecraft:overworld").unwrap().min_y, -64);
    // And they are registry entries the clients are sent.
    assert!(kiln_data::synced_id("minecraft:dimension_type", "kiln85:flat_type").is_some());
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn clients_are_told_the_dimensions_and_their_types() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("clients", &["kiln85_dims"]);
    // The registry packets: dimension types in vanilla's order (path first), the pack's with their data.
    let packets = w.sync.registry_packets().expect("registries with packs");
    let (_, entries) = packets
        .iter()
        .map(support::registry_data)
        .find(|(r, _)| r == "minecraft:dimension_type")
        .expect("the dimension type registry");
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["kiln85:cavern_type", "kiln85:flat_type", "minecraft:overworld", "minecraft:overworld_caves", "minecraft:the_end", "minecraft:the_nether"]);
    let data = entries.iter().find(|(n, _)| n == "kiln85:flat_type").unwrap().1.as_ref().expect("a pack's entry carries its data");
    assert_eq!(data.get("min_y").and_then(|t| t.as_i64()), Some(-16));
    assert_eq!(data.get("height").and_then(|t| t.as_i64()), Some(64));
    assert!(entries.iter().find(|(n, _)| n == "minecraft:overworld").unwrap().1.is_none(), "the client has the vanilla ones");
    // The Login packet lists every dimension and the player's type is the overworld's id in that registry.
    let log = w.log();
    let (dimensions, dimension_type, dimension) = login(&log).expect("a Login packet");
    assert_eq!(dimensions, ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end", CAVERN, FLAT]);
    assert_eq!(dimension, "minecraft:overworld");
    assert_eq!(dimension_type, kiln_data::synced_id("minecraft:dimension_type", "minecraft:overworld").unwrap());
    // Going to the pack's dimension sends its type's id.
    w.run(&format!("execute in {FLAT} run tp Traveller 8 -9 8"));
    w.ticks(3);
    assert_eq!(w.level().0, FLAT);
    let log = w.log();
    let (type_id, key) = respawn(&log).expect("a Respawn packet");
    assert_eq!(key, FLAT);
    assert_eq!(type_id, kiln_data::synced_id("minecraft:dimension_type", "kiln85:flat_type").unwrap());
    // And back: the type of the overworld again.
    w.run("execute in minecraft:overworld run tp Traveller 8 -60 8");
    w.ticks(3);
    assert_eq!(w.level().0, "minecraft:overworld");
    let (type_id, key) = respawn(&w.log()).expect("a Respawn packet");
    assert_eq!((type_id, key.as_str()), (kiln_data::synced_id("minecraft:dimension_type", "minecraft:overworld").unwrap(), "minecraft:overworld"));
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn a_superflat_dimension_lays_its_layers_in_its_own_height() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("flat", &["kiln85_dims"]);
    w.run(&format!("execute in {FLAT} run forceload add 0 0 15 15"));
    let at = |w: &World, y| w.sim.block_in(FLAT, 8, y, 8);
    use kiln_data::blocks::default_state as d;
    // From the type's min_y -16: bedrock, two stone, one water (a layer vanilla fills in after the others, as it is not opaque), two dirt, grass,
    // then air up to the 64 blocks of height.
    assert_eq!(at(&w, -17), Some(d::VOID_AIR), "below the level's height");
    assert_eq!(at(&w, -16), Some(d::BEDROCK));
    assert_eq!(at(&w, -15), Some(d::STONE));
    assert_eq!(at(&w, -14), Some(d::STONE));
    assert_eq!(at(&w, -13), Some(d::WATER));
    assert_eq!(at(&w, -12), Some(d::DIRT));
    assert_eq!(at(&w, -11), Some(d::DIRT));
    assert_eq!(at(&w, -10), Some(d::GRASS_BLOCK));
    assert_eq!(at(&w, -9), Some(d::AIR));
    assert_eq!(at(&w, 47), Some(d::AIR));
    assert_eq!(at(&w, 48), Some(d::VOID_AIR), "above the level's height");
    assert_eq!(w.sim.biome_in(FLAT, 8, -16, 8), kiln_data::synced_id("minecraft:worldgen/biome", "minecraft:plains").map(|b| b as u16));
    // The overworld is unchanged: its default terrain here, at its own height.
    w.run("forceload add 0 0 15 15");
    assert_eq!(w.sim.block_at(8, -64, 8).is_some(), true);
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn a_pack_can_replace_the_geometry_of_a_vanilla_dimension_type() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("nether", &["kiln85_nether"]);
    let nether = kiln_data::dimension_type("minecraft:the_nether").unwrap();
    assert_eq!((nether.min_y, nether.height, nether.logical_height, nether.coordinate_scale), (16, 192, 128, 4.0));
    assert!(nether.bed_can_sleep == kiln_data::BedWhen::Always && !nether.respawn_anchor_works && !nether.water_evaporates);
    // The nether's chunks have the new height: loaded from y 16 to 207.
    w.run("execute in minecraft:the_nether run forceload add 0 0 15 15");
    use kiln_data::blocks::default_state as d;
    let at = |y| w.sim.block_in("minecraft:the_nether", 8, y, 8);
    assert_eq!((at(15), at(16), at(207), at(208)), (Some(d::VOID_AIR), Some(d::AIR), Some(d::AIR), Some(d::VOID_AIR)));
    // The registry entry replaces vanilla's: the clients are sent its data.
    let packets = w.sync.registry_packets().unwrap();
    let (_, entries) = packets.iter().map(support::registry_data).find(|(r, _)| r == "minecraft:dimension_type").unwrap();
    let data = entries.iter().find(|(n, _)| n == "minecraft:the_nether").unwrap().1.as_ref().expect("an overridden entry carries its data");
    assert_eq!(data.get("min_y").and_then(|t| t.as_i64()), Some(16));
    drop(w);
    kiln_data::levels::reset();
}

#[test]
fn blocks_of_a_pack_dimension_are_saved_in_its_own_directory() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("saved", &["kiln85_dims"]);
    w.run(&format!("execute in {FLAT} run forceload add 0 0 15 15"));
    w.run(&format!("execute in {FLAT} run setblock 3 -5 3 minecraft:diamond_block"));
    assert_eq!(w.sim.block_in(FLAT, 3, -5, 3), Some(kiln_data::blocks::default_state::DIAMOND_BLOCK));
    let dir = w.dir.clone();
    // A normal shutdown saves the loaded chunks.
    w.shutdown();
    drop(w);
    assert!(dir.join("dimensions/kiln85/flatland/region").is_dir(), "the dimension's own save directory");
    let mut again = World::in_dir(dir.clone());
    again.run(&format!("execute in {FLAT} run forceload add 0 0 15 15"));
    assert_eq!(again.sim.block_in(FLAT, 3, -5, 3), Some(kiln_data::blocks::default_state::DIAMOND_BLOCK), "after a restart");
    drop(again);
    kiln_data::levels::reset();
}

#[test]
fn weather_follows_the_dimension_type() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new("weather", &["kiln85_dims"]);
    w.run("weather rain 100000");
    w.ticks(200);
    // The superflat dimension has sky light and no ceiling, like the overworld; the nether-like one has a ceiling and no sky light.
    assert!(w.sim.is_raining_in("minecraft:overworld"));
    assert!(w.sim.is_raining_in(FLAT), "a level with sky light and no ceiling has the weather");
    assert!(!w.sim.is_raining_in(CAVERN), "a level with a ceiling has none");
    assert!(!w.sim.is_raining_in("minecraft:the_nether") && !w.sim.is_raining_in("minecraft:the_end"));
    drop(w);
    kiln_data::levels::reset();
}

impl World {
    fn shutdown(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(!self.sim.step([ToSim::Shutdown { done: tx }]));
        rx.recv().unwrap();
        // The directory stays for the next start: this world no longer deletes it.
        self.dir = PathBuf::new();
    }
}
