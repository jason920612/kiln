//! `isFaceSturdy` (FULL, CENTER, RIGID) of a shulker box in every phase of its lid animation against vanilla (`getBlockSupportShape` reads the block
//! entity): `$KILN_LID_VECTORS`, else `<KILN_WORK>/wp57/lid/lid57.jsonl` (`tools/LidVectors.java`); skipped when absent.

use crate::behaviour::shulker::{Lid, LidStatus};
use crate::behaviour::sturdy_at;
use crate::level::{Level, flags};
use crate::pos::{BlockPos, Direction};
use crate::test_level::TestLevel;
use kiln_data::block_logic::Support;
use kiln_data::blocks::default_state as d;

#[test]
fn lid_support_matches_vanilla() {
    let path = std::env::var_os("KILN_LID_VECTORS").map(std::path::PathBuf::from).or_else(|| {
        let p = std::path::PathBuf::from(std::env::var_os("KILN_WORK")?).join("wp57/lid/lid57.jsonl");
        p.exists().then_some(p)
    });
    let Some(path) = path else {
        eprintln!("lid support parity: no vectors, skipped");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let (mut states, mut fails) = (0, Vec::new());
    for line in text.lines().skip(1) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        states += 1;
        let facing = ["down", "up", "north", "south", "west", "east"][v["facing"].as_u64().unwrap() as usize];
        let info = kiln_data::blocks_types::block_of(d::SHULKER_BOX);
        let state = info.with_property(info.default, "facing", facing).unwrap();
        let pos = BlockPos::new(8, 150, 8);
        let mut level = TestLevel::flat(-64, 384, &[d::BEDROCK]);
        level.load_chunks((-4, -4), (4, 4));
        level.set_raw(pos, state, flags::NONE);
        let status = match v["status"].as_str().unwrap() {
            "CLOSED" => LidStatus::Closed,
            "OPENING" => LidStatus::Opening,
            "OPENED" => LidStatus::Opened,
            _ => LidStatus::Closing,
        };
        let lid = Lid { status, progress: v["progress"].as_f64().unwrap() as f32, progress_old: v["progress_old"].as_f64().unwrap() as f32 };
        // A lid at rest is not kept (see `tick_lids`).
        if !(status == LidStatus::Closed && lid.progress == 0.0 && lid.progress_old == 0.0) {
            level.data().pistons.lids.insert(pos, lid);
        }
        let bits = v["sturdy"].as_i64().unwrap();
        for (i, dir) in [Direction::Down, Direction::Up, Direction::North, Direction::South, Direction::West, Direction::East].into_iter().enumerate() {
            for (j, support) in [Support::Full, Support::Center, Support::Rigid].into_iter().enumerate() {
                let want = bits >> (i * 3 + j) & 1 != 0;
                if sturdy_at(&level, pos, dir, support) != want {
                    fails.push(format!("{facing} {} {} {:?}: {dir:?} {j} != {want}", v["status"], v["progress"], lid.progress_old));
                }
            }
        }
    }
    for f in fails.iter().take(30) {
        eprintln!("FAIL {f}");
    }
    eprintln!("lid support parity: {states} lid states, {} failures", fails.len());
    assert!(fails.is_empty());
}
