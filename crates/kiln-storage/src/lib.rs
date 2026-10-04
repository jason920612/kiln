//! World persistence: Anvil region files, vanilla chunk NBT, entity chunks, `level.dat` and
//! player data.

pub mod anvil;
pub mod entities;
pub mod level;
pub mod native;
pub mod player;
pub mod poi;
pub mod region;
pub mod saved_data;

pub use anvil::AnvilSource;
pub use entities::EntityStore;
pub use level::{LevelState, LevelStore, WorldSpawn};
pub use native::{CompactionMode, CompactionStats, NativeSource, NativeStore, WorldFormat};
pub use player::{PlayerData, PlayerStore};
pub use poi::PoiStore;

use kiln_proto::nbt::{self, Tag};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// World spawn from `level.dat` (26.x: `Data.spawn.pos` as an int array).
pub fn read_spawn(world_dir: &Path) -> Option<[i32; 3]> {
    let root = read_nbt_file(&world_dir.join("level.dat")).ok()?;
    match root.get("Data")?.get("spawn")?.get("pos")? {
        Tag::IntArray(v) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

/// The world seed a save records (`data/minecraft/world_gen_settings.dat`, `seed`).
pub fn read_seed(world_dir: &Path) -> Option<i64> {
    saved_data::read(world_dir, "world_gen_settings")?.get("seed")?.as_i64()
}

/// Upper bound on a decompressed NBT file (level.dat, player data, saved data).
const MAX_FILE_NBT: u64 = 64 * 1024 * 1024;

/// Reads a gzip-compressed NBT file (vanilla `NbtIo.readCompressed`); the root compound.
pub(crate) fn read_nbt_file(path: &Path) -> std::io::Result<Tag> {
    let raw = std::fs::read(path)?;
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(&raw[..]).take(MAX_FILE_NBT).read_to_end(&mut data)?;
    match nbt::read_named(&data) {
        Ok((_, root @ Tag::Compound(_))) => Ok(root),
        Ok(_) => Err(std::io::Error::other("NBT root is not a compound")),
        Err(e) => Err(std::io::Error::other(e)),
    }
}

/// Writes `root` gzip-compressed through a temporary file in the same directory, then moves
/// it into place; with `backup`, the previous file becomes `backup` first (vanilla
/// `Util.safeReplaceFile`, as used for `level.dat` and player data).
pub(crate) fn write_nbt_file(path: &Path, root: &Tag, backup: Option<&Path>) -> std::io::Result<()> {
    let mut buf = bytes::BytesMut::new();
    root.write_named("", &mut buf);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&buf)?;
    let data = gz.finish()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&data)?;
        f.sync_all()?;
    }
    if let Some(backup) = backup.filter(|_| path.exists()) {
        if backup.exists() {
            std::fs::remove_file(backup)?;
        }
        std::fs::rename(path, backup)?;
    }
    std::fs::rename(&tmp, path)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Sets `key` in a compound, keeping its position if present.
pub(crate) fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(fields) = tag {
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => fields.push((key.to_owned(), value)),
        }
    }
}

pub(crate) fn remove(tag: &mut Tag, key: &str) {
    if let Tag::Compound(fields) = tag {
        fields.retain(|(k, _)| k != key);
    }
}

/// The compound at `key`, created empty if missing or of another type.
pub(crate) fn child<'a>(tag: &'a mut Tag, key: &str) -> &'a mut Tag {
    if !matches!(tag.get(key), Some(Tag::Compound(_))) {
        put(tag, key, Tag::Compound(Vec::new()));
    }
    let Tag::Compound(fields) = tag else { unreachable!("put made it a compound") };
    &mut fields.iter_mut().find(|(k, _)| k == key).expect("just inserted").1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbt_files_round_trip_with_backup() {
        let dir = std::env::temp_dir().join(format!("kiln-storage-nbt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("a.dat");
        let backup = dir.join("a.dat_old");
        let v1 = Tag::Compound(vec![("n".into(), Tag::Int(1))]);
        let v2 = Tag::Compound(vec![("n".into(), Tag::Int(2))]);
        write_nbt_file(&path, &v1, Some(&backup)).unwrap();
        assert!(!backup.exists());
        write_nbt_file(&path, &v2, Some(&backup)).unwrap();
        assert_eq!(read_nbt_file(&path).unwrap(), v2);
        assert_eq!(read_nbt_file(&backup).unwrap(), v1);
        assert!(!temp_path(&path).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compound_helpers() {
        let mut t = Tag::Compound(vec![("a".into(), Tag::Int(1)), ("b".into(), Tag::Int(2))]);
        put(&mut t, "a", Tag::Int(3));
        put(&mut t, "c", Tag::Int(4));
        remove(&mut t, "b");
        put(child(&mut t, "d"), "e", Tag::Byte(1));
        let expected = Tag::Compound(vec![
            ("a".into(), Tag::Int(3)),
            ("c".into(), Tag::Int(4)),
            ("d".into(), Tag::Compound(vec![("e".into(), Tag::Byte(1))])),
        ]);
        assert_eq!(t, expected);
    }
}
