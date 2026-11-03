use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wraith_client::app::clamp_font_size;
use wraith_client::reload::{Toasts, fonts_changed, format_diagnostics, reload_file};
use wraith_config::Diagnostic;

fn themes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes")
}

fn diag(line: usize, message: &str) -> Diagnostic {
    Diagnostic { file: PathBuf::from("C:/cfg/config"), line, message: message.into() }
}

#[test]
fn reload_with_bad_key_keeps_previous_good_values() {
    let dir = std::env::temp_dir().join(format!("wraith-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    std::fs::write(&path, "font-size = 14\n").unwrap();
    let (c, toast) = reload_file(&path, &themes());
    assert_eq!((c.font_size, toast), (14.0, None));
    std::fs::write(&path, "font-size = 14\nbogus = 1\n").unwrap();
    let (c, toast) = reload_file(&path, &themes());
    assert_eq!(c.font_size, 14.0);
    let toast = toast.unwrap();
    assert!(toast.contains(":2:") && toast.contains("bogus"), "{toast}");
}

#[test]
fn format_diagnostics_shows_first_and_count() {
    assert_eq!(format_diagnostics(&[]), None);
    let one = format_diagnostics(&[diag(3, "unknown key `x`")]).unwrap();
    assert_eq!(one, "config: C:/cfg/config:3: unknown key `x`");
    let three = format_diagnostics(&[diag(3, "a"), diag(4, "b"), diag(9, "c")]).unwrap();
    assert!(three.ends_with("(and 2 more)"), "{three}");
}

#[test]
fn toasts_throttle_per_kind() {
    let mut t = Toasts::default();
    let now = Instant::now();
    assert!(t.allow("config", now));
    assert!(!t.allow("config", now + Duration::from_secs(2)));
    assert!(t.allow("other", now + Duration::from_secs(2)));
    assert!(t.allow("config", now + Duration::from_secs(4)));
}

#[test]
fn font_size_actions_clamp() {
    assert_eq!(clamp_font_size(3.0), 4.0);
    assert_eq!(clamp_font_size(100.0), 72.0);
    assert_eq!(clamp_font_size(12.5), 12.5);
}

#[test]
fn row_states_match_diagnostics_by_line() {
    use wraith_config::edit::Origin;
    use wraith_config::settings::all;
    let dir = std::env::temp_dir().join(format!("wraith-rows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config");
    std::fs::write(&p, "font-size = abc\ntheme = \n").unwrap();
    let d = vec![wraith_config::Diagnostic { file: p.clone(), line: 1, message: "invalid value `abc`".into() }];
    let rows = wraith_client::reload::row_states(&p, &d);
    let i = all().iter().position(|s| s.key == "font-size").unwrap();
    assert_eq!(rows.len(), all().len());
    assert_eq!(rows[i].origin, Origin::Here { line: 1 });
    assert_eq!(rows[i].warning.as_deref(), Some("invalid value `abc`"));
    assert!(rows.iter().enumerate().all(|(j, r)| j == i || r.warning.is_none()));
}

#[test]
fn profile_font_change_rebuilds() {
    use wraith_client::profiles::merge;
    use wraith_server::profile::{Profile, ShellKind};
    let dir = std::env::temp_dir().join(format!("wraith-reload-pf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    let detected = || vec![Profile { name: "pwsh".into(), kind: ShellKind::Pwsh, command: "pwsh.exe".into(), ..Profile::default() }];
    std::fs::write(&path, "profile-detect = true\n").unwrap();
    let (old, _) = reload_file(&path, &themes());
    std::fs::write(&path, "profile-detect = true\nprofile-font-size = pwsh:20\n").unwrap();
    let (new, toast) = reload_file(&path, &themes());
    assert_eq!(toast, None);
    let (old_p, new_p) = (merge(None, detected, &old).profiles, merge(None, detected, &new).profiles);
    assert!(!fonts_changed(&old, &old, &old_p, &old_p));
    assert!(fonts_changed(&old, &new, &old_p, &new_p));
}
