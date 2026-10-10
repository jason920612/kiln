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

const KINDS: usize = 8;

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
