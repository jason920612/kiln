//! FEATURES parity against vanilla, dumped by `tools/feature_vectors.py`: every placed feature
//! vanilla ran while decorating chunks in Kiln's canonical order, with the blocks it changed.
//!
//! Each decoration is compared placement by placement. A placement whose feature Kiln does
//! not implement is skipped and vanilla's changes are applied instead; a mismatching one is
//! counted and replaced by vanilla's changes too, so every placement starts from vanilla's
//! state. With `KILN_REPLAY=0` nothing is replaced and only the final blocks of the target
//! chunks are compared (end-to-end parity).
//!
//! Slow and Mojang-derived, so it only runs with `KILN_PARITY=1` (best with `--release`):
//! `KILN_PARITY=1 cargo test -p kiln-worldgen --release --test features -- --nocapture`.
//! Environment: `KILN_WORK`, `KILN_FEATURE_VECTORS` (default `<work>/wp4-features/vectors`),
//! `KILN_FEATURE_REGIONS` (compare only the first N regions per seed).

use kiln_worldgen::decorate::{Decorator, Observer};
use kiln_worldgen::generator::{GenScratch, Generator};
use kiln_worldgen::pos::BlockPos;
use kiln_worldgen::proto::{ProtoChunk, Status};
use kiln_worldgen::region::Region;
use kiln_worldgen::sets::Loader;
use kiln_worldgen::{Datapack, order};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

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
    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }
    fn i16(&mut self) -> i16 {
        i16::from_le_bytes(self.take(2).try_into().unwrap())
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
    fn blob(&mut self) -> Vec<u8> {
        let n = self.i32() as usize;
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(self.take(n)).read_to_end(&mut out).unwrap();
        out
    }
}

/// One placement vanilla ran: kind (0 feature, 1 structure), step, index, far reads, and the
/// blocks it changed (absolute position, new state).
struct Invocation {
    kind: u8,
    step: usize,
    index: usize,
    far_reads: u32,
    changes: Vec<(BlockPos, u16)>,
}

struct Decoration {
    x: i32,
    z: i32,
    invocations: Vec<Invocation>,
}

struct RegionDump {
    targets: Vec<(i32, i32)>,
    decorations: Vec<Decoration>,
    finals: Vec<(i32, i32, Vec<u16>)>,
}

struct Dump {
    seed: i64,
    steps: Vec<Vec<String>>,
    regions: Vec<RegionDump>,
}

fn read_dump(path: &Path) -> Dump {
    let bytes = fs::read(path).unwrap();
    let mut r = Reader { b: &bytes, i: 0 };
    assert_eq!(r.take(4), b"KWGF", "bad magic");
    assert_eq!(r.i32(), 1, "unsupported dump version");
    let seed = r.i64();
    let _structures = r.i32();
    assert_eq!(r.i32() as u32, kiln_data::blocks::STATE_COUNT, "block state count differs from kiln-data");
    let (_min_y, _height) = (r.i32(), r.i32());
    let steps = (0..r.i32()).map(|_| (0..r.i32()).map(|_| r.str()).collect()).collect();
    let limit = std::env::var("KILN_FEATURE_REGIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let count = r.i32() as usize;
    let mut regions = Vec::new();
    for _ in 0..count.min(limit) {
        let targets: Vec<(i32, i32)> = (0..r.i32()).map(|_| (r.i32(), r.i32())).collect();
        let mut decorations = Vec::new();
        for _ in 0..r.i32() {
            let (x, z) = (r.i32(), r.i32());
            let blob = r.blob();
            let mut b = Reader { b: &blob, i: 0 };
            let invocations = (0..b.i32())
                .map(|_| {
                    let kind = b.u8();
                    let step = b.u8() as usize;
                    let index = b.u16() as usize;
                    let far_reads = b.i32() as u32;
                    let changes = (0..b.i32())
                        .map(|_| {
                            let dx = b.u8() as i32 - 16;
                            let dz = b.u8() as i32 - 16;
                            let y = b.i16() as i32;
                            (BlockPos::new(x * 16 + dx, y, z * 16 + dz), b.u16())
                        })
                        .collect();
                    Invocation { kind, step, index, far_reads, changes }
                })
                .collect();
            decorations.push(Decoration { x, z, invocations });
        }
        let finals = targets
            .iter()
            .map(|_| {
                let (x, z) = (r.i32(), r.i32());
                let blob = r.blob();
                (x, z, blob.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
            })
            .collect();
        regions.push(RegionDump { targets, decorations, finals });
    }
    Dump { seed, steps, regions }
}

#[derive(Default, Clone)]
struct Tally {
    placements: u64,
    matched: u64,
    mismatched: u64,
    skipped: u64,
    blocks: u64,
    first: Vec<String>,
}

/// Compares each placement with vanilla's and replays vanilla where needed.
struct Compare<'d> {
    decorator: &'d Decorator,
    expected: &'d [Invocation],
    next: usize,
    replay: bool,
    list_errors: u64,
    per_feature: BTreeMap<String, Tally>,
    far_reads: u64,
}

fn name_of(d: &Decorator, placed: usize) -> String {
    let p = &d.features.placed[placed];
    let ty = d.features.type_name(p.feature).trim_start_matches("minecraft:").to_string();
    format!("{} ({ty})", if p.name.is_empty() { "<inline>" } else { &p.name })
}

fn state_name(s: u16) -> String {
    let b = kiln_data::blocks_types::block_of(s);
    let props: Vec<String> =
        b.properties.iter().zip(b.property_indices(s)).map(|(p, i)| format!("{}={}", p.name, p.values[i])).collect();
    if props.is_empty() { b.name.to_string() } else { format!("{}[{}]", b.name, props.join(",")) }
}

/// Applies `changes` directly and re-primes the final heightmaps of the region's chunks.
fn apply(r: &mut Region, undo: &[(BlockPos, u16)], changes: &[(BlockPos, u16)]) {
    for &(p, old) in undo.iter().rev() {
        r.set_raw(p, old);
    }
    for &(p, s) in changes {
        r.set_raw(p, s);
    }
    let _ = r.take_log();
    for (cx, cz) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
        let (x, z) = (r.cx + cx, r.cz + cz);
        r.chunk_mut(x, z).unwrap().prime_heightmaps();
    }
}

impl Observer for Compare<'_> {
    fn before(&mut self, _step: usize, _index: usize, placed: usize, r: &mut Region) -> bool {
        let _ = r.take_log();
        !self.replay || self.decorator.features.is_supported(self.decorator.features.placed[placed].feature)
    }

    fn after(&mut self, step: usize, index: usize, placed: usize, r: &mut Region) {
        let log = r.take_log();
        let name = name_of(self.decorator, placed);
        let tally = self.per_feature.entry(name).or_default();
        tally.placements += 1;
        while self.next < self.expected.len() && self.expected[self.next].kind != 0 {
            self.next += 1;
        }
        let Some(inv) = self.expected.get(self.next).filter(|i| (i.step, i.index) == (step, index)) else {
            self.list_errors += 1;
            return;
        };
        self.next += 1;
        self.far_reads += inv.far_reads as u64;
        if !self.replay {
            return;
        }
        let supported = self.decorator.features.is_supported(self.decorator.features.placed[placed].feature);
        // Net changes of Kiln's placement.
        let mut first_old: HashMap<BlockPos, u16> = HashMap::new();
        for &(p, old) in &log {
            first_old.entry(p).or_insert(old);
        }
        let mut mine: Vec<(BlockPos, u16)> = Vec::new();
        for (&p, &old) in &first_old {
            let now = r.get(p);
            if now != old {
                mine.push((p, now));
            }
        }
        mine.sort_by_key(|(p, _)| *p);
        let mut theirs = inv.changes.clone();
        theirs.sort_by_key(|(p, _)| *p);
        if !supported {
            tally.skipped += 1;
            apply(r, &[], &theirs);
            return;
        }
        if mine == theirs {
            tally.matched += 1;
            tally.blocks += theirs.len() as u64;
            return;
        }
        tally.mismatched += 1;
        if tally.first.len() < 3 {
            let m: HashMap<BlockPos, u16> = mine.iter().copied().collect();
            let t: HashMap<BlockPos, u16> = theirs.iter().copied().collect();
            let mut diffs: Vec<String> = Vec::new();
            for (p, s) in &theirs {
                if m.get(p) != Some(s) {
                    diffs.push(format!("{},{},{} vanilla {} kiln {}", p.x, p.y, p.z, state_name(*s), m.get(p).map_or("-".into(), |k| state_name(*k))));
                }
            }
            for (p, s) in &mine {
                if !t.contains_key(p) {
                    diffs.push(format!("{},{},{} vanilla - kiln {}", p.x, p.y, p.z, state_name(*s)));
                }
            }
            diffs.sort();
            diffs.truncate(4);
            tally.first.push(format!(
                "chunk {},{} step {step} #{index}: {} vanilla / {} kiln changes; {}",
                r.cx,
                r.cz,
                theirs.len(),
                mine.len(),
                diffs.join("; ")
            ));
        }
        let undo: Vec<(BlockPos, u16)> = first_old.into_iter().collect();
        apply(r, &undo, &theirs);
    }
}

#[test]
fn features_match_vanilla() {
    if std::env::var_os("KILN_PARITY").is_none_or(|v| v != "1") {
        eprintln!("skipping feature parity: set KILN_PARITY=1 to run it");
        return;
    }
    let work = work_dir();
    let dir = std::env::var_os("KILN_FEATURE_VECTORS").map(PathBuf::from).unwrap_or_else(|| work.join("wp4-features/vectors"));
    let generated = work.join("generated");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "bin")).collect())
        .unwrap_or_default();
    files.sort();
    if files.is_empty() || !generated.join("reports/biome_parameters").is_dir() {
        eprintln!("skipping feature parity: need {} (tools/feature_vectors.py) and {}", dir.display(), generated.display());
        return;
    }
    let replay = std::env::var("KILN_REPLAY").map_or(true, |v| v != "0");
    let pack = Datapack::load(&generated).expect("load datapack");
    let mut all: BTreeMap<String, Tally> = BTreeMap::new();
    let mut total_bad = 0u64;
    for path in files {
        let dump = read_dump(&path);
        let generator = Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", dump.seed).expect("generator");
        let loader = Loader::new(&pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let decorator = Decorator::new(&generator, &loader).expect("decorator");
        // Feature order per step against vanilla's FeatureSorter.
        let mut order_bad = 0;
        for (step, names) in dump.steps.iter().enumerate() {
            let mine: Vec<String> = decorator.steps.get(step).map_or(Vec::new(), |s| {
                s.iter().map(|&p| decorator.features.placed[p].name.clone()).collect()
            });
            for (i, n) in names.iter().enumerate() {
                if !n.starts_with('#') && mine.get(i) != Some(n) {
                    order_bad += 1;
                    if order_bad <= 5 {
                        eprintln!("  step {step} #{i}: vanilla {n}, kiln {:?}", mine.get(i));
                    }
                }
            }
        }
        eprintln!("{}: seed {}, feature order: {order_bad} differences", path.file_name().unwrap().to_string_lossy(), dump.seed);
        total_bad += order_bad;

        let mut list_errors = 0;
        let mut far_reads = 0;
        let mut final_bad = 0u64;
        let mut final_chunks_bad = 0;
        for region in &dump.regions {
            let order = order::decoration_order(&region.targets);
            let dumped: Vec<(i32, i32)> = region.decorations.iter().map(|d| (d.x, d.z)).collect();
            assert_eq!(order, dumped, "decoration order differs from the harness");
            // Terrain for every decorated chunk and its neighbours, on all cores.
            let mut need: Vec<(i32, i32)> = Vec::new();
            for &(x, z) in &order {
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        need.push((x + dx, z + dz));
                    }
                }
            }
            need.sort();
            need.dedup();
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let mut chunks: HashMap<(i32, i32), Box<ProtoChunk>> = HashMap::new();
            std::thread::scope(|s| {
                let handles: Vec<_> = need
                    .chunks(need.len().div_ceil(threads))
                    .map(|part| {
                        let generator = &generator;
                        s.spawn(move || {
                            let mut gs = GenScratch::default();
                            part.iter().map(|&(x, z)| ((x, z), Box::new(generator.generate(&mut gs, x, z)))).collect::<Vec<_>>()
                        })
                    })
                    .collect();
                for h in handles {
                    chunks.extend(h.join().unwrap());
                }
            });
            let mut gs = GenScratch::default();
            for d in &region.decorations {
                let window: Vec<Box<ProtoChunk>> = (0..9)
                    .map(|i| chunks.remove(&(d.x + i % 3 - 1, d.z + i / 3 - 1)).expect("terrain for the window"))
                    .collect();
                let mut r = Region::new(window, d.x, d.z, &generator, &mut gs);
                r.start_log();
                let mut cmp = Compare {
                    decorator: &decorator,
                    expected: &d.invocations,
                    next: 0,
                    replay,
                    list_errors: 0,
                    per_feature: BTreeMap::new(),
                    far_reads: 0,
                };
                decorator.decorate(&mut r, &mut cmp);
                let vanilla_features = d.invocations.iter().filter(|i| i.kind == 0).count();
                if cmp.next != vanilla_features {
                    cmp.list_errors += 1;
                }
                list_errors += cmp.list_errors;
                far_reads += cmp.far_reads;
                for (k, t) in cmp.per_feature {
                    let a = all.entry(k).or_default();
                    a.placements += t.placements;
                    a.matched += t.matched;
                    a.mismatched += t.mismatched;
                    a.skipped += t.skipped;
                    a.blocks += t.blocks;
                    for f in t.first {
                        if a.first.len() < 3 {
                            a.first.push(f);
                        }
                    }
                }
                let stats = r.stats;
                assert_eq!(stats.far_writes, 0, "writes outside the window");
                for mut c in r.into_chunks() {
                    c.status = Status::Features;
                    chunks.insert((c.x, c.z), c);
                }
            }
            for (x, z, vanilla) in &region.finals {
                let c = &chunks[&(*x, *z)];
                let bad = c.blocks.iter().zip(vanilla).filter(|(a, b)| a != b).count() as u64;
                final_bad += bad;
                final_chunks_bad += (bad > 0) as usize;
            }
        }
        eprintln!(
            "  {} regions: feature list errors {list_errors}, vanilla far reads {far_reads}, final target blocks differing {final_bad} (in {final_chunks_bad} chunks)",
            dump.regions.len()
        );
        total_bad += list_errors + final_bad;
    }
    let mut by_type: BTreeMap<String, Tally> = BTreeMap::new();
    eprintln!("per placed feature: placements matched/mismatched/skipped (blocks in matched placements)");
    for (name, t) in &all {
        eprintln!("  {name:60} {:>7} {:>7}/{:>5}/{:>6} ({})", t.placements, t.matched, t.mismatched, t.skipped, t.blocks);
        for f in &t.first {
            eprintln!("      {f}");
        }
        let ty = name.rsplit('(').next().unwrap_or("").trim_end_matches(')').to_string();
        let e = by_type.entry(ty).or_default();
        e.placements += t.placements;
        e.matched += t.matched;
        e.mismatched += t.mismatched;
        e.skipped += t.skipped;
        e.blocks += t.blocks;
        total_bad += t.mismatched;
    }
    eprintln!("per feature type: placements matched/mismatched/skipped (blocks)");
    for (ty, t) in &by_type {
        eprintln!("  {ty:30} {:>8} {:>8}/{:>6}/{:>7} ({})", t.placements, t.matched, t.mismatched, t.skipped, t.blocks);
    }
    let gaps: u64 = by_type.values().map(|t| t.skipped).sum();
    eprintln!("skipped (unimplemented) placements: {gaps}");
    assert_eq!(total_bad, 0, "features differ from vanilla");
}
