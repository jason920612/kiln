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
    for len in 2..=max_len {
        let total = KINDS.pow(len as u32);
        for n in 0..total {
            let mut layout = vec![0usize; len];
            let mut m = n;
            for c in layout.iter_mut() {
                *c = m % KINDS;
                m /= KINDS;
            }
            // a machine has pistons and a way to start them again
            let has = |ks: &[usize]| layout.iter().any(|c| ks.contains(c));
            if !has(&[2, 3, 4, 5]) || !has(&[6, 7]) || layout[0] == 0 || layout[len - 1] == 0 {
                continue;
            }
            if let Some((a, b)) = run(&layout, 120)
                && b - a >= 8
            {
                found += 1;
                println!("flies {} blocks in 80 ticks: {:?} (0 air, 1 slime, 2/3 sticky east/west, 4/5 piston east/west, 6/7 observer east/west)", b - a, layout);
            }
        }
    }
    println!("{found} layouts fly");
}

/// Two rows (z = 8 and 9) of `len` cells; early exit for layouts that settle.
fn run2(rows: [&[usize]; 2], ticks: i32) -> Option<(i32, i32)> {
    let mut level = TestLevel::void();
    level.load_chunks((-1, -1), (4, 1));
    let y = 100;
    let mut pieces = 0;
    let mut spots = Vec::new();
    for (z, row) in rows.iter().enumerate() {
        for (i, &c) in row.iter().enumerate() {
            let s = cell(c);
            if !is_air(s) {
                pieces += 1;
                let p = BlockPos::new(16 + i as i32, y, 8 + z as i32);
                level.set_raw(p, s, flags::SKIP_ALL_SIDEEFFECTS);
                spots.push(p);
            }
        }
    }
    for p in spots {
        let s = level.block(p);
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
                        && y - x >= 8
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
