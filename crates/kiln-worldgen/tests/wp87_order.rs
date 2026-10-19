//! wp87: the overworld of a seed against vanilla in the decoration order Kiln takes (`tools/wp87/order87.py` runs vanilla's own generation
//! with a chosen order and dumps the final blocks and structure starts of target windows). wp84 saw, on two live servers with seed 84,
//! a dungeon (a monster room) right under a desert pyramid and six sand blocks in a warm ocean ruin that only Kiln had; vanilla's
//! decoration order is decided by thread timing, Kiln's is fixed, so first the harness shows what the same chunks are in Kiln's order
//! (this test: no difference) and then in other orders vanilla can take (`tools/wp87/order87.py`, see docs/wp87.md).
//!
//! Skipped without the vanilla data and the dump (`$KILN_WP87_DUMP`, default `<work>/wp87/vectors/pack_seed84_canonical.bin`).

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

#[allow(dead_code)]
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

/// The first differing piece (or field) of two saved starts.
fn first_difference(kiln: &kiln_proto::nbt::Tag, vanilla: &kiln_proto::nbt::Tag) -> String {
    let children = |t: &kiln_proto::nbt::Tag| t.get("Children").and_then(|c| c.as_list()).map(|l| l.to_vec()).unwrap_or_default();
    let (a, b) = (children(kiln), children(vanilla));
    for i in 0..a.len().max(b.len()) {
        if a.get(i) != b.get(i) {
            return format!("{} kiln / {} vanilla pieces; piece {i}
    kiln    {:?}
    vanilla {:?}", a.len(), b.len(), a.get(i), b.get(i));
        }
    }
    format!("
    kiln    {kiln:?}
    vanilla {vanilla:?}")
}

fn state_name(s: u16) -> String {
    let b = kiln_data::blocks_types::block_of(s);
    let props: Vec<String> = b.properties.iter().zip(b.property_indices(s)).map(|(p, i)| format!("{}={}", p.name, p.values[i])).collect();
    if props.is_empty() { b.name.to_string() } else { format!("{}[{}]", b.name, props.join(",")) }
}


/// Kiln's chunks of the dump's windows, compared with vanilla's block by block and start by start.
fn compare(name: &str) {
    let work = work_dir();
    let generated = work.join("generated");
    let file = std::env::var_os("KILN_WP87_DUMP").map(PathBuf::from).unwrap_or_else(|| work.join("wp87/vectors").join(format!("pack_{name}.bin")));
    if !generated.join("reports/biome_parameters").is_dir() || !file.is_file() {
        eprintln!("skipped {name}: need {} and {} (python3 tools/wp87/order87.py {name} ...)", generated.display(), file.display());
        return;
    }
    let dump = read_dump(&file);
    let pack = Datapack::load(&generated).expect("load the datapack");
    let world = Worldgen::overworld(&pack, dump.seed, dump.structures).expect("worldgen");
    let pipeline = Pipeline::new(Arc::new(world));
    let mut gs = GenScratch::default();
    let chunks: Vec<_> = dump.targets.iter().map(|t| pipeline.full(&mut gs, t.x, t.z)).collect();
    let (mut block_diffs, mut start_diffs, mut bad_chunks) = (0usize, 0usize, 0usize);
    let mut first = Vec::new();
    for (t, p) in dump.targets.iter().zip(&chunks) {
        let min_y = pipeline.world().generator.min_y;
        let mut chunk_bad = false;
        for (i, (&ours, &theirs)) in p.blocks.iter().zip(&t.blocks).enumerate() {
            if ours != theirs {
                block_diffs += 1;
                chunk_bad = true;
                let (s, k) = (i >> 12, i & 4095);
                if first.len() < 20 {
                    first.push(format!(
                        "chunk {},{} block ({}, {}, {}): kiln {} vanilla {}",
                        t.x,
                        t.z,
                        t.x * 16 + (k & 15) as i32,
                        min_y + (s * 16 + (k >> 8)) as i32,
                        t.z * 16 + ((k >> 4) & 15) as i32,
                        state_name(ours),
                        state_name(theirs)
                    ));
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
            let ids = |l: &[(String, kiln_proto::nbt::Tag)]| l.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>();
            let detail = ours.iter().zip(&t.starts).find(|(a, b)| a != b).map(|(a, b)| format!(" {}", first_difference(&a.1, &b.1))).unwrap_or_default();
            first.insert(0, format!("chunk {},{} structure starts: kiln {:?} vanilla {:?}{detail}", t.x, t.z, ids(&ours), ids(&t.starts)));
        }
        bad_chunks += usize::from(chunk_bad);
    }
    let structures: usize = dump.targets.iter().map(|t| t.starts.len()).sum();
    eprintln!("{name}: {} chunks, {structures} structure starts in them; {bad_chunks} chunks differ: {block_diffs} blocks, {start_diffs} structure start lists", dump.targets.len());
    for line in &first {
        eprintln!("  {line}");
    }
    assert_eq!(bad_chunks, 0, "{name}: {bad_chunks} chunks differ from vanilla");
}

/// The desert pyramid of seed 84 at (-768, 160) with the chunks around it, and the warm ocean ruins at (-544, 160).
#[test]
fn seed_84_around_the_pyramid_and_the_ruins_matches_vanilla_in_kilns_order() {
    compare("seed84_canonical");
}
