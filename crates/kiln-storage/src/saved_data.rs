//! Server-wide saved data (`SavedDataStorage`): `data/minecraft/<id>.dat`, a gzip-compressed
//! compound `{data: {...}, DataVersion: n}`.

use crate::anvil::DATA_VERSION;
use crate::{read_nbt_file, write_nbt_file};
use kiln_proto::nbt::Tag;
use std::path::{Path, PathBuf};

fn path(world_dir: &Path, id: &str) -> PathBuf {
    path_ns(world_dir, "minecraft", id)
}

/// Saved data `namespace:id` (`data/<namespace>/<id>.dat`).
fn path_ns(world_dir: &Path, namespace: &str, id: &str) -> PathBuf {
    world_dir.join("data").join(namespace).join(format!("{id}.dat"))
}

/// The `data` compound of saved data `id`, if the file exists and reads.
pub fn read(world_dir: &Path, id: &str) -> Option<Tag> {
    read_path(&path(world_dir, id))
}

/// [`read`] for saved data of another namespace (command storage keeps one file per namespace).
pub fn read_ns(world_dir: &Path, namespace: &str, id: &str) -> Option<Tag> {
    read_path(&path_ns(world_dir, namespace, id))
}

/// [`write`] for saved data of another namespace.
pub fn write_ns(world_dir: &Path, namespace: &str, id: &str, data: Tag) -> std::io::Result<()> {
    let root = Tag::Compound(vec![("data".into(), data), ("DataVersion".into(), Tag::Int(DATA_VERSION as i32))]);
    write_nbt_file(&path_ns(world_dir, namespace, id), &root, None)
}

/// The namespaces that have a saved data file `id` (`data/<namespace>/<id>.dat`), sorted.
pub fn namespaces_with(world_dir: &Path, id: &str) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(world_dir.join("data")) else { return Vec::new() };
    let mut found: Vec<String> = dir
        .filter_map(Result::ok)
        .filter(|e| e.path().join(format!("{id}.dat")).is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    found.sort();
    found
}

fn read_path(path: &Path) -> Option<Tag> {
    if !path.exists() {
        return None;
    }
    match read_nbt_file(path) {
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
        let storage = Tag::Compound(vec![("contents".into(), Tag::Compound(vec![]))]);
        write_ns(&dir, "kiln_ns", "command_storage", storage.clone()).unwrap();
        assert_eq!(read_ns(&dir, "kiln_ns", "command_storage"), Some(storage));
        assert_eq!(namespaces_with(&dir, "command_storage"), ["kiln_ns"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
