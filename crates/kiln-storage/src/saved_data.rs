//! Server-wide saved data (`SavedDataStorage`): `data/minecraft/<id>.dat`, a gzip-compressed
//! compound `{data: {...}, DataVersion: n}`.

use crate::anvil::DATA_VERSION;
use crate::{read_nbt_file, write_nbt_file};
use kiln_proto::nbt::Tag;
use std::path::{Path, PathBuf};

fn path(world_dir: &Path, id: &str) -> PathBuf {
    world_dir.join("data/minecraft").join(format!("{id}.dat"))
}

/// The `data` compound of saved data `id`, if the file exists and reads.
pub fn read(world_dir: &Path, id: &str) -> Option<Tag> {
    let path = path(world_dir, id);
    if !path.exists() {
        return None;
    }
    match read_nbt_file(&path) {
        Ok(root) => root.get("data").cloned(),
        Err(e) => {
            tracing::warn!("could not read {}: {e}", path.display());
            None
        }
    }
}

/// Writes saved data `id` with the current data version.
pub fn write(world_dir: &Path, id: &str, data: Tag) -> std::io::Result<()> {
    let root = Tag::Compound(vec![("data".into(), data), ("DataVersion".into(), Tag::Int(DATA_VERSION as i32))]);
    write_nbt_file(&path(world_dir, id), &root, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("kiln-saved-data-{}", std::process::id()));
        let data = Tag::Compound(vec![("Teams".into(), Tag::List(vec![]))]);
        write(&dir, "scoreboard", data.clone()).unwrap();
        assert_eq!(read(&dir, "scoreboard"), Some(data));
        assert_eq!(read(&dir, "missing"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
