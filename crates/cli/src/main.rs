//! `wr`: script a running Wraith server, and connect coding agents to it.
//!   wr ctl list | send-text <id> <text> | kill <id> | server-stop
//!   wr hook claude|codex|generic ...            (called by agents; always exits 0, silently)
//!   wr install-hooks|uninstall-hooks claude|codex [--wsl <distro>] [--file <path>] [--yes]
//!   wr config check [path]
//!   wr events [--json]                          (JSON lines until the server stops)
//! Exit codes: 0 ok, 1 usage error / server error / unknown pane / config error, 2 no server running (never starts one).
mod check;
mod hook;
mod install;

use std::io::{self, Write};
use std::process::ExitCode;

use wraith_ipc::{PipeReader, PipeWriter, connect, pipe_name};
use wraith_proto::model::ModelSnapshot;
use wraith_proto::pane::PaneCmd;
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, PaneInfo, ServerMsg, read_frame, write_frame};

const USAGE: &str = "usage: wr ctl list | send-text <pane> <text> | kill <pane> | server-stop
       wr install-hooks|uninstall-hooks claude|codex [--wsl <distro>] [--file <path>] [--yes]
       wr config check [path]
       wr events [--json]";

enum Fail {
    Usage(String),
    NoServer,
    Server(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Fail::NoServer) => {
            eprintln!("wr: no Wraith server is running");
            ExitCode::from(2)
        }
        Err(Fail::Usage(m) | Fail::Server(m)) => {
            eprintln!("wr: {m}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<(), Fail> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["hook", rest @ ..] => hook::run(rest),
        ["install-hooks", rest @ ..] => return install::run(true, rest).map_err(Fail::Usage),
        ["uninstall-hooks", rest @ ..] => return install::run(false, rest).map_err(Fail::Usage),
        ["config", "check", rest @ ..] => return check::run(rest).map_err(Fail::Usage),
        _ => {}
    }
    let pane = |s: &str| s.parse::<PaneId>().map_err(|_| Fail::Usage(format!("not a pane id: {s}")));
    match args.as_slice() {
        ["ctl", "list"] => {
            let mut out = io::stdout().lock();
            for p in list(&mut session()?)? {
                let state = if p.attached { "attached" } else { "detached" };
                let exit = p.exited.map_or("running".to_string(), |c| format!("exited {c}"));
                let _ = writeln!(out, "{}\t{state}\t{exit}\t{}\t{}", p.id, p.workspace, p.title);
            }
            Ok(())
        }
        ["ctl", "send-text", id, text] => {
            let id = pane(id)?;
            let mut s = existing(id)?;
            send(&mut s.1, ClientMsg::Cmd { pane: id, cmd: PaneCmd::Text((*text).to_string()) })
        }
        ["ctl", "kill", id] => {
            let id = pane(id)?;
            let mut s = existing(id)?;
            send(&mut s.1, ClientMsg::Kill { pane: id })
        }
        ["ctl", "server-stop"] => send(&mut session()?.1, ClientMsg::ServerStop),
        // JSON lines with or without the flag: the spec names only `--json`.
        ["events"] | ["events", "--json"] => {
            let (mut r, mut w) = session()?;
            send(&mut w, ClientMsg::Watch)?;
            let mut last = ModelSnapshot::default();
            let mut out = io::stdout().lock();
            // A read error is the server going away; a write error is our reader gone: both end quietly.
            while let Ok(m) = read_frame::<ServerMsg>(&mut r) {
                let ServerMsg::Snapshot(snap) = m else { continue };
                for ev in wraith_cli::events::diff(&last, &snap) {
                    if writeln!(out, "{ev}").and_then(|()| out.flush()).is_err() {
                        return Ok(());
                    }
                }
                last = snap;
            }
            Ok(())
        }
        _ => Err(Fail::Usage(USAGE.into())),
    }
}

/// Connect and say hello as a CLI client.
fn session() -> Result<(PipeReader, PipeWriter), Fail> {
    let conn = connect(&pipe_name()).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => Fail::NoServer,
        _ => Fail::Server(e.to_string()),
    })?;
    let (mut r, mut w) = conn.split();
    send(&mut w, ClientMsg::Hello { proto_version: PROTO_VERSION, kind: ClientKind::Cli })?;
    match read_frame::<ServerMsg>(&mut r).map_err(|e| Fail::Server(e.to_string()))? {
        ServerMsg::Welcome { .. } => Ok((r, w)),
        ServerMsg::VersionMismatch { server_version } => {
            Err(Fail::Server(format!("server is version {server_version}; this wr speaks protocol {PROTO_VERSION}")))
        }
        other => Err(Fail::Server(format!("unexpected reply {other:?}"))),
    }
}

/// A session, after checking that pane `id` exists (the server ignores commands for unknown panes).
fn existing(id: PaneId) -> Result<(PipeReader, PipeWriter), Fail> {
    let mut s = session()?;
    if list(&mut s)?.iter().any(|p| p.id == id) {
        Ok(s)
    } else {
        Err(Fail::Server(format!("no pane {id}")))
    }
}

fn list(s: &mut (PipeReader, PipeWriter)) -> Result<Vec<PaneInfo>, Fail> {
    send(&mut s.1, ClientMsg::List)?;
    loop {
        match read_frame::<ServerMsg>(&mut s.0).map_err(|e| Fail::Server(e.to_string()))? {
            ServerMsg::Panes(p) => return Ok(p),
            ServerMsg::Error(e) => return Err(Fail::Server(e)),
            _ => {}
        }
    }
}

fn send(w: &mut PipeWriter, m: ClientMsg) -> Result<(), Fail> {
    write_frame(w, &m).map_err(|e| Fail::Server(e.to_string()))
}
