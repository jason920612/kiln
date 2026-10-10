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
                        let (a, b) = grid.split_at(w);
                        if let Some((x, y)) = run2([a, b], 100)
                            && (y - x).abs() >= 3
                        {
                            found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            println!("flies {} blocks: row0 {:?} row1 {:?} (0 air, 1 slime, 2/3 sticky east/west, 6/7/8/9 observer east/west/north/south)", y - x, a, b);
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
