//! A panic must reach disk before the process aborts: panics inside the window procedure can't unwind,
//! and the log's background writer never gets to flush.
#[test]
fn panic_is_written_to_crash_log_before_returning() {
    let dir = std::env::temp_dir().join(format!("wraith-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    wraith_client::crash::install(dir.clone(), "client");
    let _ = std::panic::catch_unwind(|| panic!("boom {}", 42));
    let log = std::fs::read_to_string(dir.join("crash.log")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(log.contains("client"), "{log}");
    assert!(log.contains(&format!("pid {}", std::process::id())), "{log}");
    assert!(log.contains("boom 42"), "{log}");
    assert!(log.contains("crash.rs:"), "panic location missing: {log}");
}
