//! `MonsterRoomFeature`: a cobblestone dungeon with a spawner and up to two loot chests.
//!
//! The spawner's mob and each chest's loot table and seed go into the block entity data.

use super::safe_set;
use crate::block_facts::{Dir, is_solid};
use crate::blocks::{is_block, prop, state, with_prop};
use crate::pos::BlockPos;
use crate::random::WorldgenRandom;
use crate::region::Region;
use crate::vtags;
use kiln_data::block_props::solid_render;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// `MonsterRoomFeature.MOBS`.
const MOBS: [&str; 4] = ["minecraft:skeleton", "minecraft:zombie", "minecraft:zombie", "minecraft:spider"];

const CHEST: &str = "minecraft:chest";

fn replaceable(s: u16) -> bool {
    !vtags::is(s, "features_cannot_replace")
}

/// `MonsterRoomFeature.place`.
pub fn place(r: &mut Region, random: &mut WorldgenRandom, origin: BlockPos) -> bool {
    let rx = random.next_int_bounded(2) + 2;
    let (min_x, max_x) = (-rx - 1, rx + 1);
    let rz = random.next_int_bounded(2) + 2;
    let (min_z, max_z) = (-rz - 1, rz + 1);
    let mut holes = 0;
    for dx in min_x..=max_x {
        for dy in -1..=4 {
            for dz in min_z..=max_z {
                let p = origin.offset(dx, dy, dz);
                let solid = is_solid(r.get(p));
                if (dy == -1 || dy == 4) && !solid {
                    return false;
                }
                if (dx == min_x || dx == max_x || dz == min_z || dz == max_z) && dy == 0 && r.is_air(p) && r.is_air(p.above()) {
                    holes += 1;
                }
            }
        }
    }
    if !(1..=5).contains(&holes) {
        return false;
    }
    for dx in min_x..=max_x {
        for dy in (-1..=3).rev() {
            for dz in min_z..=max_z {
                let p = origin.offset(dx, dy, dz);
                let s = r.get(p);
                if dx == min_x || dy == -1 || dz == min_z || dx == max_x || dz == max_z {
                    if p.y >= r.min_y() && !is_solid(r.get(p.below())) {
                        r.set(p, state::CAVE_AIR, 2);
                    } else if is_solid(s) && !is_block(s, CHEST) {
                        let wall = if dy == -1 && random.next_int_bounded(4) != 0 { state::MOSSY_COBBLESTONE } else { state::COBBLESTONE };
                        safe_set(r, p, wall, replaceable);
                    }
                } else if !is_block(s, CHEST) && !is_block(s, "minecraft:spawner") {
                    safe_set(r, p, state::CAVE_AIR, replaceable);
                }
            }
        }
    }
    for _ in 0..2 {
        for _ in 0..3 {
            let x = origin.x + random.next_int_bounded(rx * 2 + 1) - rx;
            let z = origin.z + random.next_int_bounded(rz * 2 + 1) - rz;
            let p = BlockPos::new(x, origin.y, z);
            if !r.is_air(p) {
                continue;
            }
            let walls = Dir::HORIZONTAL.into_iter().filter(|&d| is_solid(r.get(p.relative(d)))).count();
            if walls != 1 {
                continue;
            }
            let chest = reorient(r, p, state::CHEST);
            safe_set(r, p, chest, replaceable);
            if is_block(r.get(p), CHEST) {
                let seed = random.next_long();
                set_block_entity(r, p, vec![
                    ("id".into(), Tag::String("minecraft:chest".into())),
                    ("LootTable".into(), Tag::String("minecraft:chests/simple_dungeon".into())),
                    ("LootTableSeed".into(), Tag::Long(seed)),
                ]);
            }
            break;
        }
    }
    safe_set(r, origin, state::SPAWNER, replaceable);
    if is_block(r.get(origin), "minecraft:spawner") {
        let mob = MOBS[random.next_int_bounded(MOBS.len() as i32) as usize];
        let entity = Tag::Compound(vec![("id".into(), Tag::String(mob.into()))]);
        set_block_entity(r, origin, vec![
            ("id".into(), Tag::String("minecraft:mob_spawner".into())),
            ("SpawnData".into(), Tag::Compound(vec![("entity".into(), entity)])),
        ]);
    }
    true
}

fn set_block_entity(r: &mut Region, p: BlockPos, fields: Vec<(String, Tag)>) {
    if let Some(tag) = r.block_entity_mut(p) {
        *tag = Tag::Compound(fields);
    }
}

/// `StructurePiece.reorient`: faces a chest away from its single solid neighbour, or else
/// away from walls starting from its current facing.
fn reorient(r: &mut Region, p: BlockPos, s: u16) -> u16 {
    let mut wall = None;
    for d in Dir::HORIZONTAL {
        let n = r.get(p.relative(d));
        if is_block(n, CHEST) {
            return s;
        }
        if solid_render(n) {
            if wall.is_some() {
                wall = None;
                break;
            }
            wall = Some(d);
        }
    }
    if let Some(d) = wall {
        return with_prop(s, "facing", d.opposite().name());
    }
    let mut d = prop(s, "facing").and_then(Dir::by_name).unwrap_or(Dir::North);
    if solid_render(r.get(p.relative(d))) {
        d = d.opposite();
    }
    if solid_render(r.get(p.relative(d))) {
        d = d.clockwise();
    }
    if solid_render(r.get(p.relative(d))) {
        d = d.opposite();
    }
    with_prop(s, "facing", d.name())
}
