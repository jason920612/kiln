//! World persistence: Anvil region files and vanilla chunk NBT.

pub mod anvil;
pub mod region;

pub use anvil::AnvilSource;

use kiln_proto::nbt::{self, Tag};
use std::path::Path;

/// World spawn from `level.dat` (26.x: `Data.spawn.pos` as an int array).
pub fn read_spawn(world_dir: &Path) -> Option<[i32; 3]> {
    let raw = std::fs::read(world_dir.join("level.dat")).ok()?;
    let mut data = Vec::new();
    use std::io::Read;
    flate2::read::GzDecoder::new(&raw[..]).take(16 * 1024 * 1024).read_to_end(&mut data).ok()?;
    let (_, root) = nbt::read_named(&data).ok()?;
    match root.get("Data")?.get("spawn")?.get("pos")? {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}
