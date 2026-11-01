#![cfg(windows)]
//! The real binary opens a window, presents one frame, and exits cleanly.

#[test]
fn smoke_test_presents_and_exits() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_wraith")).arg("--smoke-test").output().unwrap();
    assert!(out.status.success(), "status {:?}, stderr {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("presented"), "stdout: {stdout}");
}
