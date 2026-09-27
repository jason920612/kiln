//! Rough cost of item entity ticks: items dropped over a stone floor with some water.
//!
//! cargo run --release -p kiln-entity --example item_bench -- [items] [ticks] [nomerge]
//!
//! The in-memory level answers entity queries by scanning, so merging dominates at large counts;
//! `nomerge` measures movement physics alone.

use kiln_entity::item;
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_item::ItemStack;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let items: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(10_000);
    let ticks: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(200);
    let merge = args.next().as_deref() != Some("nomerge");
    let side = (items as f64).sqrt().ceil() as i32;
    let mut level = MemoryLevel::new(-64, 1);
    let stone = kiln_data::blocks::default_state::STONE;
    let water = kiln_data::blocks::default_state::WATER;
    for x in -1..=side / 2 + 1 {
        for z in -1..=side / 2 + 1 {
            level.blocks.insert(BlockPos::new(x, 63, z), stone);
            if (x + z) % 7 == 0 {
                level.blocks.insert(BlockPos::new(x, 64, z), water);
            }
        }
    }
    for i in 0..items {
        let (gx, gz) = ((i as i32) % side, (i as i32) / side);
        let stack = ItemStack::of(if i % 3 == 0 { "minecraft:diamond" } else { "minecraft:cobblestone" }, 1).unwrap();
        let mut e = item::new(i as i32 + 1, i as u128, stack, i as i64);
        e.set_pos(Vec3::new(gx as f64 * 0.5 + 0.25, 65.0 + (i % 5) as f64 * 0.3, gz as f64 * 0.5 + 0.25));
        e.delta = Vec3::new(((i % 7) as f64 - 3.0) * 0.01, 0.1, ((i % 5) as f64 - 2.0) * 0.01);
        if let (false, kiln_entity::EntityKind::Item(d)) = (merge, &mut e.kind) {
            d.pickup_delay = item::INFINITE_PICKUP_DELAY;
        }
        level.insert(e);
    }
    let start = Instant::now();
    for _ in 0..ticks {
        level.tick();
    }
    let elapsed = start.elapsed();
    let alive = level.entities().filter(|e| e.is_alive()).count();
    let per = elapsed.as_nanos() as f64 / (items * ticks) as f64;
    println!("{items} items x {ticks} ticks: {elapsed:?} total, {per:.0} ns per entity tick ({alive} alive at the end)");
}
