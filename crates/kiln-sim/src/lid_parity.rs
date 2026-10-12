//! The view ray (`Level.clip` with `OUTLINE`, [`crate::use_item::clip`]) through a shulker box in every phase of its lid animation against vanilla:
//! `$KILN_LID_VECTORS`, else `<KILN_WORK>/wp57/lid/lid57.jsonl` (`tools/LidVectors.java`); skipped when absent.

use crate::use_item::{FluidMode, clip};
use kiln_data::blocks::default_state as d;
use kiln_entity::math::Vec3;

fn f(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn lid_outline_rays_match_vanilla() {
    let path = std::env::var_os("KILN_LID_VECTORS").map(std::path::PathBuf::from).or_else(|| {
        let p = std::path::PathBuf::from(std::env::var_os("KILN_WORK")?).join("wp57/lid/lid57.jsonl");
        p.exists().then_some(p)
    });
    let Some(path) = path else {
        eprintln!("lid outline parity: no vectors, skipped");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    let head: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let rays: Vec<[f64; 6]> = head["rays"].as_array().unwrap().iter().map(|r| std::array::from_fn(|i| f(&r[i]))).collect();
    let o = kiln_blocks::BlockPos::new(8, 150, 8);
    let (mut states, mut fails) = (0, Vec::new());
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        states += 1;
        let facing = ["down", "up", "north", "south", "west", "east"][v["facing"].as_u64().unwrap() as usize];
        let info = kiln_data::blocks_types::block_of(d::SHULKER_BOX);
        let state = info.with_property(info.default, "facing", facing).unwrap();
        let (progress, old) = (f(&v["progress"]) as f32, f(&v["progress_old"]) as f32);
        let shown = old + 1.0f32 * (progress - old);
        let open = v["status"].as_str() != Some("CLOSED");
        let block = |p: kiln_blocks::BlockPos| if p == o { state } else { d::AIR };
        let lid = |p: kiln_blocks::BlockPos| (open && p == o).then_some(shown);
        let want_rays = v["outline"].as_array().unwrap();
        for (i, r) in rays.iter().enumerate() {
            let at = |a: usize| Vec3::new(o.x as f64 + r[a], o.y as f64 + r[a + 1], o.z as f64 + r[a + 2]);
            let (from, to) = (at(0), at(3));
            let hit = clip(from, to, &block, &lid, FluidMode::None);
            let w = want_rays[i].as_array().unwrap();
            let ok = match hit {
                None => w[0].as_i64() == Some(0),
                Some(h) => {
                    w[0].as_i64() == Some(1)
                        && (h.pos.x - o.x, h.pos.y - o.y, h.pos.z - o.z) == (w[1].as_i64().unwrap() as i32, w[2].as_i64().unwrap() as i32, w[3].as_i64().unwrap() as i32)
                        && h.face as i64 == w[4].as_i64().unwrap()
                        && h.location == [f(&w[5]), f(&w[6]), f(&w[7])]
                }
            };
            if !ok {
                fails.push(format!("{facing} {} {}: ray {i} {r:?}: {hit:?} != {w:?}", v["status"], v["progress"]));
            }
        }
    }
    for l in fails.iter().take(30) {
        eprintln!("FAIL {l}");
    }
    eprintln!("lid outline parity: {states} lid states, {} failures", fails.len());
    assert!(fails.is_empty());
}
