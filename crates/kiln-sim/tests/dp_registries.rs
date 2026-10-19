//! wp83: a server running data packs has the registries and tags of the packs. The test pack
//! `tools/wp83/packs/kiln_wp83` defines entries of its own in many data-driven registries, overrides
//! a few vanilla ones and changes tags; these tests check the numbering of the registries (vanilla
//! sorts entries by path, then namespace), the `Registry Data` and `Update Tags` the clients get,
//! and that the game logic reads the tags of the packs (and re-reads them on `/reload`, while the
//! registries stay as the world loaded them).
//!
//! The registries and tags are process-wide: every test takes `live::exclusive()`.

mod support;

use kiln_link::ToSim;
use kiln_sim::testing::join;
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;
use std::sync::Arc;
use support::{pack_world, registry_data, tag_ids, update_tags};

fn start(tag: &str) -> (Sim, PathBuf, Arc<kiln_link::DataSync>) {
    let dir = pack_world(tag, &["kiln_wp83"]);
    let config = SimConfig::new(8, 2, Some(dir.clone()));
    let sync = config.data_sync.clone();
    (Sim::new(config), dir, sync)
}

/// The entries of a vanilla registry plus `extra`, in the order vanilla numbers them.
fn numbered(registry: &str, extra: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = kiln_data::registries::SYNCHRONIZED
        .iter()
        .find(|(r, _)| *r == registry)
        .map(|(_, e)| e.iter().map(|s| (*s).to_owned()).collect())
        .unwrap();
    names.extend(extra.iter().map(|s| (*s).to_owned()));
    let key = |s: &str| {
        let (ns, path) = s.split_once(':').unwrap();
        (path.to_owned(), ns.to_owned())
    };
    names.sort_by_key(|n| key(n));
    names
}

#[test]
fn entries_are_numbered_the_way_vanilla_sorts_them() {
    let _x = kiln_data::live::exclusive();
    let (sim, dir, _) = start("numbering");
    let paintings = numbered("minecraft:painting_variant", &["kiln83:speck", "kiln83:sunrise_wide"]);
    let live: Vec<&str> = kiln_data::live::synced_entries("minecraft:painting_variant").unwrap().to_vec();
    assert_eq!(live, paintings);
    // The two new paintings are sorted among the vanilla ones by path, and shift the ids after them.
    let speck = kiln_data::synced_id("minecraft:painting_variant", "kiln83:speck").unwrap();
    assert_eq!(live[speck as usize - 1], "minecraft:skull_and_roses");
    assert_eq!(live[speck as usize + 1], "minecraft:stage");
    let vanilla = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == "minecraft:painting_variant").unwrap().1;
    let vanilla_stage = vanilla.iter().position(|n| *n == "minecraft:stage").unwrap();
    assert_eq!(kiln_data::synced_id("minecraft:painting_variant", "minecraft:stage").unwrap() as usize, vanilla_stage + 1);
    assert_eq!(kiln_item::registry::PAINTING_VARIANT.id("kiln83:speck"), Some(speck));
    // Other registries too.
    for (registry, extra) in [
        ("minecraft:damage_type", &["kiln83:acid", "kiln83:mild"][..]),
        ("minecraft:wolf_variant", &["kiln83:azure"]),
        ("minecraft:jukebox_song", &["kiln83:tune"]),
        ("minecraft:instrument", &["kiln83:horn"]),
        ("minecraft:chat_type", &["kiln83:shout"]),
    ] {
        let live: Vec<&str> = kiln_data::live::synced_entries(registry).unwrap().to_vec();
        assert_eq!(live, numbered(registry, extra), "{registry}");
    }
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn clients_get_the_data_of_the_entries_they_cannot_take_from_a_pack_they_know() {
    let _x = kiln_data::live::exclusive();
    let (sim, dir, sync) = start("registry-data");
    let packets = sync.registry_packets().expect("a pack changes the registries");
    assert_eq!(packets.len(), kiln_data::registries::SYNCHRONIZED.len());
    let decoded: Vec<_> = packets.iter().map(registry_data).collect();
    for ((registry, entries), (want, _)) in decoded.iter().zip(kiln_data::registries::SYNCHRONIZED) {
        assert_eq!(registry, want, "registries are sent in vanilla's order");
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, kiln_data::live::synced_entries(registry).unwrap(), "{registry}: entry order is the numbering");
    }
    let get = |registry: &str| &decoded.iter().find(|(r, _)| r == registry).unwrap().1;
    let with_data = |registry: &str| -> Vec<&str> { get(registry).iter().filter(|(_, d)| d.is_some()).map(|(n, _)| n.as_str()).collect() };
    // New entries and the overridden vanilla one carry their data; the rest comes from `minecraft:core`.
    assert_eq!(with_data("minecraft:painting_variant"), ["minecraft:aztec", "kiln83:speck", "kiln83:sunrise_wide"]);
    assert_eq!(with_data("minecraft:damage_type"), ["kiln83:acid", "minecraft:cactus", "kiln83:mild"]);
    assert_eq!(with_data("minecraft:cow_sound_variant"), Vec::<&str>::new());
    assert_eq!(with_data("minecraft:enchantment"), Vec::<&str>::new());
    let entry = |registry: &str, name: &str| get(registry).iter().find(|(n, _)| n == name).unwrap().1.clone().unwrap();
    let sunrise = entry("minecraft:painting_variant", "kiln83:sunrise_wide");
    assert_eq!(sunrise.get("width").and_then(|t| t.as_i64()), Some(3));
    assert_eq!(sunrise.get("height").and_then(|t| t.as_i64()), Some(2));
    assert_eq!(sunrise.get("asset_id").and_then(|t| t.as_str()), Some("kiln83:sunrise_wide"));
    assert_eq!(entry("minecraft:painting_variant", "minecraft:aztec").get("width").and_then(|t| t.as_i64()), Some(2));
    // A variant's network form has no spawn conditions (`NETWORK_CODEC`); a damage type's has everything.
    let azure = entry("minecraft:wolf_variant", "kiln83:azure");
    assert!(azure.get("assets").is_some() && azure.get("baby_assets").is_some());
    assert!(azure.get("spawn_conditions").is_none());
    let acid = entry("minecraft:damage_type", "kiln83:acid");
    assert_eq!(acid.get("scaling").and_then(|t| t.as_str()), Some("always"));
    assert_eq!(acid.get("exhaustion").and_then(|t| t.as_f64()), Some(0.7));
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_login_of_a_server_without_packs_is_the_vanilla_one() {
    let _x = kiln_data::live::exclusive();
    let config = SimConfig::new(8, 2, None);
    let sync = config.data_sync.clone();
    let sim = Sim::new(config);
    assert!(sync.registry_packets().is_none(), "the built-in packets (names only) are sent");
    assert!(!kiln_data::live::is_custom());
    for (registry, names) in kiln_data::registries::SYNCHRONIZED {
        assert_eq!(kiln_data::live::synced_entries(registry).unwrap(), *names);
    }
    drop(sim);
}

#[test]
fn the_tags_of_the_packs_reach_the_game_logic_and_the_clients() {
    let _x = kiln_data::live::exclusive();
    let (sim, dir, sync) = start("tags");
    let stone = kiln_data::blocks::default_state::STONE;
    // Game logic: block tags (`BlockState.is`), the entity crate's flags, the inventory crate's tags.
    assert!(kiln_blocks::tags::is(stone, "minecraft:climbable"));
    assert!(kiln_entity::blocks::has_tag(stone, kiln_entity::blocks::Tag::Climbable));
    assert!(kiln_blocks::tags::is(stone, "minecraft:mineable/shovel"));
    assert!(kiln_blocks::tags::is(kiln_data::blocks::default_state::LADDER, "minecraft:climbable"), "the pack adds to the vanilla tag");
    let block = |name: &str| kiln_data::builtin_id("minecraft:block", name).unwrap();
    // A new tag: entries in order, nested tags expanded in place, optional entries skipped, a tag with a missing required entry dropped.
    assert_eq!(kiln_inventory::tags::entries("minecraft:block", "kiln83:pebbles"), None, "kiln83:pebbles names #kiln83:nope, which does not exist");
    let softer = kiln_inventory::tags::entries("minecraft:block", "kiln83:softer").unwrap();
    assert_eq!(softer, [block("minecraft:dirt"), block("minecraft:sand")]);
    assert_eq!(kiln_inventory::tags::entries("minecraft:block", "kiln83:broken"), None);
    // The login gets the same.
    let tags = update_tags(&sync.config_tags().expect("tags of the packs"));
    assert_eq!(tag_ids(&tags, "minecraft:block", "kiln83:softer"), Some(&softer[..]));
    assert!(tag_ids(&tags, "minecraft:block", "minecraft:climbable").unwrap().contains(&block("minecraft:stone")));
    assert_eq!(tag_ids(&tags, "minecraft:block", "kiln83:pebbles"), None);
    // The synchronized registries' tags use the numbering of the registries.
    let placeable = tag_ids(&tags, "minecraft:painting_variant", "minecraft:placeable").unwrap();
    let id = |name: &str| kiln_data::synced_id("minecraft:painting_variant", name).unwrap();
    assert_eq!(placeable, [id("kiln83:speck"), id("kiln83:sunrise_wide"), id("minecraft:aztec")], "`replace` dropped the vanilla entries; the file's order is kept");
    let acid = kiln_data::synced_id("minecraft:damage_type", "kiln83:acid").unwrap();
    assert!(tag_ids(&tags, "minecraft:damage_type", "minecraft:bypasses_armor").unwrap().contains(&acid));
    assert!(tag_ids(&tags, "minecraft:damage_type", "minecraft:is_fire").is_some(), "vanilla tags are still there");
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reload_rereads_the_tags_but_not_the_registries() {
    let _x = kiln_data::live::exclusive();
    let (mut sim, dir, sync) = start("reload");
    let (msg, stats) = join(1, "Reloader", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    assert!(sim.step([msg]));
    let stone = kiln_data::blocks::default_state::STONE;
    let dirt = kiln_data::blocks::default_state::DIRT;
    assert!(kiln_blocks::tags::is(stone, "minecraft:climbable") && !kiln_blocks::tags::is(dirt, "minecraft:climbable"));
    let before = kiln_data::live::synced_entries("minecraft:painting_variant").unwrap().to_vec();
    let pack = dir.join("datapacks/kiln_wp83/data");
    std::fs::write(pack.join("minecraft/tags/block/climbable.json"), r#"{"replace":true,"values":["minecraft:dirt"]}"#).unwrap();
    std::fs::create_dir_all(pack.join("kiln83/painting_variant")).unwrap();
    std::fs::write(pack.join("kiln83/painting_variant/late.json"), r#"{"asset_id":"kiln83:late","width":1,"height":1}"#).unwrap();
    stats.log.lock().unwrap().as_mut().unwrap().clear();
    assert!(sim.step([ToSim::Console("reload".into())]));
    assert!(!kiln_blocks::tags::is(stone, "minecraft:climbable"), "the new tag file replaced the pack's");
    assert!(kiln_blocks::tags::is(dirt, "minecraft:climbable"));
    assert!(!kiln_blocks::tags::is(kiln_data::blocks::default_state::LADDER, "minecraft:climbable"), "`replace` dropped the vanilla entries too");
    assert_eq!(kiln_data::live::synced_entries("minecraft:painting_variant").unwrap(), &before[..], "registries are loaded with the world");
    assert_eq!(kiln_data::synced_id("minecraft:painting_variant", "kiln83:late"), None);
    // The players got the tags again, with the numbering the world has; logins from now on get them too.
    let log = stats.log.lock().unwrap().clone().unwrap();
    let id = |p: &bytes::Bytes| kiln_proto::Reader::new(p).varint().ok();
    let packet = log.iter().find(|p| id(p) == Some(kiln_data::packets::play::clientbound::UPDATE_TAGS)).expect("update tags");
    let tags = update_tags(packet);
    let dirt_id = kiln_data::builtin_id("minecraft:block", "minecraft:dirt").unwrap();
    assert_eq!(tag_ids(&tags, "minecraft:block", "minecraft:climbable"), Some(&[dirt_id][..]));
    let config = update_tags(&sync.config_tags().unwrap());
    assert_eq!(tag_ids(&config, "minecraft:block", "minecraft:climbable"), tag_ids(&tags, "minecraft:block", "minecraft:climbable"));
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn animals_spawn_with_the_variants_of_the_pack() {
    use kiln_entity::mob::{self, GroupData, MobKind, SpawnContext};
    use kiln_javamath::random::LegacyRandom;
    let _x = kiln_data::live::exclusive();
    let (sim, dir, _) = start("variants");
    let ctx = |biome: &str| SpawnContext {
        special_multiplier: 0.0,
        effective_difficulty: 1.5,
        hard: false,
        halloween: false,
        biome: kiln_data::synced_id("minecraft:worldgen/biome", biome),
        moon_brightness: 0.0,
    };
    let variant_of = |kind: MobKind, biome: &str, seed: i64| {
        let mut e = mob::new(kind, 1, 0, 5);
        mob::finalize_spawn(&mut e, &mut LegacyRandom::new(seed), &ctx(biome), &mut GroupData::default(), true);
        mob::data(&e).unwrap().variant
    };
    // The pack's variants (priority 50 in a desert) beat vanilla's (priority 1 warm / 0 fallback) there and only there.
    for (kind, ours, plains) in [
        (MobKind::Wolf, "kiln83:azure", "minecraft:pale"),
        (MobKind::Cat, "kiln83:calico", "minecraft:tabby"),
        (MobKind::Pig, "kiln83:spotted", "minecraft:temperate"),
        (MobKind::Cow, "kiln83:dun", "minecraft:temperate"),
        (MobKind::Chicken, "kiln83:speckled", "minecraft:temperate"),
        (MobKind::Frog, "kiln83:dusk", "minecraft:temperate"),
        (MobKind::ZombieNautilus, "kiln83:murky", "minecraft:temperate"),
    ] {
        let registry = format!("{}_variant", kind.type_name());
        let id = |name: &str| kiln_data::synced_id(&registry, name).unwrap_or_else(|| panic!("{name} in {registry}"));
        assert_eq!(variant_of(kind, "minecraft:desert", 9), id(ours), "{kind:?} in a desert");
        if kind != MobKind::Cat {
            assert_eq!(variant_of(kind, "minecraft:plains", 9), id(plains), "{kind:?} in plains");
        } else {
            // Cats: ten equal candidates; any of the vanilla ones.
            assert_ne!(variant_of(kind, "minecraft:plains", 9), id(ours));
        }
    }
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn damage_types_of_the_pack_scale_and_name_the_death() {
    use kiln_sim::testing::Client;
    let _x = kiln_data::live::exclusive();
    let (mut sim, dir, _) = start("damage");
    let (msg, stats) = join(1, "Dmg", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    assert!(sim.step([msg, ToSim::Console("gamemode survival Dmg".into()), ToSim::Console("difficulty hard".into())]));
    let mut client = Client::new(1, stats.clone());
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let hit = |sim: &mut Sim, command: &str| {
        assert!(sim.step([ToSim::Console(command.into())]));
        sim.health(1).expect("player").0
    };
    // `scaling`: always scales with the difficulty (hard: x1.5), never does not.
    assert_eq!(hit(&mut sim, "damage Dmg 4 kiln83:acid"), 20.0 - 6.0);
    assert!(sim.step([ToSim::Console("effect give Dmg minecraft:instant_health 1 5".into())]));
    for _ in 0..25 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    assert_eq!(hit(&mut sim, "damage Dmg 4 kiln83:mild"), 20.0 - 4.0);
    // The message of the pack's `message_id`.
    stats.log.lock().unwrap().as_mut().unwrap().clear();
    assert!(sim.step([ToSim::Console("damage Dmg 100 kiln83:acid".into())]));
    let log = stats.log.lock().unwrap().clone().unwrap();
    let key = b"death.attack.kiln83Acid";
    assert!(log.iter().any(|p| p.windows(key.len()).any(|w| w == key)), "the death message names kiln83Acid");
    drop(sim);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_disc_of_a_song_of_the_pack_plays_for_its_length_and_the_comparator_reads_it() {
    use support::{Game, level_events};
    let _x = kiln_data::live::exclusive();
    let mut g = Game::new("jukebox", &["kiln_wp83"], "Dj");
    let [x, y, z] = g.spot(2);
    let song = kiln_data::synced_id("minecraft:jukebox_song", "kiln83:tune").unwrap();
    g.console(&format!("setblock {x} {y} {z} minecraft:jukebox"));
    g.console(&format!("setblock {} {y} {z} minecraft:comparator[facing=east]", x + 1));
    g.console(r#"give Dj minecraft:stick[minecraft:jukebox_playable="kiln83:tune"]"#);
    g.ticks(2);
    g.log();
    let pkt = kiln_link::PlayIn::UseItemOn { hand: 0, pos: [x, y, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 0 };
    assert!(g.sim.step([ToSim::Packet(1, pkt)]));
    let events = level_events(&g.log());
    assert!(events.contains(&(1010, song)), "the clients play the pack's song: {events:?} (song id {song})");
    g.ticks(10);
    // The comparator reads the song's `comparator_output` (7).
    let nbt = g.sim.block_entity_nbt(x + 1, y, z).expect("comparator");
    assert_eq!(nbt.get("OutputSignal").and_then(|t| t.as_i64()), Some(7), "{nbt:?}");
    // 12.5 seconds = 250 ticks and a second more.
    g.ticks(230);
    assert!(g.sim.block_entity_nbt(x, y, z).unwrap().get("ticks_since_song_started").is_some(), "still playing after 240 ticks");
    g.ticks(40);
    assert!(g.sim.block_entity_nbt(x, y, z).unwrap().get("ticks_since_song_started").is_none(), "over after 270 ticks");
    assert!(level_events(&g.log()).iter().any(|e| e.0 == 1011), "the clients are told to stop");
    g.cleanup();
}
