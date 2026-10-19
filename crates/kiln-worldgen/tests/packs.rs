//! wp85: worldgen from data packs against vanilla loading the same packs. `tools/wp85/pack_vectors85.py` runs vanilla's own chunk
//! generation (an in-process dedicated server with the packs in its world, the chunks driven through BIOMES, STRUCTURE_STARTS,
//! TERRAIN and FEATURES in Kiln's canonical decoration order; see `tools/wp85/make_pack_vectors.py`) and dumps the final blocks,
//! the biome cells and the structure starts of target chunks; here Kiln generates the same chunks from the same stack of pack
//! roots (the vanilla data, the packs in the order vanilla loads them) and the level stems the packs define, and compares them.
//!
//! Skipped without the vanilla data and the dumps (`$KILN_PACK_VECTORS`, default `<work>/wp85/vectors`).

use kiln_worldgen::generator::GenScratch;
use kiln_worldgen::{Datapack, Pipeline, Worldgen};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn work_dir() -> PathBuf {
    match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        s
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    fn str(&mut self) -> String {
        let n = self.i32() as usize;
        String::from_utf8(self.take(n).to_vec()).unwrap()
    }
    fn blob(&mut self) -> Vec<u16> {
        let n = self.i32() as usize;
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(self.take(n)).read_to_end(&mut out).unwrap();
        out.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }
}

struct Target {
    x: i32,
    z: i32,
    blocks: Vec<u16>,
    biomes: Vec<String>,
    /// Vanilla's structure starts: id and saved NBT, by id.
    starts: Vec<(String, kiln_proto::nbt::Tag)>,
}

struct Dump {
    seed: i64,
    structures: bool,
    targets: Vec<Target>,
}

fn read_dump(path: &Path) -> Dump {
    let bytes = std::fs::read(path).unwrap();
    let mut r = Reader { b: &bytes, i: 0 };
    assert_eq!(r.take(4), b"KWGP");
    assert_eq!(r.i32(), 1);
    let seed = r.i64();
    let structures = r.i32() != 0;
    assert_eq!(r.i32() as u32, kiln_data::blocks::STATE_COUNT, "block state count differs from kiln-data");
    let (_min_y, _height) = (r.i32(), r.i32());
    let mut targets = Vec::new();
    for _ in 0..r.i32() {
        let names: Vec<String> = (0..r.i32()).map(|_| r.str()).collect();
        for _ in 0..r.i32() {
            let (x, z) = (r.i32(), r.i32());
            let blocks = r.blob();
            let biomes = r.blob().into_iter().map(|i| names[i as usize].clone()).collect();
            let starts = (0..r.i32())
                .map(|_| {
                    let id = r.str();
                    let n = r.i32() as usize;
                    (id, canonical(kiln_proto::nbt::read_named(r.take(n)).expect("vanilla start NBT").1))
                })
                .collect();
            targets.push(Target { x, z, blocks, biomes, starts });
        }
    }
    Dump { seed, structures, targets }
}

/// A tag with the fields of every compound in name order (vanilla and Kiln write them in different orders).
fn canonical(t: kiln_proto::nbt::Tag) -> kiln_proto::nbt::Tag {
    use kiln_proto::nbt::Tag;
    match t {
        Tag::Compound(mut f) => {
            f.sort_by(|a, b| a.0.cmp(&b.0));
            Tag::Compound(f.into_iter().map(|(k, v)| (k, canonical(v))).collect())
        }
        Tag::List(l) => Tag::List(l.into_iter().map(canonical).collect()),
        t => t,
    }
}

fn state_name(s: u16) -> String {
    let b = kiln_data::blocks_types::block_of(s);
    let props: Vec<String> = b.properties.iter().zip(b.property_indices(s)).map(|(p, i)| format!("{}={}", p.name, p.values[i])).collect();
    if props.is_empty() { b.name.to_string() } else { format!("{}[{}]", b.name, props.join(",")) }
}

/// Generates the dump's chunks from the packs and compares.
fn check(name: &str, dimension: &str, packs: &[&str]) {
    let work = work_dir();
    let generated = work.join("generated");
    let vectors = std::env::var_os("KILN_PACK_VECTORS").map(PathBuf::from).unwrap_or_else(|| work.join("wp85/vectors"));
    let file = vectors.join(format!("pack_{name}.bin"));
    if !generated.join("reports/biome_parameters").is_dir() || !file.is_file() {
        eprintln!("skipped {name}: need {} and {} (python3 tools/wp85/pack_vectors85.py {name})", generated.display(), file.display());
        return;
    }
    let dump = read_dump(&file);
    let mut roots = vec![generated];
    let mut sorted: Vec<&str> = packs.to_vec();
    sorted.sort_unstable();
    roots.extend(sorted.iter().map(|p| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/wp85/packs").join(p)));
    let mut pack = Datapack::load_stack(&roots).expect("load the pack stack");
    let stems = kiln_worldgen::stem::stems_of(&pack);
    let (_, json) = stems.iter().find(|(k, _)| k == dimension).unwrap_or_else(|| panic!("no dimension {dimension}"));
    let stem = pack.parse_stem(dimension, json).expect("stem");
    let world = Worldgen::from_stem(&pack, &stem, dump.seed, dump.structures).expect("worldgen").expect("a noise generator");
    let pipeline = Pipeline::new(Arc::new(world));
    let mut gs = GenScratch::default();
    let (mut block_diffs, mut biome_diffs, mut start_diffs, mut bad_chunks) = (0usize, 0usize, 0usize, 0usize);
    let mut first = Vec::new();
    for t in &dump.targets {
        let p = pipeline.full(&mut gs, t.x, t.z);
        let generator = &pipeline.world().generator;
        let mut chunk_bad = false;
        assert_eq!(p.blocks.len(), t.blocks.len(), "{name}: the chunks' heights differ");
        let min_y = generator.min_y;
        for (i, (&ours, &theirs)) in p.blocks.iter().zip(&t.blocks).enumerate() {
            if ours != theirs {
                block_diffs += 1;
                chunk_bad = true;
                let (s, k) = (i >> 12, i & 4095);
                let line = format!(
                    "chunk {},{} block ({}, {}, {}): kiln {} vanilla {}",
                    t.x,
                    t.z,
                    t.x * 16 + (k & 15) as i32,
                    min_y + (s * 16 + (k >> 8)) as i32,
                    t.z * 16 + ((k >> 4) & 15) as i32,
                    state_name(ours),
                    state_name(theirs)
                );
                if std::env::var_os("KILN_PACK_ALL").is_some() {
                    eprintln!("  {line}");
                }
                if first.len() < 12 {
                    first.push(line);
                }
            }
        }
        if let Some(b) = std::env::var("KILN_PACK_BOX").ok().and_then(|v| v.split(',').map(|n| n.parse::<i32>().ok()).collect::<Option<Vec<_>>>()).filter(|v| v.len() == 6) {
            for y in b[1]..=b[4] {
                for z in b[2]..=b[5] {
                    for x in b[0]..=b[3] {
                        if (x >> 4, z >> 4) == (t.x, t.z) {
                            let i = (((y - min_y) as usize >> 4) << 12) | (((y - min_y) as usize & 15) << 8) | (((z & 15) as usize) << 4) | (x & 15) as usize;
                            eprintln!("  box ({x}, {y}, {z}): kiln {} vanilla {}", state_name(p.blocks[i]), state_name(t.blocks[i]));
                        }
                    }
                }
            }
        }
        for (i, theirs) in t.biomes.iter().enumerate() {
            let ours = &generator.biomes[p.biomes[i] as usize].name;
            if ours != theirs {
                biome_diffs += 1;
                chunk_bad = true;
                if first.len() < 12 {
                    first.push(format!("chunk {},{} biome cell {i}: kiln {ours} vanilla {theirs}", t.x, t.z));
                }
            }
        }
        let data = pipeline.structure_data(&mut gs, t.x, t.z);
        let mut ours: Vec<(String, kiln_proto::nbt::Tag)> = match data.get("starts") {
            Some(kiln_proto::nbt::Tag::Compound(starts)) => starts.iter().map(|(k, v)| (k.clone(), canonical(v.clone()))).collect(),
            _ => Vec::new(),
        };
        ours.sort_by(|a, b| a.0.cmp(&b.0));
        if ours != t.starts {
            start_diffs += 1;
            chunk_bad = true;
            {
                let ids = |l: &[(String, kiln_proto::nbt::Tag)]| l.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>();
                let detail = ours.iter().zip(&t.starts).find(|(a, b)| a != b).map(|(a, b)| format!("
    kiln    {:?}
    vanilla {:?}", a.1, b.1)).unwrap_or_default();
                first.insert(0, format!("chunk {},{} structure starts: kiln {:?} vanilla {:?}{detail}", t.x, t.z, ids(&ours), ids(&t.starts)));
            }
        }
        bad_chunks += usize::from(chunk_bad);
    }
    let structures: usize = dump.targets.iter().map(|t| t.starts.len()).sum();
    eprintln!(
        "{name} ({dimension}): {} chunks, {structures} structure starts in them; {bad_chunks} chunks differ: {block_diffs} blocks, {biome_diffs} biome cells, {start_diffs} structure start lists",
        dump.targets.len()
    );
    for line in &first {
        eprintln!("  {line}");
    }
    assert_eq!(bad_chunks, 0, "{name}: {bad_chunks} chunks differ from vanilla");
}

#[test]
fn a_replaced_biome_generates_as_in_vanilla() {
    check("desert", "minecraft:overworld", &["kiln85_desert"]);
}

#[test]
fn a_biome_added_to_the_overworlds_list_generates_as_in_vanilla() {
    check("crystal", "minecraft:overworld", &["kiln85_crystal"]);
}

#[test]
fn noise_settings_of_a_pack_generate_as_in_vanilla() {
    check("hills", "minecraft:overworld", &["kiln85_hills"]);
}

#[test]
fn a_structure_from_a_pack_template_generates_as_in_vanilla() {
    check("huts", "minecraft:overworld", &["kiln85_structure"]);
}

#[test]
fn a_dimension_of_a_pack_generates_as_in_vanilla() {
    check("dims_cavern", "kiln85:cavern", &["kiln85_dims"]);
}

/// The control: the same window as the structure test with no pack at all.
#[test]
fn without_packs_the_same_window_matches_vanilla() {
    check("plain", "minecraft:overworld", &[]);
}
