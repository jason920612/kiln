use super::*;
use flate2::read::GzDecoder;
use std::io::Read;

#[test]
fn lines_are_vanillas() {
    // Taken from the vanilla 26.3 console and logs/latest.log.
    assert_eq!(
        format_line((11, 16, 50), &thread_label(Some("sim")), &Level::INFO, "Starting minecraft server version 26.3"),
        "[11:16:50] [Server thread/INFO]: Starting minecraft server version 26.3"
    );
    assert_eq!(format_line((22, 4, 11), "Server Watchdog", &Level::ERROR, "x"), "[22:04:11] [Server Watchdog/ERROR]: x");
    assert_eq!(format_line((0, 0, 0), &thread_label(None), &Level::WARN, "w"), "[00:00:00] [ServerMain/WARN]: w");
}

#[test]
fn the_old_log_is_compressed_under_the_first_free_number() {
    let dir = std::env::temp_dir().join(format!("kiln-logs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_792_447_451); // 2026-10-19
    assert_eq!(rotate(&dir, now).unwrap(), None, "nothing to archive");
    std::fs::write(dir.join("latest.log"), "").unwrap();
    assert_eq!(rotate(&dir, now).unwrap(), None, "an empty log is not archived");
    std::fs::write(dir.join("latest.log"), "[00:00:01] [Server thread/INFO]: one\n").unwrap();
    assert_eq!(rotate(&dir, now).unwrap().as_deref(), Some("2026-10-19-1.log.gz"));
    assert!(!dir.join("latest.log").exists());
    std::fs::write(dir.join("latest.log"), "[00:00:02] [Server thread/INFO]: two\n").unwrap();
    assert_eq!(rotate(&dir, now).unwrap().as_deref(), Some("2026-10-19-2.log.gz"));
    let mut text = String::new();
    GzDecoder::new(std::fs::File::open(dir.join("2026-10-19-1.log.gz")).unwrap()).read_to_string(&mut text).unwrap();
    assert_eq!(text, "[00:00:01] [Server thread/INFO]: one\n");
    std::fs::remove_dir_all(&dir).unwrap();
}
