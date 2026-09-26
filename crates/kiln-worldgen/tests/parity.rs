//! Bit-exact parity against vectors dumped from the vanilla server by
//! `tools/worldgen_vectors.py`. Skips (with a message) when the vectors or the datapack are
//! absent: both are Mojang-derived and live in the untracked work directory.
//!
//! It compares millions of values, so it only runs when `KILN_PARITY=1` (best with
//! `--release`): `KILN_PARITY=1 cargo test -p kiln-worldgen --release --test parity`.
//!
//! Environment: `KILN_WORK` (default `<workspace>/work`), `KILN_WORLDGEN_VECTORS` (default
//! `<work>/wp2-worldgen/vectors`).

use kiln_worldgen::noise::NoiseStack;
use kiln_worldgen::{Datapack, NoiseRouter, RandomState, SamplerRef, Scratch, Volume};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const CORNERS: usize = 5 * 49 * 5;

fn work_dir() -> PathBuf {
    match std::env::var_os("KILN_WORK") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
    }
}

fn inputs() -> Option<(Datapack, PathBuf)> {
    if std::env::var_os("KILN_PARITY").is_none_or(|v| v != "1") {
        eprintln!("skipping worldgen parity: set KILN_PARITY=1 to run it");
        return None;
    }
    let work = work_dir();
    let vectors = std::env::var_os("KILN_WORLDGEN_VECTORS")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("wp2-worldgen/vectors"));
    let generated = work.join("generated");
    if !vectors.is_dir() || !generated.join("data/minecraft/worldgen").is_dir() {
        eprintln!(
            "skipping worldgen parity: need {} (tools/worldgen_vectors.py) and {} (cargo xtask data)",
            vectors.display(),
            generated.display()
        );
        return None;
    }
    let pack = Datapack::load(&generated).expect("load datapack");
    Some((pack, vectors))
}

fn files(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
        .collect();
    v.sort();
    v
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
    fn u32(&mut self) -> u32 {
        self.i32() as u32
    }
    fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    fn f64(&mut self) -> f64 {
        f64::from_bits(self.i64() as u64)
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    fn str(&mut self) -> String {
        let n = self.i32() as usize;
        String::from_utf8(self.take(n).to_vec()).unwrap()
    }
    fn magic(&mut self, m: &[u8]) {
        assert_eq!(self.take(4), m, "bad magic");
        assert_eq!(self.i32(), 1, "unsupported vector version");
    }
}

/// Bitwise equality, except that any two NaNs match.
fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[derive(Default)]
struct Tally {
    compared: usize,
    mismatched: usize,
    first: Option<String>,
}

impl Tally {
    fn check(&mut self, expected: f32, actual: f32, what: impl FnOnce() -> String) {
        self.compared += 1;
        if !same(expected, actual) {
            self.mismatched += 1;
            if self.first.is_none() {
                self.first = Some(format!("{}: vanilla {expected:e} ({:#010x}), kiln {actual:e}", what(), expected.to_bits()));
            }
        }
    }
}

fn report(title: &str, tallies: &BTreeMap<String, Tally>) -> usize {
    let mut bad = 0;
    eprintln!("{title}");
    for (name, t) in tallies {
        eprintln!("  {name:48} {:>9} compared {:>7} mismatched", t.compared, t.mismatched);
        if let Some(f) = &t.first {
            eprintln!("      first: {f}");
        }
        bad += t.mismatched;
    }
    bad
}

fn corner_volume(cx: i32, cz: i32) -> Volume {
    Volume::new([5, 49, 5], [cx * 16, -64, cz * 16], [4, 8, 4])
}

/// Evaluates `sampler` on every chunk's corner grid in point or volume mode, in parallel.
fn evaluate(sampler: &SamplerRef, chunks: &[(i32, i32)], point: bool) -> Vec<f32> {
    let mut out = vec![0f32; chunks.len() * CORNERS];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let per = chunks.len().div_ceil(threads);
    std::thread::scope(|scope| {
        for (chunk_group, out_group) in chunks.chunks(per).zip(out.chunks_mut(per * CORNERS)) {
            scope.spawn(move || {
                let mut scratch = Scratch::default();
                for (&(cx, cz), dst) in chunk_group.iter().zip(out_group.chunks_mut(CORNERS)) {
                    let vol = corner_volume(cx, cz);
                    if point {
                        for (d, [x, y, z]) in dst.iter_mut().zip(vol.positions()) {
                            *d = sampler.point(&mut scratch, x, y, z);
                        }
                    } else {
                        sampler.fill(&mut scratch, &vol, dst);
                    }
                }
            });
        }
    });
    out
}

/// Compares a `functions_*` / `router_*` file; returns mismatches in the P and V modes.
fn check_functions(path: &Path, pack: &Datapack, compile: impl Fn(&mut RandomState, &[String]) -> Vec<SamplerRef>) -> usize {
    let bytes = fs::read(path).unwrap();
    let mut r = Reader { b: &bytes, i: 0 };
    r.magic(b"KWGV");
    let seed = r.i64();
    let names: Vec<String> = (0..r.i32()).map(|_| r.str()).collect();
    let chunks: Vec<(i32, i32)> = (0..r.i32()).map(|_| (r.i32(), r.i32())).collect();
    let settings = pack.settings("minecraft:overworld").unwrap();
    let mut state = RandomState::new(seed, settings.legacy_random_source);
    let samplers = compile(&mut state, &names);
    let mut strict_bad = 0;
    // Vanilla's own point values, to count where its volume modes round differently.
    let mut vanilla_point: Vec<Vec<f32>> = Vec::new();
    let mut mode_differences: BTreeMap<String, Tally> = BTreeMap::new();
    for _ in 0..r.i32() {
        let mode = char::from_u32(r.u32()).unwrap();
        let mut tallies = BTreeMap::new();
        for (k, (name, sampler)) in names.iter().zip(&samplers).enumerate() {
            let actual = evaluate(sampler, &chunks, mode == 'P');
            let t: &mut Tally = tallies.entry(name.clone()).or_default();
            if mode == 'P' {
                vanilla_point.push(Vec::with_capacity(actual.len()));
            }
            for (i, a) in actual.iter().enumerate() {
                let expected = r.f32();
                if mode == 'P' {
                    vanilla_point[k].push(expected);
                } else if !vanilla_point.is_empty() {
                    let d = mode_differences.entry(format!("{name} (P vs {mode})")).or_default();
                    d.check(vanilla_point[k][i], expected, || format!("index {i}"));
                }
                t.check(expected, *a, || {
                    let (cx, cz) = chunks[i / CORNERS];
                    let p = corner_volume(cx, cz).positions().nth(i % CORNERS).unwrap();
                    format!("{name} at {p:?}")
                });
            }
        }
        let title = match mode {
            'P' => "point (sampleValue, uncached)",
            'V' => "volume (sampleVolume, uncached)",
            _ => "volume with caching context (kiln evaluates uncached; informational)",
        };
        let bad = report(&format!("{} seed {seed}: {title}", path.file_name().unwrap().to_string_lossy()), &tallies);
        if mode != 'C' {
            strict_bad += bad;
        }
    }
    let file = path.file_name().unwrap().to_string_lossy();
    report(&format!("{file} seed {seed}: where vanilla's volume modes differ from its point mode (informational)"), &mode_differences);
    strict_bad
}

#[test]
fn noise_instances_match_vanilla() {
    let Some((pack, dir)) = inputs() else { return };
    let mut bad = 0;
    for path in files(&dir, "noise_") {
        let bytes = fs::read(&path).unwrap();
        let mut r = Reader { b: &bytes, i: 0 };
        r.magic(b"KWGN");
        let seed = r.i64();
        let mut state = RandomState::new(seed, false);
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        for _ in 0..r.i32() {
            let name = r.str();
            let noise: std::sync::Arc<NoiseStack> = state.noise(&pack.graph, &name).unwrap();
            let t = tallies.entry(format!("{name} get3")).or_default();
            for _ in 0..r.i32() {
                let (x, y, z) = (r.f64(), r.f64(), r.f64());
                t.check(r.f32(), noise.get3(x, y, z), || format!("({x}, {y}, {z})"));
            }
            let t = tallies.entry(format!("{name} get2")).or_default();
            for _ in 0..r.i32() {
                let (x, z) = (r.f64(), r.f64());
                t.check(r.f32(), noise.get2(x, z), || format!("({x}, {z})"));
            }
            let t = tallies.entry(format!("{name} addToVolume")).or_default();
            for _ in 0..r.i32() {
                let p: Vec<i32> = (0..9).map(|_| r.i32()).collect();
                let vol = Volume::new([p[0], p[1], p[2]], [p[3], p[4], p[5]], [p[6], p[7], p[8]]);
                let (xz, y, amp) = (r.f64(), r.f64(), r.f32());
                let mut buf = vec![0.25f32; vol.len()];
                noise.add_to_volume(&mut buf, &vol, xz, y, amp);
                for (i, a) in buf.iter().enumerate() {
                    t.check(r.f32(), *a, || format!("{vol:?} scale ({xz}, {y}) index {i}"));
                }
            }
        }
        bad += report(&format!("noise instances, seed {seed}"), &tallies);
    }
    assert_eq!(bad, 0, "noise values differ from vanilla");
}

#[test]
fn density_functions_match_vanilla() {
    let Some((pack, dir)) = inputs() else { return };
    let mut bad = 0;
    for path in files(&dir, "functions_") {
        bad += check_functions(&path, &pack, |state, names| {
            let roots: Vec<_> = names.iter().map(|n| pack.graph.function(n).expect(n)).collect();
            state.compile(&pack.graph, &roots).unwrap()
        });
    }
    assert_eq!(bad, 0, "density function values differ from vanilla");
}

#[test]
fn noise_router_matches_vanilla() {
    let Some((pack, dir)) = inputs() else { return };
    let mut bad = 0;
    for path in files(&dir, "router_") {
        bad += check_functions(&path, &pack, |state, names| {
            let (_, router) = NoiseRouter::new(&pack, "minecraft:overworld", state.seed()).unwrap();
            names.iter().map(|n| router.get(n).unwrap_or_else(|| panic!("no router output {n}")).clone()).collect()
        });
    }
    assert_eq!(bad, 0, "noise router values differ from vanilla");
}
