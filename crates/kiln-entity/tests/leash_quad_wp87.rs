//! wp87: a lead held by a happy ghast (or a ghast) hangs a horse, a camel, a sniffer or a boat on four springs (`Leashable.SHARED_QUAD_ATTACHMENT_POINTS`,
//! scaled by a quarter), anything else on one. `tools/wp87/LeashQuad.java` calls vanilla's `Leashable.tickLeash` twelve times for 84 setups (what is led,
//! who holds it, where, which way both face; neither has AI) and writes the led entity's delta, yaw and the lead's angular momentum after each call; here
//! Kiln's `tick_leash` is called on the same setups and has to give the same numbers.
//!
//! Skipped without the dump (`$KILN_LEASH_VECTORS`, default `<work>/wp87/leash.txt`; `tools/wp87/leash87.sh` writes it).

use kiln_entity::leash;
use kiln_entity::math::Vec3;
use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::{self, MobKind};
use std::path::{Path, PathBuf};

fn dump() -> Option<String> {
    let file = std::env::var_os("KILN_LEASH_VECTORS").map(PathBuf::from).unwrap_or_else(|| {
        match std::env::var_os("KILN_WORK") {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work"),
        }
        .join("wp87/leash.txt")
    });
    std::fs::read_to_string(file).ok()
}

/// The scenarios of `LeashQuad.scenarios()`, in its order.
fn scenarios() -> Vec<(String, &'static str, &'static str, [f64; 3], f32, f32)> {
    let leashed = ["horse", "donkey", "camel", "sniffer", "oak_boat", "llama", "cow"];
    let holders = ["happy_ghast", "cow", "ghast"];
    let offsets = [[9.5, 2.0, 3.0], [-8.0, 0.0, 7.25], [2.5, 3.5, -10.0], [13.0, 5.0, 0.0]];
    let yaws = [0f32, 37.0, 150.0, 271.0];
    let mut out = Vec::new();
    let mut i = 0;
    for l in leashed {
        for h in holders {
            for o in offsets {
                out.push((format!("{l}_by_{h}_{i}"), l, h, o, yaws[i % 4], yaws[(i + 1) % 4] + 11.0));
                i += 1;
            }
        }
    }
    out
}

fn entity(kind: &str, id: i32, at: Vec3, yaw: f32) -> kiln_entity::Entity {
    let mut e = if kind == "oak_boat" {
        kiln_entity::ext_entity::boat::new("minecraft:oak_boat", at, yaw, 7)
    } else {
        let mut e = mob::new(MobKind::by_name(&format!("minecraft:{kind}")).expect("a mob"), id, id as u128, 7);
        mob::data_mut(&mut e).expect("a mob").no_ai = true;
        e
    };
    e.id = id;
    e.set_pos(at);
    e.y_rot = yaw;
    e.on_ground = true;
    e.set_old_pos_and_rot();
    e
}

#[test]
fn a_lead_pulls_as_vanilla_pulls() {
    let Some(text) = dump() else {
        eprintln!("skipped: no leash vectors (tools/wp87/leash87.sh on the VM)");
        return;
    };
    let _x = kiln_data::live::exclusive();
    let (mut checked, mut bad) = (0, Vec::new());
    for (name, led, holder, o, led_yaw, holder_yaw) in scenarios() {
        let lines: Vec<Vec<&str>> = text.lines().filter(|l| l.split(' ').next() == Some(name.as_str())).map(|l| l.split(' ').collect()).collect();
        assert!(!lines.is_empty(), "{name}: not in the dump");
        let mut level = MemoryLevel::new(-64, 1);
        let mut h = entity(holder, 2, Vec3::new(0.5 + o[0], -63.0 + o[1], 0.5 + o[2]), holder_yaw);
        h.on_ground = false;
        level.insert(h);
        level.insert(entity(led, 1, Vec3::new(0.5, -63.0, 0.5), led_yaw));
        leash::set_leashed_to_in_level(&mut level, 1, 2);
        let mut e = std::mem::replace(level.entity_mut(1).unwrap(), entity("cow", -1, Vec3::ZERO, 0.0));
        for line in &lines {
            let step: usize = line[1].parse().unwrap();
            leash::tick_leash(&mut e, None, &mut level);
            let want: Vec<f64> = line[2..].iter().map(|v| v.parse().unwrap_or(f64::NAN)).collect();
            let got = [e.delta.x, e.delta.y, e.delta.z, e.y_rot as f64, e.leash.as_ref().map_or(f64::NAN, |d| d.angular_momentum)];
            checked += 1;
            // Float arithmetic in the same order is bit-exact; the yaw is a float.
            let same = got.iter().zip(&want).all(|(g, w)| (g.is_nan() && w.is_nan()) || g == w || (*g as f32 == *w as f32 && g.abs() < 1000.0 && (g - w).abs() < 1e-4));
            if !same {
                bad.push(format!("{name} step {step}: kiln {got:?} vanilla {want:?}"));
                break;
            }
            if !leash::is_leashed(&e) {
                break;
            }
        }
    }
    for b in bad.iter().take(25) {
        eprintln!("  {b}");
    }
    eprintln!("{checked} steps, {} scenarios differ", bad.len());
    assert!(bad.is_empty(), "{} lead scenarios differ from vanilla", bad.len());
}
