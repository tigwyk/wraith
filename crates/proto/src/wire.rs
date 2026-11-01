//! Client/server messages and their framing: u32 little-endian length, then a postcard body.
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{ModelCmd, ModelSnapshot, WorkspaceId};
use crate::pane::{PaneCmd, PaneEvent};

pub type PaneId = u64;

/// Bumped on any incompatible change to these types.
pub const PROTO_VERSION: u32 = 2;

/// Largest body a peer may announce; anything bigger drops the connection before allocating.
pub const MAX_FRAME: u32 = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientKind {
    Gui,
    Cli,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello { proto_version: u32, kind: ClientKind },
    /// Show a workspace in this window. `None`: the most recently detached one, else a new empty one.
    AttachWindow { workspace: Option<WorkspaceId> },
    /// Leave the workspace running in the server (window closed).
    DetachWindow,
    Model(ModelCmd),
    /// The panes this window draws now. Frames and images flow only for these.
    Subscribe { panes: Vec<PaneId> },
    Cmd { pane: PaneId, cmd: PaneCmd },
    Kill { pane: PaneId },
    List,
    ServerStop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    Welcome { server_version: String },
    VersionMismatch { server_version: String },
    WindowAttached { workspace: WorkspaceId },
    /// The whole model, after every change.
    Snapshot(ModelSnapshot),
    /// Another window took this window's workspace.
    WorkspaceTaken { workspace: WorkspaceId },
    Event { pane: PaneId, ev: PaneEvent },
    Panes(Vec<PaneInfo>),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: PaneId,
    pub title: String,
    pub attached: bool,
    pub exited: Option<u32>,
    pub shell_pid: u32,
    pub workspace: WorkspaceId,
}

pub fn write_frame<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let body = postcard::to_allocvec(msg).map_err(io::Error::other)?;
    let len = u32::try_from(body.len()).ok().filter(|&n| n <= MAX_FRAME).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "message exceeds MAX_FRAME")
    })?;
    // One write per frame: a concurrent writer on the same pipe never interleaves inside it.
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&body);
    w.write_all(&out)
}

pub fn read_frame<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame of {len} bytes exceeds MAX_FRAME")));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    postcard::from_bytes(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
