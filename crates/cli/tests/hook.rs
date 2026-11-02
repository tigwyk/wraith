#![cfg(windows)]
//! `wr hook ...` (spec §5.1): reaches the server as a Hook client, fast and silent.
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wraith_ipc::connect;
use wraith_proto::model::{AgentState, ModelCmd, ModelSnapshot};
use wraith_proto::pane::SpawnOpts;
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, ServerMsg, read_frame, write_frame};
use wraith_server::session::spawn_server_thread;

fn unique() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\wraith-hook-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// Run `wr <args>` as a hook would: pane env set (when `pane` is given), `stdin` piped in.
fn wr_hook(pipe: Option<&str>, pane: Option<PaneId>, args: &[&str], stdin: &str) -> (Output, Duration) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_wr"));
    c.args(args).env_remove("WRAITH_PIPE").env_remove("WRAITH_PANE_ID").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(p) = pipe {
        c.env("WRAITH_PIPE", p);
    }
    if let Some(p) = pane {
        c.env("WRAITH_PANE_ID", p.to_string());
    }
    let t0 = Instant::now();
    let mut child = c.spawn().unwrap();
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let out = child.wait_with_output().unwrap();
    (out, t0.elapsed())
}

/// Waits up to a timeout for a snapshot matching a predicate.
type WaitSnapshot = Box<dyn FnMut(Duration, &dyn Fn(&ModelSnapshot) -> bool) -> bool>;

/// A server with one pane in a window; returns the pipe, the pane and a snapshot reader.
fn server_with_pane() -> (String, PaneId, WaitSnapshot) {
    let pipe = unique();
    spawn_server_thread(&pipe).unwrap();
    let (mut r, mut w) = connect(&pipe).unwrap().split();
    write_frame(&mut w, &ClientMsg::Hello { proto_version: PROTO_VERSION, kind: ClientKind::Gui }).unwrap();
    let _: ServerMsg = read_frame(&mut r).unwrap();
    write_frame(&mut w, &ClientMsg::AttachWindow { workspace: None }).unwrap();
    let ws = loop {
        if let ServerMsg::WindowAttached { workspace } = read_frame(&mut r).unwrap() {
            break workspace;
        }
    };
    let spawn = SpawnOpts {
        cmdline: "cmd.exe /k".into(),
        cwd: None,
        env: vec![],
        cols: 80,
        rows: 24,
        cell_w_px: 8,
        cell_h_px: 16,
        scrollback_lines: 100,
        default_fg: (255, 255, 255),
        default_bg: (0, 0, 0),
        palette: vec![],
        image_storage_limit: 0,
        agent_detect: vec![],
    };
    write_frame(&mut w, &ClientMsg::Model(ModelCmd::NewTab { workspace: ws, profile: "cmd".into(), spawn })).unwrap();
    let pane = loop {
        if let ServerMsg::Snapshot(s) = read_frame(&mut r).unwrap()
            && let Some(t) = s.workspace(ws).and_then(|w| w.active_tab)
        {
            break s.tab(t).unwrap().focused;
        }
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        let _keep = w;
        while let Ok(m) = read_frame::<ServerMsg>(&mut r) {
            if let ServerMsg::Snapshot(s) = m
                && tx.send(s).is_err()
            {
                return;
            }
        }
    });
    let wait = move |t: Duration, pred: &dyn Fn(&ModelSnapshot) -> bool| {
        let deadline = Instant::now() + t;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if rx.recv_timeout(left).is_ok_and(|s| pred(&s)) {
                return true;
            }
        }
        false
    };
    (pipe, pane, Box::new(wait))
}

fn state_is(pane: PaneId, want: AgentState) -> impl Fn(&ModelSnapshot) -> bool {
    move |s| s.pane(pane).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.state == want)
}

#[test]
fn hook_without_env_exits_0_silently() {
    let (out, t) = wr_hook(None, None, &["hook", "claude"], "{\"hook_event_name\":\"Stop\"}");
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(t < Duration::from_millis(1000), "{t:?}");
}

#[test]
fn hook_with_no_server_still_exits_0_fast() {
    let (out, t) = wr_hook(Some(&unique()), Some(1), &["hook", "claude"], "{\"hook_event_name\":\"Stop\"}");
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(t < Duration::from_millis(1000), "{t:?}");
}

#[test]
fn claude_notification_becomes_ask_with_message() {
    let (pipe, pane, mut wait) = server_with_pane();
    let payload = r#"{"session_id":"x","hook_event_name":"Notification","message":"Claude needs your permission to use Bash"}"#;
    let (out, t) = wr_hook(Some(&pipe), Some(pane), &["hook", "claude"], payload);
    assert!(out.status.success() && out.stdout.is_empty(), "{out:?}");
    assert!(t < Duration::from_millis(1000), "{t:?}");
    assert!(wait(Duration::from_secs(5), &|s| s.pane(pane).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.state == AgentState::Asking
        && a.message == "Claude needs your permission to use Bash"
        && a.harness == "claude")));
}

#[test]
fn claude_events_map_to_states() {
    let (pipe, pane, mut wait) = server_with_pane();
    for (event, want) in [("SessionStart", AgentState::Idle), ("UserPromptSubmit", AgentState::Working), ("Stop", AgentState::Done)] {
        let payload = format!("{{\"hook_event_name\":\"{event}\"}}");
        wr_hook(Some(&pipe), Some(pane), &["hook", "claude"], &payload);
        assert!(wait(Duration::from_secs(5), &state_is(pane, want)), "{event}");
    }
    wr_hook(Some(&pipe), Some(pane), &["hook", "claude"], "{\"hook_event_name\":\"SessionEnd\"}");
    assert!(wait(Duration::from_secs(5), &|s| s.pane(pane).is_some_and(|p| p.agent.is_none())));
}

#[test]
fn codex_turn_complete_payload() {
    let (pipe, pane, mut wait) = server_with_pane();
    let payload = r#"{"type":"agent-turn-complete","turn-id":"1","input-messages":["hi"],"last-assistant-message":"All tests pass."}"#;
    let (out, _) = wr_hook(Some(&pipe), Some(pane), &["hook", "codex", payload], "");
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(wait(Duration::from_secs(5), &|s| s.pane(pane).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.state == AgentState::Done
        && a.harness == "codex"
        && a.message == "All tests pass.")));
}

#[test]
fn generic_hook() {
    let (pipe, pane, mut wait) = server_with_pane();
    wr_hook(Some(&pipe), Some(pane), &["hook", "generic", "--state", "ask", "--name", "mybot", "--message", "need a hand"], "");
    assert!(wait(Duration::from_secs(5), &|s| s.pane(pane).and_then(|p| p.agent.as_ref()).is_some_and(|a| a.harness == "mybot"
        && a.state == AgentState::Asking
        && a.message == "need a hand")));
}
