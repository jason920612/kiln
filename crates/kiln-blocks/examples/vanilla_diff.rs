//! Replays `tools/blocks_diff.py`'s timeline on a `TestLevel` and compares every snapshot
//! with the vanilla server's saved chunks: block states and pending scheduled ticks inside
//! each scenario's box.
//!
//! usage: cargo run --release -p kiln-blocks --example vanilla_diff -- <diff dir>

use kiln_blocks::commands::{BlockInput, FillMode, SetMode, fill, setblock};
use kiln_blocks::state::{BlockId, state_string};
use kiln_blocks::ticks::{SavedTick, ticks_from_nbt};
use kiln_blocks::{BlockPos, FluidType, TestLevel};
use kiln_proto::nbt::{self, Tag};
use kiln_storage::AnvilSource;
use kiln_storage::region::RegionFile;
use kiln_world::chunk::Chunk;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

struct Snapshot {
    dir: PathBuf,
    chunks: HashMap<(i32, i32), Option<(Chunk, Tag)>>,
    anvil: AnvilSource,
}

impl Snapshot {
    fn new(dir: PathBuf) -> Self {
        let anvil = AnvilSource::new(&dir);
        Self { dir, chunks: HashMap::new(), anvil }
    }

    fn chunk(&mut self, cx: i32, cz: i32) -> Option<&(Chunk, Tag)> {
        let dir = self.dir.clone();
        let anvil = &mut self.anvil;
        self.chunks
            .entry((cx, cz))
            .or_insert_with(|| {
                let path = dir.join(format!("r.{}.{}.mca", cx >> 5, cz >> 5));
                let mut region = RegionFile::open(&path).ok()?;
                let data = region.read((cx & 31) as usize, (cz & 31) as usize).ok()??;
                let chunk = anvil.decode(&data, kiln_world::OVERWORLD).ok()?;
                let (_, tag) = nbt::read_named(&data).ok()?;
                Some((chunk, tag))
            })
            .as_ref()
    }

    fn block(&mut self, p: BlockPos) -> Option<u16> {
        self.chunk(p.x >> 4, p.z >> 4).map(|(c, _)| c.get((p.x & 15) as usize, p.y, (p.z & 15) as usize))
    }
}

type TickKey = (String, BlockPos, i32, i32);

/// Ticks inside the box in saved (sub-tick) order.
fn saved_keys<T>(ticks: &[SavedTick<T>], name: impl Fn(&T) -> String, min: BlockPos, max: BlockPos) -> Vec<TickKey> {
    ticks
        .iter()
        .filter(|t| inside(t.pos, min, max))
        .map(|t| (name(&t.kind), t.pos, t.delay, t.priority.value()))
        .collect()
}

fn inside(p: BlockPos, min: BlockPos, max: BlockPos) -> bool {
    (min.x..=max.x).contains(&p.x) && (min.y..=max.y).contains(&p.y) && (min.z..=max.z).contains(&p.z)
}

fn pos(v: &Value) -> BlockPos {
    let a: Vec<i32> = v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect();
    BlockPos::new(a[0], a[1], a[2])
}

fn run_command(level: &mut TestLevel, cmd: &str) -> Result<(), String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let int = |i: usize| parts.get(i).and_then(|s| s.parse::<i32>().ok()).ok_or(format!("bad coordinate in {cmd}"));
    match parts.first() {
        Some(&"setblock") => {
            let p = BlockPos::new(int(1)?, int(2)?, int(3)?);
            let input = BlockInput::parse(parts[4]).ok_or(format!("bad block in {cmd}"))?;
            let (mode, strict) = match parts.get(5) {
                None | Some(&"replace") => (SetMode::Replace, false),
                Some(&"destroy") => (SetMode::Destroy, false),
                Some(&"keep") => (SetMode::Keep, false),
                Some(&"strict") => (SetMode::Replace, true),
                Some(m) => return Err(format!("mode {m}")),
            };
            setblock(level, p, &input, mode, strict).map_err(|_| format!("could not set: {cmd}"))
        }
        Some(&"fill") => {
            let a = BlockPos::new(int(1)?, int(2)?, int(3)?);
            let b = BlockPos::new(int(4)?, int(5)?, int(6)?);
            let input = BlockInput::parse(parts[7]).ok_or(format!("bad block in {cmd}"))?;
            let mode = match parts.get(8) {
                None | Some(&"replace") => FillMode::Replace,
                Some(&"destroy") => FillMode::Destroy,
                Some(&"hollow") => FillMode::Hollow,
                Some(&"outline") => FillMode::Outline,
                Some(&"keep") => FillMode::Keep,
                Some(m) => return Err(format!("mode {m}")),
            };
            fill(level, a, b, &input, mode, false).map(|_| ()).map_err(|_| format!("no blocks filled: {cmd}"))
        }
        _ => Err(format!("unsupported command {cmd}")),
    }
}

struct Outcome {
    name: String,
    snapshots: usize,
    matched: usize,
    first_failure: Option<(i64, Vec<String>)>,
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: vanilla_diff <diff dir>"));
    let timeline: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("timeline.json")).expect("timeline.json")).unwrap();
    let layers: Vec<u16> = timeline["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| BlockId::by_name(l.as_str().unwrap()).unwrap().default_state())
        .collect();
    let mut level = TestLevel::flat(-64, 384, &layers);
    let fl: Vec<i32> = timeline["forceload"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
    level.load_chunks((fl[0] >> 4, fl[1] >> 4), (fl[2] >> 4, fl[3] >> 4));
    let mut outcomes: Vec<Outcome> = timeline["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Outcome { name: s["name"].as_str().unwrap().to_string(), snapshots: 0, matched: 0, first_failure: None })
        .collect();
    let boxes: Vec<(BlockPos, BlockPos)> = timeline["scenarios"].as_array().unwrap().iter().map(|s| (pos(&s["min"]), pos(&s["max"]))).collect();
    let mut tick = 0i64;
    let mut command_errors = Vec::new();
    for step in timeline["steps"].as_array().unwrap() {
        let t = step["tick"].as_i64().unwrap();
        while tick < t {
            level.tick(0, &[]);
            tick += 1;
        }
        for c in step["commands"].as_array().unwrap() {
            if let Err(e) = run_command(&mut level, c.as_str().unwrap()) {
                command_errors.push(format!("t{t}: {e}"));
            }
        }
        if !step["snapshot"].as_bool().unwrap() {
            continue;
        }
        let snap_dir = dir.join("snapshots").join(format!("t{t}"));
        if !snap_dir.exists() {
            eprintln!("missing snapshot {}", snap_dir.display());
            continue;
        }
        let mut snap = Snapshot::new(snap_dir);
        if let Ok(spec) = std::env::var("DIFF_DUMP") {
            dump(&spec, t, &timeline, &mut snap);
        }
        for (o, &(min, max)) in outcomes.iter_mut().zip(&boxes) {
            let diffs = compare(&mut level, &mut snap, min, max);
            o.snapshots += 1;
            if diffs.is_empty() {
                o.matched += 1;
            } else if o.first_failure.is_none() {
                o.first_failure = Some((t, diffs));
            }
        }
    }
    report(&dir, &outcomes, &command_errors);
}

/// `DIFF_DUMP=name:dy` prints the scenario's layer `dy` above the surface as vanilla saved
/// it: fluid levels and wire power as hex digits, other blocks by initial.
fn dump(spec: &str, t: i64, timeline: &Value, snap: &mut Snapshot) {
    let (name, dy) = spec.split_once(':').unwrap_or((spec, "0"));
    let Some(sc) = timeline["scenarios"].as_array().unwrap().iter().find(|s| s["name"] == name) else { return };
    let (min, max) = (pos(&sc["min"]), pos(&sc["max"]));
    let y = -60 + dy.parse::<i32>().unwrap_or(0);
    println!("{name} t{t} y{y}:");
    for z in min.z..=max.z {
        let row: String = (min.x..=max.x)
            .map(|x| {
                let s = snap.block(BlockPos::new(x, y, z)).unwrap_or(0);
                let n = BlockId::of(s).name().trim_start_matches("minecraft:");
                let lvl = kiln_blocks::state::get(s, "level").or(kiln_blocks::state::get(s, "power"));
                match (n, lvl) {
                    ("air", _) => ' ',
                    ("water" | "lava" | "redstone_wire", Some(l)) => std::char::from_digit(l.parse().unwrap(), 16).unwrap(),
                    _ => n.chars().next().unwrap().to_ascii_uppercase(),
                }
            })
            .collect();
        println!("  |{row}|");
    }
}

fn compare(level: &mut TestLevel, snap: &mut Snapshot, min: BlockPos, max: BlockPos) -> Vec<String> {
    let mut diffs = Vec::new();
    for y in min.y..=max.y {
        for z in min.z..=max.z {
            for x in min.x..=max.x {
                let p = BlockPos::new(x, y, z);
                let ours = kiln_blocks::Level::block(level, p);
                match snap.block(p) {
                    Some(v) if v == ours => {}
                    Some(v) => diffs.push(format!("{p:?}: vanilla {} kiln {}", state_string(v), state_string(ours))),
                    None => diffs.push(format!("{p:?}: chunk missing in vanilla snapshot")),
                }
            }
        }
    }
    for cx in min.x >> 4..=max.x >> 4 {
        for cz in min.z >> 4..=max.z >> 4 {
            let (mut vanilla, mut ours) = (Vec::new(), Vec::new());
            if let Some((_, tag)) = snap.chunk(cx, cz) {
                let empty = Tag::List(Vec::new());
                let bt = ticks_from_nbt(tag.get("block_ticks").unwrap_or(&empty), (cx, cz), |n| Some(n.to_string()));
                let ft = ticks_from_nbt(tag.get("fluid_ticks").unwrap_or(&empty), (cx, cz), |n| Some(n.to_string()));
                vanilla.extend(saved_keys(&bt, |k: &String| k.clone(), min, max));
                vanilla.extend(saved_keys(&ft, |k: &String| k.clone(), min, max));
            }
            let time = level.game_time;
            if let Some(c) = level.block_ticks.container((cx, cz)) {
                ours.extend(saved_keys(&c.pack(time), |k: &BlockId| k.name().to_string(), min, max));
            }
            if let Some(c) = level.fluid_ticks.container((cx, cz)) {
                ours.extend(saved_keys(&c.pack(time), |k: &FluidType| k.name().to_string(), min, max));
            }
            let (vs, os): (BTreeSet<_>, BTreeSet<_>) = (vanilla.iter().cloned().collect(), ours.iter().cloned().collect());
            for t in vs.difference(&os) {
                diffs.push(format!("tick only in vanilla: {} at {:?} in {} (priority {})", t.0, t.1, t.2, t.3));
            }
            for t in os.difference(&vs) {
                diffs.push(format!("tick only in kiln: {} at {:?} in {} (priority {})", t.0, t.1, t.2, t.3));
            }
            if vs == os && vanilla != ours {
                diffs.push(format!("ticks of chunk ({cx}, {cz}) in a different sub-tick order: vanilla {vanilla:?} kiln {ours:?}"));
            }
        }
    }
    diffs
}

fn report(dir: &Path, outcomes: &[Outcome], command_errors: &[String]) {
    let passed = outcomes.iter().filter(|o| o.first_failure.is_none() && o.snapshots > 0).count();
    let snaps: usize = outcomes.iter().map(|o| o.snapshots).sum();
    let matched: usize = outcomes.iter().map(|o| o.matched).sum();
    let mut text = String::new();
    for o in outcomes {
        match &o.first_failure {
            None => text.push_str(&format!("PASS {:<24} {}/{} snapshots\n", o.name, o.matched, o.snapshots)),
            Some((t, diffs)) => {
                text.push_str(&format!("FAIL {:<24} {}/{} snapshots, first at tick {t}:\n", o.name, o.matched, o.snapshots));
                for d in diffs.iter().take(12) {
                    text.push_str(&format!("       {d}\n"));
                }
                if diffs.len() > 12 {
                    text.push_str(&format!("       ... {} more\n", diffs.len() - 12));
                }
            }
        }
    }
    for e in command_errors {
        text.push_str(&format!("kiln command error {e}\n"));
    }
    text.push_str(&format!("\n{passed}/{} scenarios pass; {matched}/{snaps} scenario snapshots identical\n", outcomes.len()));
    print!("{text}");
    std::fs::write(dir.join("report.txt"), &text).ok();
    if passed != outcomes.len() {
        std::process::exit(1);
    }
}
