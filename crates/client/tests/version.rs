#![cfg(windows)]
//! The exe carries its version: the installer's "Apps & features" entry and crash reports read it.
#[test]
fn exe_reports_the_crate_version() {
    let exe = env!("CARGO_BIN_EXE_wraith");
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &format!("(Get-Item '{exe}').VersionInfo.ProductVersion")])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), env!("CARGO_PKG_VERSION"));
}
