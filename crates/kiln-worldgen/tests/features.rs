//! FEATURES parity against vanilla, dumped by `tools/feature_vectors.py`: every placed feature
//! (and structure placement) vanilla ran while decorating chunks in Kiln's canonical order,
//! with the blocks it changed.
//!
//! Each decoration is compared placement by placement. A placement whose feature or
//! structure type Kiln does not implement is skipped and vanilla's changes are applied
//! instead; a mismatching one is counted and replaced by vanilla's changes too, so every
//! placement starts from vanilla's state. With `KILN_REPLAY=0` nothing is replaced and only
//! the final blocks of the target chunks are compared (end-to-end parity).
//!
//! Dumps made with structures (`--structures`) also hold vanilla's structure starts (saved
//! NBT, compared with Kiln's per structure), references and post-TERRAIN blocks (compared with
//! Kiln's TERRAIN, then used as the starting state so FEATURES is compared on its own).
//!
//! Slow and Mojang-derived, so it only runs with `KILN_PARITY=1` (best with `--release`):
//! `KILN_PARITY=1 cargo test -p kiln-worldgen --release --test features -- --nocapture`.
//! Environment: `KILN_WORK`, `KILN_FEATURE_VECTORS` (default `<work>/wp4-features/vectors`),
//! `KILN_FEATURE_REGIONS` (compare only the first N regions per seed).

use kiln_worldgen::decorate::{Decorator, Invocation as Inv, Observer};
use kiln_worldgen::generator::{GenScratch, Generator};
use kiln_worldgen::pos::BlockPos;
use kiln_worldgen::proto::{GenTick, ProtoChunk, Status};
use kiln_worldgen::region::Region;
use kiln_worldgen::sets::Loader;
use kiln_worldgen::structure::beard::Beardifier;
use kiln_worldgen::structure::{ChunkStarts, StartCache, StructureScratch, Structures};
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
    fn blocks(&mut self) -> Vec<u16> {
        self.blob().chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
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

type NamedBytes = (String, Vec<u8>);
type NamedLongs = (String, Vec<i64>);

struct RegionDump {
    targets: Vec<(i32, i32)>,
    /// Structure starts per chunk: (structure id, saved NBT).
    starts: HashMap<(i32, i32), Vec<NamedBytes>>,
    /// Structure references per chunk: (structure id, sorted packed start chunks).
    refs: HashMap<(i32, i32), Vec<NamedLongs>>,
    terrain: HashMap<(i32, i32), Vec<u16>>,
    decorations: Vec<Decoration>,
    finals: Vec<(i32, i32, Vec<u16>)>,
    /// Vanilla's post-processing positions and scheduled ticks of the targets (version 2).
    pending: Vec<Option<Pending>>,
}

type Tick = (String, i32, i32, i32, i32, i32);

/// A chunk's post-processing lists per section and scheduled (block, fluid) ticks.
#[derive(PartialEq, Debug)]
struct Pending {
    terrain_post: Vec<Vec<u16>>,
    post: Vec<Vec<u16>>,
    block_ticks: Vec<Tick>,
    fluid_ticks: Vec<Tick>,
}

impl Pending {
    fn of(c: &ProtoChunk) -> Self {
        let ticks = |t: &[GenTick]| t.iter().map(|t| (t.kind.to_string(), t.x, t.y, t.z, t.delay, t.priority)).collect();
        Pending { terrain_post: Vec::new(), post: c.post_processing.clone(), block_ticks: ticks(&c.block_ticks), fluid_ticks: ticks(&c.fluid_ticks) }
    }
}

struct Dump {
    seed: i64,
    structures: bool,
    steps: Vec<Vec<String>>,
    regions: Vec<RegionDump>,
}

fn read_dump(path: &Path) -> Dump {
    let bytes = fs::read(path).unwrap();
    let mut r = Reader { b: &bytes, i: 0 };
    assert_eq!(r.take(4), b"KWGF", "bad magic");
    let version = r.i32();
    assert!(version == 1 || version == 2, "unsupported dump version {version}");
    let seed = r.i64();
    let structures = r.i32() != 0;
    assert_eq!(r.i32() as u32, kiln_data::blocks::STATE_COUNT, "block state count differs from kiln-data");
    let (_min_y, height) = (r.i32(), r.i32());
    let steps = (0..r.i32()).map(|_| (0..r.i32()).map(|_| r.str()).collect()).collect();
    let limit = std::env::var("KILN_FEATURE_REGIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let count = r.i32() as usize;
    let mut regions = Vec::new();
    for _ in 0..count.min(limit) {
        let targets: Vec<(i32, i32)> = (0..r.i32()).map(|_| (r.i32(), r.i32())).collect();
        let mut starts = HashMap::new();
        let mut refs = HashMap::new();
        let mut terrain = HashMap::new();
        if structures {
            for _ in 0..r.i32() {
                let pos = (r.i32(), r.i32());
                let list = (0..r.i32())
                    .map(|_| {
                        let id = r.str();
                        let n = r.i32() as usize;
                        (id, r.take(n).to_vec())
                    })
                    .collect();
                starts.insert(pos, list);
            }
            for _ in 0..r.i32() {
                let pos = (r.i32(), r.i32());
                let list = (0..r.i32())
                    .map(|_| {
                        let id = r.str();
                        (id, (0..r.i32()).map(|_| r.i64()).collect())
                    })
                    .collect();
                refs.insert(pos, list);
            }
            for _ in 0..r.i32() {
                let pos = (r.i32(), r.i32());
                terrain.insert(pos, r.blocks());
            }
        }
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
        let mut finals = Vec::new();
        let mut pending = Vec::new();
        for _ in &targets {
            let (x, z) = (r.i32(), r.i32());
            finals.push((x, z, r.blocks()));
            pending.push((version >= 2).then(|| {
                let mut post = || -> Vec<Vec<u16>> { (0..height / 16).map(|_| (0..r.i32()).map(|_| r.u16()).collect()).collect() };
                let terrain_post = post();
                let post = post();
                let mut ticks = || -> Vec<Tick> { (0..r.i32()).map(|_| (r.str(), r.i32(), r.i32(), r.i32(), r.i32(), r.i32())).collect() };
                let block_ticks = ticks();
                let fluid_ticks = ticks();
                Pending { terrain_post, post, block_ticks, fluid_ticks }
            }));
        }
        regions.push(RegionDump { targets, starts, refs, terrain, decorations, finals, pending });
    }
    Dump { seed, structures, steps, regions }
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

impl Tally {
    fn add(&mut self, t: Tally) {
        self.placements += t.placements;
        self.matched += t.matched;
        self.mismatched += t.mismatched;
        self.skipped += t.skipped;
        self.blocks += t.blocks;
        for f in t.first {
            if self.first.len() < 3 {
                self.first.push(f);
            }
        }
    }
}

/// Compares each placement with vanilla's and replays vanilla where needed.
struct Compare<'d> {
    decorator: &'d Decorator,
    structures: &'d Structures,
    expected: &'d [Invocation],
    next: usize,
    replay: bool,
    list_errors: u64,
    per_feature: BTreeMap<String, Tally>,
    far_reads: u64,
}

impl Compare<'_> {
    fn name(&self, inv: Inv) -> String {
        match inv {
            Inv::Feature { placed, .. } => {
                let f = &self.decorator.features;
                let p = &f.placed[placed];
                let ty = f.type_name(p.feature).trim_start_matches("minecraft:").to_string();
                format!("{} ({ty})", if p.name.is_empty() { "<inline>" } else { &p.name })
            }
            Inv::Structure { structure, .. } => format!("{} (structure)", self.structures.structures[structure].name),
        }
    }

    fn supported(&self, inv: Inv) -> bool {
        match inv {
            Inv::Feature { placed, .. } => self.decorator.features.is_supported(self.decorator.features.placed[placed].feature),
            Inv::Structure { structure, .. } => self.structures.structures[structure].kind.gap().is_none(),
        }
    }
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
    fn before(&mut self, inv: Inv, r: &mut Region) -> bool {
        let _ = r.take_log();
        !self.replay || self.supported(inv)
    }

    fn after(&mut self, inv: Inv, r: &mut Region) {
        let log = r.take_log();
        let name = self.name(inv);
        let supported = self.supported(inv);
        let (kind, step, index) = match inv {
            Inv::Feature { step, index, .. } => (0, step, index),
            Inv::Structure { step, index, .. } => (1, step, index),
        };
        let Some(expected) = self.expected.get(self.next).filter(|i| (i.kind, i.step, i.index) == (kind, step, index)) else {
            self.list_errors += 1;
            return;
        };
        self.next += 1;
        self.far_reads += expected.far_reads as u64;
        let mut theirs = expected.changes.clone();
        // Structure invocations without blocks on either side are not worth listing.
        if kind == 1 && theirs.is_empty() && log.is_empty() {
            return;
        }
        let tally = self.per_feature.entry(name).or_default();
        tally.placements += 1;
        if !self.replay {
            return;
        }
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
                    diffs.push(format!(
                        "{},{},{} vanilla - kiln {} (was {})",
                        p.x,
                        p.y,
                        p.z,
                        state_name(*s),
                        first_old.get(p).map_or("?".into(), |o| state_name(*o))
                    ));
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

/// NBT with compound keys sorted (vanilla writes `CompoundTag`s in hash order).
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
fn first_difference(vanilla: &kiln_proto::nbt::Tag, kiln: &kiln_proto::nbt::Tag) -> String {
    let children = |t: &kiln_proto::nbt::Tag| t.get("Children").and_then(|c| c.as_list()).map(|l| l.to_vec()).unwrap_or_default();
    let (a, b) = (children(vanilla), children(kiln));
    for i in 0..a.len().max(b.len()) {
        if a.get(i) != b.get(i) {
            return format!("{} vanilla / {} kiln pieces; piece {i}\n        vanilla {:?}\n        kiln    {:?}", a.len(), b.len(), a.get(i), b.get(i));
        }
    }
    format!("\n        vanilla {vanilla:?}\n        kiln    {kiln:?}")
}

/// Structure start parity per structure: vanilla starts, Kiln starts, identical NBT.
#[derive(Default)]
struct StartTally {
    vanilla: u64,
    kiln: u64,
    same: u64,
    /// (chunk, structure) reference lists: in vanilla, identical in Kiln.
    refs: u64,
    refs_same: u64,
    first: Vec<String>,
}

fn compare_starts(
    region: &RegionDump,
    need: &[(i32, i32)],
    structures: &Structures,
    generator: &Generator,
    cache: &StartCache,
    scratch: &mut StructureScratch,
    tallies: &mut BTreeMap<String, StartTally>,
) {
    let mut area: Vec<(i32, i32)> = Vec::new();
    for &(x, z) in need {
        for dx in -8..=8 {
            for dz in -8..=8 {
                area.push((x + dx, z + dz));
            }
        }
    }
    area.sort();
    area.dedup();
    for (x, z) in area {
        let mine = cache.get(structures, generator, scratch, x, z);
        let vanilla = region.starts.get(&(x, z)).map_or(&[][..], |v| &v[..]);
        for (id, nbt) in vanilla {
            let t = tallies.entry(id.clone()).or_default();
            t.vanilla += 1;
            let theirs = canonical(kiln_proto::nbt::read_named(nbt).expect("vanilla start NBT").1);
            match mine.iter().find(|s| structures.structures[s.structure].name == *id) {
                Some(s) if canonical(s.save(structures)) == theirs => t.same += 1,
                Some(s) => {
                    if t.first.len() < 2 {
                        t.first.push(format!("chunk {x},{z}: NBT differs: {}", first_difference(&theirs, &canonical(s.save(structures)))));
                    }
                }
                None => {
                    if t.first.len() < 2 && structures.structures[structures.id(id).unwrap()].kind.gap().is_none() {
                        t.first.push(format!("chunk {x},{z}: missing"));
                    }
                }
            }
        }
        for s in mine.iter() {
            let id = &structures.structures[s.structure].name;
            let t = tallies.entry(id.clone()).or_default();
            t.kiln += 1;
            if !vanilla.iter().any(|(v, _)| v == id) && t.first.len() < 2 {
                t.first.push(format!("chunk {x},{z}: extra start"));
            }
        }
    }
    // References (`structures.References`) of the chunks that reached TERRAIN.
    for &(x, z) in need {
        let mut mine = ChunkStarts::new(structures, generator, cache, scratch, x, z).references(structures);
        for (_, l) in &mut mine {
            l.sort();
        }
        let vanilla = region.refs.get(&(x, z)).map_or(&[][..], |v| &v[..]);
        for (id, list) in vanilla {
            let t = tallies.entry(id.clone()).or_default();
            t.refs += 1;
            match mine.iter().find(|(m, _)| m == id) {
                Some((_, l)) if l == list => t.refs_same += 1,
                other => {
                    if t.first.len() < 3 && structures.structures[structures.id(id).unwrap()].kind.gap().is_none() {
                        t.first.push(format!("chunk {x},{z}: references differ: vanilla {list:?}, kiln {:?}", other.map(|o| &o.1)));
                    }
                }
            }
        }
        for (id, _) in &mine {
            if !vanilla.iter().any(|(v, _)| v == id) {
                let t = tallies.entry(id.clone()).or_default();
                if t.first.len() < 3 {
                    t.first.push(format!("chunk {x},{z}: extra references"));
                }
            }
        }
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
    let mut starts_all: BTreeMap<String, StartTally> = BTreeMap::new();
    let mut total_bad = 0u64;
    for path in files {
        let dump = read_dump(&path);
        let generator = Generator::new(&pack, "minecraft:overworld", "minecraft:overworld", dump.seed).expect("generator");
        let loader = Loader::new(&pack, generator.biomes.iter().map(|b| b.name.clone()).collect());
        let decorator = Decorator::new(&generator, &loader).expect("decorator");
        let structures = Structures::load(&generator, &loader).expect("structures");
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
        eprintln!(
            "{}: seed {}{}, feature order: {order_bad} differences",
            path.file_name().unwrap().to_string_lossy(),
            dump.seed,
            if dump.structures { " (structures)" } else { "" }
        );
        total_bad += order_bad;

        let cache = StartCache::default();
        let mut sscratch = StructureScratch::default();
        let mut list_errors = 0;
        let mut far_reads = 0;
        let mut final_bad = 0u64;
        let mut final_chunks_bad = 0;
        let (mut terrain_bad, mut terrain_chunks_bad) = (0u64, 0usize);
        let (mut pending_checked, mut post_bad, mut ticks_bad, mut post_vanilla, mut ticks_vanilla) = (0, 0, 0, 0, 0);
        let (mut terrain_post_bad, mut terrain_post_vanilla) = (0, 0);
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
            if dump.structures {
                compare_starts(region, &need, &structures, &generator, &cache, &mut sscratch, &mut starts_all);
            }
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let mut chunks: HashMap<(i32, i32), Box<ProtoChunk>> = HashMap::new();
            std::thread::scope(|s| {
                let handles: Vec<_> = need
                    .chunks(need.len().div_ceil(threads))
                    .map(|part| {
                        let (generator, structures, cache) = (&generator, &structures, &cache);
                        let with_structures = dump.structures;
                        s.spawn(move || {
                            let mut gs = GenScratch::default();
                            part.iter()
                                .map(|&(x, z)| {
                                    let beard = with_structures
                                        .then(|| {
                                            let starts = ChunkStarts::new(structures, generator, cache, &mut gs.structures, x, z);
                                            Beardifier::for_chunk(structures, &starts, x, z)
                                        })
                                        .flatten();
                                    ((x, z), Box::new(generator.generate_with(&mut gs, x, z, beard)))
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                for h in handles {
                    chunks.extend(h.join().unwrap());
                }
            });
            if dump.structures {
                // Beardified terrain: compare, then start FEATURES from vanilla's.
                for (pos, c) in chunks.iter_mut() {
                    let vanilla = &region.terrain[pos];
                    let bad = c.blocks.iter().zip(vanilla).filter(|(a, b)| a != b).count() as u64;
                    terrain_bad += bad;
                    terrain_chunks_bad += (bad > 0) as usize;
                    if bad > 0 {
                        let mut fresh = ProtoChunk::new(c.x, c.z, c.min_y, c.sections(), c.biomes.clone());
                        for (i, &s) in vanilla.iter().enumerate() {
                            let (x, y, z) = (i & 15, fresh.min_y + (i >> 8) as i32, (i >> 4) & 15);
                            if s != 0 {
                                fresh.set(x, y, z, s);
                            }
                        }
                        fresh.finish_terrain();
                        **c = fresh;
                    }
                }
            }
            let terrain_post: HashMap<(i32, i32), Vec<Vec<u16>>> =
                region.targets.iter().map(|p| (*p, chunks[p].post_processing.clone())).collect();
            let mut gs = GenScratch::default();
            for d in &region.decorations {
                let chunk_starts = dump.structures.then(|| ChunkStarts::new(&structures, &generator, &cache, &mut sscratch, d.x, d.z));
                let window: Vec<Box<ProtoChunk>> = (0..9)
                    .map(|i| chunks.remove(&(d.x + i % 3 - 1, d.z + i / 3 - 1)).expect("terrain for the window"))
                    .collect();
                let mut r = Region::new(window, d.x, d.z, &generator, &mut gs);
                r.start_log();
                let mut cmp = Compare {
                    decorator: &decorator,
                    structures: &structures,
                    expected: &d.invocations,
                    next: 0,
                    replay,
                    list_errors: 0,
                    per_feature: BTreeMap::new(),
                    far_reads: 0,
                };
                decorator.decorate(&mut r, chunk_starts.as_ref().map(|s| (&structures, s)), &mut cmp);
                if cmp.next != d.invocations.len() {
                    cmp.list_errors += 1;
                }
                list_errors += cmp.list_errors;
                far_reads += cmp.far_reads;
                for (k, t) in cmp.per_feature {
                    all.entry(k).or_default().add(t);
                }
                assert_eq!(r.stats.far_writes, 0, "writes outside the window");
                for mut c in r.into_chunks() {
                    c.status = Status::Features;
                    chunks.insert((c.x, c.z), c);
                }
            }
            for ((x, z, vanilla), pending) in region.finals.iter().zip(&region.pending) {
                let c = &chunks[&(*x, *z)];
                let bad = c.blocks.iter().zip(vanilla).filter(|(a, b)| a != b).count() as u64;
                final_bad += bad;
                final_chunks_bad += (bad > 0) as usize;
                if let Some(v) = pending {
                    let k = Pending::of(c);
                    pending_checked += 1;
                    terrain_post_vanilla += v.terrain_post.iter().map(Vec::len).sum::<usize>();
                    if terrain_post[&(*x, *z)] != v.terrain_post {
                        terrain_post_bad += 1;
                        if terrain_post_bad <= 3 {
                            let mine = &terrain_post[&(*x, *z)];
                            let pos = |sec: usize, p: u16| (x * 16 + (p & 15) as i32, -64 + sec as i32 * 16 + ((p >> 4) & 15) as i32, z * 16 + (p >> 8) as i32);
                            let mut only_v = Vec::new();
                            let mut only_k = Vec::new();
                            for (sec, (a, b)) in v.terrain_post.iter().zip(mine).enumerate() {
                                only_v.extend(a.iter().filter(|p| !b.contains(p)).map(|&p| pos(sec, p)));
                                only_k.extend(b.iter().filter(|p| !a.contains(p)).map(|&p| pos(sec, p)));
                            }
                            only_v.truncate(6);
                            only_k.truncate(6);
                            eprintln!("    post-TERRAIN post-processing differs in {x},{z}: only vanilla {only_v:?}, only kiln {only_k:?}");
                        }
                    }
                    post_vanilla += v.post.iter().map(Vec::len).sum::<usize>();
                    ticks_vanilla += v.block_ticks.len() + v.fluid_ticks.len();
                    if k.post != v.post {
                        post_bad += 1;
                        if post_bad <= 3 {
                            let count = |p: &[Vec<u16>]| p.iter().map(Vec::len).sum::<usize>();
                            eprintln!("    post-processing differs in {x},{z}: vanilla {} positions, kiln {}", count(&v.post), count(&k.post));
                        }
                    }
                    if k.block_ticks != v.block_ticks || k.fluid_ticks != v.fluid_ticks {
                        ticks_bad += 1;
                        if ticks_bad <= 3 {
                            let only = |a: &[Tick], b: &[Tick]| a.iter().filter(|t| !b.contains(t)).take(4).cloned().collect::<Vec<_>>();
                            eprintln!(
                                "    ticks differ in {x},{z}: vanilla {}+{}, kiln {}+{}; only vanilla {:?} {:?}, only kiln {:?} {:?}",
                                v.block_ticks.len(),
                                v.fluid_ticks.len(),
                                k.block_ticks.len(),
                                k.fluid_ticks.len(),
                                only(&v.block_ticks, &k.block_ticks),
                                only(&v.fluid_ticks, &k.fluid_ticks),
                                only(&k.block_ticks, &v.block_ticks),
                                only(&k.fluid_ticks, &v.fluid_ticks)
                            );
                        }
                    }
                }
            }
        }
        eprintln!(
            "  {} regions: invocation list errors {list_errors}, vanilla far reads {far_reads}, final target blocks differing {final_bad} (in {final_chunks_bad} chunks)",
            dump.regions.len()
        );
        if pending_checked > 0 {
            eprintln!(
                "  pending updates of {pending_checked} targets: post-TERRAIN post-processing lists differ in {terrain_post_bad} ({terrain_post_vanilla} vanilla positions); final post-processing lists differ in {post_bad} ({post_vanilla} vanilla positions), scheduled ticks differ in {ticks_bad} ({ticks_vanilla} vanilla ticks)"
            );
        }
        if dump.structures {
            eprintln!("  beardified terrain: {terrain_bad} blocks differ in {terrain_chunks_bad} chunks (vanilla's used for FEATURES)");
            let gaps = structures.gaps();
            if !gaps.is_empty() {
                eprintln!("  unimplemented structure types (skipped attempts): {gaps:?}");
            }
        }
        total_bad += list_errors + final_bad;
    }
    if !starts_all.is_empty() {
        eprintln!("structure starts: vanilla / kiln / identical NBT; chunk references: vanilla / identical");
        for (id, t) in &starts_all {
            eprintln!("  {id:45} {:>6} {:>6} {:>6}   {:>6} {:>6}", t.vanilla, t.kiln, t.same, t.refs, t.refs_same);
            for f in &t.first {
                eprintln!("      {f}");
            }
        }
    }
    let mut by_type: BTreeMap<String, Tally> = BTreeMap::new();
    eprintln!("per placed feature: placements matched/mismatched/skipped (blocks in matched placements)");
    for (name, t) in &all {
        eprintln!("  {name:60} {:>7} {:>7}/{:>5}/{:>6} ({})", t.placements, t.matched, t.mismatched, t.skipped, t.blocks);
        for f in &t.first {
            eprintln!("      {f}");
        }
        let ty = name.rsplit('(').next().unwrap_or("").trim_end_matches(')').to_string();
        by_type.entry(ty).or_default().add(Tally { first: Vec::new(), ..t.clone() });
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
