//! The merged profile list (wt-profiles spec §3) and colours per profile.
use std::collections::HashMap;
use std::path::PathBuf;

use wraith_client::profiles::merge;
use wraith_client::view::SpawnCtx;
use wraith_config::wt::{WtProfile, WtSettings};
use wraith_config::{Config, ProfileExtra, Rgb, Scheme};
use wraith_server::profile::{Profile, ShellKind};

fn wt_profile(name: &str, command: &str) -> WtProfile {
    WtProfile { name: name.into(), command: command.into(), dir: None, icon: None, scheme: None, ..WtProfile::default() }
}

fn wt() -> Result<WtSettings, String> {
    let mut ubuntu = wt_profile("Ubuntu 22.04", "wsl.exe -d Ubuntu");
    ubuntu.scheme = Some("Mine".into());
    ubuntu.dir = Some("~".into());
    Ok(WtSettings {
        profiles: vec![wt_profile("Windows PowerShell", "powershell.exe"), wt_profile("cmd", "cmd.exe"), ubuntu],
        default: Some("cmd".into()),
        ..WtSettings::default()
    })
}

fn detected() -> Vec<Profile> {
    vec![Profile { name: "pwsh".into(), kind: ShellKind::Pwsh, command: "pwsh.exe".into(), ..Profile::default() }]
}

fn names(p: &[Profile]) -> Vec<&str> {
    p.iter().map(|p| p.name.as_str()).collect()
}

#[test]
fn wt_profiles_in_order_and_no_detection() {
    let m = merge(Some(&wt()), || panic!("detection must not run when WT was read"), &Config::default());
    assert_eq!(names(&m.profiles), ["Windows PowerShell", "cmd", "Ubuntu 22.04"]);
    let u = &m.profiles[2];
    assert_eq!(u.kind, ShellKind::Wsl { distro: "Ubuntu".into() });
    assert_eq!((u.scheme.as_deref(), u.dir.as_deref()), (Some("Mine"), Some("~")));
}

#[test]
fn config_profile_replaces_in_place_and_appends() {
    let mut c = Config::default();
    c.profiles = vec![("CMD".into(), "cmd.exe /k".into()), ("extra".into(), "x.exe".into())];
    let m = merge(Some(&wt()), Vec::new, &c);
    assert_eq!(names(&m.profiles), ["Windows PowerShell", "CMD", "Ubuntu 22.04", "extra"]);
    assert_eq!(m.profiles[1].command, "cmd.exe /k");
}

#[test]
fn extras_apply_and_unknown_warns() {
    let mut c = Config::default();
    c.profile_extras = vec![
        ("cmd".into(), ProfileExtra::Dir, r"C:\src".into()),
        ("cmd".into(), ProfileExtra::Theme, "Campbell".into()),
        ("cmd".into(), ProfileExtra::Icon, "🐢".into()),
        ("ghost".into(), ProfileExtra::Dir, r"C:\x".into()),
    ];
    let m = merge(Some(&wt()), Vec::new, &c);
    let cmd = &m.profiles[1];
    assert_eq!((cmd.dir.as_deref(), cmd.scheme.as_deref(), cmd.icon.as_deref()), (Some(r"C:\src"), Some("Campbell"), Some("🐢")));
    assert!(m.warnings.iter().any(|w| w.contains("ghost")), "{:?}", m.warnings);
}

fn wt_with_appearance() -> Result<WtSettings, String> {
    let mut s = wt()?;
    s.profiles[2].font_size = Some(14.0);
    s.profiles[2].opacity = Some(0.9);
    Ok(s)
}

#[test]
fn wt_appearance_reaches_profile() {
    let m = merge(Some(&wt_with_appearance()), Vec::new, &Config::default());
    assert_eq!((m.profiles[2].font_size, m.profiles[2].opacity), (Some(14.0), Some(0.9)));
}

#[test]
fn profile_lines_override_wt_appearance() {
    let mut c = Config::default();
    c.profile_extras = vec![
        ("Ubuntu 22.04".into(), ProfileExtra::FontSize, "11".into()),
        ("Ubuntu 22.04".into(), ProfileExtra::Padding, "3".into()),
    ];
    let m = merge(Some(&wt_with_appearance()), Vec::new, &c);
    let u = &m.profiles[2];
    assert_eq!((u.font_size, u.padding, u.opacity), (Some(11.0), Some((3, 3)), Some(0.9)));
}

#[test]
fn wt_off_uses_detected() {
    let mut c = Config::default();
    c.windows_terminal_profiles = false;
    let m = merge(Some(&wt()), detected, &c);
    assert_eq!(names(&m.profiles), ["pwsh"]);
    assert_eq!(m.default, None);
}

#[test]
fn wt_error_falls_back_to_detected() {
    let m = merge(Some(&Err("broken".into())), detected, &Config::default());
    assert_eq!(names(&m.profiles), ["pwsh"]);
    assert!(m.warnings.iter().any(|w| w.contains("broken")));
    assert_eq!(names(&merge(None, detected, &Config::default()).profiles), ["pwsh"]);
}

#[test]
fn default_from_config_then_wt() {
    assert_eq!(merge(Some(&wt()), Vec::new, &Config::default()).default.as_deref(), Some("cmd"));
    let mut c = Config::default();
    c.default_profile = Some("Ubuntu 22.04".into());
    assert_eq!(merge(Some(&wt()), Vec::new, &c).default.as_deref(), Some("Ubuntu 22.04"));
}

fn ctx_with_scheme() -> SpawnCtx {
    let mine = Scheme { fg: Rgb(1, 2, 3), bg: Rgb(4, 5, 6), cursor: None, palette: [Rgb(9, 9, 9); 16] };
    SpawnCtx {
        config: Config::default(),
        profiles: vec![
            Profile { name: "plain".into(), command: "a.exe".into(), ..Profile::default() },
            Profile { name: "themed".into(), command: "b.exe".into(), scheme: Some("Mine".into()), ..Profile::default() },
        ],
        schemes: HashMap::from([("Mine".to_string(), mine)]),
        scripts: PathBuf::new(),
        env: vec![],
    }
}

#[test]
fn colors_use_profile_scheme() {
    let ctx = ctx_with_scheme();
    let (fg, bg, palette) = ctx.colors("themed");
    assert_eq!((fg, bg), ((1, 2, 3), (4, 5, 6)));
    assert_eq!(palette.len(), 256);
    assert_eq!(palette[15], (9, 9, 9));
    let g = &ctx.config.palette[16];
    assert_eq!(palette[16], (g.0, g.1, g.2), "only the 16 ANSI colours change");
}

#[test]
fn unknown_profile_gets_global_colors() {
    let ctx = ctx_with_scheme();
    let c = &ctx.config;
    let global = ((c.foreground.0, c.foreground.1, c.foreground.2), (c.background.0, c.background.1, c.background.2));
    for name in ["plain", "removed-from-wt"] {
        let (fg, bg, palette) = ctx.colors(name);
        assert_eq!((fg, bg), global);
        assert_eq!(palette[1], (c.palette[1].0, c.palette[1].1, c.palette[1].2));
    }
}

#[test]
fn profile_detect_off_skips_detection() {
    let mut c = Config::default();
    c.windows_terminal_profiles = false;
    c.profile_detect = false;
    c.profiles = vec![("mine".into(), "x.exe".into())];
    let m = merge(None, || panic!("detection is off"), &c);
    assert_eq!(names(&m.profiles), ["mine"]);
}

#[test]
fn empty_list_falls_back_to_detected() {
    let mut c = Config::default();
    c.windows_terminal_profiles = false;
    c.profile_detect = false;
    assert_eq!(names(&merge(None, detected, &c).profiles), ["pwsh"], "a window always has a profile to start");
}

// Per-pane appearance (per-profile-appearance spec §2.3, §3.3).
use wraith_client::profiles::appearance;

#[test]
fn appearance_defaults_to_global() {
    let c = Config::default();
    let a = appearance(None, &c, 0.0, &|_| true);
    assert_eq!(a.families, c.font_family);
    assert_eq!(a.size_pt, c.font_size);
    assert_eq!(a.pad, (c.window_padding_x, c.window_padding_y));
    assert_eq!(a.opacity, c.background_opacity);
}

#[test]
fn appearance_profile_wins_and_zoom_adds() {
    let c = Config::default();
    let p = Profile { font: Some(vec!["A".into()]), font_size: Some(14.0), padding: Some((8, 4)), opacity: Some(0.8), ..detected().remove(0) };
    let a = appearance(Some(&p), &c, 2.0, &|_| true);
    let mut want = vec!["A".to_string()];
    want.extend(c.font_family.iter().filter(|f| *f != "A").cloned());
    assert_eq!(a.families, want);
    assert_eq!((a.size_pt, a.pad, a.opacity), (16.0, (8, 4), 0.8));
    assert_eq!(appearance(Some(&p), &c, 100.0, &|_| true).size_pt, 72.0);
}

#[test]
fn uninstalled_family_falls_back_to_window_families() {
    let c = Config::default();
    let p = Profile { font: Some(vec!["Cascadia Mono".into()]), font_size: Some(14.0), ..detected().remove(0) };
    let a = appearance(Some(&p), &c, 0.0, &|f| f != "Cascadia Mono");
    assert_eq!(a.families, c.font_family);
    assert_eq!(a.size_pt, 14.0);
    assert!(a.fell_back, "the app warns about it");
    assert!(!appearance(Some(&p), &c, 0.0, &|_| true).fell_back);
    assert!(!appearance(None, &c, 0.0, &|_| false).fell_back, "no profile font, nothing to fall back from");
}

#[test]
fn window_appearance_is_normalised() {
    // The window font's key comes from here too, so a profile that sets nothing shares its font.
    let mut c = Config::default();
    (c.font_family, c.font_size) = (vec!["A".into(), "a".into(), "B".into()], 100.0);
    let a = appearance(None, &c, 0.0, &|_| true);
    assert_eq!((a.families, a.size_pt), (vec!["A".to_string(), "B".to_string()], 72.0));
}
