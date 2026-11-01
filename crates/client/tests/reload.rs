use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wraith_client::app::clamp_font_size;
use wraith_client::reload::{Toasts, format_diagnostics, reload_file};
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
