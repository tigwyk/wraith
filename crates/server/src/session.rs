//! The Wraith server: owns every pane and routes it to the one client attached to it (spec §3.3, §4.1).
//! One main loop owns all state; clients and panes talk to it through a channel, so a slow or dead
//! client can never stall it.
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, unbounded};
use wraith_ipc::PipeListener;
use wraith_proto::pane::{PaneCmd, PaneEvent, SpawnOpts};
use wraith_proto::wire::{
    ClientKind, ClientMsg, PROTO_VERSION, PaneId, PaneInfo, ServerMsg, read_frame, write_frame,
};

use crate::pane::{PaneHandle, PaneOptions, spawn_pane};

type ClientId = u64;

/// Messages queued per client before it counts as dead (it is detached, never waited on).
const CLIENT_QUEUE: usize = 256;

enum Ev {
    Connected(ClientId, Sender<ServerMsg>),
    Msg(ClientId, ClientMsg),
    Gone(ClientId),
    Pane(PaneId, PaneEvent),
}

struct Client {
    tx: Sender<ServerMsg>,
    kind: Option<ClientKind>,
}

struct Entry {
    handle: PaneHandle,
    client: Option<ClientId>,
    detached_at: Option<Instant>,
    title: String,
    exited: Option<u32>,
}

/// Serve `pipe` until a `ServerStop`. `AddrInUse` if another server already owns it.
pub fn run_server(pipe: &str) -> io::Result<()> {
    serve(pipe.to_string(), PipeListener::bind(pipe)?)
}

/// Bind now (so clients can connect as soon as this returns), serve on a new thread.
pub fn spawn_server_thread(pipe: &str) -> io::Result<JoinHandle<io::Result<()>>> {
    let listener = PipeListener::bind(pipe)?;
    let pipe = pipe.to_string();
    Ok(thread::spawn(move || serve(pipe, listener)))
}

fn serve(pipe: String, mut listener: PipeListener) -> io::Result<()> {
    let (tx, rx) = unbounded::<Ev>();
    let stopping = Arc::new(AtomicBool::new(false));
    let accept = {
        let tx = tx.clone();
        let stopping = stopping.clone();
        thread::Builder::new().name("wraith-accept".into()).spawn(move || {
            let mut next: ClientId = 1;
            while let Ok(conn) = listener.accept() {
                if stopping.load(Ordering::Acquire) {
                    return; // drops the listener: the pipe name is free again
                }
                let id = next;
                next += 1;
                let (mut r, mut w) = conn.split();
                let (out_tx, out_rx) = bounded::<ServerMsg>(CLIENT_QUEUE);
                if tx.send(Ev::Connected(id, out_tx)).is_err() {
                    return;
                }
                thread::spawn(move || {
                    for m in out_rx {
                        if write_frame(&mut w, &m).is_err() {
                            break;
                        }
                    }
                });
                let tx = tx.clone();
                thread::spawn(move || {
                    while let Ok(m) = read_frame::<ClientMsg>(&mut r) {
                        if tx.send(Ev::Msg(id, m)).is_err() {
                            return;
                        }
                    }
                    let _ = tx.send(Ev::Gone(id));
                });
            }
        })?
    };
    Server { tx, clients: HashMap::new(), panes: HashMap::new(), next_pane: 1 }.run(&rx);
    // Shells are gone (run dropped every pane). Wake the accept thread so it releases the pipe.
    stopping.store(true, Ordering::Release);
    let _ = wraith_ipc::connect(&pipe);
    let _ = accept.join();
    Ok(())
}

struct Server {
    tx: Sender<Ev>,
    clients: HashMap<ClientId, Client>,
    panes: HashMap<PaneId, Entry>,
    next_pane: PaneId,
}

impl Server {
    fn run(&mut self, rx: &Receiver<Ev>) {
        for ev in rx {
            match ev {
                Ev::Connected(id, tx) => {
                    self.clients.insert(id, Client { tx, kind: None });
                }
                Ev::Gone(id) => self.drop_client(id),
                Ev::Pane(pane, ev) => self.pane_event(pane, ev),
                Ev::Msg(id, ClientMsg::ServerStop) if self.clients.get(&id).is_some_and(|c| c.kind.is_some()) => {
                    tracing::info!("server stop requested by client {id}");
                    self.panes.clear();
                    return;
                }
                Ev::Msg(id, m) => self.client_msg(id, m),
            }
        }
    }

    fn send(&mut self, id: ClientId, m: ServerMsg) {
        let Some(c) = self.clients.get(&id) else { return };
        match c.tx.try_send(m) {
            Ok(()) => {}
            // Too slow or dead: never let one client stall the server.
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("client {id} not keeping up; detaching it");
                self.drop_client(id);
            }
        }
    }

    fn drop_client(&mut self, id: ClientId) {
        if self.clients.remove(&id).is_none() {
            return;
        }
        for e in self.panes.values_mut().filter(|e| e.client == Some(id)) {
            e.client = None;
            e.detached_at = Some(Instant::now());
        }
    }

    fn client_msg(&mut self, id: ClientId, m: ClientMsg) {
        let Some(kind) = self.clients.get(&id).map(|c| c.kind) else { return };
        let kind = match (kind, m) {
            (None, ClientMsg::Hello { proto_version, kind }) => {
                if proto_version == PROTO_VERSION {
                    self.clients.get_mut(&id).unwrap().kind = Some(kind);
                    self.send(id, ServerMsg::Welcome { server_version: env!("CARGO_PKG_VERSION").into() });
                } else {
                    self.send(id, ServerMsg::VersionMismatch { server_version: env!("CARGO_PKG_VERSION").into() });
                    self.drop_client(id);
                }
                return;
            }
            (None, _) => {
                self.send(id, ServerMsg::Error("expected Hello".into()));
                self.drop_client(id);
                return;
            }
            (Some(k), m) => (k, m),
        };
        let (kind, m) = kind;
        let owns = |e: &Entry| e.client == Some(id) || kind == ClientKind::Cli;
        match m {
            ClientMsg::Hello { .. } | ClientMsg::ServerStop => {}
            ClientMsg::Attach { spawn } => self.attach(id, spawn),
            ClientMsg::Cmd { pane, cmd } => {
                if let Some(e) = self.panes.get(&pane).filter(|e| owns(e)) {
                    let _ = e.handle.send(cmd);
                }
            }
            ClientMsg::Detach { pane } => {
                if let Some(e) = self.panes.get_mut(&pane).filter(|e| e.client == Some(id)) {
                    e.client = None;
                    e.detached_at = Some(Instant::now());
                }
            }
            ClientMsg::Kill { pane } => {
                if self.panes.get(&pane).is_some_and(owns) {
                    // Dropping the handle shuts the pane down; its Job Object kills the shell tree.
                    self.panes.remove(&pane);
                }
            }
            ClientMsg::List => {
                let mut list: Vec<PaneInfo> = self
                    .panes
                    .iter()
                    .map(|(&pid, e)| PaneInfo {
                        id: pid,
                        title: e.title.clone(),
                        attached: e.client.is_some(),
                        exited: e.exited,
                        shell_pid: e.handle.pid(),
                    })
                    .collect();
                list.sort_by_key(|p| p.id);
                self.send(id, ServerMsg::Panes(list));
            }
        }
    }

    /// Reattach the most recently detached pane, else spawn one (spec §4.1; user decision).
    fn attach(&mut self, id: ClientId, spawn: SpawnOpts) {
        let detached = self
            .panes
            .iter()
            .filter(|(_, e)| e.client.is_none())
            .max_by_key(|(_, e)| e.detached_at)
            .map(|(&p, _)| p);
        if let Some(pane) = detached {
            let e = self.panes.get_mut(&pane).unwrap();
            e.client = Some(id);
            e.detached_at = None;
            let (title, exited) = (e.title.clone(), e.exited);
            let _ = e.handle.send(PaneCmd::Reattach);
            let _ = e.handle.send(PaneCmd::Resize {
                cols: spawn.cols,
                rows: spawn.rows,
                cell_w_px: spawn.cell_w_px,
                cell_h_px: spawn.cell_h_px,
            });
            self.send(id, ServerMsg::Attached { pane, reattached: true });
            if !title.is_empty() {
                self.send(id, ServerMsg::Event { pane, ev: PaneEvent::Title(title) });
            }
            if let Some(code) = exited {
                self.send(id, ServerMsg::Event { pane, ev: PaneEvent::Exited(code) });
            }
            return;
        }
        let opts = PaneOptions {
            cmdline: spawn.cmdline,
            cwd: spawn.cwd.map(PathBuf::from),
            env: spawn.env,
            cols: spawn.cols,
            rows: spawn.rows,
            cell_w_px: spawn.cell_w_px,
            cell_h_px: spawn.cell_h_px,
            scrollback_lines: spawn.scrollback_lines,
            default_fg: spawn.default_fg,
            default_bg: spawn.default_bg,
            palette: spawn.palette,
        };
        match spawn_pane(opts) {
            Ok(handle) => {
                let pane = self.next_pane;
                self.next_pane += 1;
                let events = handle.events.clone();
                let tx = self.tx.clone();
                thread::spawn(move || {
                    for ev in events {
                        if tx.send(Ev::Pane(pane, ev)).is_err() {
                            return;
                        }
                    }
                });
                self.panes.insert(pane, Entry { handle, client: Some(id), detached_at: None, title: String::new(), exited: None });
                self.send(id, ServerMsg::Attached { pane, reattached: false });
            }
            Err(e) => self.send(id, ServerMsg::Error(format!("could not start shell: {e}"))),
        }
    }

    fn pane_event(&mut self, pane: PaneId, ev: PaneEvent) {
        let Some(e) = self.panes.get_mut(&pane) else { return };
        match &ev {
            PaneEvent::Title(t) => e.title = t.clone(),
            PaneEvent::Exited(code) => e.exited = Some(*code),
            _ => {}
        }
        // With nobody attached, frames go unacked and the pane stops producing them by itself.
        if let Some(id) = e.client {
            self.send(id, ServerMsg::Event { pane, ev });
        }
    }
}
