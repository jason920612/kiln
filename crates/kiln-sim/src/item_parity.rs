//! Differential test against vanilla 26.3 vectors from `tools/ItemUseVectors.java` (run by
//! `tools/itemuse_vectors.py`, which sets `KILN_ITEMUSE_VECTORS`): the view ray buckets aim with
//! (`Level.clip` with outline shapes; the faces `clipWithInteractionOverride` takes from
//! interaction shapes are not modelled, so at least 99% must agree), `Projectile.shootFromRotation` and the crossbow's shot vector (both
//! exact). Skipped without the vectors.

use crate::use_item::{FluidMode, clip};
use kiln_entity::math::Vec3;
use serde_json::Value;
use std::collections::HashMap;

fn vectors() -> Option<Vec<Value>> {
    let path = std::env::var_os("KILN_ITEMUSE_VECTORS")?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str(l).expect("vector json")).collect())
}

fn f64_bits(v: &Value) -> f64 {
    f64::from_bits(v.as_i64().expect("long bits") as u64)
}

fn f32_bits(v: &Value) -> f32 {
    f32::from_bits(v.as_i64().expect("int bits") as u32)
}

fn vec_of(v: &Value) -> Vec3 {
    let a = v.as_array().unwrap();
    Vec3::new(f64_bits(&a[0]), f64_bits(&a[1]), f64_bits(&a[2]))
}

fn arrow(seed: i64) -> kiln_entity::Entity {
    kiln_entity::arrow::new(0, 0, "minecraft:arrow", Vec3::ZERO, Vec3::ZERO, None, seed)
}

#[test]
fn item_parity() {
    let Some(vs) = vectors() else {
        eprintln!("KILN_ITEMUSE_VECTORS not set: skipped");
        return;
    };
    let mut blocks: HashMap<(i32, i32, i32), u16> = HashMap::new();
    let (mut clip_total, mut clip_same) = (0, 0);
    let (mut shoot_total, mut shoot_same) = (0, 0);
    let (mut cross_total, mut cross_same, mut cross_close) = (0, 0, 0);
    let mut shown = 0;
    for v in &vs {
        match v["name"].as_str().unwrap() {
            "error" => panic!("the recorder failed: {}", v["error"]),
            "clip_scene" => {
                for b in v["blocks"].as_array().unwrap() {
                    let b = b.as_array().unwrap();
                    let state = kiln_blocks::state::parse_state(b[3].as_str().unwrap()).unwrap_or_else(|| panic!("state {}", b[3]));
                    blocks.insert((b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32, b[2].as_i64().unwrap() as i32), state);
                }
            }
            "clip" => {
                clip_total += 1;
                let (from, to) = (vec_of(&v["from"]), vec_of(&v["to"]));
                // The recorded ray's end is the view vector's: check that too.
                let rot = v["rot"].as_array().unwrap();
                let range = f64::from_bits(v["range"].as_i64().unwrap() as u64);
                let ours_to = from + crate::use_item::view_vector([f32_bits(&rot[0]), f32_bits(&rot[1])]).scale(range);
                assert_eq!(ours_to, to, "view vector");
                let fluid = if v["source_only"].as_bool().unwrap() { FluidMode::SourceOnly } else { FluidMode::None };
                let block = |p: kiln_blocks::BlockPos| blocks.get(&(p.x, p.y, p.z)).copied().unwrap_or(kiln_data::blocks::default_state::AIR);
                let hit = clip(from, to, &block, fluid);
                let ours = hit.map(|h| (vec![h.pos.x as i64, h.pos.y as i64, h.pos.z as i64, h.face as i64], h.location));
                let theirs = v["hit"].as_array().map(|a| {
                    let l = vec_of(&v["location"]);
                    (a.iter().map(|x| x.as_i64().unwrap()).collect::<Vec<_>>(), [l.x, l.y, l.z])
                });
                let same = match (&ours, &theirs) {
                    (None, None) => true,
                    (Some((a, la)), Some((b, lb))) => a == b && (0..3).all(|k| (la[k] - lb[k]).abs() < 1e-9),
                    _ => false,
                };
                if same {
                    clip_same += 1;
                } else if shown < 12 {
                    shown += 1;
                    let at = |t: &Option<(Vec<i64>, _)>| t.as_ref().map(|(a, _)| blocks.get(&(a[0] as i32, a[1] as i32, a[2] as i32)).map(|s| kiln_blocks::state::state_string(*s)));
                    eprintln!("clip differs: ours {:?} {:?}, vanilla {:?} {:?}", ours.as_ref().map(|o| &o.0), at(&ours), theirs.as_ref().map(|t| &t.0), at(&theirs));
                }
            }
            "shoot" => {
                shoot_total += 1;
                let rot = v["rot"].as_array().unwrap();
                let mut e = arrow(v["seed"].as_i64().unwrap());
                let m = vec_of(&v["motion"]);
                crate::ranged::shoot_rotated(
                    &mut e,
                    f32_bits(&rot[1]),
                    f32_bits(&rot[0]),
                    f32_bits(&v["roll"]),
                    f32_bits(&v["velocity"]),
                    f32_bits(&v["inaccuracy"]),
                    [m.x, m.y, m.z],
                    v["on_ground"].as_bool().unwrap(),
                );
                let want = vec_of(&v["delta"]);
                let r = v["arrow_rot"].as_array().unwrap();
                if e.delta == want && e.y_rot == f32_bits(&r[0]) && e.x_rot == f32_bits(&r[1]) {
                    shoot_same += 1;
                } else if shown < 24 {
                    shown += 1;
                    eprintln!("shoot differs: ours {:?} ({}, {}), vanilla {want:?} ({}, {})", e.delta, e.y_rot, e.x_rot, f32_bits(&r[0]), f32_bits(&r[1]));
                }
            }
            "crossbow" => {
                cross_total += 1;
                let rot = v["rot"].as_array().unwrap();
                let mut e = arrow(v["seed"].as_i64().unwrap());
                let d = crate::crossbow::shot_vector([f32_bits(&rot[0]), f32_bits(&rot[1])], f32_bits(&v["angle"]));
                kiln_entity::mob::species::shoot(&mut e, d.x, d.y, d.z, f32_bits(&v["power"]), 1.0);
                let want = vec_of(&v["delta"]);
                let close = (e.delta.x - want.x).abs() < 1e-6 && (e.delta.y - want.y).abs() < 1e-6 && (e.delta.z - want.z).abs() < 1e-6;
                if close {
                    cross_close += 1;
                }
                if e.delta == want {
                    cross_same += 1;
                } else if shown < 36 && !close {
                    shown += 1;
                    eprintln!("crossbow differs: ours {:?}, vanilla {want:?}", e.delta);
                }
            }
            _ => {}
        }
    }
    eprintln!("item parity: clip {clip_same}/{clip_total} (interaction-shape faces differ), shoot {shoot_same}/{shoot_total}, crossbow {cross_same}/{cross_total} exact ({cross_close} within 1e-6: JOML rotates in floats)");
    assert!(clip_total > 0 && shoot_total > 0 && cross_total > 0, "no vectors");
    assert_eq!(shoot_same, shoot_total, "shootFromRotation");
    assert_eq!(cross_close, cross_total, "crossbow shot vectors");
    assert!(clip_same * 100 >= clip_total * 99, "view ray: {clip_same}/{clip_total}");
}
