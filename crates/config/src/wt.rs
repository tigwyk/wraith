//! Windows Terminal's settings (wt-profiles spec §2): its profiles and colour schemes, read so Wraith
//! can offer the same shells. Read only; WT's files are never written.
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::Rgb;
pub use crate::theme::scheme_from_theme;

/// A terminal colour scheme: the 16 ANSI colours and the defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; 16],
}

/// A WT profile Wraith can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WtProfile {
    pub name: String,
    pub command: String,
    pub dir: Option<String>,
    pub icon: Option<String>,
    /// The colour scheme's name (the dark one of a light/dark pair).
    pub scheme: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WtSettings {
    pub profiles: Vec<WtProfile>,
    /// The default profile's name.
    pub default: Option<String>,
    pub schemes: Vec<(String, Scheme)>,
    /// Profiles left out: (name, why).
    pub skipped: Vec<(String, String)>,
    /// The package family of the settings file read; `None` for unpackaged WT.
    pub family: Option<String>,
}

impl WtSettings {
    /// Scheme `name` from WT's own schemes, else from Wraith's themes folder.
    pub fn scheme(&self, name: &str, themes_dir: &Path) -> Option<Scheme> {
        self.schemes.iter().find(|(n, _)| n == name).map(|(_, s)| s.clone()).or_else(|| scheme_from_theme(themes_dir, name))
    }
}

const WINDOWS_POWERSHELL: &str = "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}";
const CMD: &str = "{0caa0dad-35be-5f56-a8ff-afceeeaa6101}";
/// Larger files are skipped unread, so a runaway fragment cannot stall start-up.
const MAX_FILE: u64 = 1 << 20;

/// `src` as plain JSON: `//` and `/* */` comments and trailing commas removed, strings untouched.
pub fn strip_jsonc(src: &str) -> String {
    let c: Vec<char> = src.trim_start_matches('\u{feff}').chars().collect();
    // Index of the first character at or after `i` that is not whitespace or inside a comment.
    let skip = |mut i: usize| -> usize {
        loop {
            match (c.get(i), c.get(i + 1)) {
                (Some(ch), _) if ch.is_whitespace() => i += 1,
                (Some('/'), Some('/')) => {
                    while c.get(i).is_some_and(|&ch| ch != '\n') {
                        i += 1;
                    }
                }
                (Some('/'), Some('*')) => {
                    i += 2;
                    while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                        i += 1;
                    }
                    i += 2;
                }
                _ => return i,
            }
        }
    };
    let mut out = String::with_capacity(src.len());
    let (mut i, mut in_str) = (0, false);
    while i < c.len() {
        let ch = c[i];
        if in_str {
            out.push(ch);
            if ch == '\\' {
                if let Some(&n) = c.get(i + 1) {
                    out.push(n);
                }
                i += 2;
                continue;
            }
            in_str = ch != '"';
            i += 1;
            continue;
        }
        match (ch, c.get(i + 1)) {
            ('"', _) => {
                in_str = true;
                out.push(ch);
                i += 1;
            }
            ('/', Some('/' | '*')) => {
                out.push(' ');
                i = skip(i);
            }
            (',', _) if matches!(c.get(skip(i + 1)), Some(']' | '}')) => i += 1,
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// `%NAME%` replaced by `env(NAME)`; unknown names stay as written.
fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%').and_then(|j| env(&after[..j]).map(|v| (j, v))) {
            Some((j, v)) if j > 0 => {
                out.push_str(&v);
                rest = &after[j + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn hex(v: Option<&Value>) -> Option<Rgb> {
    let s = v?.as_str()?.strip_prefix('#')?;
    let n = u32::from_str_radix(s.get(..6)?, 16).ok()?;
    Some(Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

const PALETTE_KEYS: [&str; 16] = [
    "black", "red", "green", "yellow", "blue", "purple", "cyan", "white",
    "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightPurple", "brightCyan", "brightWhite",
];

fn scheme_of(v: &Value) -> Option<(String, Scheme)> {
    let name = v.get("name")?.as_str()?.to_string();
    let mut palette = [Rgb(0, 0, 0); 16];
    for (slot, key) in palette.iter_mut().zip(PALETTE_KEYS) {
        *slot = hex(v.get(key)).unwrap_or(Rgb(0, 0, 0));
    }
    Some((name, Scheme { fg: hex(v.get("foreground"))?, bg: hex(v.get("background"))?, cursor: hex(v.get("cursorColor")), palette }))
}

fn text(m: &Map<String, Value>, key: &str) -> Option<String> {
    m.get(key)?.as_str().map(str::to_string).filter(|s| !s.is_empty())
}

fn guid(m: &Map<String, Value>, key: &str) -> Option<String> {
    text(m, key).map(|g| g.to_ascii_lowercase())
}

/// Copy the keys of `from` that `into` does not have.
fn fill(into: &mut Map<String, Value>, from: &Map<String, Value>) {
    for (k, v) in from {
        if k != "updates" && !into.contains_key(k) {
            into.insert(k.clone(), v.clone());
        }
    }
}

/// `c` with an unquoted program path that has spaces (`C:\Program Files\x.exe -a`, which WT accepts)
/// quoted, so the program is the whole path and not its first word.
fn quote_exe(c: String) -> String {
    if c.starts_with('"') {
        return c;
    }
    let lower = c.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(".exe") {
        let end = from + i + 4;
        if end == c.len() || c[end..].starts_with(' ') {
            let exe = &c[..end];
            return if exe.contains(' ') { format!("\"{exe}\"{}", &c[end..]) } else { c };
        }
        from = end;
    }
    c
}

/// The command WT would start for profile `m`, or why there is none.
fn command(m: &Map<String, Value>, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    if let Some(c) = text(m, "commandline") {
        return Ok(quote_exe(expand(&c, env)));
    }
    let name = text(m, "name").unwrap_or_default();
    match (guid(m, "guid").as_deref(), text(m, "source").as_deref()) {
        (Some(WINDOWS_POWERSHELL), _) => Ok(expand(r"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe", env)),
        (Some(CMD), _) => Ok(expand(r"%SystemRoot%\System32\cmd.exe", env)),
        (_, Some("Windows.Terminal.Wsl")) => Ok(format!("wsl.exe -d {name}")),
        (_, Some("Windows.Terminal.PowershellCore")) => Ok("pwsh.exe".into()),
        (_, Some(src)) => Err(format!("only Windows Terminal can start `{src}` profiles")),
        _ => Err("no command line".into()),
    }
}

/// Resolve WT's `settings` text and its fragments (`(source, json)`) into the profiles Wraith can start.
pub fn parse(settings: &str, fragments: &[(String, String)], env: &dyn Fn(&str) -> Option<String>) -> Result<WtSettings, String> {
    let root: Value = serde_json::from_str(&strip_jsonc(settings)).map_err(|e| format!("Windows Terminal settings: {e}"))?;
    let objects = |v: Option<&Value>| -> Vec<Map<String, Value>> {
        v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_object().cloned()).collect()).unwrap_or_default()
    };
    let (defaults, mut entries) = match root.get("profiles") {
        Some(Value::Object(o)) => (o.get("defaults").and_then(Value::as_object).cloned().unwrap_or_default(), objects(o.get("list"))),
        p @ Some(Value::Array(_)) => (Map::new(), objects(p)),
        _ => (Map::new(), Vec::new()),
    };
    let mut updates = Vec::new();
    for (source, json) in fragments {
        // A fragment that does not parse is left out; WT does the same.
        let Ok(frag) = serde_json::from_str::<Value>(&strip_jsonc(json)) else { continue };
        for mut p in objects(frag.get("profiles")) {
            if let Some(target) = guid(&p, "updates") {
                updates.push((target, p));
            } else if let Some(e) = guid(&p, "guid").and_then(|g| entries.iter_mut().find(|e| guid(e, "guid").as_deref() == Some(&g))) {
                fill(e, &p);
            } else {
                p.entry("source").or_insert_with(|| Value::String(source.clone()));
                entries.push(p);
            }
        }
    }
    for (target, u) in &updates {
        if let Some(e) = entries.iter_mut().find(|e| guid(e, "guid").as_deref() == Some(target)) {
            fill(e, u);
        }
    }

    let mut out = WtSettings::default();
    for e in &entries {
        let mut m = e.clone();
        fill(&mut m, &defaults);
        let Some(name) = text(&m, "name") else { continue };
        if m.get("hidden").and_then(Value::as_bool) == Some(true) || out.profiles.iter().any(|p| p.name == name) {
            continue;
        }
        match command(&m, env) {
            Ok(command) => out.profiles.push(WtProfile {
                command,
                dir: text(&m, "startingDirectory").map(|d| expand(&d, env)),
                icon: text(&m, "icon"),
                scheme: match m.get("colorScheme") {
                    Some(Value::Object(o)) => text(o, "dark"),
                    Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
                    _ => None,
                },
                name,
            }),
            Err(why) => out.skipped.push((name, why)),
        }
    }
    out.default = root.get("defaultProfile").and_then(Value::as_str).and_then(|d| {
        if d.starts_with('{') {
            let d = d.to_ascii_lowercase();
            entries.iter().find(|e| guid(e, "guid").as_deref() == Some(&d)).and_then(|e| text(e, "name"))
        } else {
            Some(d.to_string())
        }
    });
    out.schemes = root.get("schemes").and_then(Value::as_array).map(|a| a.iter().filter_map(scheme_of).collect()).unwrap_or_default();
    Ok(out)
}

fn read_small(path: &Path) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    (len <= MAX_FILE).then(|| std::fs::read_to_string(path).ok()).flatten()
}

fn sorted_dir(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    v.sort();
    v
}

/// WT's settings under `local` (`%LOCALAPPDATA%`) with the fragments there and under `program_data`.
/// `None` when WT has no settings file.
pub fn load_from(local: &Path, program_data: &Path) -> Option<Result<WtSettings, String>> {
    const STABLE: &str = "Microsoft.WindowsTerminal_8wekyb3d8bbwe";
    const PREVIEW: &str = "Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe";
    let packaged = |f: &str| local.join("Packages").join(f).join("LocalState").join("settings.json");
    let candidates = [
        (packaged(STABLE), Some(STABLE)),
        (packaged(PREVIEW), Some(PREVIEW)),
        (local.join("Microsoft").join("Windows Terminal").join("settings.json"), None),
    ];
    let (path, family) = candidates.into_iter().find(|(p, _)| p.is_file())?;
    let Some(settings) = read_small(&path) else {
        return Some(Err(format!("cannot read {}", path.display())));
    };
    let mut fragments = Vec::new();
    for root in [local, program_data] {
        for app in sorted_dir(&root.join("Microsoft").join("Windows Terminal").join("Fragments")) {
            let source = app.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for f in sorted_dir(&app).into_iter().filter(|f| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("json"))) {
                if let Some(text) = read_small(&f) {
                    fragments.push((source.clone(), text));
                }
            }
        }
    }
    let env = |k: &str| std::env::var(k).ok();
    Some(parse(&settings, &fragments, &env).map(|mut s| {
        s.family = family.map(str::to_string);
        s
    }))
}

/// WT's settings for this user, if WT is installed.
pub fn load() -> Option<Result<WtSettings, String>> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let pd = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    load_from(&local, &pd)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    /// The config already has a `profile` line with this name.
    Exists,
    /// Not written, and why.
    Unsupported(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub rows: Vec<(String, ImportOutcome)>,
    pub themes_written: Vec<String>,
}

impl ImportReport {
    pub fn imported(&self) -> usize {
        self.rows.iter().filter(|(_, o)| *o == ImportOutcome::Imported).count()
    }
}

/// An icon worth keeping in the config: an emoji or glyph, or a file path (not a WT package URI).
fn portable_icon(icon: &str) -> bool {
    icon.chars().count() <= 2 || (icon.contains(['\\', '/']) && !icon.contains("://"))
}

/// `scheme` as a Ghostty theme file.
fn theme_text(s: &Scheme) -> String {
    let h = |c: Rgb| format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2);
    let mut out: String = s.palette.iter().enumerate().map(|(i, &c)| format!("palette = {i}={}\n", h(c))).collect();
    out.push_str(&format!("foreground = {}\nbackground = {}\n", h(s.fg), h(s.bg)));
    if let Some(c) = s.cursor {
        out.push_str(&format!("cursor-color = {}\n", h(c)));
    }
    out
}

/// The config `src` with `wt`'s profiles added (wt-profiles spec §4), the report, and the theme files
/// (name, text) for schemes only WT has. Running it on its own output changes nothing.
pub fn plan_import(src: &str, wt: &WtSettings, themes_dir: &Path) -> (String, ImportReport, Vec<(String, String)>) {
    use crate::edit::{Which, append, set};
    let (cfg, _) = crate::parse_str(src, Path::new("config"), themes_dir);
    let mut out = src.to_string();
    let mut report = ImportReport::default();
    let mut themes: Vec<(String, String)> = Vec::new();
    let mut known: Vec<String> = cfg.profiles.iter().map(|(n, _)| n.to_lowercase()).collect();
    for p in &wt.profiles {
        let outcome = if p.name.contains(':') {
            ImportOutcome::Unsupported("name contains ':'".into())
        } else if known.contains(&p.name.to_lowercase()) {
            ImportOutcome::Exists
        } else {
            out = append(&out, "profile", &format!("{}:{}", p.name, p.command));
            if let Some(d) = &p.dir {
                out = append(&out, "profile-dir", &format!("{}:{d}", p.name));
            }
            if let Some(s) = &p.scheme {
                out = append(&out, "profile-theme", &format!("{}:{s}", p.name));
            }
            if let Some(i) = p.icon.as_deref().filter(|i| portable_icon(i)) {
                out = append(&out, "profile-icon", &format!("{}:{i}", p.name));
            }
            known.push(p.name.to_lowercase());
            ImportOutcome::Imported
        };
        if outcome != ImportOutcome::Unsupported("name contains ':'".into())
            && let Some(name) = &p.scheme
            && !name.contains(['/', '\\']) && !name.contains("..")
            && !themes.iter().any(|(n, _)| n == name)
            && scheme_from_theme(themes_dir, name).is_none()
            && let Some((_, s)) = wt.schemes.iter().find(|(n, _)| n == name)
        {
            themes.push((name.clone(), theme_text(s)));
        }
        report.rows.push((p.name.clone(), outcome));
    }
    for (name, why) in &wt.skipped {
        report.rows.push((name.clone(), ImportOutcome::Unsupported(why.clone())));
    }
    if cfg.default_profile.is_none()
        && let Some(d) = wt.default.as_ref().filter(|d| known.contains(&d.to_lowercase()))
    {
        out = set(&out, "default-profile", d, Which::Last);
    }
    // The imported list replaces both sources it came from: WT's settings and Wraith's own detection.
    out = set(&out, "profile-detect", "false", Which::Last);
    out = set(&out, "windows-terminal-profiles", "false", Which::Last);
    (out, report, themes)
}

/// Import `wt` into the config at `config`, writing new theme files to the `themes` folder beside it.
/// An existing theme file is never replaced.
pub fn import(config: &Path, themes_dir: &Path, wt: &WtSettings) -> std::io::Result<ImportReport> {
    let mut result = None;
    crate::edit::write(config, |src| {
        let (out, report, themes) = plan_import(src, wt, themes_dir);
        result = Some((report, themes));
        out
    })?;
    let (mut report, themes) = result.expect("write calls the edit");
    let dir = config.parent().unwrap_or(Path::new(".")).join("themes");
    for (name, text) in themes {
        let path = dir.join(&name);
        if !path.exists() {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&path, text)?;
            report.themes_written.push(name);
        }
    }
    Ok(report)
}
