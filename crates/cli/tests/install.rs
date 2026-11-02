//! `wr install-hooks` / `wr uninstall-hooks` (spec §5.1): confirm, back up, idempotent, tagged.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn tmp(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("wraith-install-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn wr(args: &[&str], stdin: &str) -> Output {
    use std::io::Write;
    let mut c = Command::new(env!("CARGO_BIN_EXE_wr")).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let _ = c.stdin.take().unwrap().write_all(stdin.as_bytes());
    c.wait_with_output().unwrap()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn ours(settings: &str) -> usize {
    settings.matches("hook claude").count()
}

const USER: &str = r#"{
  "model": "opus",
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "say done" } ] }
    ]
  }
}
"#;

#[test]
fn claude_install_is_idempotent_and_backs_up() {
    let f = tmp("settings.json");
    std::fs::write(&f, USER).unwrap();
    let path = f.to_str().unwrap();
    let out = wr(&["install-hooks", "claude", "--file", path, "--yes"], "");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let after = read(&f);
    assert_eq!(read(&f.with_extension("json.bak")), USER, "the original is kept");
    assert_eq!(ours(&after), 6, "SessionStart, UserPromptSubmit, PreToolUse, Notification, Stop, SessionEnd");
    assert!(after.contains("say done") && after.contains("\"model\": \"opus\""), "the user's settings stay");
    assert!(after.find("\"model\"").unwrap() < after.find("\"hooks\"").unwrap(), "key order kept");
    let v: serde_json::Value = serde_json::from_str(&after).unwrap();
    assert!(v["hooks"]["PreToolUse"][0]["matcher"].is_string());
    let out = wr(&["install-hooks", "claude", "--file", path, "--yes"], "");
    assert!(out.status.success());
    assert_eq!(ours(&read(&f)), 6, "reinstalling replaces, never duplicates");
}

#[test]
fn claude_install_asks_and_no_means_no() {
    let f = tmp("settings.json");
    std::fs::write(&f, USER).unwrap();
    let out = wr(&["install-hooks", "claude", "--file", f.to_str().unwrap()], "n\n");
    assert!(!out.status.success(), "declining is not success");
    assert_eq!(read(&f), USER, "nothing written");
    assert!(String::from_utf8_lossy(&out.stdout).contains("+"), "the diff was shown");
    assert!(!f.with_extension("json.bak").exists());
}

#[test]
fn claude_uninstall_removes_only_tagged() {
    let f = tmp("settings.json");
    std::fs::write(&f, USER).unwrap();
    let path = f.to_str().unwrap();
    wr(&["install-hooks", "claude", "--file", path, "--yes"], "");
    let out = wr(&["uninstall-hooks", "claude", "--file", path, "--yes"], "");
    assert!(out.status.success());
    let after = read(&f);
    assert_eq!(ours(&after), 0);
    let v: serde_json::Value = serde_json::from_str(&after).unwrap();
    let u: serde_json::Value = serde_json::from_str(USER).unwrap();
    assert_eq!(v, u, "exactly the user's own settings are left");
}

#[test]
fn claude_install_creates_missing_file() {
    let f = tmp("settings.json");
    let out = wr(&["install-hooks", "claude", "--file", f.to_str().unwrap(), "--yes"], "");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(ours(&read(&f)), 6);
}

#[test]
fn install_refuses_invalid_json_without_writing() {
    let f = tmp("settings.json");
    std::fs::write(&f, "{ not json").unwrap();
    let out = wr(&["install-hooks", "claude", "--file", f.to_str().unwrap(), "--yes"], "");
    assert!(!out.status.success());
    assert_eq!(read(&f), "{ not json");
    assert!(String::from_utf8_lossy(&out.stderr).contains("settings"), "says why");
}

#[test]
fn codex_install_replaces_top_level_notify_only() {
    let f = tmp("config.toml");
    let user = "model = \"o3\"\nnotify = [\"other\"]\n\n[profiles.fast]\nnotify = [\"keep-me\"]\n";
    std::fs::write(&f, user).unwrap();
    let path = f.to_str().unwrap();
    let out = wr(&["install-hooks", "codex", "--file", path, "--yes"], "");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let after = read(&f);
    assert!(!after.contains("[\"other\"]"), "the top-level notify is replaced");
    assert!(after.contains("keep-me"), "a table's notify is untouched");
    assert!(after.lines().next().unwrap().starts_with("model"));
    let top: Vec<&str> = after.lines().take_while(|l| !l.starts_with('[')).filter(|l| l.starts_with("notify")).collect();
    assert_eq!(top.len(), 1);
    assert!(top[0].contains("'hook', 'codex'") || top[0].contains("\"hook\", \"codex\""), "{}", top[0]);
    wr(&["install-hooks", "codex", "--file", path, "--yes"], "");
    assert_eq!(read(&f), after, "idempotent");
    assert_eq!(read(&f.with_extension("toml.bak")), user);
    wr(&["uninstall-hooks", "codex", "--file", path, "--yes"], "");
    assert!(!read(&f).lines().take_while(|l| !l.starts_with('[')).any(|l| l.starts_with("notify")));
}

#[test]
fn codex_install_refuses_multiline_notify() {
    let f = tmp("config.toml");
    let user = "notify = [\n  \"a\",\n]\n";
    std::fs::write(&f, user).unwrap();
    let out = wr(&["install-hooks", "codex", "--file", f.to_str().unwrap(), "--yes"], "");
    assert!(!out.status.success());
    assert_eq!(read(&f), user);
}
