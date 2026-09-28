//! The native world format: records written from chunks match what Anvil stores, worlds
//! convert Anvil -> native -> Anvil without changing any chunk's NBT, and native worlds load
//! the same chunks as their Anvil originals (the reference worlds are used when present).

use kiln_data::blocks::default_state as block;
use kiln_proto::nbt::Tag;
use kiln_storage::native::chunk::NativeChunk;
use kiln_storage::native::convert::{compare_worlds, convert_world};
use kiln_storage::{AnvilSource, EntityStore, NativeSource, NativeStore, WorldFormat};
use kiln_world::{Blocks, ChunkPos, ChunkSource, OVERWORLD, Terrain, World};
use std::io::Write;
use std::path::{Path, PathBuf};

const OW: &str = "dimensions/minecraft/overworld";

fn tmp(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("native-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn work_dir() -> PathBuf {
    std::env::var_os("KILN_WORK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"))
}

fn write_level_dat(world: &Path) {
    let root = Tag::Compound(vec![("Data".into(), Tag::Compound(vec![("DataVersion".into(), Tag::Int(5023))]))]);
    let mut buf = bytes::BytesMut::new();
    root.write_named("", &mut buf);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&buf).unwrap();
    std::fs::create_dir_all(world).unwrap();
    std::fs::write(world.join("level.dat"), gz.finish().unwrap()).unwrap();
}

fn pig(n: i32) -> Tag {
    Tag::Compound(vec![("id".into(), Tag::String("minecraft:pig".into())), ("n".into(), Tag::Int(n))])
}

/// A small Anvil world written by Kiln: flat chunks over two regions with edits and light,
/// and some entity chunks.
fn kiln_world(dir: &Path) -> Vec<ChunkPos> {
    write_level_dat(dir);
    let mut w = World::with_source(OVERWORLD, Box::new(AnvilSource::new(dir.join(OW).join("region"))), Terrain::Flat, 0, 67);
    let mut chunks = Vec::new();
    for (i, x) in (-40..40).step_by(7).enumerate() {
        for z in [-3, 0, 20] {
            let pos = ChunkPos::new(x, z);
            chunks.push(pos);
            w.load_chunk(pos);
            let (bx, bz) = (x * 16 + (i as i32 % 16), z * 16 + 3);
            w.set_block(bx, 100 + i as i32, bz, block::STONE);
            w.set_block(bx, 101 + i as i32, bz, block::TORCH);
            w.set_block(bx + 1, -60, bz, block::GOLD_BLOCK);
        }
    }
    w.save().unwrap();
    let mut e = EntityStore::new(dir.join(OW).join("entities"));
    e.store(ChunkPos::new(-40, -3), vec![pig(1), pig(2)]);
    e.store(ChunkPos::new(30, 20), vec![pig(3)]);
    e.flush().unwrap();
    // Plugin cell data, also of a level without chunks.
    for (level, name) in [("overworld", "r.-2.0.bin"), ("the_nether", "r.0.-1.bin")] {
        let d = dir.join("kiln/plugins/cells/minecraft").join(level);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), format!("plugin data of {level}")).unwrap();
    }
    chunks
}

fn nbt_bytes(t: &Tag) -> Vec<u8> {
    let mut b = bytes::BytesMut::new();
    t.write_named("", &mut b);
    b.to_vec()
}

/// The two sources give the same chunks: blocks, biomes, light (the Chunk Data packet) and
/// what they would save.
fn assert_same_chunks(anvil: &mut AnvilSource, native: &mut NativeSource, positions: impl IntoIterator<Item = ChunkPos>) -> usize {
    let mut n = 0;
    for pos in positions {
        let (a, b) = (anvil.load(pos, OVERWORLD), native.load(pos, OVERWORLD));
        assert_eq!(a.is_some(), b.is_some(), "chunk {pos:?} present in one source only");
        let (Some(mut a), Some(mut b)) = (a, b) else { continue };
        assert_eq!(a.packet_body(67), b.packet_body(67), "chunk {pos:?} differs");
        let (ea, eb) = (kiln_storage::anvil::encode_chunk(pos, &a, None), kiln_storage::anvil::encode_chunk(pos, &b, None));
        assert!(ea == eb, "chunk {pos:?} saves differently");
        n += 1;
    }
    n
}

#[test]
fn native_records_store_what_anvil_would() {
    let dir = tmp("records");
    let positions = kiln_world(&dir);
    let mut src = AnvilSource::new(dir.join(OW).join("region"));
    for pos in positions {
        let chunk = src.load(pos, OVERWORLD).unwrap();
        let expected = nbt_bytes(&kiln_storage::anvil::encode_chunk(pos, &chunk, None));
        let record = NativeChunk::encode_chunk(pos, &chunk, None);
        let back = NativeChunk::decode(&record).expect("record decodes");
        assert_eq!(nbt_bytes(&back.to_nbt()), expected, "chunk {pos:?}");
        // And Anvil's NBT converts to a native record exactly.
        let codec = AnvilSource::new(PathBuf::new());
        assert!(NativeChunk::from_nbt(&expected, &codec).is_some(), "chunk {pos:?} kept as NBT");
    }
}

#[test]
fn kiln_worlds_round_trip_through_native() {
    let dir = tmp("kiln-rt");
    let positions = kiln_world(&dir.join("anvil"));
    let r = convert_world(&dir.join("anvil"), &dir.join("native"), WorldFormat::Native, 4).unwrap();
    assert_eq!((r.native_chunks, r.nbt_chunks, r.entity_chunks, r.plugin_sidecars, r.unreadable), (positions.len(), 0, 2, 2, 0), "{r}");
    assert_eq!(WorldFormat::of(&dir.join("native")), WorldFormat::Native);
    assert!(!dir.join("native").join(OW).join("region").exists());
    assert!(!dir.join("native/kiln/plugins/cells/minecraft/overworld/r.-2.0.bin").exists());
    let mut nether = NativeStore::open(dir.join("native/dimensions/minecraft/the_nether/native"));
    assert_eq!(nether.read_sidecar(0, -1).as_deref(), Some(&b"plugin data of the_nether"[..]));
    let r = convert_world(&dir.join("native"), &dir.join("back"), WorldFormat::Anvil, 4).unwrap();
    assert_eq!(r.native_chunks, positions.len());
    let (chunks, diffs) = compare_worlds(&dir.join("anvil"), &dir.join("back")).unwrap();
    assert!(diffs.is_empty(), "{diffs:#?}");
    assert_eq!(chunks, positions.len() + 2);
    assert!(dir.join("back/kiln/plugins/cells/minecraft/the_nether/r.0.-1.bin").exists());

    // The native world loads the same chunks and entities.
    let store = NativeStore::shared(dir.join("native").join(OW).join("native"));
    let mut native = NativeSource::new(store.clone());
    let mut anvil = AnvilSource::new(dir.join("anvil").join(OW).join("region"));
    assert_eq!(assert_same_chunks(&mut anvil, &mut native, positions.iter().copied()), positions.len());
    let mut entities = EntityStore::native(store);
    assert_eq!(entities.load(ChunkPos::new(-40, -3)), vec![pig(1), pig(2)]);
    assert_eq!(entities.load(ChunkPos::new(0, 0)), vec![]);
}

#[test]
fn native_worlds_save_and_reload() {
    let dir = tmp("save");
    let native_dir = dir.join(OW).join("native");
    let store = NativeStore::shared(&native_dir);
    let mut w = World::with_source(OVERWORLD, Box::new(NativeSource::new(store.clone())), Terrain::Flat, 0, 67);
    w.set_block(8, 100, 10, block::STONE);
    w.set_block(8, 101, 10, block::TORCH);
    w.set_block(1000, 100, -1000, block::DIAMOND_BLOCK);
    let light = w.light_at(kiln_world::chunk::LightLayer::Block, 9, 101, 10).unwrap();
    assert!(w.save().unwrap() >= 2);
    let mut entities = EntityStore::native(store);
    entities.store(ChunkPos::new(0, 0), vec![pig(7)]);
    entities.flush().unwrap();
    drop((w, entities));

    let store = NativeStore::shared(&native_dir);
    let mut w = World::with_source(OVERWORLD, Box::new(NativeSource::new(store.clone())), Terrain::Void, 0, 67);
    w.load_chunk(ChunkPos::of_block(8, 10));
    w.load_chunk(ChunkPos::of_block(1000, -1000));
    assert_eq!(w.get_block(8, 100, 10), Some(block::STONE));
    assert_eq!(w.get_block(8, 101, 10), Some(block::TORCH));
    assert_eq!(w.get_block(1000, 100, -1000), Some(block::DIAMOND_BLOCK));
    assert_eq!(w.light_at(kiln_world::chunk::LightLayer::Block, 9, 101, 10), Some(light));
    let mut entities = EntityStore::native(store.clone());
    assert_eq!(entities.load(ChunkPos::new(0, 0)), vec![pig(7)]);
    // Deleting entities, saving again: one cell file gets appended to and stays readable.
    entities.store(ChunkPos::new(0, 0), Vec::new());
    entities.flush().unwrap();
    w.set_block(8, 102, 10, block::GOLD_BLOCK);
    w.save().unwrap();
    drop((w, entities));
    let store = NativeStore::shared(&native_dir);
    let mut w = World::with_source(OVERWORLD, Box::new(NativeSource::new(store.clone())), Terrain::Void, 0, 67);
    w.load_chunk(ChunkPos::of_block(8, 10));
    assert_eq!(w.get_block(8, 102, 10), Some(block::GOLD_BLOCK));
    assert!(EntityStore::native(store).load(ChunkPos::new(0, 0)).is_empty());
}

#[test]
fn new_worlds_pick_their_format() {
    let dir = tmp("format");
    assert_eq!(WorldFormat::resolve(&dir, WorldFormat::Anvil), WorldFormat::Anvil);
    assert_eq!(WorldFormat::resolve(&dir, WorldFormat::Native), WorldFormat::Native);
    // Marked native, it stays native.
    assert_eq!(WorldFormat::resolve(&dir, WorldFormat::Anvil), WorldFormat::Native);
    // An existing Anvil world is not switched.
    let anvil = tmp("format-anvil");
    write_level_dat(&anvil);
    assert_eq!(WorldFormat::resolve(&anvil, WorldFormat::Native), WorldFormat::Anvil);
}

/// The vanilla reference world (tools/gen_vanilla_world.py) through native and back.
#[test]
fn vanilla_world_round_trips_through_native() {
    let src = work_dir().join("vanilla-world/world");
    if !src.join("level.dat").exists() {
        eprintln!("no reference world; run tools/gen_vanilla_world.py");
        return;
    }
    let dir = tmp("vanilla-rt");
    let to = convert_world(&src, &dir.join("native"), WorldFormat::Native, 4).unwrap();
    eprintln!("to native: {to}");
    assert_eq!(to.unreadable, 0);
    let back = convert_world(&dir.join("native"), &dir.join("back"), WorldFormat::Anvil, 4).unwrap();
    eprintln!("back to Anvil: {back}");
    let (chunks, diffs) = compare_worlds(&src, &dir.join("back")).unwrap();
    assert!(diffs.is_empty(), "{diffs:#?}");
    assert!(chunks > 500, "{chunks} chunks compared");
    // Nearly every fully generated vanilla chunk takes the native layout.
    assert!(to.nbt_chunks * 100 <= to.native_chunks, "{to}");

    let mut native = NativeSource::new(NativeStore::shared(dir.join("native").join(OW).join("native")));
    let mut anvil = AnvilSource::new(src.join(OW).join("region"));
    let positions = (-16..16).flat_map(|x| (-16..16).map(move |z| ChunkPos::new(x, z)));
    assert!(assert_same_chunks(&mut anvil, &mut native, positions) >= 24 * 24);
}

#[test]
fn a_corrupt_cell_file_is_moved_aside_not_overwritten() {
    use kiln_storage::native::{ENTITIES, FORM_NBT};
    let dir = tmp("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("c.0.0.kcell"), b"not a cell file at all").unwrap();
    let mut store = NativeStore::open(&dir);
    assert!(store.read(ENTITIES, ChunkPos::new(1, 1)).is_none());
    store.write(ENTITIES, ChunkPos::new(1, 1), FORM_NBT, Some(b"x"));
    store.flush().unwrap();
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    let aside = names.iter().find(|n| n.starts_with("c.0.0.kcell.corrupt-")).expect("moved aside");
    assert_eq!(std::fs::read(dir.join(aside)).unwrap(), b"not a cell file at all");
    let mut store = NativeStore::open(&dir);
    assert_eq!(store.read(ENTITIES, ChunkPos::new(1, 1)).map(|r| r.1), Some(b"x".to_vec()));
}
