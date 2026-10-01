//! Compaction of native cell files runs on a background thread. These tests check that it
//! never loses or reorders a record: while the tick thread keeps flushing, when a compaction is
//! cut short in the middle of writing its copy, and when the whole process is killed at random
//! moments while it writes and compacts.

use kiln_storage::native::cellfile::{CHUNK, CellFile, CompactionPlan, Key, Record};
use kiln_storage::native::{FORM_NBT, cell_path};
use kiln_storage::{CompactionMode, NativeStore};
use kiln_world::ChunkPos;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tmp(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compaction-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Incompressible bytes fixed by (round, slot): what a chunk's record holds in that round.
fn payload(round: u64, slot: u8, len: usize) -> Vec<u8> {
    let mut x = round.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (slot as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03);
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

const SLOTS: u8 = 8;

/// Chunk (slot, 0) lies in cell (0, 0).
fn pos(slot: u8) -> ChunkPos {
    ChunkPos::new(slot as i32, 0)
}

fn write_round(store: &mut NativeStore, round: u64, len: usize) {
    for slot in 0..SLOTS {
        store.write(CHUNK, pos(slot), FORM_NBT, Some(&payload(round, slot, len)));
    }
    store.flush().unwrap();
}

fn read_slot(store: &mut NativeStore, slot: u8) -> Option<Vec<u8>> {
    store.read(CHUNK, pos(slot)).map(|(_, raw, _)| raw)
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".compact") || n.ends_with(".tmp"))
        .collect()
}

#[test]
fn flushing_while_compacting_in_the_background_never_loses_a_record() {
    let dir = tmp("flushing");
    let mut store = NativeStore::open(&dir);
    store.sync = false;
    let mut model: HashMap<u8, Vec<u8>> = HashMap::new();
    for round in 0..300u64 {
        // A few slots change per round, so appends land at every point of a compaction.
        let len = 20_000 + (round % 7) as usize * 3_000;
        for slot in 0..SLOTS {
            if (round + slot as u64) % 3 != 0 {
                let p = payload(round, slot, len);
                store.write(CHUNK, pos(slot), FORM_NBT, Some(&p));
                model.insert(slot, p);
            }
        }
        if round % 41 == 40 {
            // A chunk is deleted (an entity or POI chunk that went empty).
            store.write(CHUNK, pos(3), FORM_NBT, None);
            model.remove(&3);
        }
        store.flush().unwrap();
        for slot in 0..SLOTS {
            assert_eq!(read_slot(&mut store, slot).as_ref(), model.get(&slot), "round {round} slot {slot}");
        }
        if round % 16 == 0 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    store.finish_compactions();
    assert!(store.compaction.started >= 3, "{:?}", store.compaction);
    assert_eq!(store.compaction.failed, 0, "{:?}", store.compaction);
    assert!(store.compaction.finished >= 1, "{:?}", store.compaction);
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut store, slot).as_ref(), model.get(&slot));
    }
    drop(store);
    assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
    // The file on disk is complete: a fresh store reads every record, and the file is small.
    let mut store = NativeStore::open(&dir);
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut store, slot).as_ref(), model.get(&slot));
    }
    let len = std::fs::metadata(cell_path(&dir, (0, 0))).unwrap().len();
    let live: usize = model.values().map(Vec::len).sum();
    assert!(len < 4 * live as u64 + 300_000, "{len} bytes for {live} live");
}

#[test]
fn inline_and_background_compaction_store_the_same_records() {
    let a = tmp("mode-inline");
    let b = tmp("mode-background");
    let mut inline = NativeStore::open(&a);
    inline.mode = CompactionMode::Inline;
    let mut background = NativeStore::open(&b);
    for round in 0..60u64 {
        write_round(&mut inline, round, 30_000);
        write_round(&mut background, round, 30_000);
    }
    background.finish_compactions();
    assert!(inline.compaction.finished > 0 && background.compaction.finished > 0);
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut inline, slot), read_slot(&mut background, slot));
    }
}

#[test]
fn a_compaction_cut_short_leaves_every_record_and_the_copy_is_removed_on_open() {
    let dir = tmp("torn");
    {
        let mut store = NativeStore::open(&dir);
        store.mode = CompactionMode::Inline;
        for round in 0..2u64 {
            write_round(&mut store, round, 30_000);
        }
        // Stale records pile up below the compaction threshold.
        assert_eq!(store.compaction.started, 0);
    }
    let path = cell_path(&dir, (0, 0));
    // The process dies while writing the copy, at several places.
    let full = 16 + SLOTS as usize * (32 + 30_000);
    for cut in [0, 7, 16, 500, full / 2, full - 1] {
        let f = CellFile::open(&path).unwrap().unwrap();
        assert!(f.plan_compaction().run_torn(cut).is_err());
        drop(f);
        assert!(CompactionPlan::temp_path(&path).exists());
        let mut store = NativeStore::open(&dir);
        assert!(!CompactionPlan::temp_path(&path).exists(), "cut at {cut}: the copy is removed when a store opens");
        for slot in 0..SLOTS {
            assert_eq!(read_slot(&mut store, slot), Some(payload(1, slot, 30_000)), "cut at {cut}");
        }
    }
    // The copy is complete but the process dies before the rename: same.
    let f = CellFile::open(&path).unwrap().unwrap();
    let _copy = f.plan_compaction().run(true).unwrap();
    drop(f);
    let mut store = NativeStore::open(&dir);
    assert!(leftovers(&dir).is_empty());
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut store, slot), Some(payload(1, slot, 30_000)));
    }
    // And the store goes on compacting afterwards.
    for round in 2..40u64 {
        write_round(&mut store, round, 30_000);
    }
    store.finish_compactions();
    assert!(store.compaction.finished > 0);
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut store, slot), Some(payload(39, slot, 30_000)));
    }
}

#[test]
fn the_cell_file_is_replaced_while_its_compaction_runs() {
    // The last records of a cell are deleted while a copy of it is being made; the copy must not
    // bring them back, and a new file for the cell must not be confused with the old one.
    let dir = tmp("emptied");
    let mut store = NativeStore::open(&dir);
    store.sync = false;
    for round in 0..8u64 {
        write_round(&mut store, round, 40_000);
    }
    for slot in 0..SLOTS {
        store.write(CHUNK, pos(slot), FORM_NBT, None);
    }
    store.flush().unwrap();
    store.finish_compactions();
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut store, slot), None);
    }
    assert!(!cell_path(&dir, (0, 0)).exists(), "an emptied cell has no file");
    write_round(&mut store, 100, 10_000);
    drop(store);
    let mut store = NativeStore::open(&dir);
    for slot in 0..SLOTS {
        assert_eq!(read_slot(&mut store, slot), Some(payload(100, slot, 10_000)));
    }
}

/// The child of the kill test: writes rounds until it is killed, telling its parent about each
/// flush that returned.
#[test]
fn kill_child_writer() {
    let Some(dir) = std::env::var_os("KILN_COMPACT_CHILD") else { return };
    let start: u64 = std::env::var("KILN_COMPACT_START").unwrap().parse().unwrap();
    let mut store = NativeStore::open(PathBuf::from(dir));
    store.sync = false;
    for round in start.. {
        // Round sizes vary so compactions overlap flushes of different sizes.
        write_round(&mut store, round, 60_000 + (round % 5) as usize * 20_000);
        println!("ack {round}");
    }
}

#[test]
fn killing_the_process_at_random_moments_loses_nothing_that_was_flushed() {
    let dir = tmp("kill");
    let exe = std::env::current_exe().unwrap();
    let mut next_round = 0u64;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut with_copy = 0;
    for attempt in 0..14 {
        let mut child = Command::new(&exe)
            .args(["--exact", "kill_child_writer", "--nocapture", "--test-threads=1"])
            .env("KILN_COMPACT_CHILD", &dir)
            .env("KILN_COMPACT_START", next_round.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        // Let it flush some rounds (and start compactions), then cut it off after a random pause.
        let acks_wanted = 4 + rand() % 40;
        let mut last_ack = None;
        let mut acks = 0;
        while acks < acks_wanted {
            let Some(Ok(line)) = lines.next() else { break };
            if let Some(n) = line.strip_prefix("ack ") {
                last_ack = Some(n.parse::<u64>().unwrap());
                acks += 1;
            }
        }
        std::thread::sleep(std::time::Duration::from_micros(rand() % 30_000));
        child.kill().unwrap();
        child.wait().unwrap();
        // Lines the child managed to print before it died count too.
        for line in lines.map_while(Result::ok) {
            if let Some(n) = line.strip_prefix("ack ") {
                last_ack = Some(n.parse::<u64>().unwrap());
            }
        }
        let Some(acked) = last_ack else { continue };
        if leftovers(&dir).iter().any(|n| n.ends_with(".compact")) {
            with_copy += 1;
        }
        let mut store = NativeStore::open(&dir);
        // Every flush that returned is there; one that was cut off is there whole or not at all.
        let state = read_slot(&mut store, 0).expect("record 0");
        let round = [acked, acked + 1]
            .into_iter()
            .find(|&r| state == payload(r, 0, 60_000 + (r % 5) as usize * 20_000))
            .unwrap_or_else(|| panic!("attempt {attempt}: record 0 is not the state of round {acked} or {}", acked + 1));
        for slot in 0..SLOTS {
            let len = 60_000 + (round % 5) as usize * 20_000;
            assert_eq!(read_slot(&mut store, slot), Some(payload(round, slot, len)), "attempt {attempt} slot {slot}");
        }
        drop(store);
        assert!(leftovers(&dir).is_empty(), "{:?}", leftovers(&dir));
        next_round = round + 1;
    }
    println!("{next_round} rounds; killed with a compaction copy on disk in {with_copy} attempts");
    // Every record is still readable through the raw file (index and checksums consistent).
    let mut f = CellFile::open(&cell_path(&dir, (0, 0))).unwrap().unwrap();
    for slot in 0..SLOTS {
        let rec: Record = f.read(Key { kind: CHUNK, slot }).unwrap().unwrap();
        assert!(rec.raw_len > 0);
    }
}
