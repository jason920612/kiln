//! `eula.txt` as vanilla's `Eula` handles it.

use crate::javaprops::JavaProps;
use std::path::Path;
use std::time::SystemTime;
use tracing::warn;

const COMMENT: &str = "By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).";

/// `new Eula(file).hasAgreedToEULA()`: `eula=true` in the file (`Boolean.parseBoolean`). A file
/// that cannot be read is logged, replaced by the default one (`eula=false`), and not agreed to.
pub fn has_agreed(file: &Path) -> bool {
    match std::fs::read(file) {
        Ok(bytes) => {
            let props = JavaProps::parse(&String::from_utf8_lossy(&bytes));
            props.get("eula").is_some_and(|v| v.eq_ignore_ascii_case("true"))
        }
        Err(_) => {
            warn!("Failed to load {}", file.display());
            save_defaults(file);
            false
        }
    }
}

fn save_defaults(file: &Path) {
    let mut props = JavaProps::default();
    props.set("eula", "false");
    if std::fs::write(file, props.store(COMMENT, SystemTime::now())).is_err() {
        warn!("Failed to save {}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_written_and_refused() {
        let dir = std::env::temp_dir().join(format!("kiln-eula-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("eula.txt");
        assert!(!has_agreed(&file));
        let text = std::fs::read_to_string(&file).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(format!("#{COMMENT}").as_str()));
        assert!(lines.next().unwrap().starts_with("#"));
        assert_eq!(lines.next(), Some("eula=false"));
        assert!(!has_agreed(&file));
        std::fs::write(&file, "eula=TRUE\n").unwrap();
        assert!(has_agreed(&file));
        std::fs::write(&file, "eula=yes\n").unwrap();
        assert!(!has_agreed(&file));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
