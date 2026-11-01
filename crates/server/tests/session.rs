#![cfg(windows)]
//! Server session over a real pipe: windows, workspaces, subscriptions, persistence, routing, latency.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use wraith_ipc::{PipeWriter, connect};
use wraith_proto::model::{Axis, ModelCmd, ModelSnapshot, PaneNode, TabId, WorkspaceId};
use wraith_proto::pane::{PaneCmd, PaneEvent, SpawnOpts};
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, PaneInfo, ServerMsg, read_frame, write_frame};
use wraith_server::pty::is_alive;
use wraith_server::session::{spawn_server_thread, spawn_server_thread_at};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn unique() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!(r"\\.\pipe\wraith-session-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn layout_path(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("wraith-layout-test-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("layout.json")
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
        image_storage_limit: 320 * 1024 * 1024,
    }
}

/// A pipe client: writes on the caller's thread, reads on a helper thread into a channel.
struct Client {
    w: PipeWriter,
    rx: Receiver<ServerMsg>,
    snap: ModelSnapshot,
    ws: WorkspaceId,
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
        let mut c = Client { w, rx, snap: ModelSnapshot::default(), ws: 0 };
        c.send(ClientMsg::Hello { proto_version: PROTO_VERSION, kind });
        assert!(matches!(c.next(secs(5)), Some(ServerMsg::Welcome { .. })));
        c
    }
    fn gui(pipe: &str) -> Client {
        Client::connect(pipe, ClientKind::Gui)
    }
    fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.w, &m).unwrap();
    }
    /// Next message; snapshots are remembered on the way past.
    fn next(&mut self, t: Duration) -> Option<ServerMsg> {
        let m = self.rx.recv_timeout(t).ok()?;
        match &m {
            ServerMsg::Snapshot(s) => self.snap = s.clone(),
            ServerMsg::WindowAttached { workspace } => self.ws = *workspace,
            _ => {}
        }
        Some(m)
    }
    /// Attach a window and return the workspace it got (after its first snapshot).
    fn attach_window(&mut self, ws: Option<WorkspaceId>) -> WorkspaceId {
        self.send(ClientMsg::AttachWindow { workspace: ws });
        let mut got = None;
        loop {
            match self.next(secs(5)).expect("attach reply") {
                ServerMsg::WindowAttached { workspace } => got = Some(workspace),
                ServerMsg::Snapshot(_) if got.is_some() => break,
                ServerMsg::Error(e) => panic!("attach failed: {e}"),
                _ => {}
            }
        }
        self.ws = got.unwrap();
        self.ws
    }
    /// Send a model command and read everything the server sends up to a `List` barrier, so every
    /// snapshot the command caused (and any re-attach) has been seen when this returns.
    fn model(&mut self, cmd: ModelCmd) -> Result<(), String> {
        self.send(ClientMsg::Model(cmd));
        self.send(ClientMsg::List);
        let mut err = None;
        loop {
            match self.next(secs(5)).expect("model reply") {
                ServerMsg::Panes(_) => return err.map_or(Ok(()), Err),
                ServerMsg::Error(e) => err = Some(e),
                _ => {}
            }
        }
    }
    fn new_tab(&mut self, cmdline: &str) -> (TabId, PaneId) {
        self.model(ModelCmd::NewTab { workspace: self.ws, profile: "test".into(), spawn: spawn(cmdline) }).unwrap();
        let tab = self.snap.workspace(self.ws).unwrap().active_tab.unwrap();
        (tab, self.snap.tab(tab).unwrap().focused)
    }
    fn split(&mut self, pane: PaneId, cmdline: &str) -> PaneId {
        self.model(ModelCmd::Split { pane, axis: Axis::Horizontal, new_first: false, profile: "test".into(), spawn: spawn(cmdline) }).unwrap();
        let tab = self.snap.workspace(self.ws).unwrap().active_tab.unwrap();
        self.snap.tab(tab).unwrap().focused
    }
    fn subscribe(&mut self, panes: &[PaneId]) {
        self.send(ClientMsg::Subscribe { panes: panes.to_vec() });
    }
    fn list(&mut self) -> Vec<PaneInfo> {
        self.send(ClientMsg::List);
        loop {
            if let ServerMsg::Panes(p) = self.next(secs(5)).expect("list reply") {
                return p;
            }
        }
    }
    /// Pump events (acking frames) until `pred` sees visible text or an event.
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
    /// Wait for a snapshot satisfying `pred`.
    fn wait_snapshot(&mut self, t: Duration, mut pred: impl FnMut(&ModelSnapshot) -> bool) -> bool {
        if pred(&self.snap) {
            return true;
        }
        let deadline = Instant::now() + t;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if let Some(ServerMsg::Snapshot(s)) = self.next(left)
                && pred(&s)
            {
                return true;
            }
        }
        false
    }
}

/// A server with a private layout file.
fn server() -> String {
    let pipe = unique();
    spawn_server_thread(&pipe).unwrap();
    pipe
}

#[test]
fn attach_window_creates_workspace_and_snapshot() {
    let pipe = server();
    let mut c = Client::gui(&pipe);
    let ws = c.attach_window(None);
    let w = c.snap.workspace(ws).unwrap();
    assert!(w.attached && w.tabs.is_empty());
    assert_eq!(c.snap.workspaces.len(), 1);
}

#[test]
fn new_tab_then_split_spawns_two_panes_and_pushes_snapshots() {
    let pipe = server();
    let mut c = Client::gui(&pipe);
    c.attach_window(None);
    let (tab, p1) = c.new_tab("cmd.exe /k");
    let p2 = c.split(p1, "cmd.exe /k");
    assert_eq!(c.snap.tab(tab).unwrap().root.leaves(), vec![p1, p2]);
    let list = c.list();
    assert_eq!(list.iter().map(|p| p.id).collect::<Vec<_>>(), vec![p1, p2]);
    assert!(list.iter().all(|p| is_alive(p.shell_pid) && p.attached && p.workspace == c.ws));
}

#[test]
fn subscribe_gates_frames() {
    let pipe = server();
    let mut c = Client::gui(&pipe);
    c.attach_window(None);
    let (_, pane) = c.new_tab(&emit("print hello-session"));
    assert!(!c.pump_until(pane, Duration::from_millis(1200), |_, ev| matches!(ev, PaneEvent::Frame(_))), "frames flowed with no subscription");
    c.subscribe(&[pane]);
    let mut first_frame_full = None;
    assert!(c.pump_until(pane, secs(5), |t, ev| {
        if let PaneEvent::Frame(f) = ev {
            first_frame_full.get_or_insert(f.full);
        }
        t.contains("hello-session")
    }));
    assert_eq!(first_frame_full, Some(true), "subscribing must deliver a full frame first");
}

#[test]
fn title_updates_reach_the_snapshot_without_a_subscription() {
    let pipe = server();
    let mut c = Client::gui(&pipe);
    c.attach_window(None);
    let (_, pane) = c.new_tab(&emit("title"));
    assert!(c.wait_snapshot(secs(5), |s| s.pane(pane).is_some_and(|p| p.title == "wraith-title")));
}

#[test]
fn second_window_steals_workspace() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let ws = a.attach_window(None);
    let (_, pane) = a.new_tab("cmd.exe /k");
    let mut b = Client::gui(&pipe);
    assert_eq!(b.attach_window(Some(ws)), ws);
    let mut taken = false;
    let deadline = Instant::now() + secs(5);
    while Instant::now() < deadline && !taken {
        taken = matches!(a.next(Duration::from_millis(200)), Some(ServerMsg::WorkspaceTaken { workspace }) if workspace == ws);
    }
    assert!(taken, "A was never told its workspace moved");
    assert!(a.list().iter().any(|p| p.id == pane), "A's connection still works");
    // A may no longer touch the pane; B may.
    a.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("echo from-a\r".into()) });
    b.subscribe(&[pane]);
    assert!(!b.pump_until(pane, Duration::from_millis(1200), |t, _| t.contains("from-a")));
}

#[test]
fn a_second_window_gets_its_own_workspace() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let mut b = Client::gui(&pipe);
    let wa = a.attach_window(None);
    let wb = b.attach_window(None);
    assert_ne!(wa, wb);
    assert!(b.snap.workspaces.len() >= 2);
}

#[test]
fn detached_workspace_keeps_shells_and_next_window_reattaches() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let ws = a.attach_window(None);
    let (_, pane) = a.new_tab("cmd.exe /k");
    a.subscribe(&[pane]);
    a.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("set WRAITH_X=kept\r".into()) });
    assert!(a.pump_until(pane, secs(5), |t, _| t.contains("WRAITH_X=kept")));
    drop(a);
    thread::sleep(Duration::from_millis(300));

    let mut b = Client::gui(&pipe);
    assert_eq!(b.attach_window(None), ws, "most recently detached workspace");
    b.subscribe(&[pane]);
    assert!(b.pump_until(pane, secs(5), |_, ev| matches!(ev, PaneEvent::Frame(f) if f.full)));
    b.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("echo got-%WRAITH_X%\r".into()) });
    assert!(b.pump_until(pane, secs(5), |t, _| t.contains("got-kept")), "shell state survived the detach");
}

#[test]
fn reattach_picks_most_recently_detached() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let mut b = Client::gui(&pipe);
    let wa = a.attach_window(None);
    let wb = b.attach_window(None);
    a.send(ClientMsg::DetachWindow);
    thread::sleep(Duration::from_millis(100));
    b.send(ClientMsg::DetachWindow);
    thread::sleep(Duration::from_millis(100));
    let mut c = Client::gui(&pipe);
    assert_eq!(c.attach_window(None), wb);
    let mut d = Client::gui(&pipe);
    assert_eq!(d.attach_window(None), wa);
}

#[test]
fn close_surface_kills_shell() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    a.attach_window(None);
    let (_, p1) = a.new_tab("cmd.exe /k");
    let p2 = a.split(p1, "cmd.exe /k");
    let pid = a.list().iter().find(|p| p.id == p2).unwrap().shell_pid;
    assert!(is_alive(pid));
    a.model(ModelCmd::ClosePane(p2)).unwrap();
    let deadline = Instant::now() + secs(2);
    while is_alive(pid) {
        assert!(Instant::now() < deadline, "shell {pid} survived ClosePane");
        thread::sleep(Duration::from_millis(50));
    }
    assert!(a.list().iter().all(|p| p.id != p2));
}

#[test]
fn kill_message_closes_the_pane() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    a.attach_window(None);
    let (_, p1) = a.new_tab("cmd.exe /k");
    let p2 = a.split(p1, "cmd.exe /k");
    a.send(ClientMsg::Kill { pane: p2 });
    assert!(a.wait_snapshot(secs(5), |s| s.pane(p2).is_none() && s.pane(p1).is_some()));
}

#[test]
fn version_mismatch_is_reported() {
    let pipe = server();
    let (mut r, mut w) = connect(&pipe).unwrap().split();
    write_frame(&mut w, &ClientMsg::Hello { proto_version: 999, kind: ClientKind::Gui }).unwrap();
    assert!(matches!(read_frame::<ServerMsg>(&mut r).unwrap(), ServerMsg::VersionMismatch { .. }));
}

#[test]
fn gui_cannot_touch_other_workspaces_panes() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let mut b = Client::gui(&pipe);
    a.attach_window(None);
    b.attach_window(None);
    let (_, pa) = a.new_tab("cmd.exe /k");
    a.subscribe(&[pa]);
    b.send(ClientMsg::Cmd { pane: pa, cmd: PaneCmd::Text("echo from-b\r".into()) });
    assert!(!a.pump_until(pa, Duration::from_millis(1500), |t, _| t.contains("from-b")), "GUI client B typed into A's pane");
    assert!(b.model(ModelCmd::ClosePane(pa)).is_err(), "B closed A's pane");
    b.send(ClientMsg::Kill { pane: pa });
    thread::sleep(Duration::from_millis(300));
    assert!(b.list().iter().any(|p| p.id == pa), "pane survived B's Kill");
    let mut cli = Client::connect(&pipe, ClientKind::Cli);
    cli.send(ClientMsg::Cmd { pane: pa, cmd: PaneCmd::Text("echo from-cli\r".into()) });
    assert!(a.pump_until(pa, secs(5), |t, _| t.contains("from-cli")), "CLI clients may target any pane");
}

#[test]
fn client_crash_detaches_workspace_and_shell_survives() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    a.attach_window(None);
    let (_, pane) = a.new_tab(&emit("flood"));
    a.subscribe(&[pane]);
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
    let mut a = Client::gui(&pipe);
    a.attach_window(None);
    let (_, pane) = a.new_tab(&emit("flood"));
    a.subscribe(&[pane]);
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
    let mut a = Client::gui(&pipe);
    a.attach_window(None);
    a.new_tab("cmd.exe /k");
    a.new_tab("cmd.exe /k");
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

#[test]
fn exited_pane_survives_detach() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let ws = a.attach_window(None);
    let (_, dead) = a.new_tab("cmd.exe /c exit 3");
    assert!(a.wait_snapshot(secs(5), |s| s.pane(dead).is_some_and(|p| p.exit == Some(3))));
    a.send(ClientMsg::DetachWindow);
    thread::sleep(Duration::from_millis(200));
    let mut b = Client::gui(&pipe);
    assert_eq!(b.attach_window(Some(ws)), ws);
    assert_eq!(b.snap.pane(dead).unwrap().exit, Some(3), "the exit code is still shown after reattach");
    assert!(b.list().iter().any(|p| p.id == dead && p.exited == Some(3)));
}

#[test]
fn dropped_slow_client_sees_its_pipe_close() {
    let pipe = server();
    let mut c = Client::gui(&pipe);
    c.attach_window(None);
    let (_, pane) = c.new_tab("cmd.exe /k");
    c.subscribe(&[pane]);
    let Client { mut w, rx, .. } = c;
    // Never read: each Text produces an InputWritten event until the server gives up on us.
    for _ in 0..20_000 {
        if write_frame(&mut w, &ClientMsg::Cmd { pane, cmd: PaneCmd::Text(String::new()) }).is_err() {
            break;
        }
    }
    let (tx, done) = crossbeam_channel::bounded(1);
    thread::spawn(move || {
        while rx.recv().is_ok() {}
        let _ = tx.send(());
    });
    assert!(done.recv_timeout(secs(10)).is_ok(), "a client the server dropped must see EOF, not hang forever");
}

#[test]
fn wire_cannot_shut_down_or_reattach_a_pane() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    a.attach_window(None);
    let (_, pane) = a.new_tab("cmd.exe /k");
    a.subscribe(&[pane]);
    a.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Shutdown });
    thread::sleep(Duration::from_millis(300));
    a.send(ClientMsg::Cmd { pane, cmd: PaneCmd::Text("echo still-alive\r".into()) });
    assert!(a.pump_until(pane, secs(5), |t, _| t.contains("still-alive")), "Shutdown must not be accepted from the pipe");
}

#[test]
fn closing_the_last_tab_moves_the_window_to_a_workspace() {
    let pipe = server();
    let mut a = Client::gui(&pipe);
    let ws = a.attach_window(None);
    let (tab, _) = a.new_tab("cmd.exe /k");
    a.model(ModelCmd::CloseTab(tab)).unwrap();
    assert_ne!(a.ws, ws, "window left on a workspace that no longer exists");
    assert!(a.snap.workspace(a.ws).is_some_and(|w| w.attached));
    assert!(a.snap.workspace(ws).is_none());
}

fn stop_and_join(pipe: &str, handle: thread::JoinHandle<std::io::Result<()>>) {
    let mut c = Client::connect(pipe, ClientKind::Cli);
    c.send(ClientMsg::ServerStop);
    handle.join().unwrap().unwrap();
}

#[test]
fn layout_survives_server_restart() {
    let path = layout_path("restart");
    let pipe = unique();
    let h = spawn_server_thread_at(&pipe, Some(path.clone())).unwrap();
    let mut a = Client::gui(&pipe);
    let ws = a.attach_window(None);
    a.model(ModelCmd::RenameWorkspace(ws, "api".into())).unwrap();
    let mut sp = spawn("cmd.exe /k");
    sp.cwd = Some(r"C:\Windows".into());
    a.model(ModelCmd::NewTab { workspace: ws, profile: "test".into(), spawn: sp.clone() }).unwrap();
    let tab = a.snap.workspace(ws).unwrap().active_tab.unwrap();
    let p1 = a.snap.tab(tab).unwrap().focused;
    a.model(ModelCmd::Split { pane: p1, axis: Axis::Vertical, new_first: false, profile: "test".into(), spawn: sp }).unwrap();
    a.model(ModelCmd::NewWorkspace { name: Some("web".into()), root_dir: None, profile: "test".into(), layout: "cols".into(), spawn: spawn("cmd.exe /k") }).unwrap();
    let old_pids: Vec<u32> = a.list().iter().map(|p| p.shell_pid).collect();
    assert_eq!(old_pids.len(), 4);
    let before = a.snap.clone();
    drop(a);
    stop_and_join(&pipe, h);
    assert!(path.exists(), "ServerStop must leave the layout on disk");

    let h = spawn_server_thread_at(&pipe, Some(path)).unwrap();
    let mut b = Client::gui(&pipe);
    let got = b.attach_window(Some(ws));
    assert_eq!(got, ws);
    let names: Vec<_> = b.snap.workspaces.iter().map(|w| w.name.clone()).collect();
    assert!(names.contains(&"api".to_string()) && names.contains(&"web".to_string()), "{names:?}");
    let t = b.snap.tab(tab).unwrap();
    assert_eq!(t.root, before.tab(tab).unwrap().root, "same split tree");
    assert!(matches!(t.root, PaneNode::Split { axis: Axis::Vertical, .. }));
    assert!(b.snap.panes.iter().filter(|p| p.cwd.as_deref() == Some(r"C:\Windows")).count() >= 2, "cwd preserved");
    let list = b.list();
    assert_eq!(list.len(), 4);
    assert!(list.iter().all(|p| !old_pids.contains(&p.shell_pid) && is_alive(p.shell_pid)), "fresh shells");
    stop_and_join(&pipe, h);
}

#[test]
fn bad_layout_file_is_quarantined() {
    let junk: [&[u8]; 4] = [b"garbage", b"{\"version\":1,\"workspaces\":[", b"", br#"{"version":99,"workspaces":[],"tabs":[],"panes":[],"clock":0}"#];
    for (i, bytes) in junk.iter().enumerate() {
        let path = layout_path(&format!("bad{i}"));
        std::fs::write(&path, bytes).unwrap();
        let pipe = unique();
        let h = spawn_server_thread_at(&pipe, Some(path.clone())).unwrap();
        let mut c = Client::gui(&pipe);
        c.attach_window(None);
        assert_eq!(c.snap.workspaces.len(), 1, "case {i}");
        assert!(c.snap.tabs.is_empty(), "case {i}");
        let bad = PathBuf::from(format!("{}.bad", path.display()));
        assert!(bad.exists(), "case {i}: bad file not quarantined");
        assert_eq!(std::fs::read(&bad).unwrap(), *bytes, "case {i}");
        drop(c);
        stop_and_join(&pipe, h);
    }
}

#[test]
fn save_is_debounced() {
    let path = layout_path("debounce");
    let pipe = unique();
    let h = spawn_server_thread_at(&pipe, Some(path.clone())).unwrap();
    let mut c = Client::gui(&pipe);
    let ws = c.attach_window(None);
    for i in 0..10 {
        c.model(ModelCmd::RenameWorkspace(ws, format!("name-{i}"))).unwrap();
    }
    thread::sleep(Duration::from_millis(300));
    let early = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(!early.contains("name-9"), "saved before the 1 s debounce elapsed");
    thread::sleep(Duration::from_millis(1500));
    let late = std::fs::read_to_string(&path).unwrap();
    assert!(late.contains("name-9"), "the debounced save never happened");
    drop(c);
    stop_and_join(&pipe, h);
}
