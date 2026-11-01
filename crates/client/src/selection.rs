//! Mouse selection gestures and the Windows clipboard.
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use wraith_proto::SelRange;

#[derive(Debug, PartialEq, Eq)]
pub enum SelAction {
    /// Start a character selection (clears any existing one).
    Start,
    Word { x: u16, y: u16 },
    Line { y: u16 },
}

pub struct Selection {
    double_click: Duration,
    last_press: Option<(Instant, (u16, u16))>,
    clicks: u8,
    anchor: Option<(u16, u16)>,
    current: Option<SelRange>,
    rectangle: bool,
}

impl Selection {
    pub fn new(double_click: Duration) -> Selection {
        Selection { double_click, last_press: None, clicks: 0, anchor: None, current: None, rectangle: false }
    }

    pub fn press(&mut self, cell: (u16, u16), now: Instant, alt: bool) -> SelAction {
        let repeat = self.last_press.is_some_and(|(t, c)| c == cell && now.duration_since(t) <= self.double_click);
        self.clicks = if repeat { self.clicks % 3 + 1 } else { 1 };
        self.last_press = Some((now, cell));
        self.anchor = Some(cell);
        self.current = None;
        self.rectangle = alt;
        match self.clicks {
            2 => SelAction::Word { x: cell.0, y: cell.1 },
            3 => SelAction::Line { y: cell.1 },
            _ => SelAction::Start,
        }
    }

    /// New range while the button is held (only for character selections).
    pub fn drag(&mut self, cell: (u16, u16)) -> Option<SelRange> {
        let anchor = self.anchor.filter(|_| self.clicks == 1)?;
        if anchor == cell && self.current.is_none() {
            return None;
        }
        let r = SelRange { start: anchor, end: cell, rectangle: self.rectangle };
        self.current = Some(r);
        Some(r)
    }

    /// Final range of a character drag; `None` for a plain click.
    pub fn release(&mut self) -> Option<SelRange> {
        self.anchor = None;
        self.current.take()
    }
}

pub fn set_clipboard(hwnd: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            let p = GlobalLock(mem) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(mem);
                // The clipboard owns the memory after a successful SetClipboardData.
                let _ = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(mem.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

pub fn get_clipboard(hwnd: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let text = (|| {
            let h = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?;
            let mem = HGLOBAL(h.0);
            let p = GlobalLock(mem) as *const u16;
            if p.is_null() {
                return None;
            }
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(mem);
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}
