//! The lid of a shulker box in every phase of its animation, against vanilla's `ShulkerBoxBlock.getShape`, `getCollisionShape`,
//! `getBlockSupportShape`/`isFaceSturdy` and `Level.clip` (`COLLIDER`): the boxes, the sturdy faces and what the fixed set of rays hits.
//!
//! Vectors: `$KILN_LID_VECTORS`, else `<KILN_WORK or workspace/work>/wp57/lid/lid57.jsonl` (`tools/LidVectors.java`); skipped when absent.

use kiln_entity::clip::{shape_clip, traverse_blocks};
use kiln_entity::collision::{CollisionContext, collision_shape_at, is_face_sturdy_at};
use kiln_entity::lid::lid_shape;
use kiln_entity::math::{BlockPos, Direction, Vec3};
use kiln_entity::memory::MemoryLevel;
use serde_json::Value;
use std::path::PathBuf;

const O: BlockPos = BlockPos { x: 8, y: 150, z: 8 };

fn vectors() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KILN_LID_VECTORS") {
        return Some(PathBuf::from(p));
    }
    let work = std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
    let p = work.join("wp57/lid/lid57.jsonl");
    p.exists().then_some(p)
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn boxes(v: &Value) -> Vec<[f64; 6]> {
    v.as_array().unwrap().iter().map(|b| std::array::from_fn(|i| f(&b[i]))).collect()
}

/// A shape's boxes as vanilla's `toAabbs` lists them: merged and sorted the way `Shape::boxes` gives them.
fn shape_boxes(s: &kiln_entity::shape::Shape) -> Vec<[f64; 6]> {
    s.boxes().iter().map(|b| [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]).collect()
}

#[test]
fn lid_matches_vanilla() {
    let Some(path) = vectors() else {
        eprintln!("lid parity: no vectors, skipped");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    let head: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let rays: Vec<[f64; 6]> = head["rays"].as_array().unwrap().iter().map(|r| std::array::from_fn(|i| f(&r[i]))).collect();
    let (mut states, mut fails) = (0, Vec::new());
    for line in lines {
        let v: Value = serde_json::from_str(line).unwrap();
        states += 1;
        let facing = Direction::ALL[v["facing"].as_u64().unwrap() as usize];
        let name = format!("{:?} {} {} {}", facing, v["status"].as_str().unwrap(), f(&v["progress"]), f(&v["progress_old"]));
        let info = kiln_data::blocks_types::block_of(kiln_data::blocks::default_state::SHULKER_BOX);
        let face = ["down", "up", "north", "south", "west", "east"][facing.index()];
        let state = info.with_property(info.default, "facing", face).unwrap();
        let mut level = MemoryLevel::new(-64, 1);
        level.blocks.insert(O, state);
        let (progress, old) = (f(&v["progress"]) as f32, f(&v["progress_old"]) as f32);
        let status = v["status"].as_str().unwrap();
        // A box that is shut (closed, nothing on the move) has no entry.
        let shown = old + 1.0f32 * (progress - old);
        if status != "CLOSED" {
            level.lids.insert(O, shown);
        }
        // the shape and the collision shape
        let (shape, _) = collision_shape_at(&level, O, &CollisionContext::EMPTY);
        let want = boxes(&v["shape"]);
        let got: Vec<[f64; 6]> = shape_boxes(&shape);
        let near = |a: &[[f64; 6]], b: &[[f64; 6]]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.iter().zip(y).all(|(p, q)| p == q));
        if !near(&got, &want) {
            fails.push(format!("{name}: shape {got:?} != {want:?}"));
        }
        if v["collision"] != v["shape"] {
            fails.push(format!("{name}: vanilla's collision shape is not its shape"));
        }
        // lid_shape alone for a box on the move agrees with the lid's box
        if status != "CLOSED" {
            let ls = lid_shape(facing, shown).map(|s| shape_boxes(&s)).unwrap_or_default();
            if !near(&ls, &want) {
                fails.push(format!("{name}: lid_shape {ls:?} != {want:?}"));
            }
        }
        // the sturdy faces (FULL, CENTER, RIGID): the entity side asks FULL only
        let bits = v["sturdy"].as_i64().unwrap();
        for d in Direction::ALL {
            let full = bits >> (d.index() * 3) & 1 != 0;
            if is_face_sturdy_at(&level, O, d) != full {
                fails.push(format!("{name}: sturdy {d:?} {} != {full}", !full));
            }
        }
        // the rays
        let rel = |p: Vec3| Vec3::new(O.x as f64 + p.x, O.y as f64 + p.y, O.z as f64 + p.z);
        let want_rays = v["collider"].as_array().unwrap();
        for (i, r) in rays.iter().enumerate() {
            let (from, to) = (rel(Vec3::new(r[0], r[1], r[2])), rel(Vec3::new(r[3], r[4], r[5])));
            let hit = traverse_blocks(from, to, |p| {
                let (s, _) = collision_shape_at(&level, p, &CollisionContext::EMPTY);
                shape_clip(&s, from, to, p).map(|(l, face)| (p, face, l))
            });
            let w = want_rays[i].as_array().unwrap();
            let ok = match hit {
                None => w[0].as_i64() == Some(0),
                Some((p, face, l)) => {
                    w[0].as_i64() == Some(1)
                        && (p.x - O.x, p.y - O.y, p.z - O.z) == (w[1].as_i64().unwrap() as i32, w[2].as_i64().unwrap() as i32, w[3].as_i64().unwrap() as i32)
                        && face.index() as i64 == w[4].as_i64().unwrap()
                        && (l.x, l.y, l.z) == (f(&w[5]), f(&w[6]), f(&w[7]))
                }
            };
            if !ok {
                fails.push(format!("{name}: ray {i} {r:?}: {hit:?} != {w:?}"));
            }
        }
    }
    for l in fails.iter().take(40) {
        eprintln!("FAIL {l}");
    }
    eprintln!("lid parity: {states} lid states, {} failures", fails.len());
    assert!(fails.is_empty());
}
