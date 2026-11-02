use regex::Regex;
use wraith_agent::{harness, harnesses, match_process, sanitize};

#[test]
fn table_has_the_spec_harnesses() {
    let ids: Vec<&str> = harnesses().iter().map(|h| h.id).collect();
    for id in ["claude", "codex", "gemini", "aider", "opencode", "copilot", "cursor-agent", "amp", "goose"] {
        assert!(ids.contains(&id), "{id} missing");
    }
    assert_eq!(harness("claude").unwrap().name, "Claude Code");
    assert!(harness("nope").is_none());
}

#[test]
fn claude_and_codex_exe_match() {
    assert_eq!(match_process("C:\\Users\\x\\.local\\bin\\claude.exe", "claude", &[]).as_deref(), Some("claude"));
    assert_eq!(match_process("C:\\tools\\CODEX.EXE", "codex --full-auto", &[]).as_deref(), Some("codex"));
    assert_eq!(match_process("C:\\Windows\\System32\\cmd.exe", "cmd /k", &[]), None);
}

#[test]
fn node_launcher_matches_by_cmdline() {
    let node = "C:\\nvm4w\\nodejs\\node.exe";
    assert_eq!(
        match_process(node, r#"node "C:\Users\x\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js""#, &[]).as_deref(),
        Some("claude")
    );
    assert_eq!(match_process(node, "node server.js", &[]), None, "plain node is no agent");
    assert_eq!(match_process("C:\\py\\python.exe", "python -m aider --model x", &[]).as_deref(), Some("aider"));
}

#[test]
fn user_detect_regex_extends_table() {
    let extra = vec![("mybot".to_string(), Regex::new("(?i)mybot\\.exe").unwrap())];
    assert_eq!(match_process("C:\\bin\\mybot.exe", "mybot", &extra).as_deref(), Some("mybot"));
}

#[test]
fn sanitize_strips_controls_and_trims_200() {
    assert_eq!(sanitize("  a\x1b[31mb\u{7}c\nd  "), "a[31mbc d");
    let long = "é".repeat(300);
    assert_eq!(sanitize(&long).chars().count(), 200);
    assert_eq!(sanitize("<b>x</b>"), "<b>x</b>", "kept as text, never markup");
}
