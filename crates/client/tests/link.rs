#![cfg(windows)]
//! The client side of the pipe: connect-or-spawn, attach, detach, and surviving the client.
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use wraith_client::link::{AttachError, RemotePane, ServerLink, kill_server};
use wraith_ipc::connect;
use wraith_proto::model::ModelCmd;
use wraith_proto::pane::{PaneEvent, SpawnOpts};
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneInfo, ServerMsg, read_frame, write_frame};

fn unique() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\wraith-remote-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

/// Env for a server a test starts: its own layout file, never the user's.
fn layout_env() -> Vec<(String, String)> {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("wraith-link-layout-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    vec![("WRAITH_LAYOUT_PATH".into(), dir.join("layout.json").display().to_string())]
}

fn attach(pipe: &str) -> Result<RemotePane, AttachError> {
    RemotePane::attach_with_env(pipe, spawn(), exe(), &layout_env())
}

fn exe() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_wraith"))
}

fn spawn() -> SpawnOpts {
    SpawnOpts {
        cmdline: "cmd.exe /k".into(),
        cwd: None,
        env: vec![],
        cols: 80,
        rows: 24,
        cell_w_px: 8,
        cell_h_px: 16,
        scrollback_lines: 1000,
        default_fg: (255, 255, 255),
        default_bg: (0, 0, 0),
        palette: vec![(0, 0, 0); 256],
        image_storage_limit: 320 * 1024 * 1024,
        agent_detect: vec![],
    }
}

/// Talk to the server as the CLI would: one request, one reply.
fn cli(pipe: &str, m: ClientMsg) -> Option<ServerMsg> {
    let (mut r, mut w) = connect(pipe).ok()?.split();
    write_frame(&mut w, &ClientMsg::Hello { proto_version: PROTO_VERSION, kind: ClientKind::Cli }).ok()?;
    let _welcome: ServerMsg = read_frame(&mut r).ok()?;
    let stop = m == ClientMsg::ServerStop;
    write_frame(&mut w, &m).ok()?;
    if stop {
        return None;
    }
    read_frame(&mut r).ok()
}

fn list(pipe: &str) -> Vec<PaneInfo> {
    match cli(pipe, ClientMsg::List) {
        Some(ServerMsg::Panes(p)) => p,
        other => panic!("no pane list: {other:?}"),
    }
}

fn stop(pipe: &str) {
    cli(pipe, ClientMsg::ServerStop);
}

#[test]
fn spawns_server_when_absent() {
    let pipe = unique();
    assert!(connect(&pipe).is_err());
    let pane = attach(&pipe).unwrap();
    assert!(!pane.reattached);
    let server_pid = connect(&pipe).unwrap().server_pid().unwrap();
    assert_ne!(server_pid, std::process::id(), "server must be its own process");
    assert_eq!(list(&pipe).len(), 1);
    stop(&pipe);
}

#[test]
fn detach_then_attach_reattaches() {
    let pipe = unique();
    let mut a = attach(&pipe).unwrap();
    let id = a.id;
    a.detach();
    let mut b = attach(&pipe).unwrap();
    assert_eq!((b.id, b.reattached), (id, true));
    b.kill();
    thread::sleep(Duration::from_millis(200));
    assert!(list(&pipe).is_empty());
    stop(&pipe);
}

#[test]
fn server_outlives_killed_client_process() {
    let pipe = unique();
    let dir = std::env::temp_dir().join(format!("wraith-remote-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut client = std::process::Command::new(exe())
        .args(["-e", "cmd.exe", "/k"])
        .env("WRAITH_PIPE_NAME", &pipe)
        .env("APPDATA", &dir)
        .env("LOCALAPPDATA", &dir)
        .env("WRAITH_LAYOUT_PATH", dir.join("layout.json"))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while connect(&pipe).is_err() || list(&pipe).is_empty() {
        assert!(Instant::now() < deadline, "client never attached");
        thread::sleep(Duration::from_millis(100));
    }
    client.kill().unwrap(); // TerminateProcess: no Detach is sent
    let _ = client.wait();
    thread::sleep(Duration::from_millis(500));
    let panes = list(&pipe);
    assert_eq!(panes.len(), 1);
    assert!(!panes[0].attached, "pane must be detached once its client is gone");
    stop(&pipe);
}

#[test]
fn concurrent_launches_share_one_server() {
    let pipe = unique();
    let panes: Vec<RemotePane> = (0..8)
        .map(|_| {
            let pipe = pipe.clone();
            thread::spawn(move || attach(&pipe).unwrap())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    let mut ids: Vec<u64> = panes.iter().map(|p| p.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "every launch gets its own pane");
    assert_eq!(list(&pipe).len(), 8, "all eight live in one server");
    stop(&pipe);
}

#[test]
fn no_server_binary_reports_no_server() {
    let t = Instant::now();
    let err = RemotePane::attach(&unique(), spawn(), Path::new("C:/nope/wraith.exe")).err().expect("must fail");
    assert!(matches!(err, AttachError::NoServer { .. }), "{err:?}");
    assert!(t.elapsed() < Duration::from_millis(2500), "{:?}", t.elapsed());
}

#[test]
fn server_death_disconnects_events() {
    let pipe = unique();
    let pane = attach(&pipe).unwrap();
    // Drain the first frames so only the disconnect is left.
    while let Ok(PaneEvent::Frame(_) | PaneEvent::Title(_)) = pane.events.recv_timeout(Duration::from_millis(300)) {}
    kill_server(&pipe).unwrap();
    let t = Instant::now();
    loop {
        match pane.events.recv_timeout(Duration::from_secs(1)) {
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => panic!("events still open 1 s after the server died"),
            Ok(_) => assert!(t.elapsed() < Duration::from_secs(1)),
        }
    }
}

#[test]
fn shells_get_the_launching_windows_environment() {
    // The server is long-lived; each new shell must see the env of the window that asked for it.
    let env = wraith_client::app::spawn_env();
    assert!(env.iter().any(|(k, _)| k.eq_ignore_ascii_case("PATH")), "PATH missing");
    assert!(env.iter().any(|(k, v)| k == "TERM_PROGRAM" && v == "wraith"));
}

#[test]
fn link_attach_window_and_open_a_tab() {
    let pipe = unique();
    let mut link = ServerLink::connect_with_env(&pipe, exe(), &layout_env()).unwrap();
    assert!(link.send(ClientMsg::AttachWindow { workspace: None }));
    let ws = loop {
        if let ServerMsg::WindowAttached { workspace } = link.msgs.recv_timeout(Duration::from_secs(5)).unwrap() {
            break workspace;
        }
    };
    link.send(ClientMsg::Model(ModelCmd::NewTab { workspace: ws, profile: "cmd".into(), spawn: spawn() }));
    let deadline = Instant::now() + Duration::from_secs(5);
    let pane = loop {
        assert!(Instant::now() < deadline, "no snapshot with a tab");
        if let Ok(ServerMsg::Snapshot(s)) = link.msgs.recv_timeout(Duration::from_millis(200))
            && let Some(t) = s.workspace(ws).and_then(|w| w.active_tab)
        {
            break s.tab(t).unwrap().focused;
        }
    };
    assert_eq!(list(&pipe).iter().map(|p| p.id).collect::<Vec<_>>(), vec![pane]);
    link.finish(ClientMsg::DetachWindow);
    assert!(!link.send(ClientMsg::List), "a finished link refuses sends");
    thread::sleep(Duration::from_millis(200));
    assert!(!list(&pipe)[0].attached);
    stop(&pipe);
}
