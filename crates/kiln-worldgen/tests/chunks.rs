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
use kiln_worldgen::{Datapack, GenScratch, Generator};
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
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let per = dump.chunks.len().div_ceil(threads).max(1);
        let sections = generator.sections();
        std::thread::scope(|scope| {
            for group in dump.chunks.chunks(per) {
                let (generator, biome_map, tallies, names) = (&generator, &biome_map, &tallies, &dump.biome_names);
                scope.spawn(move || {
                    let mut gs = GenScratch::default();
                    let mut local: Vec<Tally> = (0..LAYERS.len()).map(|_| Tally::default()).collect();
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
                });
            }
        });
        let tallies = tallies.into_inner().unwrap();
        let mut report = BTreeMap::new();
        for (name, t) in LAYERS.iter().zip(&tallies) {
            report.insert(*name, (t.chunks, t.bad_chunks, t.compared, t.mismatched));
        }
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
    }
    assert_eq!(total_bad, 0, "chunks differ from vanilla");
}
