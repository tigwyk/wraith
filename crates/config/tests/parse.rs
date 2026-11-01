use std::path::{Path, PathBuf};

use wraith_config::{CursorStyle, Rgb, load, parse_str};

fn p() -> &'static Path {
    Path::new("C:/cfg/config")
}
fn themes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/themes")
}

#[test]
fn parses_basic_keys() {
    let (c, d) = parse_str("font-size = 14\nfont-family = Iosevka\nfont-family = Segoe UI Emoji\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.font_size, 14.0);
    assert_eq!(c.font_family, vec!["Iosevka", "Segoe UI Emoji"]);
}

#[test]
fn defaults() {
    let (c, _) = parse_str("", p(), &themes());
    assert_eq!(c.font_family, vec!["Cascadia Mono"]);
    assert_eq!(c.font_size, 12.0);
    assert_eq!(c.scrollback_limit, 10_000);
    assert_eq!(c.cursor_style, CursorStyle::Block);
    assert_eq!(c.palette[1], Rgb(0xcd, 0, 0)); // xterm default red
    assert_eq!(c.palette[255], Rgb(0xee, 0xee, 0xee));
}

#[test]
fn unknown_key_keeps_default_and_reports_line() {
    let (c, d) = parse_str("font-size = 14\nfont-sise = 99\n", p(), &themes());
    assert_eq!(c.font_size, 14.0);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].line, 2);
}

#[test]
fn invalid_value_keeps_default() {
    let (c, d) = parse_str("font-size = huge\n", p(), &themes());
    assert_eq!(c.font_size, 12.0);
    assert_eq!(d[0].line, 1);
}

#[test]
fn bom_and_crlf_are_ignored() {
    let (c, d) = parse_str("\u{feff}font-size = 15\r\ncopy-on-select = true\r\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.font_size, 15.0);
    assert!(c.copy_on_select);
}

#[test]
fn comments_quotes_and_empty_value_reset() {
    let (c, _) = parse_str("# x\nfont-family = \"Cascadia Code\"\nfont-family =\nfont-family = Iosevka\n", p(), &themes());
    assert_eq!(c.font_family, vec!["Iosevka"]);
}

#[test]
fn theme_then_override() {
    let (c, d) = parse_str("background = #000001\ntheme = Dracula\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.background, Rgb(0, 0, 1)); // explicit beats theme regardless of order
    assert_eq!(c.foreground, Rgb(0xf8, 0xf8, 0xf2));
    assert_eq!(c.palette[1], Rgb(0xff, 0x55, 0x55));
}

#[test]
fn unknown_theme_is_reported() {
    let (_, d) = parse_str("theme = NoSuchTheme\n", p(), &themes());
    assert_eq!(d.len(), 1);
}

#[test]
fn palette_and_enums() {
    let (c, d) = parse_str("palette = 4=#0000ff\ncursor-style = bar\nright-click-action = menu\nbackground-opacity = 0.9\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.palette[4], Rgb(0, 0, 255));
    assert_eq!(c.cursor_style, CursorStyle::Bar);
    assert_eq!(c.background_opacity, 0.9);
}

#[test]
fn include_cycle_is_reported_not_infinite() {
    let dir = std::env::temp_dir().join(format!("wraith-cfg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a"), "font-size = 13\nconfig-file = b\n").unwrap();
    std::fs::write(dir.join("b"), "config-file = a\n").unwrap();
    let (c, d) = load(&dir.join("a"), &themes());
    assert_eq!(c.font_size, 13.0);
    assert_eq!(d.len(), 1, "{d:?}");
}

#[test]
fn optional_missing_include_is_silent() {
    let (_, d) = parse_str("config-file = ?C:/nope/missing\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn every_bundled_theme_parses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let name = e.unwrap().file_name().into_string().unwrap();
        if name == "LICENSE" || name == "README.md" {
            continue;
        }
        let (c, d) = parse_str(&format!("theme = {name}\n"), p(), &dir);
        assert!(d.is_empty(), "{name}: {d:?}");
        assert_eq!(c.theme.as_deref(), Some(name.as_str()));
        n += 1;
    }
    assert!(n > 800, "only {n} themes");
}
