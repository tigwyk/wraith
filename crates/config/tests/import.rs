//! `wr import-wt`: Windows Terminal profiles written into the Wraith config (wt-profiles spec §4).
use std::path::{Path, PathBuf};

use wraith_config::Rgb;
use wraith_config::wt::{ImportOutcome, Scheme, WtProfile, WtSettings, import, plan_import};

fn themes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/themes")
}

fn prof(name: &str, command: &str) -> WtProfile {
    WtProfile { name: name.into(), command: command.into(), dir: None, icon: None, scheme: None, ..WtProfile::default() }
}

fn scheme(n: u8) -> Scheme {
    Scheme { fg: Rgb(n, n, n), bg: Rgb(0, 0, n), cursor: Some(Rgb(1, 1, 1)), palette: std::array::from_fn(|i| Rgb(i as u8, 0, 0)) }
}

fn wt() -> WtSettings {
    let mut ubuntu = prof("Ubuntu", "wsl.exe -d Ubuntu");
    ubuntu.dir = Some("~".into());
    ubuntu.scheme = Some("Mine".into());
    ubuntu.icon = Some("🦀".into());
    let mut camp = prof("cmd", "cmd.exe");
    camp.scheme = Some("Campbell".into());
    camp.icon = Some("ms-appx:///ProfileIcons/cmd.png".into());
    WtSettings {
        profiles: vec![ubuntu, camp, prof("a:b", "x.exe")],
        default: Some("Ubuntu".into()),
        schemes: vec![("Mine".into(), scheme(7)), ("Campbell".into(), scheme(8))],
        skipped: vec![("Azure Cloud Shell".into(), "only Windows Terminal can start it".into())],
        family: None,
        ..WtSettings::default()
    }
}

fn outcome<'a>(r: &'a wraith_config::wt::ImportReport, name: &str) -> &'a ImportOutcome {
    &r.rows.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no row {name}")).1
}

#[test]
fn writes_profile_lines_and_extras() {
    let (src, report, _) = plan_import("# mine\n", &wt(), &themes());
    let lines: Vec<&str> = src.lines().collect();
    assert_eq!(
        lines,
        [
            "# mine",
            "profile = Ubuntu:wsl.exe -d Ubuntu",
            "profile-dir = Ubuntu:~",
            "profile-theme = Ubuntu:Mine",
            "profile-icon = Ubuntu:🦀",
            "profile = cmd:cmd.exe",
            "profile-theme = cmd:Campbell",
            "default-profile = Ubuntu",
            "profile-detect = false",
            "windows-terminal-profiles = false",
        ]
    );
    assert_eq!(report.imported(), 2);
    assert_eq!(outcome(&report, "Ubuntu"), &ImportOutcome::Imported);
    assert!(matches!(outcome(&report, "Azure Cloud Shell"), ImportOutcome::Unsupported(_)));
}

#[test]
fn existing_default_is_kept() {
    let (src, _, _) = plan_import("default-profile = pwsh\n", &wt(), &themes());
    assert!(src.starts_with("default-profile = pwsh\n") && !src.contains("default-profile = Ubuntu"), "{src}");
}

#[test]
fn second_run_writes_nothing() {
    let (once, _, _) = plan_import("", &wt(), &themes());
    let (twice, report, themes_out) = plan_import(&once, &wt(), &themes());
    assert_eq!(twice, once);
    assert_eq!(report.imported(), 0);
    assert_eq!(outcome(&report, "cmd"), &ImportOutcome::Exists);
    assert!(themes_out.is_empty() || themes_out.iter().all(|(n, _)| n == "Mine"));
}

#[test]
fn comments_and_crlf_kept() {
    let (src, _, _) = plan_import("# keep me\r\nfont-size = 14\r\n", &wt(), &themes());
    assert!(src.starts_with("# keep me\r\nfont-size = 14\r\nprofile = Ubuntu:wsl.exe -d Ubuntu\r\n"), "{src:?}");
    assert!(!src.replace("\r\n", "").contains('\n'), "every line ends CRLF");
}

#[test]
fn colon_in_name_is_skipped() {
    let (src, report, _) = plan_import("", &wt(), &themes());
    assert_eq!(outcome(&report, "a:b"), &ImportOutcome::Unsupported("name contains ':'".into()));
    assert!(!src.contains("a:b"));
}

#[test]
fn wt_only_scheme_becomes_theme_file() {
    let (_, _, files) = plan_import("", &wt(), &themes());
    assert_eq!(files.len(), 1, "Campbell exists in the themes folder: {files:?}");
    let (name, text) = &files[0];
    assert_eq!(name, "Mine");
    for line in ["palette = 0=#000000", "palette = 15=#0f0000", "foreground = #070707", "background = #000007", "cursor-color = #010101"] {
        assert!(text.lines().any(|l| l == line), "{line} missing from {text}");
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wraith-import-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn import_does_not_overwrite_theme() {
    let dir = tmp("keep");
    std::fs::create_dir_all(dir.join("themes")).unwrap();
    std::fs::write(dir.join("themes").join("Mine"), "background = #123456\n").unwrap();
    let report = import(&dir.join("config"), &themes(), &wt()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("themes").join("Mine")).unwrap(), "background = #123456\n");
    assert!(report.themes_written.is_empty());
    assert!(std::fs::read_to_string(dir.join("config")).unwrap().contains("profile = Ubuntu:"));
}

#[test]
fn import_writes_new_theme_beside_config() {
    let dir = tmp("new");
    let report = import(&dir.join("config"), &themes(), &wt()).unwrap();
    assert_eq!(report.themes_written, ["Mine"]);
    assert!(std::fs::read_to_string(dir.join("themes").join("Mine")).unwrap().contains("background = #000007"));
}

#[test]
fn scheme_names_that_are_not_file_names_are_not_written() {
    let mut w = wt();
    w.profiles[0].scheme = Some("Bad: Name?".into());
    w.schemes.push(("Bad: Name?".into(), scheme(3)));
    let (_, _, files) = plan_import("", &w, &themes());
    assert!(files.iter().all(|(n, _)| n != "Bad: Name?"), "{files:?}");
}
