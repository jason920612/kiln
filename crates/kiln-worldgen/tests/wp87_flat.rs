//! wp87: superflat dimensions that generate more than their layers, against vanilla loading the same data pack (`tools/wp87/packs/kiln87_flat`):
//! `features` (the biome's decoration but for its structure steps), `lakes` (lava lakes), and the structure sets (`structure_overrides`: none, villages only,
//! or, when absent, all of them). `tools/wp87/flat87.py` runs vanilla's own generation for the dimension and dumps the final blocks and structure starts of
//! windows of chunks; here Kiln generates the same chunks from the same stack of roots and the level stem the pack defines.
//!
//! Skipped without the vanilla data and the dumps (`$KILN_WP87_VECTORS`, default `<work>/wp87/vectors`).

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



fn check(case: &str, dimension: &str, expect_blocks: &[(&str, usize)], expect_starts: &[&str]) {
    let work = work_dir();
    let generated = work.join("generated");
    let vectors = std::env::var_os("KILN_WP87_VECTORS").map(PathBuf::from).unwrap_or_else(|| work.join("wp87/vectors"));
    let file = vectors.join(format!("pack_flat_{case}.bin"));
    if !generated.join("reports/biome_parameters").is_dir() || !file.is_file() {
        eprintln!("skipped {case}: need {} and {} (python3 tools/wp87/flat87.py {case})", generated.display(), file.display());
        return;
    }
    let dump = read_dump(&file);
    let roots = vec![generated, Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/wp87/packs/kiln87_flat")];
    let mut pack = Datapack::load_stack(&roots).expect("load the pack stack");
    let stems = kiln_worldgen::stem::stems_of(&pack);
    let (_, json) = stems.iter().find(|(k, _)| k == dimension).unwrap_or_else(|| panic!("no dimension {dimension}"));
    let stem = pack.parse_stem(dimension, json).expect("stem");
    let world = Worldgen::from_stem(&pack, &stem, dump.seed, dump.structures).expect("worldgen").expect("a generator");
    let pipeline = Pipeline::new(Arc::new(world));
    let mut gs = GenScratch::default();
    let chunks: Vec<_> = dump.targets.iter().map(|t| pipeline.full(&mut gs, t.x, t.z)).collect();
    let (mut block_diffs, mut start_diffs, mut bad_chunks) = (0usize, 0usize, 0usize);
    let mut first = Vec::new();
    let mut pairs = std::collections::BTreeMap::<(u16, u16), usize>::new();
    for (t, p) in dump.targets.iter().zip(&chunks) {
        let min_y = pipeline.world().generator.min_y;
        let mut chunk_bad = false;
        for (i, (&ours, &theirs)) in p.blocks.iter().zip(&t.blocks).enumerate() {
            if ours != theirs {
                block_diffs += 1;
                chunk_bad = true;
                *pairs.entry((ours, theirs)).or_default() += 1;
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
    // The dump itself shows what the data pack makes.
    let count = |block: &str| -> usize {
        let b = kiln_data::blocks_types::block_by_name(block).unwrap_or_else(|| panic!("no block {block}"));
        dump.targets.iter().map(|t| t.blocks.iter().filter(|&&s| (b.first..=b.last).contains(&s)).count()).sum()
    };
    for &(block, at_least) in expect_blocks {
        assert!(count(block) >= at_least, "{case}: vanilla's chunks have {} {block}, expected at least {at_least}", count(block));
    }
    for &id in expect_starts {
        assert!(dump.targets.iter().any(|t| t.starts.iter().any(|(s, _)| s == id)), "{case}: no {id} start in vanilla's chunks");
    }
    let structures: usize = dump.targets.iter().map(|t| t.starts.len()).sum();
    eprintln!("{case} ({dimension}): {} chunks, {structures} structure starts in them; {bad_chunks} chunks differ: {block_diffs} blocks, {start_diffs} structure start lists", dump.targets.len());
    let mut by_count: Vec<_> = pairs.into_iter().collect();
    by_count.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for ((ours, theirs), n) in by_count.iter().take(8) {
        eprintln!("  {n} x kiln {} vanilla {}", state_name(*ours), state_name(*theirs));
    }
    for line in &first {
        eprintln!("  {line}");
    }
    assert_eq!(bad_chunks, 0, "{case}: {bad_chunks} chunks differ from vanilla");
}

#[test]
fn a_flat_level_with_features_decorates_as_vanilla_does() {
    check("feat", "kiln87:flatfeat", &[("minecraft:oak_log", 1), ("minecraft:short_grass", 100)], &[]);
}

#[test]
fn a_flat_level_with_lakes_has_its_lava() {
    check("lakes", "kiln87:flatlakes", &[("minecraft:short_grass", 100)], &[]);
}

#[test]
fn a_flat_level_generates_the_structure_sets_it_names() {
    check("village", "kiln87:flatvillage", &[("minecraft:oak_planks", 50)], &["minecraft:village_plains"]);
}

#[test]
fn a_flat_level_without_overrides_generates_every_structure_set() {
    check("all", "kiln87:flatall", &[("minecraft:short_grass", 100)], &[]);
}
