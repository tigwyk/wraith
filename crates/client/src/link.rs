//! The client's connection to the server (spec §3.1, §3.4, §8): connect-or-spawn, hello, and two
//! threads so the UI never blocks on the pipe.
use std::io;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use wraith_ipc::connect;
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, ServerMsg, read_frame, write_frame};

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

/// Connect, asking `retry` after each failure whether to try again (spec §8). A version mismatch is
/// returned at once: restarting the server is a different question, asked by the caller.
pub fn retry_attach<L>(
    mut connect: impl FnMut() -> Result<L, AttachError>,
    mut retry: impl FnMut(&AttachError) -> bool,
) -> Result<L, AttachError> {
    loop {
        match connect() {
            Err(e) if !matches!(e, AttachError::VersionMismatch { .. }) && retry(&e) => {}
            r => return r,
        }
    }
}

pub fn log_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from).join("wraith").join("logs")
}

/// Start `exe --server` detached, outside any job the client is in, so it outlives the client.
pub fn spawn_server(exe: &Path, pipe: &str, env: &[(String, String)]) -> io::Result<()> {
    spawn_detached(exe, "--server", pipe, env)
}

/// Start `exe <arg>` for `pipe` detached from this process (the server, or the quick terminal).
pub fn spawn_detached(exe: &Path, arg: &str, pipe: &str, env: &[(String, String)]) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let start = |flags| {
        std::process::Command::new(exe)
            .arg(arg)
            .env("WRAITH_PIPE_NAME", pipe)
            .envs(env.iter().map(|(k, v)| (k, v)))
            // Don't pin the launching window's directory for the server's whole life.
            .current_dir(std::env::var_os("USERPROFILE").map_or_else(std::env::temp_dir, PathBuf::from))
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

/// A connected, greeted link to the server. Messages arrive on `msgs`; it disconnects when the
/// server goes away.
pub struct ServerLink {
    pub msgs: Receiver<ServerMsg>,
    out: Option<Sender<ClientMsg>>,
    writer: Option<JoinHandle<()>>,
}

impl ServerLink {
    /// Connect (starting `server_exe` if nobody listens) and say hello.
    pub fn connect(pipe: &str, server_exe: &Path) -> Result<ServerLink, AttachError> {
        ServerLink::connect_with_env(pipe, server_exe, &[])
    }

    /// Like `connect`, with extra environment for a server this call has to start.
    pub fn connect_with_env(pipe: &str, server_exe: &Path, server_env: &[(String, String)]) -> Result<ServerLink, AttachError> {
        let deadline = Instant::now() + START_TIMEOUT;
        let mut started = false;
        let conn = loop {
            match connect(pipe) {
                Ok(c) => break c,
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(AttachError::Denied),
                Err(e) if e.kind() == io::ErrorKind::NotFound || e.kind() == io::ErrorKind::TimedOut => {
                    if !started {
                        started = true;
                        if let Err(e) = spawn_server(server_exe, pipe, server_env) {
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
        let (msg_tx, msgs) = unbounded();
        thread::Builder::new()
            .name("wraith-pipe-read".into())
            .spawn(move || {
                // Ends (dropping msg_tx, so `msgs` disconnects) when the server goes away.
                while let Ok(m) = read_frame::<ServerMsg>(&mut r) {
                    if msg_tx.send(m).is_err() {
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
        Ok(ServerLink { msgs, out: Some(out), writer: Some(writer) })
    }

    /// Queue a message; never blocks the UI thread on the pipe. False once the link is finished.
    pub fn send(&self, msg: ClientMsg) -> bool {
        self.out.as_ref().is_some_and(|o| o.send(msg).is_ok())
    }

    /// Send a last message and wait until it is on the pipe (the process may exit right after).
    pub fn finish(&mut self, last: ClientMsg) {
        if let Some(out) = self.out.take() {
            let _ = out.send(last);
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}
