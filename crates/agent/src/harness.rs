//! Built-in harness table (spec §5.2) and process matching.
use std::sync::LazyLock;

use regex::Regex;

pub struct Harness {
    pub id: &'static str,
    pub name: &'static str,
    /// Drawn before the tab title.
    pub icon: char,
    /// Executable names (lowercase) that are this agent.
    pub exe: &'static [&'static str],
    /// For script launchers (node, python, bun): a command line that is this agent.
    pub cmdline: Option<Regex>,
    /// A window title that means the agent waits for the user.
    pub ask_title: Option<Regex>,
}

fn re(s: &str) -> Option<Regex> {
    Some(Regex::new(s).expect("built-in harness regex"))
}

static HARNESSES: LazyLock<Vec<Harness>> = LazyLock::new(|| {
    let h = |id, name, icon, exe, cmdline: &str, ask_title: Option<&str>| Harness {
        id,
        name,
        icon,
        exe,
        cmdline: re(cmdline),
        ask_title: ask_title.and_then(re),
    };
    vec![
        h("claude", "Claude Code", '✻', &["claude.exe"], r"(?i)@anthropic-ai[\\/]claude-code|claude-code[\\/]cli\.js", None),
        h("codex", "Codex", '◆', &["codex.exe"], r"(?i)@openai[\\/]codex", Some(r"(?i)action required")),
        h("gemini", "Gemini CLI", '✦', &["gemini.exe"], r"(?i)@google[\\/]gemini-cli", None),
        h("aider", "Aider", 'λ', &["aider.exe"], r"(?i)(^|[\s\\/])aider(\.exe)?(\s|$)", None),
        h("opencode", "OpenCode", '◇', &["opencode.exe"], r"(?i)[\\/]opencode(-ai)?[\\/]", None),
        h("copilot", "Copilot CLI", '◉', &["copilot.exe"], r"(?i)@github[\\/]copilot", None),
        h("cursor-agent", "Cursor Agent", '▶', &["cursor-agent.exe"], r"(?i)cursor-agent", None),
        h("amp", "Amp", '⚡', &["amp.exe"], r"(?i)@sourcegraph[\\/]amp", None),
        h("goose", "Goose", '◈', &["goose.exe"], r"(?i)[\\/]goose(\.exe)?(\s|$)", None),
    ]
});

pub fn harnesses() -> &'static [Harness] {
    &HARNESSES
}

pub fn harness(id: &str) -> Option<&'static Harness> {
    harnesses().iter().find(|h| h.id == id)
}

/// Which agent the process `exe` (full path) running `cmdline` is. `extra` (from `agent-detect`)
/// is matched against "exe cmdline" first.
pub fn match_process(exe: &str, cmdline: &str, extra: &[(String, Regex)]) -> Option<String> {
    let both = format!("{exe} {cmdline}");
    if let Some((name, _)) = extra.iter().find(|(_, r)| r.is_match(&both)) {
        return Some(name.clone());
    }
    let base = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_ascii_lowercase();
    if let Some(h) = harnesses().iter().find(|h| h.exe.contains(&base.as_str())) {
        return Some(h.id.to_string());
    }
    const LAUNCHERS: &[&str] = &["node.exe", "python.exe", "python3.exe", "pythonw.exe", "bun.exe", "deno.exe", "py.exe"];
    if LAUNCHERS.contains(&base.as_str()) {
        return harnesses().iter().find(|h| h.cmdline.as_ref().is_some_and(|r| r.is_match(cmdline))).map(|h| h.id.to_string());
    }
    None
}

/// Untrusted text (hook payloads, OSC bodies) made safe to show: no control characters, newlines as
/// spaces, at most 200 characters.
pub fn sanitize(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect();
    flat.trim().chars().take(200).collect::<String>().trim_end().to_string()
}
