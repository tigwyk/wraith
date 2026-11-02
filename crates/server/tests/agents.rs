//! Toast text built from untrusted terminal titles (spec §5.5).
use wraith_server::agents::notify_title;

#[test]
fn notify_title_strips_control_characters_from_titles() {
    let t0 = std::time::Instant::now();
    let a = wraith_agent::step(None, &wraith_agent::Signal::Attached { harness: "claude".into() }, wraith_agent::Source::Hook, &wraith_agent::Ctx { visible: false, now: t0 })
        .and_then(|a| wraith_agent::step(Some(a), &wraith_agent::Signal::Ask { message: String::new() }, wraith_agent::Source::Hook, &wraith_agent::Ctx { visible: false, now: t0 }))
        .unwrap();
    let t = notify_title(&a, "ws\u{1b}[31m", "tab\u{7}\u{0}name");
    assert_eq!(t, "Claude Code needs input — ws[31m/tabname");
}

#[test]
fn agent_scan_reads_long_command_lines() {
    // e.g. `node ...claude...` with many arguments: over 4096 characters must still be read.
    let script = format!("Start-Sleep 20 # {} AGENT_MARKER_END", "x".repeat(7000));
    let mut p = std::process::Command::new("powershell.exe").args(["-NoProfile", "-Command", &script]).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let detect = wraith_server::agentscan::compile(&[("marked".into(), "AGENT_MARKER_END".into())]);
    let found = wraith_server::agentscan::scan(&[p.id()], &detect);
    let _ = p.kill();
    assert_eq!(found.as_deref(), Some("marked"));
}
