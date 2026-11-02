//! The Wraith server: owns the layout model and every pane, and routes panes to the window that has
//! their workspace attached (spec §3.3, §3.4, §4.1). One main loop owns all state; clients and panes
//! talk to it through a channel, so a slow or dead client can never stall it.
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError, bounded, unbounded};
use wraith_ipc::PipeListener;
use wraith_proto::model::{ModelCmd, TabId, WorkspaceId};
use wraith_proto::pane::{PaneCmd, PaneEvent, SpawnOpts};
use wraith_proto::wire::{
    ClientKind, ClientMsg, PROTO_VERSION, PaneId, PaneInfo, ServerMsg, read_frame, write_frame,
};

use crate::model::{Effect, Model};
use crate::pane::{PaneHandle, PaneOptions, spawn_pane};

type ClientId = u64;

/// Messages queued per client before it counts as dead (it is detached, never waited on).
const CLIENT_QUEUE: usize = 256;

/// Layout writes are coalesced: one write this long after the first unsaved change (spec §4.4).
const SAVE_DELAY: Duration = Duration::from_secs(1);

enum Ev {
    Connected(ClientId, Sender<ServerMsg>),
    Msg(ClientId, ClientMsg),
    Gone(ClientId),
    Pane(PaneId, PaneEvent),
}

struct Client {
    tx: Sender<ServerMsg>,
    kind: Option<ClientKind>,
    /// The workspace this window shows.
    workspace: Option<WorkspaceId>,
    /// Panes whose frames and images this window wants.
    subs: HashSet<PaneId>,
    /// Refused (bad Hello): told why, then ignored until it closes its end. The server never closes
    /// first, because closing a pipe discards a reply the client has not read yet.
    refused: bool,
}

/// `WRAITH_LAYOUT_PATH`, else `%LOCALAPPDATA%\wraith\layout.json`.
pub fn default_layout_path() -> PathBuf {
    if let Some(p) = std::env::var_os("WRAITH_LAYOUT_PATH") {
        return PathBuf::from(p);
    }
    std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from).join("wraith").join("layout.json")
}

/// Serve `pipe` until a `ServerStop`, persisting the layout at `default_layout_path()`.
/// `AddrInUse` if another server already owns the pipe.
pub fn run_server(pipe: &str) -> io::Result<()> {
    serve(pipe.to_string(), PipeListener::bind(pipe)?, Some(default_layout_path()))
}

/// Bind now (so clients can connect as soon as this returns), serve on a new thread without persistence.
pub fn spawn_server_thread(pipe: &str) -> io::Result<JoinHandle<io::Result<()>>> {
    spawn_server_thread_at(pipe, None)
}

/// Like `spawn_server_thread`, persisting the layout to `layout` when given.
pub fn spawn_server_thread_at(pipe: &str, layout: Option<PathBuf>) -> io::Result<JoinHandle<io::Result<()>>> {
    let listener = PipeListener::bind(pipe)?;
    let pipe = pipe.to_string();
    Ok(thread::spawn(move || serve(pipe, listener, layout)))
}

/// Load the saved layout. A missing file is a fresh start; a bad one is moved to `<file>.bad`.
fn load_model(path: &Path) -> Model {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Model::new(),
        Err(e) => {
            tracing::warn!("layout {}: {e}", path.display());
            return Model::new();
        }
    };
    let parsed = String::from_utf8(bytes).map_err(|e| e.to_string()).and_then(|s| Model::from_json(&s));
    match parsed {
        Ok(m) => m,
        Err(e) => {
            let bad = PathBuf::from(format!("{}.bad", path.display()));
            tracing::warn!("layout {} is unusable ({e}); moved to {}", path.display(), bad.display());
            let _ = std::fs::remove_file(&bad);
            if let Err(e) = std::fs::rename(path, &bad) {
                tracing::warn!("could not quarantine the layout file: {e}");
            }
            Model::new()
        }
    }
}

fn serve(pipe: String, mut listener: PipeListener, layout: Option<PathBuf>) -> io::Result<()> {
    let (tx, rx) = unbounded::<Ev>();
    let stopping = Arc::new(AtomicBool::new(false));
    let accept = {
        let tx = tx.clone();
        let stopping = stopping.clone();
        thread::Builder::new().name("wraith-accept".into()).spawn(move || {
            let mut next: ClientId = 1;
            loop {
                let conn = match listener.accept() {
                    Ok(c) => c,
                    Err(e) => {
                        // One bad connection must never stop the server from accepting the next.
                        tracing::warn!("accept: {e}");
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                };
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
                    // Dropped (or gone): close the pipe so the client sees EOF instead of a silent server.
                    w.disconnect();
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
    let model = layout.as_deref().map_or_else(Model::new, load_model);
    let saved_rev = model.rev();
    let mut server = Server { tx, pipe: pipe.clone(), clients: HashMap::new(), panes: HashMap::new(), model, layout, saved_rev, save_due: None };
    server.restore();
    server.run(&rx);
    // Shells are gone (run dropped every pane). Wake the accept thread so it releases the pipe.
    stopping.store(true, Ordering::Release);
    let _ = wraith_ipc::connect(&pipe);
    let _ = accept.join();
    Ok(())
}

struct Server {
    tx: Sender<Ev>,
    pipe: String,
    clients: HashMap<ClientId, Client>,
    /// Running panes. A pane in the model without a handle failed to start.
    panes: HashMap<PaneId, PaneHandle>,
    model: Model,
    layout: Option<PathBuf>,
    saved_rev: u64,
    save_due: Option<Instant>,
}

impl Server {
    fn restore(&mut self) {
        let fx = self.model.restore_effects();
        self.run_effects(fx);
    }

    fn run(&mut self, rx: &Receiver<Ev>) {
        loop {
            let ev = match self.save_due {
                Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                    Ok(ev) => ev,
                    Err(RecvTimeoutError::Timeout) => {
                        self.save();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(ev) => ev,
                    Err(_) => break,
                },
            };
            match ev {
                Ev::Connected(id, tx) => {
                    self.clients.insert(id, Client { tx, kind: None, workspace: None, subs: HashSet::new(), refused: false });
                }
                Ev::Gone(id) => self.drop_client(id),
                Ev::Pane(pane, ev) => self.pane_event(pane, ev),
                Ev::Msg(id, ClientMsg::ServerStop) if self.clients.get(&id).is_some_and(|c| c.kind.is_some()) => {
                    tracing::info!("server stop requested by client {id}");
                    self.save();
                    self.panes.clear();
                    return;
                }
                Ev::Msg(id, m) => self.client_msg(id, m),
            }
            if self.layout.is_some() && self.model.rev() != self.saved_rev && self.save_due.is_none() {
                self.save_due = Some(Instant::now() + SAVE_DELAY);
            }
        }
    }

    /// Write the layout: temp file, then replace (spec §4.4).
    fn save(&mut self) {
        self.save_due = None;
        let Some(path) = &self.layout else { return };
        self.saved_rev = self.model.rev();
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        let write = || -> io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&tmp, self.model.to_json())?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            tracing::warn!("saving layout {}: {e}", path.display());
        }
    }

    fn send(&mut self, id: ClientId, m: ServerMsg) {
        let Some(c) = self.clients.get(&id) else { return };
        match c.tx.try_send(m) {
            Ok(()) => {}
            // Too slow or dead: never let one client stall the server.
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                tracing::warn!("client {id} not keeping up; dropping it");
                self.drop_client(id);
            }
        }
    }

    fn gui_ids(&self) -> Vec<ClientId> {
        self.clients.iter().filter(|(_, c)| c.kind == Some(ClientKind::Gui)).map(|(&id, _)| id).collect()
    }

    fn push_snapshot(&mut self) {
        let snap = self.model.snapshot();
        for id in self.gui_ids() {
            self.send(id, ServerMsg::Snapshot(snap.clone()));
        }
    }

    fn drop_client(&mut self, id: ClientId) {
        let Some(c) = self.clients.remove(&id) else { return };
        if let Some(ws) = c.workspace {
            self.model.set_attached(ws, false);
            self.push_snapshot();
        }
    }

    /// Give `id` workspace `ws`, taking it from whichever window had it.
    fn take_workspace(&mut self, id: ClientId, ws: WorkspaceId) {
        let old = self.clients.get_mut(&id).and_then(|c| {
            c.subs.clear();
            c.workspace.take()
        });
        if let Some(old) = old {
            self.model.set_attached(old, false);
        }
        let holders: Vec<ClientId> = self.clients.iter().filter(|(_, c)| c.workspace == Some(ws)).map(|(&h, _)| h).collect();
        for h in holders {
            if let Some(c) = self.clients.get_mut(&h) {
                c.workspace = None;
                c.subs.clear();
            }
            self.send(h, ServerMsg::WorkspaceTaken { workspace: ws });
        }
        if let Some(c) = self.clients.get_mut(&id) {
            c.workspace = Some(ws);
        }
        self.model.set_attached(ws, true);
        self.send(id, ServerMsg::WindowAttached { workspace: ws });
        self.push_snapshot();
    }

    /// `None`: the most recently detached workspace, else a new empty one.
    fn attach_window(&mut self, id: ClientId, requested: Option<WorkspaceId>) {
        let ws = match requested {
            Some(w) if self.model.has_workspace(w) => w,
            Some(w) => return self.send(id, ServerMsg::Error(format!("no workspace {w}"))),
            None => {
                // Release first so a window asking again gets its own workspace back.
                if let Some(old) = self.clients.get_mut(&id).and_then(|c| c.workspace.take()) {
                    self.model.set_attached(old, false);
                }
                match self.model.most_recent_detached() {
                    Some(w) => w,
                    None => self.model.new_empty_workspace(),
                }
            }
        };
        self.take_workspace(id, ws);
    }

    fn detach_window(&mut self, id: ClientId) {
        let Some(ws) = self.clients.get_mut(&id).and_then(|c| {
            c.subs.clear();
            c.workspace.take()
        }) else {
            return;
        };
        self.model.set_attached(ws, false);
        self.push_snapshot();
    }

    fn mine(&self, c: &Client, ws: Option<WorkspaceId>) -> bool {
        ws.is_some() && ws == c.workspace
    }

    /// A GUI client may only touch its own workspace's panes and tabs (a CLI client, anything).
    fn owns_pane(&self, id: ClientId, pane: PaneId) -> bool {
        self.clients.get(&id).is_some_and(|c| c.kind == Some(ClientKind::Cli) || self.mine(c, self.model.workspace_of(pane)))
    }

    fn permitted(&self, id: ClientId, cmd: &ModelCmd) -> bool {
        let Some(c) = self.clients.get(&id) else { return false };
        if c.kind == Some(ClientKind::Cli) {
            return true;
        }
        let held_by_other = |ws: WorkspaceId| self.clients.iter().any(|(&h, o)| h != id && o.workspace == Some(ws));
        let tab = |t: TabId| self.mine(c, self.model.workspace_of_tab(t));
        let free_or_mine = |ws: WorkspaceId| c.workspace == Some(ws) || !held_by_other(ws);
        match cmd {
            ModelCmd::NewWorkspace { .. } => true,
            ModelCmd::CloseWorkspace(w) | ModelCmd::RenameWorkspace(w, _) => free_or_mine(*w),
            ModelCmd::NewTab { workspace, .. } => c.workspace == Some(*workspace),
            ModelCmd::CloseTab(t)
            | ModelCmd::SelectTab(t)
            | ModelCmd::RenameTab(t, _)
            | ModelCmd::SetRatio { tab: t, .. }
            | ModelCmd::Equalize(t)
            | ModelCmd::ToggleZoom(t) => tab(*t),
            ModelCmd::MoveTab { tab: t, to, .. } => tab(*t) && free_or_mine(*to),
            ModelCmd::Split { pane, .. } | ModelCmd::ClosePane(pane) | ModelCmd::FocusPane(pane) | ModelCmd::NudgeSplit { pane, .. } => {
                self.mine(c, self.model.workspace_of(*pane))
            }
        }
    }

    fn refuse(&mut self, id: ClientId) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.refused = true;
        }
    }

    fn client_msg(&mut self, id: ClientId, m: ClientMsg) {
        let Some((kind, refused)) = self.clients.get(&id).map(|c| (c.kind, c.refused)) else { return };
        if refused {
            return;
        }
        let m = match (kind, m) {
            (None, ClientMsg::Hello { proto_version, kind }) => {
                if proto_version == PROTO_VERSION {
                    self.clients.get_mut(&id).unwrap().kind = Some(kind);
                    self.send(id, ServerMsg::Welcome { server_version: env!("CARGO_PKG_VERSION").into() });
                } else {
                    self.send(id, ServerMsg::VersionMismatch { server_version: env!("CARGO_PKG_VERSION").into() });
                    self.refuse(id);
                }
                return;
            }
            (None, _) => {
                self.send(id, ServerMsg::Error("expected Hello".into()));
                self.refuse(id);
                return;
            }
            (Some(_), m) => m,
        };
        match m {
            ClientMsg::Hello { .. } | ClientMsg::ServerStop => {}
            ClientMsg::AttachWindow { workspace } => self.attach_window(id, workspace),
            ClientMsg::DetachWindow => self.detach_window(id),
            ClientMsg::Model(cmd) => {
                if !self.permitted(id, &cmd) {
                    return self.send(id, ServerMsg::Error("not your workspace".into()));
                }
                self.model_cmd(id, cmd);
            }
            ClientMsg::Subscribe { panes } => {
                let wanted: HashSet<PaneId> = panes.into_iter().filter(|&p| self.owns_pane(id, p)).collect();
                let Some(c) = self.clients.get_mut(&id) else { return };
                let added: Vec<PaneId> = wanted.difference(&c.subs).copied().collect();
                c.subs = wanted;
                for p in added {
                    // Fresh view: forget the unacked frame of whoever looked before and resend everything.
                    if let Some(h) = self.panes.get(&p) {
                        let _ = h.send(PaneCmd::Reattach);
                    }
                }
            }
            // Shutdown and Reattach are the server's own lifecycle commands, never a client's.
            ClientMsg::Cmd { cmd: PaneCmd::Shutdown | PaneCmd::Reattach, .. } => {}
            ClientMsg::Cmd { pane, cmd } => {
                if !self.owns_pane(id, pane) {
                    return;
                }
                let restart = matches!(cmd, PaneCmd::Restart);
                if let Some(h) = self.panes.get(&pane) {
                    let _ = h.send(cmd);
                }
                if restart && self.model.clear_exit(pane) {
                    self.push_snapshot();
                }
            }
            ClientMsg::Kill { pane } => {
                if self.owns_pane(id, pane) {
                    self.model_cmd(id, ModelCmd::ClosePane(pane));
                }
            }
            ClientMsg::List => {
                let snap = self.model.snapshot();
                let attached = |ws| snap.workspace(ws).is_some_and(|w| w.attached);
                let mut list: Vec<PaneInfo> = snap
                    .panes
                    .iter()
                    .map(|p| {
                        let workspace = self.model.workspace_of(p.id).unwrap_or(0);
                        PaneInfo {
                            id: p.id,
                            title: p.title.clone(),
                            attached: attached(workspace),
                            exited: p.exit,
                            shell_pid: self.panes.get(&p.id).map_or(0, PaneHandle::pid),
                            workspace,
                        }
                    })
                    .collect();
                list.sort_by_key(|p| p.id);
                self.send(id, ServerMsg::Panes(list));
            }
        }
    }

    fn model_cmd(&mut self, id: ClientId, cmd: ModelCmd) {
        match self.model.apply(cmd) {
            Ok(fx) => {
                self.run_effects(fx);
                self.after_model_change();
            }
            Err(e) => self.send(id, ServerMsg::Error(e)),
        }
    }

    /// Windows whose workspace was deleted move to another one; everyone gets the new snapshot.
    fn after_model_change(&mut self) {
        let lost: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, c)| c.kind == Some(ClientKind::Gui) && c.workspace.is_some_and(|w| !self.model.has_workspace(w)))
            .map(|(&id, _)| id)
            .collect();
        for id in lost {
            if let Some(c) = self.clients.get_mut(&id) {
                c.workspace = None;
            }
            self.attach_window(id, None);
        }
        self.push_snapshot();
    }

    fn run_effects(&mut self, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::Spawn { pane, spawn } => self.start_pane(pane, spawn),
                Effect::Kill(pane) => {
                    // Dropping the handle shuts the pane down; its Job Object kills the shell tree.
                    self.panes.remove(&pane);
                    for c in self.clients.values_mut() {
                        c.subs.remove(&pane);
                    }
                }
            }
        }
    }

    fn start_pane(&mut self, pane: PaneId, spawn: SpawnOpts) {
        let mut env = spawn.env;
        let ours = [("WRAITH_PANE_ID", pane.to_string()), ("WRAITH_PIPE", self.pipe.clone()), ("TERM_PROGRAM", "wraith".to_string())];
        let wslenv = crate::profile::wslenv_with(env.iter().find(|(k, _)| k == "WSLENV").map(|(_, v)| v.clone()).or_else(|| std::env::var("WSLENV").ok()).as_deref());
        env.retain(|(k, _)| k != "WSLENV" && !ours.iter().any(|(o, _)| o == k));
        env.extend(ours.into_iter().map(|(k, v)| (k.to_string(), v)));
        env.push(("WSLENV".into(), wslenv));
        let opts = PaneOptions {
            cmdline: spawn.cmdline,
            cwd: spawn.cwd.map(PathBuf::from),
            env,
            cols: spawn.cols,
            rows: spawn.rows,
            cell_w_px: spawn.cell_w_px,
            cell_h_px: spawn.cell_h_px,
            scrollback_lines: spawn.scrollback_lines,
            default_fg: spawn.default_fg,
            default_bg: spawn.default_bg,
            palette: spawn.palette,
            image_storage_limit: spawn.image_storage_limit,
        };
        match spawn_pane(opts) {
            Ok(handle) => {
                let events = handle.events.clone();
                let tx = self.tx.clone();
                thread::spawn(move || {
                    for ev in events {
                        if tx.send(Ev::Pane(pane, ev)).is_err() {
                            return;
                        }
                    }
                });
                self.panes.insert(pane, handle);
            }
            Err(e) => {
                tracing::warn!("pane {pane}: could not start shell: {e}");
                self.model.note_title(pane, format!("could not start: {e}"));
                self.model.note_exit(pane, 1);
            }
        }
    }

    fn pane_event(&mut self, pane: PaneId, ev: PaneEvent) {
        if !self.panes.contains_key(&pane) {
            return; // closed since the event was queued
        }
        let changed = match &ev {
            PaneEvent::Title(t) => self.model.note_title(pane, t.clone()),
            PaneEvent::Exited(code) => self.model.note_exit(pane, *code),
            PaneEvent::Cwd(c) => self.model.note_cwd(pane, c.clone()),
            _ => false,
        };
        if changed {
            self.push_snapshot();
        }
        // Frames and images only go to windows that draw this pane; everything else to the workspace's window.
        let subscribed_only = matches!(
            ev,
            PaneEvent::Frame(_) | PaneEvent::ImageUpload { .. } | PaneEvent::ImageDelete { .. } | PaneEvent::MouseTracking(_) | PaneEvent::InputWritten { .. }
        );
        // With nobody subscribed, frames go unacked and the pane stops producing them by itself.
        let ws = self.model.workspace_of(pane);
        let to: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, c)| c.kind == Some(ClientKind::Gui) || c.subs.contains(&pane))
            .filter(|(_, c)| if subscribed_only { c.subs.contains(&pane) } else { c.workspace.is_some() && c.workspace == ws })
            .map(|(&id, _)| id)
            .collect();
        for id in to {
            self.send(id, ServerMsg::Event { pane, ev: ev.clone() });
        }
    }
}
