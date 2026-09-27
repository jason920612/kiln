//! Cost of the phases of a resting item's tick (development aid).

use kiln_entity::collision;
use kiln_entity::entity::MoverType;
use kiln_entity::item;
use kiln_entity::math::{BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_item::ItemStack;
use std::time::Instant;

fn time(name: &str, n: u32, mut f: impl FnMut()) {
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    println!("{name:28} {:8.0} ns", start.elapsed().as_nanos() as f64 / n as f64);
}

fn main() {
    let mut level = MemoryLevel::new(-64, 1);
    for x in -3..=3 {
        for z in -3..=3 {
            level.blocks.insert(BlockPos::new(x, 63, z), kiln_data::blocks::default_state::STONE);
        }
    }
    let mut e = item::new(1, 1, ItemStack::of("minecraft:stone", 1).unwrap(), 1);
    e.set_pos(Vec3::new(0.3, 64.0, 0.7));
    for _ in 0..40 {
        e.common_tick();
        e.tick(&mut level);
    }
    let n = 200_000;
    let ctx = e.collision_context();
    let bb = e.bounding_box();
    time("full tick (resting)", n, || {
        e.common_tick();
        e.tick(&mut level);
    });
    time("no_collision", n, || {
        std::hint::black_box(collision::no_collision(&level, &ctx, 1, &bb.deflate_all(1.0e-7)));
    });
    time("update_fluid_interaction", n, || {
        e.update_fluid_interaction(&mut level);
    });
    time("base_tick", n, || e.base_tick(&mut level));
    time("do_move(0,-0.04,0)", n, || {
        e.do_move(&mut level, MoverType::SelfMove, Vec3::new(0.0, -0.04, 0.0));
    });
    time("apply_effects_from_blocks", n, || e.apply_effects_from_blocks(&mut level));
    time("64 kind()", n, || {
        let mut acc = 0usize;
        for s in 0..64u16 {
            acc += kiln_entity::blocks::kind(std::hint::black_box(s)) as usize;
        }
        std::hint::black_box(acc);
    });
    time("64 physics::collision_shape", n, || {
        let mut acc = 0usize;
        for s in 0..64u16 {
            acc += kiln_entity::physics::collision_shape(std::hint::black_box(s)).size(kiln_entity::math::Axis::X);
        }
        std::hint::black_box(acc);
    });
    time("64 collision_offset", n, || {
        let mut acc = 0usize;
        for s in 0..64u16 {
            acc += kiln_entity::physics::collision_offset(std::hint::black_box(s), 3, 4).is_some() as usize;
        }
        std::hint::black_box(acc);
    });
    time("64 collision::collision_shape(s)", n, || {
        let mut acc = 0usize;
        for s in 0..64u16 {
            acc += collision::collision_shape(std::hint::black_box(s), BlockPos::new(1, 2, 3), &ctx).1 as usize;
        }
        std::hint::black_box(acc);
    });
    time("Shape::from_box", n, || {
        std::hint::black_box(kiln_entity::shape::Shape::from_box(&bb));
    });
    time("64 block lookups", n, || {
        let mut acc = 0u32;
        for x in -2..2 {
            for y in 62..66 {
                for z in -2..2 {
                    acc += kiln_entity::level::EntityLevel::block(&level, BlockPos::new(x, y, z)) as u32;
                }
            }
        }
        std::hint::black_box(acc);
    });
    time("64 collision_shape", n, || {
        let mut acc = 0usize;
        for x in -2..2 {
            for y in 62..66 {
                for z in -2..2 {
                    let p = BlockPos::new(x, y, z);
                    let s = kiln_entity::level::EntityLevel::block(&level, p);
                    acc += collision::collision_shape(s, p, &ctx).0.size(kiln_entity::math::Axis::X);
                }
            }
        }
        std::hint::black_box(acc);
    });
    time("collide", n, || {
        std::hint::black_box(e.collide(&level, Vec3::new(0.0, -0.04, 0.0)));
    });
}
