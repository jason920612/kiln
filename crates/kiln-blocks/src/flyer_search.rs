//! A search for self-propelling flying machines (`cargo test --release -p kiln-blocks --lib flyer_search -- --ignored --nocapture`):
//! every line of up to `KILN_FLYER_LEN` cells (air, slime, pistons, observers; the machine travels east), run for a hundred ticks on a
//! void level; the ones that keep moving print their layout. kiln-blocks' pistons and observers are vanilla's (see the piston vectors),
//! so a layout that flies here is a candidate for the vanilla recording of `tools/InteractVectors.java` (`flyer55`).

use crate::level::{Level, flags};
use crate::pos::BlockPos;
use crate::state;
use crate::test_level::TestLevel;
use kiln_data::blocks::default_state as d;
use kiln_data::blocks_types::is_air;

fn cell(i: usize) -> u16 {
    match i {
        8 => state::parse_state("minecraft:observer[facing=north,powered=false]").unwrap(),
        9 => state::parse_state("minecraft:observer[facing=south,powered=false]").unwrap(),
        10 => state::parse_state("minecraft:observer[facing=up,powered=false]").unwrap(),
        11 => state::parse_state("minecraft:observer[facing=down,powered=false]").unwrap(),
        0 => d::AIR,
        1 => d::SLIME_BLOCK,
        2 => state::parse_state("minecraft:sticky_piston[facing=east,extended=false]").unwrap(),
        3 => state::parse_state("minecraft:sticky_piston[facing=west,extended=false]").unwrap(),
        4 => state::parse_state("minecraft:piston[facing=east,extended=false]").unwrap(),
        5 => state::parse_state("minecraft:piston[facing=west,extended=false]").unwrap(),
        6 => state::parse_state("minecraft:observer[facing=east,powered=false]").unwrap(),
        _ => state::parse_state("minecraft:observer[facing=west,powered=false]").unwrap(),
    }
}

const KINDS: usize = 12;

fn run(layout: &[usize], ticks: i32) -> Option<(i32, i32)> {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let y = 100;
    let mut pieces = 0;
    for (i, &c) in layout.iter().enumerate() {
        let s = cell(c);
        if !is_air(s) {
            pieces += 1;
            level.set_raw(BlockPos::new(16 + i as i32, y, 8), s, flags::SKIP_ALL_SIDEEFFECTS);
        }
    }
    // Whatever stands there gets a neighbour update: observers see it, pistons check their power.
    for (i, &c) in layout.iter().enumerate() {
        let p = BlockPos::new(16 + i as i32, y, 8);
        let s = level.block(p);
        if !is_air(s) {
            crate::update::set_block_and_update(&mut level, p, s);
            let _ = c;
        }
    }
    let mut first = None;
    for t in 1..=ticks {
        level.tick(0, &[(0, 0), (1, 0)]);
        if t == 40 || t == ticks {
            let (mut min, mut count) = (i32::MAX, 0);
            for x in 0..64 {
                let s = level.block(BlockPos::new(x, y, 8));
                if !is_air(s) && !state::is(s, d::PISTON_HEAD) && !state::is(s, d::MOVING_PISTON) {
                    min = min.min(x);
                    count += 1;
                }
            }
            if count != pieces {
                return None;
            }
            if t == 40 {
                first = Some(min);
            } else {
                return first.map(|f| (f, min));
            }
        }
    }
    None
}

#[test]
#[ignore]
fn flyer_search() {
    let max_len: usize = std::env::var("KILN_FLYER_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let mut found = 0;
    // (a line along the flight: observers that look east or west only)
    const LINE_KINDS: usize = 8;
    for len in 2..=max_len {
        let total = LINE_KINDS.pow(len as u32);
        for n in 0..total {
            let mut layout = vec![0usize; len];
            let mut m = n;
            for c in layout.iter_mut() {
                *c = m % LINE_KINDS;
                m /= LINE_KINDS;
            }
            // a machine has pistons and a way to start them again
            let has = |ks: &[usize]| layout.iter().any(|c| ks.contains(c));
            if !has(&[2, 3, 4, 5]) || !has(&[6, 7]) || layout[0] == 0 || layout[len - 1] == 0 {
                continue;
            }
            if let Some((a, b)) = run2([&layout, &[]], 120)
                && (b - a).abs() >= 3
            {
                found += 1;
                println!("flies {} blocks between tick 40 and 120: {:?} (0 air, 1 slime, 2/3 sticky east/west, 4/5 piston east/west, 6/7 observer east/west)", b - a, layout);
            }
        }
    }
    println!("{found} layouts fly");
}

/// Two rows (z = 8 and 9) of `len` cells; early exit for layouts that settle.
fn run2(rows: [&[usize]; 2], ticks: i32) -> Option<(i32, i32)> {
    // Every piece in turn is the one placed last (the machine is started by placing a block).
    let spots: Vec<(usize, usize)> = (0..2).flat_map(|z| (0..rows[z].len()).map(move |i| (z, i))).filter(|&(z, i)| rows[z][i] != 0).collect();
    if let Some(r) = spots.iter().find_map(|&last| run2_from(rows, ticks, last)) {
        return Some(r);
    }
    // or the machine stands, and a block is put beside it and taken away again
    let w = rows[0].len().max(rows[1].len());
    (0..2usize).flat_map(|z| (0..w).map(move |i| (z, i))).filter(|&(z, i)| rows[z].get(i).is_none_or(|&c| c == 0)).find_map(|tapped| run2_with(rows, ticks, tapped, true))
}

/// `last` is the piece placed last, or (when `tap` is set) an empty cell where a block is put and taken away again after the machine stands.
fn run2_from(rows: [&[usize]; 2], ticks: i32, last: (usize, usize)) -> Option<(i32, i32)> {
    run2_with(rows, ticks, last, false)
}

fn run2_with(rows: [&[usize]; 2], ticks: i32, last: (usize, usize), tap: bool) -> Option<(i32, i32)> {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let y = 100;
    let mut pieces = 0;
    let mut spots = Vec::new();
    let mut trigger = None;
    for (z, row) in rows.iter().enumerate() {
        for (i, &c) in row.iter().enumerate() {
            let s = cell(c);
            if !is_air(s) {
                pieces += 1;
                let p = if std::env::var_os("KILN_FLYER_VERTICAL").is_some() { BlockPos::new(16 + i as i32, y + z as i32, 8) } else { BlockPos::new(16 + i as i32, y, 8 + z as i32) };
                if (z, i) == last && !tap {
                    trigger = Some((p, s));
                } else {
                    level.set_raw(p, s, flags::SKIP_ALL_SIDEEFFECTS);
                }
                spots.push(p);
            }
        }
    }
    if tap {
        let vertical = std::env::var_os("KILN_FLYER_VERTICAL").is_some();
        let (z, i) = last;
        let p = if vertical { BlockPos::new(16 + i as i32, y + z as i32, 8) } else { BlockPos::new(16 + i as i32, y, 8 + z as i32) };
        crate::update::set_block_and_update(&mut level, p, d::STONE);
        level.tick(0, &[(0, 0), (1, 0)]);
        crate::update::set_block_and_update(&mut level, p, d::AIR);
    }
    // (the blocks of the machine were placed without updates; the last one is placed the way a player does)
    if let Some((p, s)) = trigger {
        crate::update::set_block_and_update(&mut level, p, s);
    }
    let measure = |level: &TestLevel| {
        let (mut min, mut count, mut h) = (i32::MAX, 0, 0u64);
        for x in 0..80 {
            for z in 6..12 {
                for yy in 98..103 {
                    let s = level.block(BlockPos::new(x, yy, z));
                    if !is_air(s) && !state::is(s, d::PISTON_HEAD) && !state::is(s, d::MOVING_PISTON) {
                        min = min.min(x);
                        count += 1;
                        h = h.wrapping_mul(1_000_003).wrapping_add(s as u64 + (x as u64) * 77 + (z as u64) * 13 + yy as u64);
                    }
                }
            }
        }
        (min, count, h)
    };
    let (mut at20, mut first) = (0u64, 0);
    for t in 1..=ticks {
        level.tick(0, &[(0, 0), (1, 0)]);
        if t == 20 || t == 30 || t == 40 || t == ticks {
            let (min, count, h) = measure(&level);
            if count != pieces {
                return None;
            }
            if t == 20 {
                at20 = h;
            }
            if t == 30 && h == at20 {
                return None;
            }
            if t == 40 {
                first = min;
            }
            if t == ticks {
                return Some((first, min));
            }
        }
    }
    None
}

#[test]
#[ignore]
fn flyer_search2() {
    let len: usize = std::env::var("KILN_FLYER_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let threads: usize = std::env::var("KILN_FLYER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let total = KINDS.pow((2 * len) as u32);
    let found = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for t in 0..threads {
            let found = &found;
            sc.spawn(move || {
                let mut n = t;
                while n < total {
                    let mut cells = vec![0usize; 2 * len];
                    let mut m = n;
                    for c in cells.iter_mut() {
                        *c = m % KINDS;
                        m /= KINDS;
                    }
                    n += threads;
                    let (a, b) = cells.split_at(len);
                    let has = |ks: &[usize]| cells.iter().any(|c| ks.contains(c));
                    if !has(&[2, 3, 4, 5]) || !has(&[6, 7, 8, 9, 10, 11]) || !has(&[1]) {
                        continue;
                    }
                    if let Some((x, y)) = run2([a, b], 120)
                        && (y - x).abs() >= 3
                    {
                        found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        println!("flies {} blocks: row0 {:?} row1 {:?} (0 air, 1 slime, 2/3 sticky east/west, 4/5 piston east/west, 6/7/8/9/10/11 observer east/west/north/south/up/down)", y - x, a, b);
                    }
                }
            });
        }
    });
    println!("{} layouts fly", found.load(std::sync::atomic::Ordering::Relaxed));
}

/// Two sticky pistons, two observers and `slimes` slime blocks placed anywhere in a `w` x 2 grid.
#[test]
#[ignore]
fn flyer_search3() {
    let w: usize = std::env::var("KILN_FLYER_W").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
    let slimes: usize = std::env::var("KILN_FLYER_SLIMES").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let threads: usize = std::env::var("KILN_FLYER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let cells = 2 * w;
    let obs: [usize; 4] = std::env::var("KILN_FLYER_OBS").ok().map_or([6, 7, 8, 9], |v| {
        let l: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect();
        [l[0], l[1], l[2], l[3]]
    });
    // pieces: 2 sticky pistons (kinds 2/3), 2 observers (6..9), slimes (1)
    let pieces = 4 + slimes;
    let total: usize = (0..pieces).fold(1, |a, i| a * (cells - i));
    let found = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for t in 0..threads {
            let found = &found;
            sc.spawn(move || {
                let mut n = t;
                while n < total {
                    // positions of the pieces: a permutation prefix
                    let mut m = n;
                    n += threads;
                    let mut free: Vec<usize> = (0..cells).collect();
                    let mut at = Vec::new();
                    for i in 0..pieces {
                        let k = m % (cells - i);
                        m /= cells - i;
                        at.push(free.remove(k));
                    }
                    // pieces 0 and 1 are the pistons (0 < 1 to skip mirror duplicates), 2 and 3 observers, the rest slime
                    if at[0] > at[1] || at[2] > at[3] {
                        continue;
                    }
                    // (every piston is a sticky or a regular one, facing east or west)
                    for facing in 0..256usize {
                        let (p0, p1, o0, o1) = (facing & 3, (facing >> 2) & 3, (facing >> 4) & 3, (facing >> 6) & 3);
                        let mut grid = vec![0usize; cells];
                        grid[at[0]] = 2 + p0;
                        grid[at[1]] = 2 + p1;
                        grid[at[2]] = obs[o0];
                        grid[at[3]] = obs[o1];
                        for &s in &at[4..] {
                            grid[s] = 1;
                        }
                        let (a, b) = grid.split_at(w);
                        if let Some((x, y)) = run2([a, b], 100)
                            && (y - x).abs() >= 3
                        {
                            found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            println!("flies {} blocks: row0 {:?} row1 {:?} (0 air, 1 slime, 2/3 sticky east/west, 4/5 piston east/west, 6/7/8/9 observer east/west/north/south)", y - x, a, b);
                        }
                    }
                }
            });
        }
    });
    println!("{} layouts fly", found.load(std::sync::atomic::Ordering::Relaxed));
}

/// The search is only as good as its pistons: an observer's pulse extends the piston it points away from, then it retracts.
#[test]
fn observer_pulse_runs_a_piston() {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let y = 100;
    level.set_raw(BlockPos::new(10, y, 8), state::parse_state("minecraft:sticky_piston[facing=east,extended=false]").unwrap(), flags::SKIP_ALL_SIDEEFFECTS);
    level.set_raw(BlockPos::new(9, y, 8), state::parse_state("minecraft:observer[facing=west,powered=false]").unwrap(), flags::SKIP_ALL_SIDEEFFECTS);
    // a block put where the observer looks
    crate::update::set_block_and_update(&mut level, BlockPos::new(8, y, 8), d::STONE);
    let mut extended_at = None;
    for t in 1..=12 {
        level.tick(0, &[(0, 0), (1, 0)]);
        let s = level.block(BlockPos::new(10, y, 8));
        if state::get_bool(s, "extended") && extended_at.is_none() {
            extended_at = Some(t);
        }
    }
    assert!(extended_at.is_some(), "the observer's pulse never extended the piston");
    assert!(!state::get_bool(level.block(BlockPos::new(10, y, 8)), "extended"), "it retracted again");
}

/// What a layout does for 120 ticks: how many pieces survive, where its west edge is at tick 40 and 120, and whether it still changes at the end.
fn outcome(rows: [&[usize]; 2], last: (usize, usize), tap: bool) -> (usize, usize, i32, i32, bool, usize, Vec<(usize, usize)>) {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let y = 100;
    let (mut pieces, mut trigger) = (0, None);
    for (z, row) in rows.iter().enumerate() {
        for (i, &c) in row.iter().enumerate() {
            let s = cell(c);
            if !is_air(s) {
                pieces += 1;
                let p = BlockPos::new(16 + i as i32, y, 8 + z as i32);
                if (z, i) == last && !tap {
                    trigger = Some((p, s));
                } else {
                    level.set_raw(p, s, flags::SKIP_ALL_SIDEEFFECTS);
                }
            }
        }
    }
    if tap {
        let p = BlockPos::new(16 + last.1 as i32, y, 8 + last.0 as i32);
        crate::update::set_block_and_update(&mut level, p, d::STONE);
        level.tick(0, &[(0, 0), (1, 0)]);
        crate::update::set_block_and_update(&mut level, p, d::AIR);
    }
    if let Some((p, s)) = trigger {
        crate::update::set_block_and_update(&mut level, p, s);
    }
    let measure = |level: &TestLevel| {
        let (mut min, mut count, mut h) = (i32::MAX, 0usize, 0u64);
        for x in 0..80 {
            for z in 6..12 {
                for yy in 98..103 {
                    let s = level.block(BlockPos::new(x, yy, z));
                    if !is_air(s) && !state::is(s, d::PISTON_HEAD) && !state::is(s, d::MOVING_PISTON) {
                        min = min.min(x);
                        count += 1;
                        h = h.wrapping_mul(1_000_003).wrapping_add(s as u64 + (x as u64) * 77 + (z as u64) * 13 + yy as u64);
                    }
                }
            }
        }
        (min, count, h)
    };
    let (mut a, mut h_before) = (0, 0);
    // the cells of the starting slime blocks that are not slime at some tick (the slime moved away: a rider there is carried)
    let start_slimes: Vec<(usize, usize)> = (0..2).flat_map(|z| (0..rows[z].len()).map(move |i| (z, i))).filter(|&(z, i)| rows[z][i] == 1).collect();
    let mut left = std::collections::BTreeSet::new();
    // where the slime blocks are, every tick from the 40th: how many different arrangements there are
    let mut slime_sets = std::collections::BTreeSet::new();
    for t in 1..=120 {
        level.tick(0, &[(0, 0), (1, 0)]);
        for &(z, i) in &start_slimes {
            if !state::is(level.block(BlockPos::new(16 + i as i32, y, 8 + z as i32)), d::SLIME_BLOCK) {
                left.insert((z, i));
            }
        }
        if t >= 40 {
            let mut at = Vec::new();
            for x in 0..80 {
                for z in 6..12 {
                    if state::is(level.block(BlockPos::new(x, 100, z)), d::SLIME_BLOCK) {
                        at.push((x, z));
                    }
                }
            }
            slime_sets.insert(at);
        }
        if t == 40 {
            a = measure(&level).0;
        }
        if t == 110 {
            h_before = measure(&level).2;
        }
    }
    let (min, count, h) = measure(&level);
    (pieces, count, a, min, h != h_before, slime_sets.len(), left.into_iter().collect())
}

/// Layouts of the `flyer_search3` family that stay together and keep changing (for the vanilla check of `tools/FlyerCheck.java`).
#[test]
#[ignore]
fn flyer_dump() {
    let w: usize = std::env::var("KILN_FLYER_W").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let threads: usize = std::env::var("KILN_FLYER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let path = std::env::var("KILN_FLYER_DUMP").unwrap_or_else(|_| "/tmp/flyer_dump.jsonl".into());
    let cells = 2 * w;
    let pieces = 6;
    let all = std::env::var_os("KILN_FLYER_ALL").is_some();
    let total: usize = (0..pieces).fold(1, |a, i| a * (cells - i));
    let out = std::sync::Mutex::new(std::fs::File::create(path).unwrap());
    let (runs, kept) = (std::sync::atomic::AtomicUsize::new(0), std::sync::atomic::AtomicUsize::new(0));
    std::thread::scope(|sc| {
        for t in 0..threads {
            let (out, runs, kept) = (&out, &runs, &kept);
            sc.spawn(move || {
                use std::io::Write;
                let mut n = t;
                while n < total {
                    let mut m = n;
                    n += threads;
                    let mut free: Vec<usize> = (0..cells).collect();
                    let mut at = Vec::new();
                    for i in 0..pieces {
                        let k = m % (cells - i);
                        m /= cells - i;
                        at.push(free.remove(k));
                    }
                    if at[0] > at[1] || at[2] > at[3] {
                        continue;
                    }
                    for facing in 0..64usize {
                        let (p0, p1, o0, o1) = (facing & 1, (facing >> 1) & 1, (facing >> 2) & 3, (facing >> 4) & 3);
                        let mut grid = vec![0usize; cells];
                        grid[at[0]] = 2 + p0;
                        grid[at[1]] = 2 + p1;
                        grid[at[2]] = [6, 7, 8, 9][o0];
                        grid[at[3]] = [6, 7, 8, 9][o1];
                        for &s in &at[4..] {
                            grid[s] = 1;
                        }
                        let (a, b) = grid.split_at(w);
                        let spots: Vec<(usize, usize)> = (0..2).flat_map(|z| (0..w).map(move |i| (z, i))).filter(|&(z, i)| grid[z * w + i] != 0).collect();
                        for last in spots {
                            runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let (p, c, x40, x120, changing, slime_sets, left) = outcome([a, b], last, false);
                            // intact, and still doing something at the end
                            if (c == p && changing) || all {
                                kept.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                writeln!(out.lock().unwrap(), "{{\"rows\":[{:?},{:?}],\"last\":[{},{}],\"x40\":{x40},\"x120\":{x120},\"intact\":{},\"slime_sets\":{slime_sets},\"left\":{:?}}}", a, b, last.0, last.1, (c == p) as i32, left.iter().map(|&(z, i)| [z, i]).collect::<Vec<_>>()).unwrap();
                            }
                        }
                    }
                }
            });
        }
    });
    println!("{} runs, {} intact and still changing", runs.load(std::sync::atomic::Ordering::Relaxed), kept.load(std::sync::atomic::Ordering::Relaxed));
}

/// Every 2 x `len` grid of air, slime, sticky pistons (east, west) and observers (east, west, north, south).
#[test]
#[ignore]
fn flyer_search4() {
    let len: usize = std::env::var("KILN_FLYER_LEN").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let threads: usize = std::env::var("KILN_FLYER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    // kinds as in `cell`: air, slime, sticky east, sticky west, observer east, west, north, south
    const MAP: [usize; 8] = [0, 1, 2, 3, 6, 7, 8, 9];
    let total = 8usize.pow((2 * len) as u32);
    let found = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for t in 0..threads {
            let found = &found;
            sc.spawn(move || {
                let mut n = t;
                while n < total {
                    let mut cells = vec![0usize; 2 * len];
                    let mut m = n;
                    for c in cells.iter_mut() {
                        *c = MAP[m % 8];
                        m /= 8;
                    }
                    n += threads;
                    let (a, b) = cells.split_at(len);
                    let has = |ks: &[usize]| cells.iter().any(|c| ks.contains(c));
                    if !has(&[2, 3]) || !has(&[6, 7, 8, 9]) || !has(&[1]) || cells.iter().filter(|&&c| c != 0).count() < 4 {
                        continue;
                    }
                    if let Some((x, y)) = run2([a, b], 120)
                        && (y - x).abs() >= 3
                    {
                        found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        println!("flies {} blocks: row0 {:?} row1 {:?} (0 air, 1 slime, 2/3 sticky east/west, 6/7/8/9 observer east/west/north/south)", y - x, a, b);
                    }
                }
            });
        }
    });
    println!("{} layouts fly", found.load(std::sync::atomic::Ordering::Relaxed));
}

/// `rows` rows of `w` cells (z = 8, 9, 10...): the layout runs from the piece placed last (each in turn).
fn run_rows(grid: &[usize], rows: usize, w: usize, ticks: i32) -> Option<(i32, i32)> {
    let spots: Vec<usize> = (0..grid.len()).filter(|&i| grid[i] != 0).collect();
    spots.iter().find_map(|&last| run_rows_from(grid, rows, w, ticks, last))
}

fn run_rows_from(grid: &[usize], rows: usize, w: usize, ticks: i32, last: usize) -> Option<(i32, i32)> {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let y = 100;
    let vertical = std::env::var_os("KILN_FLYER_VERTICAL").is_some();
    let pos = |i: usize| if vertical { BlockPos::new(16 + (i % w) as i32, y + (i / w) as i32, 8) } else { BlockPos::new(16 + (i % w) as i32, y, 8 + (i / w) as i32) };
    let mut pieces = 0;
    for (i, &c) in grid.iter().enumerate() {
        let s = cell(c);
        if is_air(s) {
            continue;
        }
        pieces += 1;
        if i != last {
            level.set_raw(pos(i), s, flags::SKIP_ALL_SIDEEFFECTS);
        }
    }
    crate::update::set_block_and_update(&mut level, pos(last), cell(grid[last]));
    let measure = |level: &TestLevel| {
        let (mut min, mut count, mut h) = (i32::MAX, 0, 0u64);
        for x in 0..80 {
            for z in 6..8 + rows as i32 + 2 {
                for yy in 98..103 + rows as i32 {
                    let s = level.block(BlockPos::new(x, yy, z));
                    if !is_air(s) && !state::is(s, d::PISTON_HEAD) && !state::is(s, d::MOVING_PISTON) {
                        min = min.min(x);
                        count += 1;
                        h = h.wrapping_mul(1_000_003).wrapping_add(s as u64 + (x as u64) * 77 + (z as u64) * 13 + yy as u64);
                    }
                }
            }
        }
        (min, count, h)
    };
    let (mut at20, mut first) = (0u64, 0);
    for t in 1..=ticks {
        level.tick(0, &[(0, 0), (1, 0)]);
        if t == 20 || t == 30 || t == 40 || t == ticks {
            let (min, count, h) = measure(&level);
            if count != pieces {
                return None;
            }
            if t == 20 {
                at20 = h;
            }
            if t == 30 && h == at20 {
                return None;
            }
            if t == 40 {
                first = min;
            }
            if t == ticks {
                return Some((first, min));
            }
        }
    }
    None
}

/// Two sticky pistons (east, west), two observers (any of the four horizontal ways) and `slimes` slime blocks anywhere in `rows` x `w`.
#[test]
#[ignore]
fn flyer_search5() {
    let w: usize = std::env::var("KILN_FLYER_W").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let rows: usize = std::env::var("KILN_FLYER_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let slimes: usize = std::env::var("KILN_FLYER_SLIMES").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let threads: usize = std::env::var("KILN_FLYER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let cells = rows * w;
    let pieces = 4 + slimes;
    let total: usize = (0..pieces).fold(1, |a, i| a * (cells - i));
    let found = std::sync::atomic::AtomicUsize::new(0);
    let obs: [usize; 4] = std::env::var("KILN_FLYER_OBS").ok().map_or([6, 7, 8, 9], |v| {
        let l: Vec<usize> = v.split(',').filter_map(|x| x.parse().ok()).collect();
        [l[0], l[1], l[2], l[3]]
    });
    std::thread::scope(|sc| {
        for t in 0..threads {
            let found = &found;
            sc.spawn(move || {
                let mut n = t;
                while n < total {
                    let mut m = n;
                    n += threads;
                    let mut free: Vec<usize> = (0..cells).collect();
                    let mut at = Vec::new();
                    for i in 0..pieces {
                        let k = m % (cells - i);
                        m /= cells - i;
                        at.push(free.remove(k));
                    }
                    if at[0] > at[1] || at[2] > at[3] {
                        continue;
                    }
                    for facing in 0..64usize {
                        let (p0, p1, o0, o1) = (facing & 1, (facing >> 1) & 1, (facing >> 2) & 3, (facing >> 4) & 3);
                        let mut grid = vec![0usize; cells];
                        grid[at[0]] = 2 + p0;
                        grid[at[1]] = 2 + p1;
                        grid[at[2]] = obs[o0];
                        grid[at[3]] = obs[o1];
                        for &s in &at[4..] {
                            grid[s] = 1;
                        }
                        if let Some((x, y)) = run_rows(&grid, rows, w, 120)
                            && (y - x).abs() >= 3
                        {
                            found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            println!("flies {} blocks: {:?} ({rows} rows of {w}; 0 air, 1 slime, 2/3 sticky east/west, 6/7/8/9 observer east/west/north/south)", y - x, grid);
                        }
                    }
                }
            });
        }
    });
    println!("{} layouts fly", found.load(std::sync::atomic::Ordering::Relaxed));
}

// ---------------------------------------------------------------------------------------------------------------------------------
// wp56: the whole 3D space of small machines. `flyer_search3` only looked at planes with the pistons along the flight; this one takes
// a box of `KILN_FLYER_X` x `_Y` x `_Z` cells and puts in it two pistons (sticky, or regular too with `KILN_FLYER_ANY_PISTON`; any of the
// six facings), two observers (any facing) and `KILN_FLYER_SLIMES` slime blocks (and `KILN_FLYER_HONEY` honey blocks) anywhere, every
// observer with a piston at its back (what its pulse can reach), and runs each layout started in every way a player can start one.

/// (name, dx, dy, dz) of a facing.
const DIRS: [(&str, i32, i32, i32); 6] = [("north", 0, 0, -1), ("south", 0, 0, 1), ("west", -1, 0, 0), ("east", 1, 0, 0), ("down", 0, -1, 0), ("up", 0, 1, 0)];

fn piston_state(sticky: bool, dir: usize) -> u16 {
    state::parse_state(&format!("minecraft:{}piston[facing={},extended=false]", if sticky { "sticky_" } else { "" }, DIRS[dir].0)).unwrap()
}

fn observer_state(dir: usize) -> u16 {
    state::parse_state(&format!("minecraft:observer[facing={},powered=false]", DIRS[dir].0)).unwrap()
}

type Cell = (i32, i32, i32);

/// Runs a machine given as (cell, state), started by `start`: the piece placed last, or (`tap`) a cell where a stone is put and taken away.
/// Returns how far the machine's pieces (not piston heads) have moved on each axis between tick 40 and tick `ticks`, if they are all still there.
fn run3(pieces: &[(Cell, u16)], start: Cell, tap: bool, ticks: i32) -> Option<[i32; 3]> {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let origin = (24, 100, 8);
    let at = |c: Cell| BlockPos::new(origin.0 + c.0, origin.1 + c.1, origin.2 + c.2);
    let mut trigger = None;
    for &(c, s) in pieces {
        if c == start && !tap {
            trigger = Some((at(c), s));
        } else {
            level.set_raw(at(c), s, flags::SKIP_ALL_SIDEEFFECTS);
        }
    }
    let chunks = [(0, 0), (1, 0), (2, 0), (-1, 0), (0, -1), (1, -1), (2, -1), (-1, -1), (0, 1), (1, 1), (2, 1), (-1, 1)];
    if tap {
        crate::update::set_block_and_update(&mut level, at(start), d::STONE);
        level.tick(0, &chunks);
        crate::update::set_block_and_update(&mut level, at(start), d::AIR);
    }
    if let Some((p, s)) = trigger {
        crate::update::set_block_and_update(&mut level, p, s);
    }
    // where the machine is: the sums of the coordinates of everything that is not air or a moving part, how many there are, a hash
    let measure = |level: &TestLevel| {
        let (mut sum, mut count, mut h) = ([0i64; 3], 0usize, 0u64);
        for x in 0..64 {
            for z in -8..24 {
                for y in 80..125 {
                    let s = level.block(BlockPos::new(x, y, z));
                    if !is_air(s) && !state::is(s, d::PISTON_HEAD) && !state::is(s, d::MOVING_PISTON) {
                        sum[0] += x as i64;
                        sum[1] += y as i64;
                        sum[2] += z as i64;
                        count += 1;
                        h = h.wrapping_mul(1_000_003).wrapping_add(s as u64 + (x as u64) * 77 + (z as u64) * 13 + y as u64);
                    }
                }
            }
        }
        (sum, count, h)
    };
    let (mut h20, mut at40) = (0u64, [0i64; 3]);
    for t in 1..=ticks {
        level.tick(0, &chunks);
        if t == 20 || t == 30 || t == 40 || t == ticks {
            let (sum, count, h) = measure(&level);
            if count != pieces.len() {
                return None;
            }
            if t == 20 {
                h20 = h;
            }
            if t == 30 && h == h20 {
                return None;
            }
            if t == 40 {
                at40 = sum;
            }
            if t == ticks {
                let n = count as i64;
                return Some([0, 1, 2].map(|a| ((sum[a] - at40[a]) / n) as i32));
            }
        }
    }
    None
}

#[test]
#[ignore]
fn flyer_search5() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (nx, ny, nz) = (env("KILN_FLYER_X", 3), env("KILN_FLYER_Y", 2), env("KILN_FLYER_Z", 2));
    let (slimes, honeys, threads) = (env("KILN_FLYER_SLIMES", 2), env("KILN_FLYER_HONEY", 0), env("KILN_FLYER_THREADS", 8));
    let sticky_only = std::env::var_os("KILN_FLYER_ANY_PISTON").is_none();
    let (part, parts) = (env("KILN_FLYER_PART", 0), env("KILN_FLYER_PARTS", 1));
    let cells: Vec<Cell> = (0..nx as i32).flat_map(|x| (0..ny as i32).flat_map(move |y| (0..nz as i32).map(move |z| (x, y, z)))).collect();
    let n = cells.len();
    let npieces = 4 + slimes + honeys;
    let total: usize = (0..npieces).fold(1, |a, i| a * (n - i));
    let found = std::sync::atomic::AtomicUsize::new(0);
    let tried = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for t in 0..threads {
            let (found, tried, cells) = (&found, &tried, &cells);
            sc.spawn(move || {
                let mut k = t + threads * part;
                while k < total {
                    let mut m = k;
                    k += threads * parts;
                    let mut free: Vec<usize> = (0..n).collect();
                    let mut at = Vec::new();
                    for i in 0..npieces {
                        let j = m % (n - i);
                        m /= n - i;
                        at.push(free.remove(j));
                    }
                    // two pistons, two observers, the slimes, the honeys: equal pieces in the order of their cells
                    if at[0] > at[1] || at[2] > at[3] || (slimes == 2 && at[4] > at[5]) || (honeys == 2 && at[npieces - 2] > at[npieces - 1]) {
                        continue;
                    }
                    let pos = |i: usize| cells[at[i]];
                    for (o0, o1) in (0..36).map(|v| (v % 6, v / 6)) {
                        // an observer's pulse goes to the block behind it: a piston must be there
                        let back = |o: usize, dir: usize| {
                            let c = pos(o);
                            (c.0 - DIRS[dir].1, c.1 - DIRS[dir].2, c.2 - DIRS[dir].3)
                        };
                        let (b0, b1) = (back(2, o0), back(3, o1));
                        if ![pos(0), pos(1)].contains(&b0) || ![pos(0), pos(1)].contains(&b1) {
                            continue;
                        }
                        for pk in 0..(if sticky_only { 36 } else { 144 }) {
                            let (d0, d1) = (pk % 6, (pk / 6) % 6);
                            let (s0, s1) = if sticky_only { (true, true) } else { ((pk / 36) & 1 == 0, (pk / 72) & 1 == 0) };
                            let mut pieces: Vec<(Cell, u16)> = vec![
                                (pos(0), piston_state(s0, d0)),
                                (pos(1), piston_state(s1, d1)),
                                (pos(2), observer_state(o0)),
                                (pos(3), observer_state(o1)),
                            ];
                            for i in 0..slimes {
                                pieces.push((pos(4 + i), d::SLIME_BLOCK));
                            }
                            for i in 0..honeys {
                                pieces.push((pos(4 + slimes + i), d::HONEY_BLOCK));
                            }
                            tried.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            // started by a piece placed last, or by a block put beside the machine and taken away
                            let mut starts: Vec<(Cell, bool)> = pieces.iter().map(|&(c, _)| (c, false)).collect();
                            starts.extend(cells.iter().filter(|c| !pieces.iter().any(|p| p.0 == **c)).map(|&c| (c, true)));
                            for (c, tap) in starts {
                                if let Some(v) = run3(&pieces, c, tap, 100)
                                    && v.iter().any(|d| d.abs() >= 3)
                                {
                                    found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let list: Vec<String> = pieces.iter().map(|&(c, s)| format!("{c:?}={}", kiln_data::blocks_types::block_of(s).name.trim_start_matches("minecraft:").to_owned() + &kiln_data::blocks_types::block_of(s).property(s, "facing").map(|f| format!("[{f}]")).unwrap_or_default())).collect();
                                    println!("FLIES {v:?}: {} started by {} at {c:?}", list.join(" "), if tap { "a block put and taken away" } else { "the piece placed last" });
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        }
    });
    println!("{} layouts fly of {} tried", found.load(std::sync::atomic::Ordering::Relaxed), tried.load(std::sync::atomic::Ordering::Relaxed));
}
