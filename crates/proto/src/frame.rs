//! Screen updates sent from a pane to the client (spec §3.4).
use serde::{Deserialize, Serialize};

/// Changed rows of one pane since the last acknowledged frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameDelta {
    pub seq: u64,
    /// Replace everything; also sent after resize or color changes.
    pub full: bool,
    pub cols: u16,
    pub rows: u16,
    pub dirty_rows: Vec<Row>,
    pub cursor: CursorState,
    pub colors: Colors,
    pub scrollbar: Scrollbar,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub y: u16,
    pub runs: Vec<StyleRun>,
    pub wrapped: bool,
}

/// Consecutive cells sharing one style. Wide graphemes occupy two cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleRun {
    pub fg: Color,
    pub bg: Color,
    pub flags: StyleFlags,
    pub hyperlink_id: u32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    Default,
    Palette(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleFlags(pub u16);

impl StyleFlags {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 2;
    pub const FAINT: u16 = 4;
    pub const UNDERLINE: u16 = 8;
    pub const DOUBLE_UNDERLINE: u16 = 16;
    pub const CURLY_UNDERLINE: u16 = 32;
    pub const STRIKE: u16 = 64;
    pub const INVERSE: u16 = 128;
    pub const INVISIBLE: u16 = 256;
    pub const BLINK: u16 = 512;

    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub style: CursorShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
    BlockHollow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Colors {
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
    pub cursor: Option<(u8, u8, u8)>,
    /// Sent only on full frames; empty otherwise.
    pub palette: Vec<(u8, u8, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scrollbar {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// Viewport-coordinate selection, inclusive (col, row).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelRange {
    pub start: (u16, u16),
    pub end: (u16, u16),
    pub rectangle: bool,
}
