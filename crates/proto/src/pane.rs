//! Commands to and events from one pane. They cross the client/server pipe (spec §3.4).
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{FrameDelta, KeyEvent, MouseEvent, SelRange};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaneCmd {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Typed or IME-committed text, written as-is.
    Text(String),
    /// Clipboard paste; unsafe text comes back as `PasteNeedsConfirm`.
    Paste(String),
    /// A paste the user confirmed; encoded (bracketed if enabled) without re-checking.
    PasteConfirmed(String),
    Resize { cols: u16, rows: u16, cell_w_px: u32, cell_h_px: u32 },
    Scroll(ScrollTo),
    Ready(u64),
    SetSelection(Option<SelRange>),
    SelectWord { x: u16, y: u16 },
    SelectLine { y: u16 },
    CopySelection,
    SetColors { fg: (u8, u8, u8), bg: (u8, u8, u8), palette: Vec<(u8, u8, u8)> },
    /// Resend every row (e.g. the client's fonts changed but the cell size did not).
    Redraw,
    /// A client (re)attached: forget the unacked frame of whoever had it before and resend everything.
    Reattach,
    Restart,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollTo {
    Delta(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaneEvent {
    Frame(FrameDelta),
    Title(String),
    Bell,
    Clipboard(String),
    Copied(String),
    PasteNeedsConfirm(String),
    Exited(u32),
    /// The shell's working directory changed (OSC 7), as a path for its own OS.
    Cwd(String),
    /// Pixels of an image a pane displays (Kitty graphics), sent once per image per view.
    ImageUpload { image_id: u32, width: u32, height: u32, rgba: Vec<u8> },
    ImageDelete { image_id: u32 },
    MouseTracking(bool),
    /// How long an input command waited between being queued and its bytes reaching the PTY.
    InputWritten { waited: Duration },
}

/// What a client asks the server to spawn when there is no detached pane to reattach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnOpts {
    pub cmdline: String,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    pub cell_w_px: u32,
    pub cell_h_px: u32,
    pub scrollback_lines: usize,
    pub default_fg: (u8, u8, u8),
    pub default_bg: (u8, u8, u8),
    pub palette: Vec<(u8, u8, u8)>,
    pub image_storage_limit: u64,
}
