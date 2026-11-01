#![cfg(windows)]
//! `wr ctl ...` against an in-process server on a private pipe.
use std::process::Output;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wraith_ipc::{PipeReader, PipeWriter, connect};
use wraith_proto::pane::{PaneCmd, PaneEvent, SpawnOpts};
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, ServerMsg, read_frame, write_frame};
use wraith_server::session::spawn_server_thread;

fn unique() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\wraith-cli-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn wr(pipe: &str, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_wr")).args(args).env("WRAITH_PIPE_NAME", pipe).output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A GUI client holding one pane.
fn gui(pipe: &str) -> (PipeReader, PipeWriter, PaneId) {
    let (mut r, mut w) = connect(pipe).unwrap().split();
    write_frame(&mut w, &ClientMsg::Hello { proto_version: PROTO_VERSION, kind: ClientKind::Gui }).unwrap();
    let _: ServerMsg = read_frame(&mut r).unwrap();
    let spawn = SpawnOpts {
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
    };
    write_frame(&mut w, &ClientMsg::Attach { spawn }).unwrap();
    loop {
        if let ServerMsg::Attached { pane, .. } = read_frame(&mut r).unwrap() {
            return (r, w, pane);
        }
    }
}

fn server() -> String {
    let pipe = unique();
    spawn_server_thread(&pipe).unwrap();
    pipe
}

#[test]
fn list_shows_panes() {
    let pipe = server();
    let (_r, _w, pane) = gui(&pipe);
    let o = wr(&pipe, &["ctl", "list"]);
    assert!(o.status.success(), "{o:?}");
    let line = stdout(&o).lines().next().unwrap_or_default().to_string();
    let cols: Vec<&str> = line.split('\t').collect();
    assert_eq!(cols[0], pane.to_string(), "{line}");
    assert_eq!(cols[1], "attached");
    assert_eq!(cols[2], "running");
}

#[test]
fn send_text_reaches_pane() {
    let pipe = server();
    let (mut r, mut w, pane) = gui(&pipe);
    let o = wr(&pipe, &["ctl", "send-text", &pane.to_string(), "echo cli-ok\r"]);
    assert!(o.status.success(), "{o:?}");
    // Read on a helper thread: a blocked pipe read must not outlive the deadline.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(m) = read_frame::<ServerMsg>(&mut r) {
            if tx.send(m).is_err() {
                return;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut seen = false;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(ServerMsg::Event { ev: PaneEvent::Frame(f), .. }) = rx.recv_timeout(left) else { continue };
        write_frame(&mut w, &ClientMsg::Cmd { pane, cmd: PaneCmd::Ready(f.seq) }).unwrap();
        if f.dirty_rows.iter().any(|row| row.runs.iter().any(|s| s.text.contains("cli-ok"))) {
            seen = true;
            break;
        }
    }
    assert!(seen, "text sent by wr never reached the pane");
}

#[test]
fn kill_by_id() {
    let pipe = server();
    let (_r, _w, pane) = gui(&pipe);
    assert!(stdout(&wr(&pipe, &["ctl", "list"])).starts_with(&pane.to_string()), "pane must be listed before the kill");
    assert!(wr(&pipe, &["ctl", "kill", &pane.to_string()]).status.success());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(stdout(&wr(&pipe, &["ctl", "list"])).trim(), "");
}

#[test]
fn no_server_exits_2() {
    let o = wr(&unique(), &["ctl", "list"]);
    assert_eq!(o.status.code(), Some(2), "{o:?}");
}

#[test]
fn unknown_pane_exits_1() {
    let pipe = server();
    let o = wr(&pipe, &["ctl", "kill", "99999"]);
    assert_eq!(o.status.code(), Some(1), "{o:?}");
}

#[test]
fn usage_errors_exit_1() {
    let pipe = server();
    assert_eq!(wr(&pipe, &["ctl", "frobnicate"]).status.code(), Some(1));
    assert_eq!(wr(&pipe, &["ctl", "kill", "not-a-number"]).status.code(), Some(1));
    assert_eq!(wr(&pipe, &[]).status.code(), Some(1));
}

#[test]
fn server_stop_stops_it() {
    let pipe = server();
    assert!(wr(&pipe, &["ctl", "server-stop"]).status.success());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(wr(&pipe, &["ctl", "list"]).status.code(), Some(2));
}
