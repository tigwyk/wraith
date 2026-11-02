//! `wr config check [path]` (spec §7): every error printed as `file:line:`, exit 1 on any.
use std::path::PathBuf;
use std::process::Output;
use std::sync::atomic::{AtomicU32, Ordering};

fn temp() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("wr-check-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wr(appdata: &std::path::Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_wr")).args(args).env("APPDATA", appdata).output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn config_check_clean_file_exits_0() {
    let d = temp();
    let f = d.join("config");
    std::fs::write(&f, "font-size = 13\n").unwrap();
    let o = wr(&d, &["config", "check", f.to_str().unwrap()]);
    assert!(o.status.success(), "{o:?}");
    // Success is not an error: PowerShell shows a program's stderr in red.
    assert!(String::from_utf8_lossy(&o.stdout).contains("config ok"), "{o:?}");
    assert!(o.stderr.is_empty(), "{}", stderr(&o));
}

#[test]
fn config_check_bad_key_exits_1() {
    let d = temp();
    let f = d.join("config");
    std::fs::write(&f, "font-size = huge\n").unwrap();
    let o = wr(&d, &["config", "check", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert!(stderr(&o).contains(&format!("{}:1:", f.display())), "{}", stderr(&o));
}

#[test]
fn config_check_missing_file_is_ok() {
    let d = temp();
    let o = wr(&d, &["config", "check", d.join("nope").to_str().unwrap()]);
    assert!(o.status.success(), "{o:?}");
}

#[test]
fn config_check_reports_include_errors() {
    let d = temp();
    let f = d.join("config");
    std::fs::write(&f, "config-file = inc\n").unwrap();
    std::fs::write(d.join("inc"), "cursor-style = triangle\n").unwrap();
    let o = wr(&d, &["config", "check", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
    assert!(stderr(&o).contains("inc:1:"), "{}", stderr(&o));
}

#[test]
fn config_check_default_path() {
    let d = temp();
    std::fs::create_dir_all(d.join("wraith")).unwrap();
    std::fs::write(d.join("wraith").join("config"), "font-size = huge\n").unwrap();
    let o = wr(&d, &["config", "check"]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
}
