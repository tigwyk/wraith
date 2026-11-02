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
    assert_eq!(c.scrollback_limit, 10_000_000);
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
fn close_on_exit_defaults_to_clean_and_keeps_boolean_spellings() {
    use wraith_config::CloseOnExit;
    let at = |v: &str| parse_str(&format!("close-on-exit = {v}\n"), p(), &themes()).0.close_on_exit;
    assert_eq!(parse_str("", p(), &themes()).0.close_on_exit, CloseOnExit::Clean);
    assert_eq!(at("clean"), CloseOnExit::Clean);
    assert_eq!((at("true"), at("always")), (CloseOnExit::Always, CloseOnExit::Always));
    assert_eq!((at("false"), at("never")), (CloseOnExit::Never, CloseOnExit::Never));
    let (_, d) = parse_str("close-on-exit = sometimes\n", p(), &themes());
    assert_eq!(d.len(), 1, "a bad value is reported");
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

#[test]
fn scrollback_limit_is_bytes_like_ghostty() {
    let (c, d) = parse_str("scrollback-limit = 10000000
", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.scrollback_limit, 10_000_000);
    // ~12 bytes per cell: 80 columns -> ~10k lines; never zero for tiny limits.
    assert_eq!(c.scrollback_lines(80), 10_416);
    let (c, _) = parse_str("scrollback-limit = 0
", p(), &themes());
    assert_eq!(c.scrollback_lines(80), 0);
}

#[test]
fn scrollback_limit_is_capped() {
    let (c, d) = parse_str("scrollback-limit = 1000000000000
", p(), &themes());
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(c.scrollback_limit, 1 << 30);
}

fn tmp(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wraith-enc-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("config");
    std::fs::write(&f, bytes).unwrap();
    f
}

#[test]
fn utf16_config_from_notepad_parses() {
    let mut bytes = vec![0xFF, 0xFE];
    for u in "font-size = 15
".encode_utf16() {
        bytes.extend(u.to_le_bytes());
    }
    let (c, d) = load(&tmp("utf16", &bytes), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.font_size, 15.0);
}

#[test]
fn non_utf8_config_keeps_valid_lines_and_reports() {
    // cp1252 "Cafe" with e-acute (0xE9) in a font name: not UTF-8.
    let (c, d) = load(&tmp("cp1252", b"font-size = 15\nfont-family = Caf\xe9\n"), &themes());
    assert_eq!(c.font_size, 15.0);
    assert!(d.iter().any(|d| d.message.contains("UTF-8")), "{d:?}");
}

#[test]
fn unreadable_config_is_an_error_not_defaults() {
    let dir = std::env::temp_dir().join(format!("wraith-dir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(wraith_config::load_checked(&dir, &themes()).is_err(), "a directory is not readable as a file");
    assert!(wraith_config::load_checked(&dir.join("missing"), &themes()).is_ok(), "missing file = defaults");
}

#[test]
fn workspace_key_parses_name_dir_layout() {
    let (c, d) = parse_str("workspace = api:C:/src/api:cols\nworkspace = web\nworkspace = lin:/home/x/app\nworkspace = g::grid\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    let w = &c.workspaces;
    assert_eq!((w[0].name.as_str(), w[0].dir.as_deref(), w[0].layout.as_str()), ("api", Some("C:/src/api"), "cols"));
    assert_eq!((w[1].name.as_str(), w[1].dir.as_deref(), w[1].layout.as_str()), ("web", None, "single"));
    assert_eq!((w[2].name.as_str(), w[2].dir.as_deref(), w[2].layout.as_str()), ("lin", Some("/home/x/app"), "single"));
    assert_eq!((w[3].name.as_str(), w[3].dir.as_deref(), w[3].layout.as_str()), ("g", None, "grid"));
    let (c, _) = parse_str("workspace = a\nworkspace =\n", p(), &themes());
    assert!(c.workspaces.is_empty(), "empty value clears");
}

#[test]
fn unknown_layout_is_diagnostic() {
    let (c, d) = parse_str("workspace = api:C:/src:diagonal\n", p(), &themes());
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(c.workspaces.is_empty());
}

#[test]
fn profile_and_default_profile() {
    let (c, d) = parse_str("profile = dev:C:/tools/sh.exe -l\ndefault-profile = dev\nshell-integration = none\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.profiles, vec![("dev".to_string(), "C:/tools/sh.exe -l".to_string())]);
    assert_eq!(c.default_profile.as_deref(), Some("dev"));
    assert!(!c.shell_integration);
    let (c, _) = parse_str("", p(), &themes());
    assert!(c.shell_integration && c.default_profile.is_none() && c.profiles.is_empty());
}

#[test]
fn sidebar_and_split_opacity_clamp() {
    let (c, d) = parse_str("sidebar-visible = false\nsidebar-width = 200\nunfocused-split-opacity = 0.01\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert!(!c.sidebar_visible);
    assert_eq!(c.sidebar_width, 60);
    assert_eq!(c.unfocused_split_opacity, 0.15);
    let (c, _) = parse_str("sidebar-width = 2\nunfocused-split-opacity = 7\n", p(), &themes());
    assert_eq!((c.sidebar_width, c.unfocused_split_opacity), (10, 1.0));
    let (c, _) = parse_str("", p(), &themes());
    assert_eq!((c.sidebar_visible, c.sidebar_width, c.unfocused_split_opacity), (true, 24, 0.85));
}

#[test]
fn image_storage_limit_parses_bytes() {
    let (c, d) = parse_str("image-storage-limit = 1048576\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(c.image_storage_limit, 1_048_576);
    let (c, _) = parse_str("", p(), &themes());
    assert_eq!(c.image_storage_limit, 320 * 1024 * 1024);
}
