//! The client's view of one pane living in the server (spec §3.1, §8).
//! Same `send` / `events` shape as the in-process pane, so the app barely notices the pipe.
use std::io;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use wraith_ipc::connect;
use wraith_proto::pane::{PaneCmd, PaneEvent, SpawnOpts};
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, ServerMsg, read_frame, write_frame};

/// How long a client keeps trying to reach a server it just started (spec §8).
const START_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum AttachError {
    /// No server answered within 2 s, even after trying to start one.
    NoServer { log_dir: PathBuf },
    /// The running server speaks another protocol version.
    VersionMismatch { server_version: String },
    /// The pipe is served by a process owned by another user.
    Denied,
    Io(io::Error),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::NoServer { log_dir } => {
                write!(f, "The Wraith server did not start. Logs: {}", log_dir.display())
            }
            AttachError::VersionMismatch { server_version } => {
                write!(f, "The running Wraith server is version {server_version}, which this window can't talk to.")
            }
            AttachError::Denied => write!(f, "Wraith's pipe is held by a process belonging to another user."),
            AttachError::Io(e) => write!(f, "Could not attach to the Wraith server: {e}"),
        }
    }
}

pub struct RemotePane {
    pub id: PaneId,
    pub reattached: bool,
    /// Disconnects when the server goes away.
    pub events: Receiver<PaneEvent>,
    out: Option<Sender<ClientMsg>>,
    writer: Option<JoinHandle<()>>,
}

pub fn log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from).join("wraith").join("logs")
}

/// Start `exe --server` detached, outside any job the client is in, so it outlives the client.
pub fn spawn_server(exe: &Path, pipe: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let start = |flags| {
        std::process::Command::new(exe)
            .arg("--server")
            .env("WRAITH_PIPE_NAME", pipe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    // Breakaway is refused inside jobs that forbid it; then the server shares the client's fate.
    start(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| start(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP))
        .map(drop)
}

/// Terminate the process serving `pipe` (only reachable when it belongs to us; see `connect`).
/// Used when its protocol version differs, so it would not accept a ServerStop.
pub fn kill_server(pipe: &str) -> io::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject};
    let pid = connect(pipe)?.server_pid()?;
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | windows::Win32::System::Threading::PROCESS_SYNCHRONIZE, false, pid)
            .map_err(io::Error::other)?;
        let r = TerminateProcess(h, 1).map_err(io::Error::other);
        WaitForSingleObject(h, 2000);
        let _ = CloseHandle(h);
        r
    }
}

impl RemotePane {
    /// Connect (starting `server_exe` if nobody listens), say hello, and attach a pane.
    pub fn attach(pipe: &str, spawn: SpawnOpts, server_exe: &Path) -> Result<RemotePane, AttachError> {
        let deadline = Instant::now() + START_TIMEOUT;
        let mut started = false;
        let conn = loop {
            match connect(pipe) {
                Ok(c) => break c,
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(AttachError::Denied),
                Err(e) if e.kind() == io::ErrorKind::NotFound || e.kind() == io::ErrorKind::TimedOut => {
                    if !started {
                        started = true;
                        if let Err(e) = spawn_server(server_exe, pipe) {
                            tracing::warn!("starting server {}: {e}", server_exe.display());
                        }
                    }
                    if Instant::now() >= deadline {
                        return Err(AttachError::NoServer { log_dir: log_dir() });
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(AttachError::Io(e)),
            }
        };
        let (mut r, mut w) = conn.split();
        let io = AttachError::Io;
        write_frame(&mut w, &ClientMsg::Hello { proto_version: PROTO_VERSION, kind: ClientKind::Gui }).map_err(io)?;
        match read_frame::<ServerMsg>(&mut r).map_err(io)? {
            ServerMsg::Welcome { .. } => {}
            ServerMsg::VersionMismatch { server_version } => return Err(AttachError::VersionMismatch { server_version }),
            other => return Err(AttachError::Io(io::Error::other(format!("unexpected reply {other:?}")))),
        }
        write_frame(&mut w, &ClientMsg::Attach { spawn }).map_err(io)?;
        let (id, reattached) = loop {
            match read_frame::<ServerMsg>(&mut r).map_err(io)? {
                ServerMsg::Attached { pane, reattached } => break (pane, reattached),
                ServerMsg::Error(e) => return Err(AttachError::Io(io::Error::other(e))),
                _ => {}
            }
        };

        let (ev_tx, events) = unbounded();
        thread::Builder::new()
            .name("wraith-pipe-read".into())
            .spawn(move || {
                // Ends (dropping ev_tx, so `events` disconnects) when the server goes away.
                while let Ok(m) = read_frame::<ServerMsg>(&mut r) {
                    if let ServerMsg::Event { pane, ev } = m
                        && pane == id
                        && ev_tx.send(ev).is_err()
                    {
                        return;
                    }
                }
            })
            .map_err(io)?;
        let (out, out_rx) = unbounded::<ClientMsg>();
        let writer = thread::Builder::new()
            .name("wraith-pipe-write".into())
            .spawn(move || {
                for m in out_rx {
                    if write_frame(&mut w, &m).is_err() {
                        return;
                    }
                }
            })
            .map_err(io)?;
        Ok(RemotePane { id, reattached, events, out: Some(out), writer: Some(writer) })
    }

    /// Queue a command for this pane; never blocks the UI thread on the pipe.
    pub fn send(&self, cmd: PaneCmd) -> Result<(), PaneCmd> {
        match &self.out {
            Some(out) => out.send(ClientMsg::Cmd { pane: self.id, cmd }).map_err(|e| match e.into_inner() {
                ClientMsg::Cmd { cmd, .. } => cmd,
                _ => unreachable!(),
            }),
            None => Err(cmd),
        }
    }

    /// Leave the shell running in the server (window X). Later sends fail.
    pub fn detach(&mut self) {
        self.finish(ClientMsg::Detach { pane: self.id });
    }

    /// End the shell (close_surface). Later sends fail.
    pub fn kill(&mut self) {
        self.finish(ClientMsg::Kill { pane: self.id });
    }

    /// Send a last message and wait until it is on the pipe (the process may exit right after).
    fn finish(&mut self, last: ClientMsg) {
        if let Some(out) = self.out.take() {
            let _ = out.send(last);
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}
