//! Every sound id written out in the sources is a sound the game has (wp68). A sound event that is not in the registry is
//! dropped without a word when its packet is made, so a wrong id (`entity.sniffer.egg_plop` for `block.sniffer_egg.plop`, the
//! turtle egg's `block.turtle_egg.crack` for `entity.turtle.egg_crack`, the nautilus saddle's) is a sound that is never heard.
//! Found by comparing the packets a real client received from Kiln and from a vanilla server.

use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "gen" || n == "tests" || n == "target") {
                continue;
            }
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_sound_id_in_the_sources_exists() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = entry.path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    assert!(files.len() > 100, "found the sources: {}", files.len());
    // Prefixes that a sound name is finished from, not sounds.
    let prefixes = ["minecraft:entity.player.attack", "minecraft:entity.player."];
    let mut missing = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for (n, line) in text.lines().enumerate() {
            let mut rest = line;
            while let Some(at) = rest.find("\"minecraft:") {
                let tail = &rest[at + 1..];
                let Some(end) = tail.find('"') else { break };
                let id = &tail[..end];
                rest = &tail[end + 1..];
                let path = &id["minecraft:".len()..];
                let family = path.split('.').next().unwrap_or("");
                if !path.contains('.') || id.ends_with('.') || prefixes.contains(&id) {
                    continue;
                }
                if !["entity", "item", "block", "ui", "music", "ambient", "weather", "event"].contains(&family) {
                    continue;
                }
                if kiln_data::builtin_id("minecraft:sound_event", id).is_none() {
                    missing.push(format!("{id}  ({}:{})", file.strip_prefix(&crates).unwrap().display(), n + 1));
                }
            }
        }
    }
    assert!(missing.is_empty(), "sound ids that are not in the registry:\n{}", missing.join("\n"));
}
