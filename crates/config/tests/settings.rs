use std::path::{Path, PathBuf};

use wraith_config::edit::set;
use wraith_config::settings::{Kind, Page, all, on_page};
use wraith_config::{Config, parse_str};

fn themes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/themes")
}

/// Values each control can write.
fn samples(kind: &Kind) -> Vec<String> {
    match kind {
        Kind::Toggle => vec!["true".into(), "false".into()],
        Kind::Choice(cs) => cs.iter().map(|(v, _)| v.to_string()).collect(),
        Kind::Stepper { min, max } => vec![min.to_string(), max.to_string()],
        Kind::Slider { min, max, .. } => vec![format!("{min:.2}"), format!("{max:.2}")],
        Kind::NotifySet => vec!["toast,sound".into(), "none".into(), "toast,flash,badge,sound".into()],
        Kind::QuietHours => vec!["22:00-07:30".into()],
        Kind::List(_) => vec![],
    }
}

#[test]
fn every_value_round_trips_through_the_parser() {
    for s in all() {
        for v in samples(&s.kind) {
            let (c, d) = parse_str(&set("", s.key, &v, s.which), Path::new("config"), &themes());
            assert!(d.is_empty(), "{} = {v}: {d:?}", s.key);
            assert_eq!((s.get)(&c), v, "{}", s.key);
        }
    }
}

#[test]
fn list_values_round_trip() {
    let theme = std::fs::read_dir(themes()).unwrap().next().unwrap().unwrap().file_name().into_string().unwrap();
    for (key, v) in [("theme", theme.as_str()), ("font-family", "Consolas"), ("default-profile", "pwsh")] {
        let s = all().iter().find(|s| s.key == key).unwrap();
        let (c, d) = parse_str(&set("", key, v, s.which), Path::new("config"), &themes());
        assert!(d.is_empty(), "{key}: {d:?}");
        assert_eq!((s.get)(&c), v, "{key}");
    }
}

#[test]
fn defaults_get_what_the_parser_defaults_to() {
    let c = Config::default();
    for s in all() {
        let v = (s.get)(&c);
        let (c2, d) = parse_str(&set("", s.key, &v, s.which), Path::new("config"), &themes());
        assert!(d.is_empty(), "{} = {v}: {d:?}", s.key);
        assert_eq!((s.get)(&c2), v, "{}", s.key);
    }
}

#[test]
fn pages_hold_the_spec_rows() {
    assert_eq!(all().len(), 20);
    assert_eq!(on_page(Page::Appearance).len(), 9);
    assert_eq!(on_page(Page::Behaviour).len(), 7);
    assert_eq!(on_page(Page::Agents).len(), 4);
    assert_eq!(all()[on_page(Page::Agents)[0]].key, "agent-notify");
}
