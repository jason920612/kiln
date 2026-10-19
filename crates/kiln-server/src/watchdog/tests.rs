use super::*;

#[test]
fn the_message_is_vanillas() {
    // The line the vanilla server logged when its tick was 1.00 seconds late with a 50 ms tick.
    assert_eq!(tick_message(1000, 50), "A single server tick took 1.00 seconds (should be max 0.05)");
    assert_eq!(tick_message(60_004, 50), "A single server tick took 60.00 seconds (should be max 0.05)");
}

#[test]
fn the_report_has_vanillas_sections() {
    let info = ServerInfo::new("A Minecraft Server", 20, "world");
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_447_451); // 2026-10-19 22:04:11 UTC
    let r = crash_report(t, 1000, &info);
    assert!(r.starts_with("---- Minecraft Crash Report ----\n"));
    assert!(r.contains("\nTime: 2026-10-19 22:04:11\nDescription: Watching Server\n"));
    assert!(r.contains("-- Head --\nThread: Server Watchdog"));
    assert!(r.contains("-- Performance stats --") && r.contains("-- System Details --"));
    assert!(r.contains("Player Count: 0 / 20; []"));
}

#[test]
fn no_watchdog_without_a_limit() {
    let info = Arc::new(ServerInfo::new("m", 20, "world"));
    assert!(start(info.clone(), 0, 50).is_none());
    assert!(start(info, -1, 50).is_none());
}
