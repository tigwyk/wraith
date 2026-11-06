//! `wr install-hooks claude|codex` and `wr uninstall-hooks ...` (spec §5.1). Shows the change, asks
//! before writing, keeps a `.bak`, and only ever touches its own tagged entries.
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Map, Value, json};

const CLAUDE_EVENTS: [&str; 6] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "Notification", "Stop", "SessionEnd"];

struct Opts {
    file: Option<PathBuf>,
    wsl: Option<String>,
    yes: bool,
}

pub fn run(install: bool, args: &[&str]) -> Result<(), String> {
    let (harness, rest) = args.split_first().ok_or(USAGE)?;
    let mut o = Opts { file: None, wsl: None, yes: false };
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match *a {
            "--file" => o.file = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
            "--wsl" => o.wsl = Some(it.next().ok_or(USAGE)?.to_string()),
            "--yes" | "-y" => o.yes = true,
            _ => return Err(USAGE.into()),
        }
    }
    let wr = wr_path(o.wsl.as_deref())?;
    match *harness {
        "claude" => {
            let path = target(&o, ".claude/settings.json")?;
            let old = read(&path)?;
            let mut v: Value = if old.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&old).map_err(|e| format!("{} is not valid JSON ({e}); the settings file was not changed", path.display()))?
            };
            if !v.is_object() {
                return Err(format!("{}: the settings file is not a JSON object; nothing changed", path.display()));
            }
            claude_strip(&mut v);
            if install {
                claude_add(&mut v, &format!("\"{wr}\" hook claude"));
            }
            let new = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";
            apply(&path, &old, &new, o.yes)
        }
        "codex" => {
            let path = target(&o, ".codex/config.toml")?;
            let old = read(&path)?;
            let new = codex_edit(&old, install.then(|| format!("notify = ['{wr}', 'hook', 'codex']")))?;
            apply(&path, &old, &new, o.yes)
        }
        _ => Err(USAGE.into()),
    }
}

const USAGE: &str = "usage: wr install-hooks|uninstall-hooks claude|codex [--wsl <distro>] [--file <path>] [--yes]";

fn read(path: &PathBuf) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn wsl(distro: &str, script: &str) -> Result<String, String> {
    let out = Command::new("wsl.exe").args(["-d", distro, "-e", "sh", "-c", script]).output().map_err(|e| format!("wsl.exe: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !s.is_empty() { Ok(s) } else { Err(format!("wsl {distro}: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

/// The settings file: `--file`, else the harness's file in the Windows profile or the WSL distro's home.
fn target(o: &Opts, rel: &str) -> Result<PathBuf, String> {
    if let Some(f) = &o.file {
        return Ok(f.clone());
    }
    if let Some(d) = &o.wsl {
        return wsl(d, &format!("wslpath -w \"$HOME/{rel}\"")).map(PathBuf::from);
    }
    let home = std::env::var_os("USERPROFILE").ok_or("USERPROFILE is not set")?;
    Ok(PathBuf::from(home).join(rel))
}

/// How the harness should run `wr`: forward slashes on Windows (fine for cmd and Git Bash), a /mnt path in WSL.
fn wr_path(distro: Option<&str>) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let win = exe.display().to_string();
    match distro {
        Some(d) => wsl(d, &format!("wslpath -u '{}'", win.replace('\'', ""))),
        None => Ok(win.replace('\\', "/")),
    }
}

/// `"<dir>/wr.exe" hook claude <Event>` as `claude_add` writes it (a WSL path may drop the `.exe`).
fn is_ours(cmd: &str) -> bool {
    let cmd = cmd.trim_start();
    let split = match cmd.strip_prefix('"') {
        Some(quoted) => quoted.split_once('"'),
        None => cmd.split_once(' '),
    };
    let Some((exe, rest)) = split else { return false };
    let name = exe.rsplit(['/', '\\']).next().unwrap_or(exe).to_ascii_lowercase();
    matches!(name.as_str(), "wr" | "wr.exe") && rest.trim_start().starts_with("hook claude")
}

/// Remove every hook entry `wr` added, and any group or event list that leaves empty.
fn claude_strip(v: &mut Value) {
    let Some(hooks) = v.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    for groups in hooks.values_mut().filter_map(Value::as_array_mut) {
        groups.retain_mut(|g| {
            let Some(list) = g.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let before = list.len();
            list.retain(|h| !h.get("command").and_then(Value::as_str).is_some_and(is_ours));
            !(list.is_empty() && before > 0)
        });
    }
    hooks.retain(|_, g| g.as_array().is_none_or(|a| !a.is_empty()));
    if hooks.is_empty() {
        v.as_object_mut().unwrap().remove("hooks");
    }
}

fn claude_add(v: &mut Value, cmd: &str) {
    let obj = v.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let Some(hooks) = hooks.as_object_mut() else { return };
    for event in CLAUDE_EVENTS {
        let mut group = Map::new();
        if event == "PreToolUse" {
            group.insert("matcher".into(), json!("*"));
        }
        group.insert("hooks".into(), json!([{ "type": "command", "command": format!("{cmd} {event}") }]));
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        if let Some(list) = list.as_array_mut() {
            list.push(Value::Object(group));
        }
    }
}

/// Set (or with `None`, remove) the top-level `notify` of a Codex config, line by line.
fn codex_edit(old: &str, line: Option<String>) -> Result<String, String> {
    let mut lines: Vec<String> = old.lines().map(str::to_string).collect();
    let top = lines.iter().position(|l| l.trim_start().starts_with('[')).unwrap_or(lines.len());
    let is_notify = |l: &str| l.trim_start().strip_prefix("notify").is_some_and(|r| r.trim_start().starts_with('='));
    let at = lines[..top].iter().position(|l| is_notify(l));
    if let Some(i) = at
        && lines[i].contains('[')
        && !lines[i].contains(']')
    {
        return Err("config.toml has a multi-line top-level notify; edit it by hand (nothing changed)".into());
    }
    match (at, line) {
        (Some(i), Some(l)) => lines[i] = l,
        (Some(i), None) if lines[i].contains("hook") && lines[i].contains("codex") => {
            lines.remove(i);
        }
        (Some(_), None) => {}
        (None, Some(l)) => {
            // After the last top-level setting, before any blank lines and tables.
            let end = lines[..top].iter().rposition(|l| !l.trim().is_empty()).map_or(0, |i| i + 1);
            lines.insert(end, l);
        }
        (None, None) => {}
    }
    Ok(lines.join("\n") + "\n")
}

/// Show the change, confirm, back up, write.
fn apply(path: &PathBuf, old: &str, new: &str, yes: bool) -> Result<(), String> {
    let old_norm = if old.is_empty() { String::new() } else { old.replace("\r\n", "\n") };
    if old_norm == new || (old.is_empty() && new.trim() == "{}") {
        println!("{}: already up to date", path.display());
        return Ok(());
    }
    println!("--- {}", path.display());
    for d in diff(&old_norm, new) {
        println!("{d}");
    }
    if !yes {
        print!("Apply these changes? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !answer.trim().eq_ignore_ascii_case("y") {
            return Err("cancelled; nothing changed".into());
        }
    }
    if !old.is_empty() {
        std::fs::write(format!("{}.bak", path.display()), old).map_err(|e| format!("backup: {e}"))?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("written (backup: {}.bak)", path.display());
    Ok(())
}

/// Changed lines only, as `- old` / `+ new` (a plain LCS diff; settings files are small).
fn diff(a: &str, b: &str) -> Vec<String> {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            (i, j) = (i + 1, j + 1);
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push(format!("+ {}", b[j]));
            j += 1;
        } else {
            out.push(format!("- {}", a[i]));
            i += 1;
        }
    }
    out
}
