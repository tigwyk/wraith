#![cfg(windows)]
//! Server session over a real pipe: attach, detach, reattach, kill, routing, latency.
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use wraith_ipc::{PipeWriter, connect};
use wraith_proto::pane::{PaneCmd, PaneEvent, SpawnOpts};
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, PaneInfo, ServerMsg, read_frame, write_frame};
use wraith_server::pty::is_alive;
use wraith_server::session::spawn_server_thread;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn unique() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\wraith-session-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn emit(args: &str) -> String {
    format!("\"{}\" {args}", env!("CARGO_BIN_EXE_emit-seq"))
}

fn spawn(cmdline: &str) -> SpawnOpts {
    SpawnOpts {
        cmdline: cmdline.into(),
        cwd: None,
        env: vec![],
        cols: 80,
        rows: 24,
        cell_w_px: 8,
        cell_h_px: 16,
        scrollback_lines: 1000,
        default_fg: (255, 255, 255),
        default_bg: (0, 0, 0),
        palette: vec![(1, 2, 3); 256],
    }
}

/// A pipe client: writes on the caller's thread, reads on a helper thread into a channel.
struct Client {
    w: PipeWriter,
    rx: Receiver<ServerMsg>,
}

impl Client {
    fn connect(pipe: &str, kind: ClientKind) -> Client {
        let (mut r, w) = connect(pipe).unwrap().split();
        let (tx, rx) = unbounded();
        thread::spawn(move || {
            while let Ok(m) = read_frame::<ServerMsg>(&mut r) {
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        let mut c = Client { w, rx };
        c.send(ClientMsg::Hello { proto_version: PROTO_VERSION, kind });
        assert!(matches!(c.next(secs(5)), Some(ServerMsg::Welcome { .. })));
        c
    }
    fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.w, &m).unwrap();
    }
    fn next(&self, t: Duration) -> Option<ServerMsg> {
        self.rx.recv_timeout(t).ok()
    }
    fn attach(&mut self, cmdline: &str) -> (PaneId, bool) {
        self.send(ClientMsg::Attach { spawn: spawn(cmdline) });
        loop {
            match self.next(secs(5)).expect("attach reply") {
                ServerMsg::Attached { pane, reattached } => return (pane, reattached),
                ServerMsg::Error(e) => panic!("attach failed: {e}"),
                _ => {}
            }
        }
    }
    fn list(&mut self) -> Vec<PaneInfo> {
        self.send(ClientMsg::List);
        loop {
            if let ServerMsg::Panes(p) = self.next(secs(5)).expect("list reply") {
                return p;
            }
        }
    }
    /// Pump events (acking frames) until `pred` sees visible text or an event; returns the screen text.
    fn pump_until(&mut self, pane: PaneId, t: Duration, mut pred: impl FnMut(&str, &PaneEvent) -> bool) -> bool {
        let deadline = Instant::now() + t;
        let mut screen = std::collections::BTreeMap::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(ServerMsg::Event { pane: p, ev }) = self.next(left) else { continue };
            if p != pane {
                continue;
            }
            if let PaneEvent::Frame(f) = &ev {
                if f.full {
                    screen.clear();
                }
                for r in &f.dirty_rows {
                    screen.insert(r.y, r.runs.iter().map(|s| s.text.clone()).collect::<String>());
                }
                self.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Ready(f.seq) });
            }
            let text: String = screen.values().cloned().collect::<Vec<_>>().join("\n");
            if pred(&text, &ev) {
                return true;
            }
        }
        false
    }
}

fn server() -> String {
    let pipe = unique();
    spawn_server_thread(&pipe).unwrap();
    pipe
}

#[test]
fn attach_new_spawns_pane_and_streams_frames() {
    let pipe = server();
    let mut c = Client::connect(&pipe, ClientKind::Gui);
    let (pane, reattached) = c.attach(&emit("print hello-session"));
    assert!(!reattached);
    assert!(c.pump_until(pane, secs(5), |t, _| t.contains("hello-session")));
}

#[test]
fn disconnect_keeps_shell_alive_and_reattach_gets_full_frame() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let (pane, _) = a.attach("cmd.exe /k");
    a.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("set WRAITH_X=kept\r".into()) });
    assert!(a.pump_until(pane, secs(5), |t, _| t.contains("WRAITH_X=kept")));
    drop(a);
    thread::sleep(Duration::from_millis(300));

    let mut b = Client::connect(&pipe, ClientKind::Gui);
    let (again, reattached) = b.attach("cmd.exe /k");
    assert_eq!((again, reattached), (pane, true));
    // A full frame arrives even though the last one sent to `a` was never acked.
    assert!(b.pump_until(pane, secs(5), |_, ev| matches!(ev, PaneEvent::Frame(f) if f.full)));
    b.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("echo got-%WRAITH_X%\r".into()) });
    assert!(b.pump_until(pane, secs(5), |t, _| t.contains("got-kept")), "shell state survived the detach");
}

#[test]
fn second_client_gets_a_new_pane() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let mut b = Client::connect(&pipe, ClientKind::Gui);
    let (pa, _) = a.attach("cmd.exe /k");
    let (pb, rb) = b.attach("cmd.exe /k");
    assert_ne!(pa, pb);
    assert!(!rb);
}

#[test]
fn reattach_picks_most_recently_detached() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let mut b = Client::connect(&pipe, ClientKind::Gui);
    let (pa, _) = a.attach("cmd.exe /k");
    let (pb, _) = b.attach("cmd.exe /k");
    a.send(ClientMsg::Detach { pane: pa });
    thread::sleep(Duration::from_millis(100));
    b.send(ClientMsg::Detach { pane: pb });
    thread::sleep(Duration::from_millis(100));
    let mut c = Client::connect(&pipe, ClientKind::Gui);
    assert_eq!(c.attach("cmd.exe /k"), (pb, true));
    let mut d = Client::connect(&pipe, ClientKind::Gui);
    assert_eq!(d.attach("cmd.exe /k"), (pa, true));
}

#[test]
fn kill_ends_the_shell() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let (pane, _) = a.attach("cmd.exe /k");
    let pid = a.list().iter().find(|p| p.id == pane).unwrap().shell_pid;
    assert!(is_alive(pid));
    a.send(ClientMsg::Kill { pane });
    let deadline = Instant::now() + secs(2);
    while is_alive(pid) {
        assert!(Instant::now() < deadline, "shell {pid} survived Kill");
        thread::sleep(Duration::from_millis(50));
    }
    assert!(a.list().iter().all(|p| p.id != pane));
}

#[test]
fn version_mismatch_is_reported() {
    let pipe = server();
    let (mut r, mut w) = connect(&pipe).unwrap().split();
    write_frame(&mut w, &ClientMsg::Hello { proto_version: 999, kind: ClientKind::Gui }).unwrap();
    assert!(matches!(read_frame::<ServerMsg>(&mut r).unwrap(), ServerMsg::VersionMismatch { .. }));
}

#[test]
fn cmd_for_someone_elses_pane_is_ignored() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let mut b = Client::connect(&pipe, ClientKind::Gui);
    let (pa, _) = a.attach("cmd.exe /k");
    b.send(ClientMsg::Cmd { pane: pa, cmd: PaneCmd::Text("echo from-b\r".into()) });
    assert!(!a.pump_until(pa, Duration::from_millis(1500), |t, _| t.contains("from-b")), "GUI client B typed into A's pane");
    let mut cli = Client::connect(&pipe, ClientKind::Cli);
    cli.send(ClientMsg::Cmd { pane: pa, cmd: PaneCmd::Text("echo from-cli\r".into()) });
    assert!(a.pump_until(pa, secs(5), |t, _| t.contains("from-cli")), "CLI clients may target any pane");
}

#[test]
fn client_crash_detaches_pane_and_shell_survives() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let (pane, _) = a.attach(&emit("flood"));
    a.pump_until(pane, Duration::from_millis(500), |_, _| false);
    drop(a); // mid-flood, with a frame possibly unacked
    thread::sleep(Duration::from_millis(300));
    let mut b = Client::connect(&pipe, ClientKind::Cli);
    let t = Instant::now();
    let list = b.list();
    assert!(t.elapsed() < Duration::from_millis(100), "server main loop stalled: {:?}", t.elapsed());
    let p = list.iter().find(|p| p.id == pane).expect("pane survives its client");
    assert!(!p.attached && p.exited.is_none() && is_alive(p.shell_pid));
}

#[test]
fn remote_input_round_trip_is_fast() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let (pane, _) = a.attach(&emit("flood"));
    a.pump_until(pane, Duration::from_millis(500), |_, _| false);
    let mut rtts = Vec::new();
    for _ in 0..50 {
        let sent = Instant::now();
        a.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("x".into()) });
        let got = a.pump_until(pane, secs(2), |_, ev| matches!(ev, PaneEvent::InputWritten { .. }));
        assert!(got, "no InputWritten");
        rtts.push(sent.elapsed());
        thread::sleep(Duration::from_millis(20));
    }
    rtts.sort();
    assert!(rtts[49] < Duration::from_millis(10), "max {:?}", rtts[49]);
    assert!(rtts[48] < Duration::from_millis(3), "p99 {:?}", rtts[48]);
}

#[test]
fn server_stop_kills_all_shells() {
    let pipe = server();
    let mut a = Client::connect(&pipe, ClientKind::Gui);
    let mut b = Client::connect(&pipe, ClientKind::Gui);
    a.attach("cmd.exe /k");
    b.attach("cmd.exe /k");
    let pids: Vec<u32> = a.list().iter().map(|p| p.shell_pid).collect();
    assert_eq!(pids.len(), 2);
    a.send(ClientMsg::ServerStop);
    let deadline = Instant::now() + secs(3);
    while pids.iter().any(|&p| is_alive(p)) {
        assert!(Instant::now() < deadline, "shells survived ServerStop: {pids:?}");
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(200));
    assert!(connect(&pipe).is_err(), "pipe still served after ServerStop");
}
