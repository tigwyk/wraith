//! The M1 app: one window, one pane. Window messages are always handled before frame work (spec §6.3).
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetDoubleClickTime, GetKeyState, GetKeyboardLayout, GetKeyboardState, VK_F4, VK_MENU, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, IDYES, MB_ICONQUESTION, MB_YESNO, MessageBoxW, PostMessageW, SWP_NOACTIVATE,
    SWP_NOZORDER, SetWindowPos, SetWindowTextW, WM_APP, WM_IME_COMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
    WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::{PCWSTR, Result};
use wraith_config::{Action, Config, Keybind, RightClick, Trigger, TriggerKey};
use wraith_proto::{KeyCode, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent};
use wraith_server::pane::{PaneCmd, PaneEvent, PaneHandle, PaneOptions, ScrollTo, spawn_pane};

use crate::atlas::GlyphAtlas;
use crate::font::{CellMetrics, FontStack};
use crate::gpu::{Gpu, SwapChain};
use crate::grid::{DrawOpts, GridRenderer};
use crate::input::{Ime, ImeUpdate, KeyTranslator};
pub use crate::selection::{SelAction, Selection};
use crate::selection::{get_clipboard, set_clipboard};
use crate::window::{LoopControl, WinMsg, Window, run_message_loop};

/// Grid that fits a client area, or `None` for a zero-size (minimized) window. Never below 2×1.
pub fn grid_size(client_w: u32, client_h: u32, m: &CellMetrics, pad_x: u16, pad_y: u16) -> Option<(u16, u16)> {
    if client_w == 0 || client_h == 0 {
        return None;
    }
    let cols = client_w.saturating_sub(2 * u32::from(pad_x)) / m.w.max(1);
    let rows = client_h.saturating_sub(2 * u32::from(pad_y)) / m.h.max(1);
    Some((cols.clamp(2, u32::from(u16::MAX)) as u16, rows.clamp(1, u32::from(u16::MAX)) as u16))
}

pub fn pixel_to_cell(x: f32, y: f32, origin: (f32, f32), m: &CellMetrics) -> (u16, u16) {
    let cx = ((x - origin.0) / m.w as f32).floor().max(0.0);
    let cy = ((y - origin.1) / m.h as f32).floor().max(0.0);
    (cx as u16, cy as u16)
}

#[derive(Debug, PartialEq)]
pub enum Match {
    Action(Action),
    /// A sequence prefix matched; keys are held until it completes or times out.
    Pending,
    /// No binding: send these keys to the pane (buffered prefix included).
    None(Vec<KeyEvent>),
}

pub struct KeybindMatcher {
    binds: Vec<Keybind>,
    pending: Vec<KeyEvent>,
    since: Option<Instant>,
}

const SEQUENCE_TIMEOUT: Duration = Duration::from_secs(1);

impl KeybindMatcher {
    pub fn new(binds: &[Keybind]) -> KeybindMatcher {
        KeybindMatcher { binds: binds.to_vec(), pending: Vec::new(), since: None }
    }

    pub fn feed(&mut self, ev: &KeyEvent, now: Instant) -> Match {
        if is_modifier(ev.code) {
            return if self.pending.is_empty() { Match::None(vec![ev.clone()]) } else { Match::Pending };
        }
        self.pending.push(ev.clone());
        let n = self.pending.len();
        let mut prefix = false;
        for b in &self.binds {
            if b.trigger.len() >= n && b.trigger[..n].iter().zip(&self.pending).all(|(t, e)| trigger_matches(t, e)) {
                if b.trigger.len() == n {
                    self.pending.clear();
                    self.since = None;
                    return Match::Action(b.action);
                }
                prefix = true;
            }
        }
        if prefix {
            self.since = Some(now);
            Match::Pending
        } else {
            self.since = None;
            Match::None(std::mem::take(&mut self.pending))
        }
    }

    /// Keys to forward once a pending sequence has timed out.
    pub fn tick(&mut self, now: Instant) -> Vec<KeyEvent> {
        match self.since {
            Some(t) if now.duration_since(t) > SEQUENCE_TIMEOUT => {
                self.since = None;
                std::mem::take(&mut self.pending)
            }
            _ => Vec::new(),
        }
    }
}

fn is_modifier(c: KeyCode) -> bool {
    use KeyCode as K;
    matches!(c, K::ShiftLeft | K::ShiftRight | K::ControlLeft | K::ControlRight | K::AltLeft | K::AltRight | K::MetaLeft | K::MetaRight)
}

fn trigger_matches(t: &Trigger, e: &KeyEvent) -> bool {
    const MASK: u8 = Mods::SHIFT | Mods::CTRL | Mods::ALT | Mods::SUPER;
    if t.mods.0 != e.mods.0 & MASK {
        return false;
    }
    match &t.key {
        TriggerKey::Physical(code) => format!("{:?}", e.code) == *code,
        TriggerKey::Char(c) => key_char(e.code) == Some(*c),
        TriggerKey::Named(n) => named_key(n) == Some(e.code),
    }
}

/// The unshifted US character a physical key is named by in keybinds.
fn key_char(c: KeyCode) -> Option<char> {
    let name = format!("{c:?}");
    if let Some(l) = name.strip_prefix("Key") {
        return l.chars().next().map(|c| c.to_ascii_lowercase());
    }
    if let Some(d) = name.strip_prefix("Digit") {
        return d.chars().next();
    }
    use KeyCode as K;
    Some(match c {
        K::Minus => '-',
        K::Equal => '=',
        K::BracketLeft => '[',
        K::BracketRight => ']',
        K::Backslash => '\\',
        K::Semicolon => ';',
        K::Quote => '\'',
        K::Backquote => '`',
        K::Comma => ',',
        K::Period => '.',
        K::Slash => '/',
        _ => return None,
    })
}

fn named_key(n: &str) -> Option<KeyCode> {
    use KeyCode as K;
    let digits = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    if let Some(i) = digits.iter().position(|d| *d == n) {
        return Some([K::Digit0, K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6, K::Digit7, K::Digit8, K::Digit9][i]);
    }
    if let Some(f) = n.strip_prefix('f').and_then(|f| f.parse::<u8>().ok()).filter(|f| (1..=24).contains(f)) {
        let all = [K::F1, K::F2, K::F3, K::F4, K::F5, K::F6, K::F7, K::F8, K::F9, K::F10, K::F11, K::F12, K::F13, K::F14, K::F15, K::F16, K::F17, K::F18, K::F19, K::F20, K::F21, K::F22, K::F23, K::F24];
        return Some(all[f as usize - 1]);
    }
    Some(match n {
        "enter" | "return" => K::Enter,
        "escape" => K::Escape,
        "tab" => K::Tab,
        "space" => K::Space,
        "backspace" => K::Backspace,
        "delete" => K::Delete,
        "insert" => K::Insert,
        "home" => K::Home,
        "end" => K::End,
        "page_up" => K::PageUp,
        "page_down" => K::PageDown,
        "up" | "arrow_up" => K::ArrowUp,
        "down" | "arrow_down" => K::ArrowDown,
        "left" | "arrow_left" => K::ArrowLeft,
        "right" | "arrow_right" => K::ArrowRight,
        "equal" | "plus" => K::Equal,
        "minus" => K::Minus,
        "comma" => K::Comma,
        "period" => K::Period,
        "slash" => K::Slash,
        "semicolon" => K::Semicolon,
        "grave_accent" | "backquote" => K::Backquote,
        _ => return None,
    })
}

/// Bundled themes ship beside the exe; fall back to the repo copy for `cargo run`.
// shortcut: repo fallback until the installer (M5) lays out themes/ next to wraith.exe.
pub fn themes_dir() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("themes")));
    beside.filter(|d| d.is_dir()).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes"))
}

fn default_shell() -> String {
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("pwsh.exe").is_file()));
    if on_path { "pwsh.exe".into() } else { "powershell.exe".into() }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn palette(c: &Config) -> Vec<(u8, u8, u8)> {
    c.palette.iter().map(|p| (p.0, p.1, p.2)).collect()
}

pub(crate) struct App {
    pub(crate) win: Window,
    pub(crate) gpu: Gpu,
    pub(crate) sc: SwapChain,
    pub(crate) fonts: FontStack,
    pub(crate) atlas: GlyphAtlas,
    pub(crate) grid: GridRenderer,
    pub(crate) pane: PaneHandle,
    pub(crate) events: Receiver<PaneEvent>,
    pub(crate) woken: Arc<AtomicBool>,
    pub(crate) config: Config,
    pub(crate) font_size: f32,
    keys: KeyTranslator,
    ime: Ime,
    sel: Selection,
    pub(crate) matcher: KeybindMatcher,
    pub(crate) rs: crate::reload::State,
    mouse_tracking: bool,
    focused: bool,
    dirty: bool,
    unacked: Option<u64>,
    exited: Option<u32>,
    size_px: (u32, u32),
    grid_cells: (u16, u16),
    blink_on: bool,
    next_blink: Instant,
    mouse_down: bool,
    pub(crate) quit: bool,
}

impl App {
    fn origin(&self) -> (f32, f32) {
        let s = self.win.dpi() as f32 / 96.0;
        ((f32::from(self.config.window_padding_x) * s).round(), (f32::from(self.config.window_padding_y) * s).round())
    }

    fn pad_px(&self) -> (u16, u16) {
        let o = self.origin();
        (o.0 as u16, o.1 as u16)
    }

    pub(crate) fn rebuild_fonts(&mut self) {
        match FontStack::new(&self.config.font_family, self.font_size, self.win.dpi(), &self.config.font_feature) {
            Ok(f) => self.fonts = f,
            Err(e) => tracing::warn!("font: {e}"),
        }
        if let Ok(a) = GlyphAtlas::new(&self.gpu, 2048) {
            self.atlas = a;
        }
        self.resize(self.size_px.0, self.size_px.1, true);
    }

    fn resize(&mut self, w: u32, h: u32, force: bool) {
        self.size_px = (w, h);
        let m = self.fonts.metrics();
        let (px, py) = self.pad_px();
        let Some(cells) = grid_size(w, h, &m, px, py) else { return };
        let _ = self.sc.resize(w, h);
        self.dirty = true;
        if cells != self.grid_cells || force {
            self.grid_cells = cells;
            let _ = self.pane.send(PaneCmd::Resize { cols: cells.0, rows: cells.1, cell_w_px: m.w, cell_h_px: m.h });
        }
    }

    fn send_key(&mut self, ev: KeyEvent) {
        if self.exited.is_some() {
            if ev.code == KeyCode::Enter && ev.action != wraith_proto::KeyAction::Release {
                self.exited = None;
                let _ = self.pane.send(PaneCmd::Restart);
            }
            return;
        }
        let _ = self.pane.send(PaneCmd::Key(ev));
    }

    fn on_key(&mut self, ev: KeyEvent) {
        if ev.action == wraith_proto::KeyAction::Release {
            return self.send_key(ev);
        }
        match self.matcher.feed(&ev, Instant::now()) {
            Match::Action(a) => self.action(a),
            Match::Pending => {}
            Match::None(keys) => keys.into_iter().for_each(|k| self.send_key(k)),
        }
    }

    pub(crate) fn action(&mut self, a: Action) {
        match a {
            Action::Copy => {
                let _ = self.pane.send(PaneCmd::CopySelection);
            }
            Action::Paste => self.paste(),
            Action::IncreaseFontSize(d) => self.set_font_size(self.font_size + d),
            Action::DecreaseFontSize(d) => self.set_font_size(self.font_size - d),
            Action::ResetFontSize => self.set_font_size(self.config.font_size),
            Action::ScrollPageUp => self.scroll(ScrollTo::PageUp),
            Action::ScrollPageDown => self.scroll(ScrollTo::PageDown),
            Action::ScrollToTop => self.scroll(ScrollTo::Top),
            Action::ScrollToBottom => self.scroll(ScrollTo::Bottom),
            Action::ClearScreen => {
                let _ = self.pane.send(PaneCmd::Text("\x0c".into()));
            }
            Action::CloseSurface => self.close(),
            Action::ReloadConfig | Action::ToggleFrameOverlay | Action::Unbind => crate::reload::action(self, a),
        }
    }

    pub(crate) fn set_font_size(&mut self, pt: f32) {
        self.font_size = clamp_font_size(pt);
        self.rebuild_fonts();
    }

    fn scroll(&mut self, s: ScrollTo) {
        let _ = self.pane.send(PaneCmd::Scroll(s));
    }

    fn paste(&mut self) {
        if let Some(t) = get_clipboard(self.win.hwnd()) {
            let _ = self.pane.send(PaneCmd::Paste(t));
        }
    }

    fn close(&mut self) {
        if self.exited.is_none() {
            let text = wide("A process is still running in this terminal. Close it?");
            let r = unsafe { MessageBoxW(Some(self.win.hwnd()), PCWSTR(text.as_ptr()), windows::core::w!("Wraith"), MB_YESNO | MB_ICONQUESTION) };
            if r != IDYES {
                return;
            }
        }
        unsafe {
            let _ = DestroyWindow(self.win.hwnd());
        }
    }

    fn mouse(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        let x = f32::from((lparam.0 & 0xffff) as i16);
        let y = f32::from(((lparam.0 >> 16) & 0xffff) as i16);
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        let alt = unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0;
        let o = self.origin();
        let m = self.fonts.metrics();
        if msg == WM_MOUSEWHEEL {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            let button = if delta > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
            // Wheel coordinates are screen-relative; position only matters for mouse reports.
            let ev = MouseEvent { x_px: 0.0, y_px: 0.0, button: Some(button), action: MouseAction::Press, mods: Mods(0) };
            let _ = self.pane.send(PaneCmd::Mouse(ev));
            return;
        }
        if self.mouse_tracking && !shift {
            let (button, action) = match msg {
                WM_LBUTTONDOWN => (Some(MouseButton::Left), MouseAction::Press),
                WM_LBUTTONUP => (Some(MouseButton::Left), MouseAction::Release),
                WM_RBUTTONDOWN => (Some(MouseButton::Right), MouseAction::Press),
                WM_RBUTTONUP => (Some(MouseButton::Right), MouseAction::Release),
                WM_MBUTTONDOWN => (Some(MouseButton::Middle), MouseAction::Press),
                WM_MBUTTONUP => (Some(MouseButton::Middle), MouseAction::Release),
                _ => (None, MouseAction::Motion),
            };
            let ev = MouseEvent { x_px: x - o.0, y_px: y - o.1, button, action, mods: Mods(0) };
            let _ = self.pane.send(PaneCmd::Mouse(ev));
            return;
        }
        let cell = pixel_to_cell(x, y, o, &m);
        let cell = (cell.0.min(self.grid_cells.0.saturating_sub(1)), cell.1.min(self.grid_cells.1.saturating_sub(1)));
        match msg {
            WM_LBUTTONDOWN => {
                self.mouse_down = true;
                let cmd = match self.sel.press(cell, Instant::now(), alt) {
                    SelAction::Start => PaneCmd::SetSelection(None),
                    SelAction::Word { x, y } => PaneCmd::SelectWord { x, y },
                    SelAction::Line { y } => PaneCmd::SelectLine { y },
                };
                let _ = self.pane.send(cmd);
            }
            WM_MOUSEMOVE if self.mouse_down => {
                if let Some(r) = self.sel.drag(cell) {
                    let _ = self.pane.send(PaneCmd::SetSelection(Some(r)));
                }
            }
            WM_LBUTTONUP => {
                self.mouse_down = false;
                if self.sel.release().is_some() && self.config.copy_on_select {
                    let _ = self.pane.send(PaneCmd::CopySelection);
                }
            }
            WM_RBUTTONDOWN if self.config.right_click_action == RightClick::Paste => self.paste(),
            _ => {}
        }
    }

    fn on_msg(&mut self, hwnd: HWND, msg: WinMsg) -> Option<LRESULT> {
        match msg {
            WinMsg::Resize { w, h } => self.resize(w, h, false),
            WinMsg::DpiChanged { suggested: r, .. } => {
                unsafe {
                    let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                let (w, h) = self.win.client_size();
                self.size_px = (w, h);
                self.rebuild_fonts();
            }
            WinMsg::Close => {
                self.close();
                return Some(LRESULT(0));
            }
            WinMsg::Raw { msg, wparam, lparam } => match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN => {
                    let vk = wparam.0 as u16;
                    let mut state = [0u8; 256];
                    unsafe {
                        let _ = GetKeyboardState(&mut state);
                    }
                    if msg == WM_SYSKEYDOWN && vk == VK_F4.0 {
                        return None; // Alt+F4 closes the window as usual.
                    }
                    let sc = ((lparam.0 >> 16) & 0xff) as u16;
                    let ext = (lparam.0 >> 24) & 1 == 1;
                    let repeat = (lparam.0 >> 30) & 1 == 1;
                    let hkl = unsafe { GetKeyboardLayout(0) };
                    if let Some(ev) = self.keys.on_keydown(vk, sc, ext, repeat, &state, hkl) {
                        crate::reload::note_input(self);
                        self.on_key(ev);
                    }
                    return Some(LRESULT(0));
                }
                WM_KEYUP | WM_SYSKEYUP => {
                    let mut state = [0u8; 256];
                    unsafe {
                        let _ = GetKeyboardState(&mut state);
                    }
                    let ev = self.keys.on_keyup(wparam.0 as u16, ((lparam.0 >> 16) & 0xff) as u16, (lparam.0 >> 24) & 1 == 1, &state);
                    self.on_key(ev);
                    return Some(LRESULT(0));
                }
                WM_IME_STARTCOMPOSITION => {
                    if let Some(c) = self.grid.cursor_pos() {
                        let m = self.fonts.metrics();
                        let o = self.origin();
                        Ime::set_candidate_pos(hwnd, (o.0 + f32::from(c.0) * m.w as f32) as i32, (o.1 + f32::from(c.1) * m.h as f32) as i32, m.h as i32);
                    }
                }
                WM_IME_COMPOSITION => {
                    if let ImeUpdate::Commit(t) = self.ime.on_composition(hwnd, lparam) {
                        let _ = self.pane.send(PaneCmd::Text(t));
                        return Some(LRESULT(0));
                    }
                }
                WM_SETFOCUS | WM_KILLFOCUS => {
                    self.focused = msg == WM_SETFOCUS;
                    self.dirty = true;
                }
                WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
                    self.mouse(msg, wparam, lparam);
                    return Some(LRESULT(0));
                }
                WM_APP => return Some(LRESULT(0)),
                _ => {}
            },
        }
        None
    }

    fn on_pane_event(&mut self, ev: PaneEvent) {
        match ev {
            PaneEvent::Frame(f) => {
                self.grid.apply(&f, &self.fonts);
                self.unacked = Some(f.seq);
                self.dirty = true;
            }
            PaneEvent::Title(t) => unsafe {
                let w = wide(&t);
                let _ = SetWindowTextW(self.win.hwnd(), PCWSTR(w.as_ptr()));
            },
            PaneEvent::Bell => {}
            PaneEvent::Clipboard(t) | PaneEvent::Copied(t) => set_clipboard(self.win.hwnd(), &t),
            PaneEvent::PasteNeedsConfirm(t) => {
                let lines = t.lines().count().max(1);
                let q = wide(&format!("Paste {lines} line(s) into the terminal?\n\n{}", t.chars().take(400).collect::<String>()));
                let r = unsafe { MessageBoxW(Some(self.win.hwnd()), PCWSTR(q.as_ptr()), windows::core::w!("Wraith: paste"), MB_YESNO | MB_ICONQUESTION) };
                if r == IDYES {
                    let _ = self.pane.send(PaneCmd::PasteConfirmed(t));
                }
            }
            PaneEvent::Exited(code) => {
                if self.config.close_on_exit {
                    unsafe {
                        let _ = DestroyWindow(self.win.hwnd());
                    }
                } else {
                    self.exited = Some(code);
                    self.dirty = true;
                }
            }
            PaneEvent::MouseTracking(on) => self.mouse_tracking = on,
            PaneEvent::InputWritten { .. } => {}
        }
    }

    /// Drain pane events, render if needed. Returns the loop's sleep budget in ms.
    fn idle(&mut self) -> (LoopControl, u32) {
        if self.quit {
            return (LoopControl::Exit, 0);
        }
        self.woken.store(false, Ordering::Release);
        while let Ok(ev) = self.events.try_recv() {
            self.on_pane_event(ev);
        }
        for k in self.matcher.tick(Instant::now()) {
            self.send_key(k);
        }
        let now = Instant::now();
        if self.config.cursor_style_blink && now >= self.next_blink {
            self.blink_on = !self.blink_on;
            self.next_blink = now + Duration::from_millis(600);
            self.dirty = true;
        }
        crate::reload::poll(self);
        if self.dirty {
            self.render();
        }
        let until_blink = self.next_blink.saturating_duration_since(Instant::now()).as_millis() as u32;
        let mut sleep = if self.config.cursor_style_blink { until_blink.max(1) } else { 1000 };
        if let Some(t) = crate::reload::next_deadline(self) {
            sleep = sleep.min(t.saturating_duration_since(Instant::now()).as_millis() as u32 + 1);
        }
        // A pending key sequence must time out even with no other activity.
        (LoopControl::Continue, if self.matcher.since.is_some() { sleep.min(100) } else { sleep })
    }

    pub(crate) fn request_redraw(&mut self) {
        self.dirty = true;
    }

    fn render(&mut self) {
        self.dirty = false;
        crate::reload::render_start(self);
        let exit_line = self.exited.map(|c| format!("[process exited {c}] Enter: restart · ctrl+shift+w: close"));
        self.grid.set_overlay(crate::grid::Overlay::Exit, exit_line.as_deref(), &self.fonts);
        self.sc.wait();
        let m = self.fonts.metrics();
        let sel = self.config.selection_background.map_or((0x3e, 0x44, 0x51), |c| (c.0, c.1, c.2));
        let opts = DrawOpts {
            selection: None,
            selection_color: sel,
            cursor_blink_on: self.blink_on || !self.config.cursor_style_blink,
            focused: self.focused,
            opacity: self.config.background_opacity,
        };
        let origin = self.origin();
        self.grid.draw(&self.gpu, self.sc.rtv(), self.size_px, &mut self.atlas, &self.fonts, origin, &m, &opts);
        let _ = self.sc.present();
        crate::reload::presented(self);
        // Ack only after presenting: the pane sends the next frame when we can show it.
        if let Some(seq) = self.unacked.take() {
            let _ = self.pane.send(PaneCmd::Ready(seq));
        }
    }
}

pub fn clamp_font_size(pt: f32) -> f32 {
    pt.clamp(4.0, 72.0)
}

pub fn run() -> Result<()> {
    let path = wraith_config::default_path();
    let (config, diags) = wraith_config::load(&path, &themes_dir());
    let win = Window::create("Wraith", 1000, 640, config.background_opacity < 1.0)?;
    let gpu = Gpu::new()?;
    let fonts = FontStack::new(&config.font_family, config.font_size, win.dpi(), &config.font_feature)?;
    let atlas = GlyphAtlas::new(&gpu, 2048)?;
    let grid = GridRenderer::new(&gpu)?;
    let (w, h) = win.client_size();
    let sc = gpu.create_swapchain(win.hwnd(), w, h)?;
    let m = fonts.metrics();
    let s = win.dpi() as f32 / 96.0;
    let pad = ((f32::from(config.window_padding_x) * s) as u16, (f32::from(config.window_padding_y) * s) as u16);
    let cells = grid_size(w, h, &m, pad.0, pad.1).unwrap_or((80, 24));
    let fg = config.foreground;
    let bg = config.background;
    let pane = spawn_pane(PaneOptions {
        cmdline: config.command.clone().unwrap_or_else(default_shell),
        cwd: std::env::var_os("USERPROFILE").map(PathBuf::from),
        env: vec![("TERM_PROGRAM".into(), "wraith".into())],
        cols: cells.0,
        rows: cells.1,
        cell_w_px: m.w,
        cell_h_px: m.h,
        scrollback_lines: config.scrollback_limit,
        default_fg: (fg.0, fg.1, fg.2),
        default_bg: (bg.0, bg.1, bg.2),
        palette: palette(&config),
    })
    .map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;

    // Forward pane events to the UI thread and wake its message loop (coalesced).
    let (tx, rx) = crossbeam_channel::unbounded();
    let woken = Arc::new(AtomicBool::new(false));
    {
        let events = pane.events.clone();
        let woken = woken.clone();
        let hwnd = win.hwnd().0 as usize;
        std::thread::Builder::new()
            .name("wraith-wake".into())
            .spawn(move || {
                while let Ok(ev) = events.recv() {
                    if tx.send(ev).is_err() {
                        break;
                    }
                    if !woken.swap(true, Ordering::AcqRel) {
                        unsafe {
                            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP, WPARAM(0), LPARAM(0));
                        }
                    }
                }
            })
            .map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    }

    let font_size = config.font_size;
    let matcher = KeybindMatcher::new(&config.keybinds);
    let app = Rc::new(RefCell::new(App {
        win,
        gpu,
        sc,
        fonts,
        atlas,
        grid,
        pane,
        events: rx,
        woken,
        config,
        font_size,
        keys: KeyTranslator::default(),
        ime: Ime,
        sel: Selection::new(Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }))),
        matcher,
        rs: crate::reload::State::default(),
        mouse_tracking: false,
        focused: true,
        dirty: true,
        unacked: None,
        exited: None,
        size_px: (w, h),
        grid_cells: cells,
        blink_on: true,
        next_blink: Instant::now() + Duration::from_millis(600),
        mouse_down: false,
        quit: false,
    }));
    crate::reload::start(&mut app.borrow_mut(), &path, diags);
    app.borrow().win.show();
    let a = app.clone();
    run_message_loop(
        move |hwnd, msg| a.try_borrow_mut().ok().and_then(|mut a| a.on_msg(hwnd, msg)),
        move || app.borrow_mut().idle(),
    );
    Ok(())
}
