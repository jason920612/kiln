//! Rough throughput of the block tick on a TestLevel: spreading water, toggled wire lines and
//! pistons moving block lines.
//!
//! usage: cargo run --release -p kiln-blocks --example bench

use kiln_blocks::state::parse_state;
use kiln_blocks::{BlockPos, TestLevel, flags, set_block};
use kiln_data::blocks::default_state as d;
use std::time::Instant;

fn level() -> TestLevel {
    let mut l = TestLevel::flat(-64, 384, &[d::BEDROCK, d::STONE, d::STONE, d::STONE]);
    l.load_chunks((-2, -2), (10, 10));
    l
}

fn main() {
    let mut l = level();
    for i in 0..16 {
        let (x, z) = ((i % 4) * 40 + 8, (i / 4) * 40 + 8);
        set_block(&mut l, BlockPos::new(x, -60, z), d::WATER, flags::ALL);
    }
    let t = Instant::now();
    for _ in 0..200 {
        l.tick(0, &[]);
    }
    let water = t.elapsed();

    let mut l = level();
    let wire = parse_state("minecraft:redstone_wire").unwrap();
    for line in 0..64 {
        for x in 1..=15 {
            set_block(&mut l, BlockPos::new(x, -60, line * 2), wire, flags::ALL);
        }
    }
    let t = Instant::now();
    for tick in 0..200 {
        if tick % 10 == 0 {
            let s = if tick % 20 == 0 { d::REDSTONE_BLOCK } else { d::AIR };
            for line in 0..64 {
                set_block(&mut l, BlockPos::new(0, -60, line * 2), s, flags::ALL);
            }
        }
        l.tick(0, &[]);
    }
    let redstone = t.elapsed();
    let wire_reads = l.reads.get();

    let mut l = level();
    let sticky = parse_state("minecraft:sticky_piston[facing=east]").unwrap();
    for line in 0..64 {
        set_block(&mut l, BlockPos::new(1, -60, line * 2), sticky, flags::ALL);
        for x in 2..=12 {
            set_block(&mut l, BlockPos::new(x, -60, line * 2), d::STONE, flags::ALL);
        }
    }
    let t = Instant::now();
    for tick in 0..200 {
        if tick % 10 == 0 {
            let s = if tick % 20 == 0 { d::REDSTONE_BLOCK } else { d::AIR };
            for line in 0..64 {
                set_block(&mut l, BlockPos::new(0, -60, line * 2), s, flags::ALL);
            }
        }
        l.tick(0, &[]);
    }
    let pistons = t.elapsed();
    println!("redstone block reads: {} ({} per toggle per line)", wire_reads, wire_reads / (20 * 64));
    println!("water: 16 sources spreading, 200 ticks: {:.2} ms/tick", water.as_secs_f64() * 1000.0 / 200.0);
    println!("redstone: 64 wire lines toggled every 10 ticks, 200 ticks: {:.2} ms/tick", redstone.as_secs_f64() * 1000.0 / 200.0);
    println!("pistons: 64 sticky pistons moving 11 blocks, toggled every 10 ticks, 200 ticks: {:.2} ms/tick", pistons.as_secs_f64() * 1000.0 / 200.0);
}
