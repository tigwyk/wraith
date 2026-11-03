use std::path::{Path, PathBuf};

use wraith_config::Rgb;
use wraith_config::wt::{WtSettings, load_from, parse, strip_jsonc};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wt").join(name)).unwrap()
}

fn themes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/themes")
}

fn env(k: &str) -> Option<String> {
    match k {
        "SystemRoot" => Some(r"C:\Windows".into()),
        "USERPROFILE" => Some(r"C:\Users\u".into()),
        _ => None,
    }
}

fn settings() -> WtSettings {
    parse(&fixture("settings.json"), &[("Vendor.App".into(), fixture("fragment.json"))], &env).unwrap()
}

fn command(s: &WtSettings, name: &str) -> String {
    s.profiles.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no {name}")).command.clone()
}

#[test]
fn strip_jsonc_keeps_strings() {
    let v: serde_json::Value = serde_json::from_str(&strip_jsonc(r#"{"a":"//x", /* c */ "b":[1,],}// end"#)).unwrap();
    assert_eq!(v, serde_json::json!({"a": "//x", "b": [1]}));
    let v: serde_json::Value = serde_json::from_str(&strip_jsonc(r#"{"a":"q\"//,]"}"#)).unwrap();
    assert_eq!(v, serde_json::json!({"a": "q\"//,]"}));
}

#[test]
fn builtin_guids_get_commands() {
    let s = settings();
    assert_eq!(command(&s, "Windows PowerShell"), r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
    assert_eq!(command(&s, "cmd"), r"C:\Windows\System32\cmd.exe");
}

#[test]
fn sources_get_commands() {
    let s = settings();
    assert_eq!(command(&s, "Ubuntu-22.04"), "wsl.exe -d Ubuntu-22.04");
    assert_eq!(command(&s, "PowerShell 7"), "pwsh.exe");
}

#[test]
fn azure_is_skipped_with_reason() {
    let s = settings();
    assert!(!s.profiles.iter().any(|p| p.name == "Azure Cloud Shell"));
    assert!(s.skipped.iter().any(|(n, _)| n == "Azure Cloud Shell"), "{:?}", s.skipped);
}

#[test]
fn hidden_is_skipped() {
    assert!(!settings().profiles.iter().any(|p| p.name == "Secret"));
}

#[test]
fn env_vars_expand_and_unknown_stay() {
    let s = settings();
    let c = s.profiles.iter().find(|p| p.name == "Custom").unwrap();
    assert_eq!(c.command, r"C:\Windows\System32\cmd.exe /k");
    assert_eq!(c.dir.as_deref(), Some(r"C:\Users\u\src"));
    assert_eq!(command(&s, "Echo"), "cmd.exe /k echo http://x /* not a comment */ %NOPE%");
}

#[test]
fn defaults_merge_and_entry_wins() {
    let s = settings();
    assert_eq!(s.profiles.iter().find(|p| p.name == "cmd").unwrap().scheme.as_deref(), Some("Campbell"));
    assert_eq!(s.profiles.iter().find(|p| p.name == "Custom").unwrap().scheme.as_deref(), Some("Mine"));
}

#[test]
fn fragment_merges_by_guid_and_appends() {
    let s = settings();
    assert_eq!(s.profiles.iter().find(|p| p.name == "Custom").unwrap().icon.as_deref(), Some("🦀"));
    assert_eq!(s.profiles.last().unwrap().name, "From Fragment");
    assert_eq!(s.profiles.last().unwrap().command, "bash.exe");
}

#[test]
fn fragment_update_fills_missing_keys() {
    let src = r#"{"profiles":{"list":[{"guid":"{g}","name":"A","commandline":"a.exe"}]}}"#;
    let frag = r#"{"profiles":[{"updates":"{g}","icon":"a.png"}]}"#.to_string();
    let s = parse(src, &[("X".into(), frag)], &env).unwrap();
    assert_eq!(s.profiles[0].icon.as_deref(), Some("a.png"));
}

#[test]
fn duplicate_names_keep_first() {
    let s = settings();
    assert_eq!(s.profiles.iter().filter(|p| p.name == "Custom").count(), 1);
    assert_eq!(command(&s, "Custom"), r"C:\Windows\System32\cmd.exe /k");
}

#[test]
fn default_resolves_from_guid_and_name() {
    assert_eq!(settings().default.as_deref(), Some("Windows PowerShell"));
    let src = fixture("settings.json").replace(r#""defaultProfile": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}""#, r#""defaultProfile": "cmd""#);
    assert_eq!(parse(&src, &[], &env).unwrap().default.as_deref(), Some("cmd"));
}

#[test]
fn scheme_lookup_order() {
    let s = settings();
    let mine = s.scheme("Mine", &themes()).unwrap();
    assert_eq!(mine.fg, Rgb(0x10, 0x10, 0x10));
    assert_eq!(mine.bg, Rgb(0x20, 0x20, 0x20));
    assert_eq!(mine.cursor, Some(Rgb(0x30, 0x30, 0x30)));
    assert_eq!(mine.palette[0], Rgb(0, 0, 1));
    assert_eq!(mine.palette[15], Rgb(0, 0, 0x10));
    let campbell = s.scheme("Campbell", &themes()).unwrap();
    assert_eq!(campbell.bg, Rgb(0x0c, 0x0c, 0x0c));
    assert_eq!(campbell.palette[1], Rgb(0xc5, 0x0f, 0x1f));
    assert!(s.scheme("Nope", &themes()).is_none());
}

#[test]
fn truncated_file_is_an_error() {
    let f = fixture("settings.json");
    assert!(parse(&f[..f.len() / 2], &[], &env).is_err());
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wraith-wt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn load_from_reads_first_file_and_fragments() {
    let local = tmp("first");
    let pd = tmp("first-pd");
    let stable = r#"{"profiles":{"list":[{"name":"Stable","commandline":"s.exe"}]}}"#;
    let unpackaged = r#"{"profiles":{"list":[{"name":"Unpackaged","commandline":"u.exe"}]}}"#;
    put(&local.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"), stable);
    put(&local.join(r"Microsoft\Windows Terminal\settings.json"), unpackaged);
    put(&local.join(r"Microsoft\Windows Terminal\Fragments\Vendor.App\f.json"), r#"{"profiles":[{"name":"Frag","commandline":"f.exe"}]}"#);
    put(&pd.join(r"Microsoft\Windows Terminal\Fragments\Other\g.json"), r#"{"profiles":[{"name":"Machine","commandline":"m.exe"}]}"#);
    let s = load_from(&local, &pd).unwrap().unwrap();
    let names: Vec<&str> = s.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Stable", "Frag", "Machine"]);
    assert_eq!(s.family.as_deref(), Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe"));
}

#[test]
fn oversized_fragment_is_skipped() {
    let local = tmp("big");
    put(&local.join(r"Microsoft\Windows Terminal\settings.json"), r#"{"profiles":{"list":[]}}"#);
    let big = format!(r#"{{"profiles":[{{"name":"Big","commandline":"b.exe","pad":"{}"}}]}}"#, "x".repeat(2 << 20));
    put(&local.join(r"Microsoft\Windows Terminal\Fragments\V\big.json"), &big);
    let s = load_from(&local, &tmp("big-pd")).unwrap().unwrap();
    assert!(s.profiles.is_empty());
    assert_eq!(s.family, None);
}

#[test]
fn load_from_without_wt_is_none() {
    assert!(load_from(&tmp("none"), &tmp("none-pd")).is_none());
}

#[test]
fn unquoted_exe_path_with_spaces_is_quoted() {
    let env = |k: &str| (k == "PROGRAMFILES").then(|| r"C:\Program Files".to_string());
    let cmd = |c: &str| {
        let src = format!(r#"{{"profiles":{{"list":[{{"name":"A","commandline":{}}}]}}}}"#, serde_json::to_string(c).unwrap());
        parse(&src, &[], &env).unwrap().profiles[0].command.clone()
    };
    assert_eq!(cmd("%PROGRAMFILES%/PowerShell/7/pwsh.exe -nologo"), r#""C:\Program Files/PowerShell/7/pwsh.exe" -nologo"#);
    assert_eq!(cmd("%PROGRAMFILES%/PowerShell/7/pwsh.exe"), r#""C:\Program Files/PowerShell/7/pwsh.exe""#);
    assert_eq!(cmd("cmd.exe /k echo a b"), "cmd.exe /k echo a b");
    assert_eq!(cmd(r#""C:\Program Files\x.exe" -a"#), r#""C:\Program Files\x.exe" -a"#);
    assert_eq!(cmd("wsl -d Ubuntu"), "wsl -d Ubuntu", "no .exe: left as written");
}

#[test]
fn resolve_icon_rules() {
    use wraith_config::wt::resolve_icon;
    let dir = Path::new(r"C:\WT");
    assert_eq!(resolve_icon(r"C:\icons\a.png", Some(dir)), Some(PathBuf::from(r"C:\icons\a.png")));
    assert_eq!(resolve_icon("ms-appx:///ProfileIcons/x.png", Some(dir)), Some(dir.join("ProfileIcons").join("x.png")));
    assert_eq!(resolve_icon("ms-appx:///ProfileIcons/x.png", None), None, "no package folder");
    assert_eq!(resolve_icon("ms-appx:///../../evil.png", Some(dir)), None, "stays inside the package");
    assert_eq!(resolve_icon("🦀", Some(dir)), None);
    assert_eq!(resolve_icon(r"C:\Program Files\WSL\wsl.exe", Some(dir)), None, "only PNG files");
}

#[test]
fn profiles_without_icon_get_wts_default_by_guid() {
    let s = settings();
    let icon = |n: &str| s.profiles.iter().find(|p| p.name == n).unwrap().icon.clone();
    assert_eq!(icon("cmd").as_deref(), Some("ms-appx:///ProfileIcons/{0caa0dad-35be-5f56-a8ff-afceeeaa6101}.png"));
    assert_eq!(icon("Custom").as_deref(), Some("🦀"), "a profile's own icon wins");
    assert_eq!(icon("From Fragment").as_deref(), Some("ms-appx:///ProfileIcons/{11111111-0000-0000-0000-000000000001}.png"));
}

#[test]
fn disabled_profile_sources_are_left_out() {
    let src = r#"{"disabledProfileSources":["Windows.Terminal.Wsl","Vendor.App"],
        "profiles":{"list":[{"name":"Ubuntu","source":"Windows.Terminal.Wsl"},{"name":"cmd","commandline":"cmd.exe"}]}}"#;
    let frag = r#"{"profiles":[{"name":"Frag","commandline":"f.exe"}]}"#.to_string();
    let s = parse(src, &[("Vendor.App".into(), frag)], &env).unwrap();
    let names: Vec<&str> = s.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["cmd"]);
}

#[test]
fn quote_exe_leaves_later_exe_arguments_alone() {
    let cmd = |c: &str| {
        let src = format!(r#"{{"profiles":{{"list":[{{"name":"A","commandline":{}}}]}}}}"#, serde_json::to_string(c).unwrap());
        parse(&src, &[], &env).unwrap().profiles[0].command.clone()
    };
    assert_eq!(cmd("wsl -d Ubuntu -- /mnt/c/x y/foo.exe"), "wsl -d Ubuntu -- /mnt/c/x y/foo.exe");
    assert_eq!(cmd(r"C:\tools\run.cmd C:\a b\x.exe"), r"C:\tools\run.cmd C:\a b\x.exe");
    assert_eq!(cmd(r"C:\Program Files\x.exe -a"), r#""C:\Program Files\x.exe" -a"#);
}

#[test]
fn wsl_distro_name_with_spaces_is_quoted() {
    let src = r#"{"profiles":{"list":[{"name":"My Distro","source":"Windows.Terminal.Wsl"}]}}"#;
    assert_eq!(parse(src, &[], &env).unwrap().profiles[0].command, r#"wsl.exe -d "My Distro""#);
}
