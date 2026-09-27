//! Block-for-block chunk parity against vanilla's own generation, dumped by
//! `tools/chunk_vectors.py` (BIOMES, then TERRAIN's fill, surface and carver steps without
//! structures). Skips (with a message) when the dumps or the datapack are absent: both are
//! Mojang-derived and live in the untracked work directory.
//!
//! Slow, so it only runs with `KILN_PARITY=1` (best with `--release`):
//! `KILN_PARITY=1 cargo test -p kiln-worldgen --release --test chunks -- --nocapture`.
//!
//! Environment: `KILN_WORK` (default `<workspace>/work`), `KILN_CHUNK_VECTORS` (default
//! `<work>/wp3-worldgen/chunks`), `KILN_CHUNK_LIMIT` (compare only the first N chunks per seed).

use kiln_worldgen::generator::Step;
use kiln_worldgen::{Datapack, GenScratch, Generator, ProtoChunk};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn work_dir() -> PathBuf {
    match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    }
}

fn inputs() -> Option<(Datapack, Vec<PathBuf>)> {
    if std::env::var_os("KILN_PARITY").is_none_or(|v| v != "1") {
        eprintln!("skipping chunk parity: set KILN_PARITY=1 to run it");
        return None;
    }
    let work = work_dir();
    let dir = std::env::var_os("KILN_CHUNK_VECTORS").map(PathBuf::from).unwrap_or_else(|| work.join("wp3-worldgen/chunks"));
    let generated = work.join("generated");
    let files: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|d| {
            let mut v: Vec<_> = d
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("chunks_"))
                .collect();
            v.sort();
            v
        })
        .unwrap_or_default();
    if files.is_empty() || !generated.join("reports/biome_parameters").is_dir() {
        eprintln!(
            "skipping chunk parity: need {} (tools/chunk_vectors.py) and {} (cargo xtask data)",
            dir.display(),
            generated.display()
        );
        return None;
    }
    Some((Datapack::load(&generated).expect("load datapack"), files))
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
}

struct Chunk {
    x: i32,
    z: i32,
    data: Vec<u8>,
}

struct Dump {
    seed: i64,
    min_y: i32,
    height: i32,
    biome_names: Vec<String>,
    parameters: Vec<(u32, [i64; 14])>,
    chunks: Vec<Chunk>,
}

fn read_dump(path: &Path) -> Dump {
    let bytes = fs::read(path).unwrap();
    let mut r = Reader { b: &bytes, i: 0 };
    assert_eq!(r.take(4), b"KWGC", "bad magic");
    assert_eq!(r.i32(), 1, "unsupported dump version");
    let seed = r.i64();
    let min_y = r.i32();
    let height = r.i32();
    let states = r.i32();
    assert_eq!(states as u32, kiln_data::blocks::STATE_COUNT, "block state count differs from kiln-data");
    let biome_names = (0..r.i32()).map(|_| r.str()).collect();
    let parameters = (0..r.i32())
        .map(|_| {
            let b = r.i32() as u32;
            let mut p = [0i64; 14];
            for v in &mut p {
                *v = r.i64();
            }
            (b, p)
        })
        .collect();
    let limit = std::env::var("KILN_CHUNK_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let mut chunks = Vec::new();
    for _ in 0..r.i32() {
        let (x, z) = (r.i32(), r.i32());
        let n = r.i32() as usize;
        let compressed = r.take(n);
        if chunks.len() < limit {
            let mut data = Vec::new();
            flate2::read::ZlibDecoder::new(compressed).read_to_end(&mut data).unwrap();
            chunks.push(Chunk { x, z, data });
        }
    }
    Dump { seed, min_y, height, biome_names, parameters, chunks }
}

#[derive(Default)]
struct Tally {
    chunks: usize,
    bad_chunks: usize,
    compared: usize,
    mismatched: usize,
    first: Vec<String>,
}

impl Tally {
    fn merge(&mut self, o: Tally) {
        self.chunks += o.chunks;
        self.bad_chunks += o.bad_chunks;
        self.compared += o.compared;
        self.mismatched += o.mismatched;
        for f in o.first {
            if self.first.len() < 8 {
                self.first.push(f);
            }
        }
    }
}

fn state_name(s: u16) -> String {
    let b = kiln_data::blocks_types::block_of(s);
    let props: Vec<String> = b
        .properties
        .iter()
        .zip(b.property_indices(s))
        .map(|(p, i)| format!("{}={}", p.name, p.values[i]))
        .collect();
    if props.is_empty() { b.name.to_string() } else { format!("{}[{}]", b.name, props.join(",")) }
}

const LAYERS: [&str; 4] = ["biomes", "fill", "surface", "carvers"];

/// Counts of the features each step exercised (identical in vanilla when the step matches),
/// to show what the comparison covered.
fn count_features(out: &mut BTreeMap<&'static str, u64>, step: Step, chunk: &ProtoChunk, surface: &mut Vec<u16>, min_y: i32) {
    use kiln_data::blocks::default_state as d;
    let mut add = |k: &'static str, n: u64| *out.entry(k).or_default() += n;
    match step {
        Step::Fill => {
            let lava_above = chunk
                .blocks
                .iter()
                .enumerate()
                .filter(|&(i, &b)| b == d::LAVA && min_y + ((i >> 12) * 16 + ((i >> 8) & 15)) as i32 >= -54)
                .count();
            add("aquifer lava blocks", lava_above as u64);
            add("fill water blocks", chunk.blocks.iter().filter(|&&b| b == d::WATER).count() as u64);
        }
        Step::Surface => {
            let name = |b: u16| kiln_data::blocks_types::block_of(b).name;
            let mut terracotta = 0;
            let mut ores = 0;
            let mut ice = 0;
            for &b in &chunk.blocks {
                if b == d::STONE || b == d::AIR || b == d::WATER || b == d::DEEPSLATE {
                    continue;
                }
                let n = name(b);
                if n.ends_with("terracotta") {
                    terracotta += 1;
                } else if matches!(n, "minecraft:copper_ore" | "minecraft:raw_copper_block" | "minecraft:deepslate_iron_ore" | "minecraft:raw_iron_block") {
                    ores += 1;
                } else if matches!(n, "minecraft:packed_ice" | "minecraft:snow_block") {
                    ice += 1;
                }
            }
            add("band terracotta blocks", terracotta);
            add("ore vein ore blocks", ores);
            add("packed ice/snow blocks", ice);
            surface.clear();
            surface.extend_from_slice(&chunk.blocks);
        }
        Step::Carvers => {
            let mut carved = 0;
            let mut top = 0;
            for (&a, &b) in surface.iter().zip(&chunk.blocks) {
                if a != b {
                    carved += 1;
                    if a == d::DIRT && !kiln_data::blocks_types::is_air(b) && !kiln_data::blocks_types::has_fluid(b) {
                        top += 1;
                    }
                }
            }
            add("carved blocks", carved);
            add("top material fixes", top);
        }
    }
}

#[test]
fn chunks_match_vanilla() {
    let Some((pack, files)) = inputs() else { return };
    let mut total_bad = 0;
    for path in files {
        let dump = read_dump(&path);
        let generator = Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", dump.seed).expect("generator");
        assert_eq!((generator.min_y, generator.height), (dump.min_y, dump.height));

        // The parameter list as loaded from the report against vanilla's quantized values.
        let kiln_params = generator.parameters().values();
        assert_eq!(kiln_params.len(), dump.parameters.len(), "parameter list length");
        let mut param_bad = 0;
        for ((space, biome), (vb, vp)) in kiln_params.iter().zip(&dump.parameters) {
            let mine: Vec<i64> = space.iter().flat_map(|p| [p.min, p.max]).collect();
            if mine != vp || generator.biomes[*biome as usize].name != dump.biome_names[*vb as usize] {
                param_bad += 1;
            }
        }
        eprintln!("seed {}: parameter list {} entries, {param_bad} differ from vanilla", dump.seed, kiln_params.len());
        total_bad += param_bad;

        // Vanilla biome index -> Kiln biome index.
        let biome_map: Vec<u16> = dump
            .biome_names
            .iter()
            .map(|n| generator.biomes.iter().position(|b| &b.name == n).map_or(u16::MAX, |i| i as u16))
            .collect();

        let tallies: Mutex<Vec<Tally>> = Mutex::new((0..LAYERS.len()).map(|_| Tally::default()).collect());
        let coverage: Mutex<BTreeMap<&'static str, u64>> = Mutex::new(BTreeMap::new());
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let per = dump.chunks.len().div_ceil(threads).max(1);
        let sections = generator.sections();
        std::thread::scope(|scope| {
            for group in dump.chunks.chunks(per) {
                let (generator, biome_map, tallies, names) = (&generator, &biome_map, &tallies, &dump.biome_names);
                let coverage = &coverage;
                scope.spawn(move || {
                    let mut gs = GenScratch::default();
                    let mut local: Vec<Tally> = (0..LAYERS.len()).map(|_| Tally::default()).collect();
                    let mut features: BTreeMap<&'static str, u64> = BTreeMap::new();
                    let mut surface_blocks: Vec<u16> = Vec::new();
                    for c in group {
                        let quarts = sections * 64;
                        let vanilla_biomes = &c.data[..quarts];
                        let mut chunk_biomes_bad = 0;
                        let mut kiln = generator.new_chunk(&mut gs, c.x, c.z);
                        for (i, (&v, &k)) in vanilla_biomes.iter().zip(&kiln.biomes).enumerate() {
                            if biome_map[v as usize] != k {
                                chunk_biomes_bad += 1;
                                if local[0].first.len() < 8 {
                                    let (s, j) = (i / 64, i % 64);
                                    local[0].first.push(format!(
                                        "chunk {},{} quart x{} y{} z{} (section {s}): vanilla {} kiln {}",
                                        c.x,
                                        c.z,
                                        j & 3,
                                        j >> 4,
                                        (j >> 2) & 3,
                                        names[v as usize],
                                        generator.biomes[k as usize].name
                                    ));
                                }
                            }
                        }
                        let t = &mut local[0];
                        t.chunks += 1;
                        t.compared += quarts;
                        t.mismatched += chunk_biomes_bad;
                        t.bad_chunks += (chunk_biomes_bad > 0) as usize;

                        let blocks = sections * 4096;
                        generator.run_steps(&mut gs, &mut kiln, &mut |step, chunk| {
                            let layer = match step {
                                Step::Fill => 1,
                                Step::Surface => 2,
                                Step::Carvers => 3,
                            };
                            let off = quarts + (layer - 1) * blocks * 2;
                            let vanilla = &c.data[off..off + blocks * 2];
                            let mut bad = 0;
                            for (i, &k) in chunk.blocks.iter().enumerate() {
                                let v = u16::from_le_bytes([vanilla[2 * i], vanilla[2 * i + 1]]);
                                if v != k {
                                    bad += 1;
                                    if local[layer].first.len() < 8 {
                                        let (s, j) = (i / 4096, i % 4096);
                                        local[layer].first.push(format!(
                                            "chunk {},{} block {},{},{}: vanilla {} kiln {}",
                                            c.x,
                                            c.z,
                                            c.x * 16 + (j & 15) as i32,
                                            generator.min_y + (s * 16 + (j >> 8)) as i32,
                                            c.z * 16 + ((j >> 4) & 15) as i32,
                                            state_name(v),
                                            state_name(k)
                                        ));
                                    }
                                }
                            }
                            count_features(&mut features, step, chunk, &mut surface_blocks, generator.min_y);
                            let t = &mut local[layer];
                            t.chunks += 1;
                            t.compared += blocks;
                            t.mismatched += bad;
                            t.bad_chunks += (bad > 0) as usize;
                        });
                    }
                    let mut all = tallies.lock().unwrap();
                    for (a, l) in all.iter_mut().zip(local) {
                        a.merge(l);
                    }
                    let mut cov = coverage.lock().unwrap();
                    for (k, v) in features {
                        *cov.entry(k).or_default() += v;
                    }
                });
            }
        });
        let tallies = tallies.into_inner().unwrap();
        eprintln!("{}: seed {}", path.file_name().unwrap().to_string_lossy(), dump.seed);
        for (name, t) in LAYERS.iter().zip(&tallies) {
            eprintln!(
                "  {name:8} {:>6} chunks ({:>5} with mismatches) {:>11} compared {:>9} mismatched",
                t.chunks, t.bad_chunks, t.compared, t.mismatched
            );
            for f in &t.first {
                eprintln!("      {f}");
            }
            total_bad += t.mismatched;
        }
        let cov = coverage.into_inner().unwrap();
        eprintln!("  coverage: {}", cov.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "));
    }
    assert_eq!(total_bad, 0, "chunks differ from vanilla");
}

/// The `kiln_world::ChunkGenerator` adapter: missing chunks of a `World` come from the
/// generator, block for block and with network biome ids. Needs only the datapack.
#[test]
fn noise_chunks_fill_a_world() {
    let generated = work_dir().join("generated");
    if !generated.join("reports/biome_parameters").is_dir() {
        eprintln!("skipping: need {} (cargo xtask data)", generated.display());
        return;
    }
    let pack = Datapack::load(&generated).expect("load datapack");
    let generator = std::sync::Arc::new(Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", 12345).unwrap());
    let provider = kiln_world::ChunkProvider::flat(kiln_world::OVERWORLD, 0, 67)
        .with_generator(Box::new(kiln_worldgen::NoiseChunks::new(generator.clone())));
    let mut world = kiln_world::World::new(provider);
    let mut gs = GenScratch::default();
    for (cx, cz) in [(0, 0), (-1, 5)] {
        let expected = generator.generate(&mut gs, cx, cz);
        let chunk = world.load_chunk(kiln_world::ChunkPos::new(cx, cz));
        for y in (generator.min_y..generator.min_y + generator.height).step_by(3) {
            for z in 0..16 {
                for x in 0..16 {
                    assert_eq!(chunk.get(x, y, z), expected.get(x, y, z), "block {x},{y},{z} of chunk {cx},{cz}");
                }
            }
        }
        let body = chunk.packet_body(67);
        assert!(!body.is_empty());
    }
}
