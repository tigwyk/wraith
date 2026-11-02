//! The M1 app: one window, one pane. Window messages are always handled before frame work (spec §6.3).
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetDoubleClickTime, GetKeyState, GetKeyboardLayout, GetKeyboardState, ReleaseCapture, SetCapture, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent, VK_F4, VK_MENU, VK_SHIFT, VK_SPACE};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW, PostMessageW, SWP_NOACTIVATE,
    SWP_NOZORDER, SetWindowPos, SetWindowTextW, WM_APP, WM_IME_COMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
    WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    SC_CLOSE, SC_KEYMENU, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::{PCWSTR, Result};
use wraith_config::{Action, Config, Keybind, RightClick, Trigger, TriggerKey};
use wraith_proto::{KeyCode, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent};
use wraith_proto::model::{AgentState, Rect, TabId};
use wraith_proto::pane::{PaneCmd, PaneEvent, ScrollTo};
use wraith_proto::wire::{ClientMsg, PaneId, ServerMsg};

use crate::chrome::{self, Hit, chrome_colors};
use crate::link::{AttachError, ServerLink, kill_server};
use crate::images::ImageStore;
use crate::paint::Painter;
use crate::titlebar::{self, Caption, NcGeom};
use crate::palette::Mode;
use crate::view::{ExitOut, Geometry, Out, SpawnCtx, Ui, WindowView};

use crate::atlas::GlyphAtlas;
use crate::font::{CellMetrics, FontStack};
use crate::gpu::{Gpu, SwapChain};
use crate::grid::{Batch, DrawOpts, GridRenderer, Overlay, Pipeline, Quad};
use crate::input::{Ime, ImeUpdate, KeyTranslator};
pub use crate::selection::{SelAction, Selection};
use crate::selection::{get_clipboard, set_clipboard};
use crate::window::{Idle, LoopControl, WinMsg, Window, run_message_loop};

/// Grid that fits a client area, or `None` for a zero-size (minimized) window. Never below 2×1.
pub fn grid_size(client_w: u32, client_h: u32, m: &CellMetrics, pad_x: u16, pad_y: u16) -> Option<(u16, u16)> {
    if client_w == 0 || client_h == 0 {
        return None;
    }
    let cols = client_w.saturating_sub(2 * u32::from(pad_x)) / m.w.max(1);
    let rows = client_h.saturating_sub(2 * u32::from(pad_y)) / m.h.max(1);
    Some((cols.clamp(2, u32::from(u16::MAX)) as u16, rows.clamp(1, u32::from(u16::MAX)) as u16))
}

/// Whole wheel notches in `acc + delta`; the remainder carries over (precision touchpads send small deltas).
pub fn wheel_notches(acc: &mut i32, delta: i16) -> i32 {
    *acc += i32::from(delta);
    let n = *acc / 120;
    *acc -= n * 120;
    n
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

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Not exported by the windows crate.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Tab badge for a working agent, one frame per `SPIN_MS`.
const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
const SPIN_MS: u128 = 150;
const WM_NCMOUSELEAVE: u32 = 0x02A2;

pub(crate) struct App {
    pub(crate) win: Window,
    pub(crate) gpu: Gpu,
    pub(crate) sc: SwapChain,
    pub(crate) fonts: FontStack,
    pub(crate) atlas: GlyphAtlas,
    pipe: Rc<Pipeline>,
    /// Row caches of the panes drawn so far; a pane's entry appears with its first frame.
    grids: HashMap<PaneId, GridRenderer>,
    /// Inline image textures, per pane.
    images: ImageStore,
    link: ServerLink,
    pub(crate) view: WindowView,
    spawn_ctx: SpawnCtx,
    pipe_name: String,
    /// Set by the wake thread when the server's pipe closes (spec §8).
    server_lost: Arc<AtomicBool>,
    server_stopped: bool,
    pub(crate) events: Receiver<ServerMsg>,
    forward_tx: crossbeam_channel::Sender<ServerMsg>,
    pub(crate) woken: Arc<AtomicBool>,
    pub(crate) config: Config,
    pub(crate) font_size: f32,
    keys: KeyTranslator,
    ime: Ime,
    pub(crate) matcher: KeybindMatcher,
    pub(crate) rs: crate::reload::State,
    focused: bool,
    dirty: bool,
    /// Newest frame applied (debug timing only).
    last_seq: Option<u64>,
    size_px: (u32, u32),
    blink_on: bool,
    next_blink: Instant,
    /// Working-agent spinner: animation clock and next frame.
    started: Instant,
    next_spin: Instant,
    /// The pane a selection drag started in.
    mouse_down: Option<PaneId>,
    wheel_acc: i32,
    /// The title last given to the window.
    title: String,
    /// A present acquired from the swap chain's frame-latency object but not yet used.
    present_token: bool,
    /// Bottom-right notice (config diagnostics and similar) and the top-right frame-time overlay.
    pub(crate) toast: Option<String>,
    pub(crate) stats: Option<String>,
    pub(crate) quit: bool,
    /// The caption button under the pointer, and the one a press started on.
    caption_hover: Option<Caption>,
    caption_down: Option<Caption>,
    /// Toasts, taskbar flash and badge for agents that need the user (spec §5.4).
    notifier: crate::notify::Notifier,
}

impl App {
    fn pad_px(&self) -> (i32, i32) {
        let s = self.win.dpi() as f32 / 96.0;
        ((f32::from(self.config.window_padding_x) * s).round() as i32, (f32::from(self.config.window_padding_y) * s).round() as i32)
    }

    fn geometry(&self) -> Geometry {
        let m = self.fonts.metrics();
        Geometry {
            client: (self.size_px.0 as i32, self.size_px.1 as i32),
            cell: (m.w, m.h),
            pad: self.pad_px(),
            bar_h: m.h as i32 + 10,
            sidebar_cols: self.view.geometry().sidebar_cols,
            caption_w: titlebar::caption_width(self.win.dpi()),
        }
    }

    fn caption_rects(&self) -> [(Caption, Rect); 3] {
        titlebar::caption_rects(self.size_px.0 as i32, self.geometry().bar_h, self.win.dpi())
    }

    /// The caption button for a `WM_NCHITTEST` code.
    fn caption_of(ht: usize) -> Option<Caption> {
        match ht as isize {
            titlebar::HTMINBUTTON => Some(Caption::Minimize),
            titlebar::HTMAXBUTTON => Some(Caption::Maximize),
            titlebar::HTCLOSE => Some(Caption::Close),
            _ => None,
        }
    }

    fn nc_hit_test(&self, lparam: LPARAM) -> isize {
        let mut pt = POINT { x: i32::from(lparam.0 as i16), y: i32::from((lparam.0 >> 16) as i16) };
        unsafe {
            let _ = ScreenToClient(self.win.hwnd(), &mut pt);
        }
        let g = NcGeom {
            client: (self.size_px.0 as i32, self.size_px.1 as i32),
            border: (8 * self.win.dpi() / 96) as i32,
            caption_h: self.geometry().bar_h,
            maximized: self.win.is_maximized(),
            caps: self.caption_rects(),
        };
        let interactive = !matches!(self.view.hit(pt.x, pt.y), Hit::Bar | Hit::Sidebar | Hit::None);
        titlebar::nc_hit(&g, pt.x, pt.y, interactive)
    }

    fn send_all(&self, msgs: Vec<ClientMsg>) {
        for m in msgs {
            self.link.send(m);
        }
    }

    fn send_pane(&self, pane: PaneId, cmd: PaneCmd) {
        self.link.send(ClientMsg::Cmd { pane, cmd });
    }

    fn send_focused(&self, cmd: PaneCmd) {
        if let Some(m) = self.view.input(cmd) {
            self.link.send(m);
        }
    }

    pub(crate) fn rebuild_fonts(&mut self) {
        match FontStack::new(&self.config.font_family, self.font_size, self.win.dpi(), &self.config.font_feature) {
            Ok(f) => self.fonts = f,
            Err(e) => tracing::warn!("font: {e}"),
        }
        if let Ok(a) = GlyphAtlas::new(&self.gpu, 2048) {
            self.atlas = a;
        }
        self.resize(self.size_px.0, self.size_px.1);
        // Cached rows hold glyph ids from the old fonts; get every row re-sent even if the cell size is unchanged.
        for (p, _) in self.view.visible() {
            self.send_pane(p, PaneCmd::Redraw);
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.size_px = (w, h);
        let msgs = self.view.set_geometry(self.geometry());
        self.send_all(msgs);
        let _ = self.sc.resize(w, h);
        self.dirty = true;
    }

    fn send_key(&mut self, ev: KeyEvent) {
        if self.server_stopped {
            if ev.code == KeyCode::Enter && ev.action != wraith_proto::KeyAction::Release {
                self.reconnect();
            }
            return;
        }
        if let Some(m) = self.view.restart_key(&ev) {
            self.link.send(m);
            return;
        }
        if self.view.focused().is_some_and(|p| self.view.pane_exit(p).is_some()) {
            return; // an exited shell takes no input
        }
        self.send_focused(PaneCmd::Key(ev));
    }

    fn on_key(&mut self, ev: KeyEvent) {
        if let Some(out) = self.view.palette_key(&ev, &self.spawn_ctx).or_else(|| self.view.find_key(&ev)) {
            self.run_out(out);
            self.dirty = true;
            return;
        }
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
            Action::Copy => self.send_focused(PaneCmd::CopySelection),
            Action::Paste => self.paste(),
            Action::IncreaseFontSize(d) => self.set_font_size(self.font_size + d),
            Action::DecreaseFontSize(d) => self.set_font_size(self.font_size - d),
            Action::ResetFontSize => self.set_font_size(self.config.font_size),
            Action::ScrollPageUp => self.scroll(ScrollTo::PageUp),
            Action::ScrollPageDown => self.scroll(ScrollTo::PageDown),
            Action::ScrollToTop => self.scroll(ScrollTo::Top),
            Action::ScrollToBottom => self.scroll(ScrollTo::Bottom),
            Action::ClearScreen => self.send_focused(PaneCmd::Text("\x0c".into())),
            Action::ReloadConfig | Action::ToggleFrameOverlay | Action::Unbind => crate::reload::action(self, a),
            _ => {
                let out = self.view.action(a, &self.spawn_ctx);
                self.run_out(out);
            }
        }
        self.dirty = true;
    }

    /// Carry out what a view action asked for: messages, confirmations, UI.
    fn run_out(&mut self, out: Out) {
        self.send_all(out.msgs);
        if let Some(pane) = out.confirm_close
            && self.confirm("A process is still running in this terminal. End it?")
        {
            let msgs = self.view.close_pane(pane);
            self.send_all(msgs);
        }
        if let Some(ws) = out.confirm_close_workspace
            && self.confirm("Processes are still running in this workspace. Close it?")
        {
            let msgs = self.view.close_workspace(ws);
            self.send_all(msgs);
        }
        match out.ui {
            Some(Ui::NewWindow) => self.new_window(),
            Some(Ui::Palette) => self.view.open_palette(Mode::Palette, &self.spawn_ctx),
            Some(Ui::Picker) => self.view.open_palette(Mode::Picker, &self.spawn_ctx),
            Some(Ui::MoveTab) => self.view.open_palette(Mode::MoveTab, &self.spawn_ctx),
            None => {}
        }
    }

    fn confirm(&self, text: &str) -> bool {
        let text = wide(text);
        unsafe { MessageBoxW(Some(self.win.hwnd()), PCWSTR(text.as_ptr()), windows::core::w!("Wraith"), MB_YESNO | MB_ICONQUESTION) == IDYES }
    }

    /// Another window is another client process: it attaches the most recently detached workspace or makes one.
    fn new_window(&self) {
        if let Ok(exe) = std::env::current_exe()
            && let Err(e) = std::process::Command::new(exe).spawn()
        {
            tracing::warn!("new window: {e}");
        }
    }

    pub(crate) fn set_font_size(&mut self, pt: f32) {
        self.font_size = clamp_font_size(pt);
        self.rebuild_fonts();
    }

    fn scroll(&mut self, s: ScrollTo) {
        self.send_focused(PaneCmd::Scroll(s));
    }

    fn paste(&mut self) {
        if let Some(t) = get_clipboard(self.win.hwnd()) {
            self.send_focused(PaneCmd::Paste(t));
        }
    }

    /// Close one tab, asking first if a shell in it is still running.
    fn close_tab(&mut self, tab: TabId) {
        let running = self.view.snapshot().tab(tab).is_some_and(|t| t.root.leaves().iter().any(|p| self.view.pane_exit(*p).is_none()));
        if !running || self.confirm("A process is still running in this tab. End it?") {
            self.link.send(ClientMsg::Model(wraith_proto::model::ModelCmd::CloseTab(tab)));
        }
    }

    /// Window X: close the window, keep the workspace's shells running in the server for the next launch.
    fn detach_and_close(&mut self) {
        if !self.server_stopped {
            self.link.finish(ClientMsg::DetachWindow);
        }
        unsafe {
            let _ = DestroyWindow(self.win.hwnd());
        }
    }

    fn reconnect(&mut self) {
        match attach_with_restart(self.win.hwnd(), &self.pipe_name) {
            Some(link) => {
                start_forwarder(&link, self.forward_tx.clone(), self.woken.clone(), self.server_lost.clone(), self.win.hwnd());
                link.send(ClientMsg::AttachWindow { workspace: None });
                self.link = link;
                self.view.on_reconnect();
                self.grids.clear();
                self.server_stopped = false;
                self.dirty = true;
            }
            None => self.quit = true,
        }
    }

    fn mouse(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        let mut x = i32::from((lparam.0 & 0xffff) as i16);
        let mut y = i32::from(((lparam.0 >> 16) & 0xffff) as i16);
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        let alt = unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0;
        let pad = self.pad_px();
        let m = self.fonts.metrics();
        if msg == WM_MOUSEWHEEL {
            let n = wheel_notches(&mut self.wheel_acc, ((wparam.0 >> 16) & 0xffff) as i16);
            // Wheel coordinates are screen-relative.
            let mut pt = POINT { x, y };
            unsafe {
                let _ = ScreenToClient(self.win.hwnd(), &mut pt);
            }
            let pane = match self.view.hit(pt.x, pt.y) {
                Hit::Pane(p) => Some(p),
                _ => self.view.focused(),
            };
            let Some((pane, rect)) = pane.and_then(|p| self.view.panes.get(&p).map(|v| (p, v.rect))) else { return };
            let button = if n > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
            for _ in 0..n.unsigned_abs() {
                let ev = MouseEvent { x_px: (pt.x - rect.x - pad.0) as f32, y_px: (pt.y - rect.y - pad.1) as f32, button: Some(button), action: MouseAction::Press, mods: Mods(0) };
                self.send_pane(pane, PaneCmd::Mouse(ev));
            }
            return;
        }
        if self.view.dragging() || self.view.dragging_tab() {
            match msg {
                WM_MOUSEMOVE => {
                    let msgs = if self.view.dragging_tab() { self.view.drag_tab(x) } else { self.view.drag_divider(x, y) };
                    self.send_all(msgs);
                    self.dirty = true;
                }
                WM_LBUTTONUP => {
                    self.view.end_drag();
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                _ => {}
            }
            return;
        }
        if msg == WM_MOUSEMOVE {
            // Ask for WM_MOUSELEAVE so the hover-revealed sidebar can start its grace when the pointer leaves.
            let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.win.hwnd(), dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
            let before = self.view.floating_sidebar();
            self.view.pointer_moved(x, y, Instant::now());
            self.dirty |= before != self.view.floating_sidebar();
        }
        let hit = self.view.hit(x, y);
        if msg == WM_LBUTTONDOWN {
            match hit {
                Hit::Workspace(w) => {
                    let msgs = self.view.workspace_click(w);
                    return self.send_all(msgs);
                }
                Hit::NewWorkspace => return self.action(Action::NewWorkspace),
                Hit::SidebarTab(t) => {
                    let msgs = self.view.tab_click(t);
                    return self.send_all(msgs);
                }
                Hit::Agent(p) => {
                    let msgs = self.view.focus_pane(p);
                    return self.send_all(msgs);
                }
                Hit::Divider(i) => {
                    self.view.start_divider_drag(i);
                    unsafe {
                        SetCapture(self.win.hwnd());
                    }
                    return;
                }
                Hit::Tab(t) => {
                    self.link.send(ClientMsg::Model(wraith_proto::model::ModelCmd::SelectTab(t)));
                    // A press on a tab may become a drag along the bar (reorder).
                    self.view.start_tab_drag(t);
                    unsafe {
                        SetCapture(self.win.hwnd());
                    }
                    return;
                }
                Hit::TabClose(t) => return self.close_tab(t),
                Hit::NewTab => return self.action(Action::NewTab),
                Hit::Pane(p) => {
                    let msgs = self.view.pane_click(p);
                    self.send_all(msgs);
                }
                _ => return,
            }
        }
        let pane = self.mouse_down.or(if let Hit::Pane(p) = hit { Some(p) } else { None });
        let Some(pane) = pane else { return };
        let (copy_on_select, right_paste) = (self.config.copy_on_select, self.config.right_click_action == RightClick::Paste);
        let Some(pv) = self.view.panes.get_mut(&pane) else { return };
        let (ox, oy) = (pv.rect.x + pad.0, pv.rect.y + pad.1);
        if pv.mouse_tracking && !shift {
            let (button, action) = match msg {
                WM_LBUTTONDOWN => (Some(MouseButton::Left), MouseAction::Press),
                WM_LBUTTONUP => (Some(MouseButton::Left), MouseAction::Release),
                WM_RBUTTONDOWN => (Some(MouseButton::Right), MouseAction::Press),
                WM_RBUTTONUP => (Some(MouseButton::Right), MouseAction::Release),
                WM_MBUTTONDOWN => (Some(MouseButton::Middle), MouseAction::Press),
                WM_MBUTTONUP => (Some(MouseButton::Middle), MouseAction::Release),
                _ => (None, MouseAction::Motion),
            };
            let ev = MouseEvent { x_px: (x - ox) as f32, y_px: (y - oy) as f32, button, action, mods: Mods(0) };
            return self.send_pane(pane, PaneCmd::Mouse(ev));
        }
        x = x.max(ox);
        y = y.max(oy);
        let cell = pixel_to_cell(x as f32, y as f32, (ox as f32, oy as f32), &m);
        let cell = (cell.0.min(pv.cells.0.saturating_sub(1)), cell.1.min(pv.cells.1.saturating_sub(1)));
        let mut out = Vec::new();
        match msg {
            WM_LBUTTONDOWN => {
                self.mouse_down = Some(pane);
                out.push(match pv.sel.press(cell, Instant::now(), alt) {
                    SelAction::Start => PaneCmd::SetSelection(None),
                    SelAction::Word { x, y } => PaneCmd::SelectWord { x, y },
                    SelAction::Line { y } => PaneCmd::SelectLine { y },
                });
            }
            WM_MOUSEMOVE if self.mouse_down.is_some() => {
                if let Some(r) = pv.sel.drag(cell) {
                    out.push(PaneCmd::SetSelection(Some(r)));
                }
            }
            WM_LBUTTONUP => {
                self.mouse_down = None;
                if pv.sel.release().is_some() && copy_on_select {
                    out.push(PaneCmd::CopySelection);
                }
            }
            WM_RBUTTONDOWN if right_paste => {
                if let Some(t) = get_clipboard(self.win.hwnd()) {
                    out.push(PaneCmd::Paste(t));
                }
            }
            _ => {}
        }
        for c in out {
            self.send_pane(pane, c);
        }
    }

    fn on_msg(&mut self, hwnd: HWND, msg: WinMsg) -> Option<LRESULT> {
        match msg {
            WinMsg::Resize { w, h } => {
                self.resize(w, h);
                self.render_now(true);
            }
            WinMsg::DpiChanged { suggested: r, .. } => {
                unsafe {
                    let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                let (w, h) = self.win.client_size();
                self.size_px = (w, h);
                self.rebuild_fonts();
            }
            WinMsg::Close => {
                self.detach_and_close();
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
                    if msg == WM_SYSKEYDOWN && vk == VK_SPACE.0 {
                        // Alt+Space: the window menu. Keys skip TranslateMessage, so no WM_SYSCHAR would open it.
                        unsafe { DefWindowProcW(hwnd, WM_SYSCOMMAND, WPARAM(SC_KEYMENU as usize), LPARAM(i32::from(b' ') as isize)) };
                        return Some(LRESULT(0));
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
                    if let Some(p) = self.view.focused()
                        && let Some(c) = self.grids.get(&p).and_then(GridRenderer::cursor_pos)
                        && let Some(v) = self.view.panes.get(&p)
                    {
                        let m = self.fonts.metrics();
                        let pad = self.pad_px();
                        Ime::set_candidate_pos(hwnd, v.rect.x + pad.0 + (u32::from(c.0) * m.w) as i32, v.rect.y + pad.1 + (u32::from(c.1) * m.h) as i32, m.h as i32);
                    }
                }
                WM_IME_COMPOSITION => {
                    if let ImeUpdate::Commit(t) = self.ime.on_composition(hwnd, lparam) {
                        if self.view.palette().is_some() {
                            self.view.palette_text(&t);
                            self.dirty = true;
                        } else if let Some(out) = self.view.find_text(&t) {
                            self.run_out(out);
                            self.dirty = true;
                        } else {
                            self.send_focused(PaneCmd::Text(t));
                        }
                        return Some(LRESULT(0));
                    }
                }
                WM_MOUSELEAVE => {
                    self.view.pointer_left(Instant::now());
                }
                WM_NCHITTEST => return Some(LRESULT(self.nc_hit_test(lparam))),
                WM_NCMOUSEMOVE => {
                    let mut track = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                    unsafe {
                        let _ = TrackMouseEvent(&mut track);
                    }
                    self.set_caption_hover(Self::caption_of(wparam.0));
                }
                WM_NCMOUSELEAVE => {
                    self.set_caption_hover(None);
                    self.caption_down = None;
                }
                WM_NCLBUTTONDOWN if Self::caption_of(wparam.0).is_some() => {
                    // Our buttons, not DefWindowProc's (it would draw classic ones over the bar).
                    self.caption_down = Self::caption_of(wparam.0);
                    self.dirty = true;
                    return Some(LRESULT(0));
                }
                WM_NCLBUTTONUP if Self::caption_of(wparam.0).is_some() => {
                    let cap = Self::caption_of(wparam.0);
                    if self.caption_down.take() == cap {
                        let sc = match cap {
                            Some(Caption::Minimize) => SC_MINIMIZE,
                            Some(Caption::Maximize) if self.win.is_maximized() => SC_RESTORE,
                            Some(Caption::Maximize) => SC_MAXIMIZE,
                            _ => SC_CLOSE,
                        };
                        unsafe {
                            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(sc as usize), LPARAM(0));
                        }
                    }
                    self.dirty = true;
                    return Some(LRESULT(0));
                }
                WM_SETFOCUS | WM_KILLFOCUS => {
                    self.focused = msg == WM_SETFOCUS;
                    self.link.send(ClientMsg::Foreground(self.focused));
                    if self.focused {
                        self.notifier.clear_badge();
                    }
                    self.dirty = true;
                }
                WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
                    self.set_caption_hover(None);
                    self.mouse(msg, wparam, lparam);
                    return Some(LRESULT(0));
                }
                crate::notify::WM_TOAST_CLICKED => {
                    // The user clicked an agent's toast: bring its pane up here.
                    let msgs = self.view.focus_pane(wparam.0 as PaneId);
                    self.send_all(msgs);
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_RESTORE);
                        let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
                    }
                    return Some(LRESULT(0));
                }
                WM_APP => {
                    // Server messages arrived; also delivered inside modal size/move loops.
                    self.render_now(false);
                    return Some(LRESULT(0));
                }
                _ => {}
            },
        }
        None
    }

    fn set_caption_hover(&mut self, c: Option<Caption>) {
        if self.caption_hover != c {
            self.caption_hover = c;
            self.dirty = true;
        }
    }

    fn set_title(&mut self) {
        let snap = self.view.snapshot();
        let Some(tab) = self.view.active_tab().map(|t| t.id) else { return };
        let ws = self.view.workspace().and_then(|w| snap.workspace(w)).map(|w| w.name.clone()).unwrap_or_default();
        let title = format!("{} - {ws}", snap.tab_title(tab));
        if title != self.title {
            let w = wide(&title);
            unsafe {
                let _ = SetWindowTextW(self.win.hwnd(), PCWSTR(w.as_ptr()));
            }
            self.title = title;
        }
    }

    fn on_server_msg(&mut self, m: ServerMsg) {
        match m {
            ServerMsg::WindowAttached { workspace } => {
                self.view.set_workspace(workspace);
                self.grids.clear();
                self.images = ImageStore::default();
            }
            ServerMsg::Snapshot(s) => {
                let msgs = self.view.on_snapshot(s, &self.spawn_ctx);
                self.send_all(msgs);
                let live: std::collections::HashSet<PaneId> = self.view.snapshot().panes.iter().map(|p| p.id).collect();
                self.grids.retain(|p, _| live.contains(p));
                self.images.retain_panes(|p| live.contains(&p));
                self.set_title();
                self.dirty = true;
            }
            ServerMsg::WorkspaceTaken { workspace } => {
                // Another window took ours: show whatever is free (or a new workspace).
                let msgs = self.view.on_workspace_taken(workspace);
                self.send_all(msgs);
                self.grids.clear();
                self.images = ImageStore::default();
                self.dirty = true;
            }
            ServerMsg::Event { pane, ev } => self.on_pane_event(pane, ev),
            ServerMsg::Error(e) => {
                tracing::warn!("server: {e}");
                crate::reload::toast_text(self, "server", e);
            }
            ServerMsg::Notify { pane, title, body } => {
                let visible = self.view.visible().iter().any(|(p, _)| *p == pane);
                let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
                let ways = crate::notify::notify_ways(&self.config.agent_notify, self.focused, visible, u32::from(t.wHour) * 60 + u32::from(t.wMinute));
                tracing::info!(pane, visible, foreground = self.focused, ?ways, "agent notify");
                self.notifier.show(ways, pane, &title, &body);
            }
            ServerMsg::Welcome { .. } | ServerMsg::VersionMismatch { .. } | ServerMsg::Panes(_) => {}
        }
    }

    fn on_pane_event(&mut self, pane: PaneId, ev: PaneEvent) {
        match ev {
            PaneEvent::Frame(f) => {
                let t = Instant::now();
                let pipe = self.pipe.clone();
                self.grids.entry(pane).or_insert_with(|| GridRenderer::with_pipeline(pipe)).apply(&f, &self.fonts);
                tracing::debug!(seq = f.seq, rows = f.dirty_rows.len(), apply_us = t.elapsed().as_micros() as u64, "frame arrived");
                // Ack on apply, not on present: the pane can send the newest state while we wait for
                // vsync, so what we draw is fresh. One frame in flight still bounds the queue.
                self.send_pane(pane, PaneCmd::Ready(f.seq));
                self.last_seq = Some(f.seq);
                self.dirty = true;
            }
            PaneEvent::Title(_) | PaneEvent::Bell | PaneEvent::InputWritten { .. } | PaneEvent::Cwd(_) | PaneEvent::Agent(_) | PaneEvent::AgentProcess(_) => {}
            PaneEvent::Clipboard(t) | PaneEvent::Copied(t) => set_clipboard(self.win.hwnd(), &t),
            PaneEvent::PasteNeedsConfirm(t) => {
                let lines = t.lines().count().max(1);
                let q = wide(&format!("Paste {lines} line(s) into the terminal?\n\n{}", t.chars().take(400).collect::<String>()));
                let r = unsafe { MessageBoxW(Some(self.win.hwnd()), PCWSTR(q.as_ptr()), windows::core::w!("Wraith: paste"), MB_YESNO | MB_ICONQUESTION) };
                if r == IDYES {
                    self.send_pane(pane, PaneCmd::PasteConfirmed(t));
                }
            }
            PaneEvent::Exited(code) => {
                match self.view.on_exit(pane, code, self.config.close_on_exit) {
                    ExitOut::Keep => {}
                    ExitOut::ClosePane(msgs) => self.send_all(msgs),
                    ExitOut::CloseWindow(msgs) => {
                        self.send_all(msgs);
                        self.detach_and_close();
                    }
                }
                self.dirty = true;
            }
            PaneEvent::MouseTracking(on) => {
                if let Some(v) = self.view.panes.get_mut(&pane) {
                    v.mouse_tracking = on;
                }
            }
            PaneEvent::ImageUpload { image_id, width, height, rgba } => {
                if let Err(e) = self.images.upload(&self.gpu, pane, image_id, width, height, &rgba) {
                    tracing::warn!(pane, image_id, "image upload: {e}");
                }
                self.dirty = true;
            }
            PaneEvent::ImageDelete { image_id } => {
                self.images.delete(pane, image_id);
                self.dirty = true;
            }
        }
    }

    fn drain_server_msgs(&mut self) {
        self.woken.store(false, Ordering::Release);
        while let Ok(m) = self.events.try_recv() {
            self.on_server_msg(m);
        }
        if self.server_lost.swap(false, Ordering::AcqRel) {
            self.server_stopped = true;
            self.dirty = true;
        }
    }

    /// Frame work from inside a window message. While the user drags the window edge, Windows runs
    /// its own modal sizing loop and our idle never runs, so resize and wake messages render here.
    /// `blocking` waits for the compositor; only resize uses it (the sizing loop never reaches our
    /// idle, so a skipped frame there would not be retried).
    fn render_now(&mut self, blocking: bool) {
        self.drain_server_msgs();
        self.try_render(blocking);
    }

    /// Draw if something changed and the compositor can take a frame. Never blocks unless asked:
    /// a blocked UI thread cannot take in newer frames, so what it finally draws is stale.
    fn try_render(&mut self, blocking: bool) {
        if !self.dirty {
            return;
        }
        if !self.present_token {
            self.present_token = if blocking {
                self.sc.wait();
                true
            } else {
                self.sc.try_acquire()
            };
        }
        if self.present_token {
            self.present_token = false;
            self.render();
        }
    }

    /// Drain server messages, render if possible, and say how the loop should wait.
    fn idle(&mut self, signaled: bool) -> Idle {
        if self.quit {
            return Idle { ctl: LoopControl::Exit, sleep_ms: 0, wait_on: None };
        }
        // The loop's wait on the frame-latency object acquired a present for us.
        self.present_token |= signaled;
        self.drain_server_msgs();
        for k in self.matcher.tick(Instant::now()) {
            self.send_key(k);
        }
        let now = Instant::now();
        if self.config.cursor_style_blink && now >= self.next_blink {
            self.blink_on = !self.blink_on;
            self.next_blink = now + Duration::from_millis(600);
            self.dirty = true;
        }
        if self.view.tick(now) {
            self.dirty = true;
        }
        let spinning = self.view.tab_layout().0.iter().any(|(t, _)| self.view.tab_agent(*t).is_some_and(|a| a.state == AgentState::Working));
        if spinning && now >= self.next_spin {
            self.next_spin = now + Duration::from_millis(SPIN_MS as u64);
            self.dirty = true;
        }
        crate::reload::poll(self);
        self.try_render(false);
        let until_blink = self.next_blink.saturating_duration_since(Instant::now()).as_millis() as u32;
        let mut sleep = if self.config.cursor_style_blink { until_blink.max(1) } else { 1000 };
        if let Some(t) = crate::reload::next_deadline(self).into_iter().chain(self.view.next_deadline()).min() {
            sleep = sleep.min(t.saturating_duration_since(Instant::now()).as_millis() as u32 + 1);
        }
        if spinning {
            sleep = sleep.min(SPIN_MS as u32);
        }
        // A pending key sequence must time out even with no other activity.
        let sleep_ms = if self.matcher.since.is_some() { sleep.min(100) } else { sleep };
        // Still dirty: wake as soon as the compositor can take the frame.
        let wait_on = (self.dirty && !self.present_token).then(|| self.sc.waitable());
        Idle { ctl: LoopControl::Continue, sleep_ms, wait_on }
    }

    pub(crate) fn request_redraw(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn rebuild_spawn_ctx(&mut self) {
        self.spawn_ctx = build_spawn_ctx(&self.config);
        self.view.set_dim_opacity(self.config.unfocused_split_opacity);
    }

    /// Send the configured colors to every pane in view (after a config reload).
    pub(crate) fn resend_colors(&mut self) {
        let c = &self.config;
        let rgb = |p: wraith_config::Rgb| (p.0, p.1, p.2);
        for (p, _) in self.view.visible() {
            self.send_pane(p, PaneCmd::SetColors { fg: rgb(c.foreground), bg: rgb(c.background), palette: c.palette.iter().copied().map(rgb).collect() });
        }
    }

    fn render(&mut self) {
        self.dirty = false;
        crate::reload::render_start(self);
        let t0 = Instant::now();
        let m = self.fonts.metrics();
        let c = &self.config;
        let rgb = |p: wraith_config::Rgb| (p.0, p.1, p.2);
        let (bg, fg) = (rgb(c.background), rgb(c.foreground));
        let accent = c.selection_background.map_or((0x61, 0xaf, 0xef), rgb);
        let col = chrome_colors(bg, fg, accent);
        let sel = c.selection_background.map_or((0x3e, 0x44, 0x51), rgb);
        let opacity = c.background_opacity;
        let blink = self.blink_on || !c.cursor_style_blink;
        let pad = self.pad_px();
        let regions = self.view.regions();
        let focused_pane = self.view.focused();
        let panes = self.view.visible();
        let split = panes.len() > 1;
        let mut q: Vec<Quad> = Vec::new();
        // Untextured quads are batched in `q`; image placements split it into more batches.
        let mut batches: Vec<Batch> = Vec::new();
        // Pane grids first, then chrome over and around them.
        for &(id, rect) in &panes {
            let exit = self.view.pane_exit(id).map(|code| format!("[process exited {code}] Enter: restart · ctrl+shift+w: close"));
            let opts = DrawOpts { selection: None, selection_color: sel, cursor_blink_on: blink, focused: self.focused && focused_pane == Some(id), opacity };
            match self.grids.get_mut(&id) {
                Some(g) => {
                    g.set_overlay(Overlay::Exit, exit.as_deref(), &self.fonts);
                    let origin = ((rect.x + pad.0) as f32, (rect.y + pad.1) as f32);
                    g.push_layers(&mut batches, &mut q, &self.images, id, &self.gpu, &mut self.atlas, &self.fonts, origin, &m, &opts, rect);
                }
                None => Painter { q: &mut q }.rect(rect, bg, opacity),
            }
            let dim = self.view.dim_alpha(id);
            if dim > 0.0 {
                Painter { q: &mut q }.rect(rect, bg, dim);
            }
        }
        let mut p = Painter { q: &mut q };
        if split {
            for d in self.view.dividers() {
                p.rect(d.line, col.divider, 1.0);
            }
            if let Some((_, r)) = panes.iter().find(|(id, _)| Some(*id) == focused_pane) {
                for edge in [
                    Rect { x: r.x, y: r.y, w: r.w, h: 1 },
                    Rect { x: r.x, y: r.y + r.h - 1, w: r.w, h: 1 },
                    Rect { x: r.x, y: r.y, w: 1, h: r.h },
                    Rect { x: r.x + r.w - 1, y: r.y, w: 1, h: r.h },
                ] {
                    p.rect(edge, accent, 1.0);
                }
            }
        }
        // Tab bar.
        p.rect(regions.tabbar, col.bar_bg, 1.0);
        let spin_frame = self.started.elapsed().as_millis() / SPIN_MS;
        let (tabs, plus) = self.view.tab_layout();
        let active = self.view.active_tab().map(|t| t.id);
        let pad_x = m.w as i32 / 2;
        for (id, r) in &tabs {
            let is_active = Some(*id) == active;
            if is_active {
                p.rect(*r, col.tab_active_bg, 1.0);
                p.rect(Rect { x: r.x, y: r.y + r.h - 2, w: r.w, h: 2 }, accent, 1.0);
            }
            // An agent's icon leads the title; its state badge sits before the close button.
            let agent = self.view.tab_agent(*id).map(|a| (a.icon, a.state, a.attention));
            let badge = agent.and_then(|(_, state, attention)| match state {
                AgentState::Working => Some(SPINNER[(spin_frame % SPINNER.len() as u128) as usize]),
                AgentState::Asking | AgentState::Done if attention => Some("●"),
                _ => None,
            });
            let cells = (((r.w - 2 * pad_x - 2 * m.w as i32) / m.w as i32).max(0) as usize).saturating_sub(usize::from(badge.is_some()) * 2);
            let title = self.view.snapshot().tab_title(*id);
            let title = truncate_cells(&agent.map_or(title.clone(), |(icon, ..)| format!("{icon} {title}")), cells);
            let ty = r.y + (r.h - m.h as i32) / 2;
            let tc = if is_active { col.bar_fg } else { col.bar_fg_dim };
            p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + pad_x, ty, &title, tc, 1.0);
            let x = chrome::close_rect(*r, m.w as i32);
            if let Some(b) = badge {
                p.text(&self.gpu, &mut self.atlas, &self.fonts, x.x - m.w as i32, ty, b, accent, 1.0);
            }
            p.text(&self.gpu, &mut self.atlas, &self.fonts, x.x + (x.w - m.w as i32) / 2, ty, "×", col.bar_fg_dim, 1.0);
        }
        let ty = plus.y + (plus.h - m.h as i32) / 2;
        p.text(&self.gpu, &mut self.atlas, &self.fonts, plus.x + (plus.w - m.w as i32) / 2, ty, "+", col.bar_fg_dim, 1.0);
        // Caption buttons, drawn like Windows 11's: a light wash on hover, red behind close.
        let glyph = (10 * self.win.dpi() / 96) as i32;
        let maximized = self.win.is_maximized();
        for (cap, r) in self.caption_rects() {
            let lit = self.caption_hover == Some(cap) || self.caption_down == Some(cap);
            if lit {
                let wash = if cap == Caption::Close { (0xc4, 0x2b, 0x1c) } else { chrome::mix(col.bar_bg, col.bar_fg, 0.12) };
                p.rect(r, wash, 1.0);
            }
            let fg = if lit && cap == Caption::Close { (255, 255, 255) } else { col.bar_fg };
            let (gx, gy) = (r.x + (r.w - glyph) / 2, r.y + (r.h - glyph) / 2);
            let outline = |p: &mut Painter, x: i32, y: i32| {
                for e in [Rect { x, y, w: glyph, h: 1 }, Rect { x, y: y + glyph - 1, w: glyph, h: 1 }, Rect { x, y, w: 1, h: glyph }, Rect { x: x + glyph - 1, y, w: 1, h: glyph }] {
                    p.rect(e, fg, 1.0);
                }
            };
            match cap {
                Caption::Minimize => p.rect(Rect { x: gx, y: gy + glyph / 2, w: glyph, h: 1 }, fg, 1.0),
                Caption::Maximize if maximized => {
                    let o = (2 * self.win.dpi() / 96) as i32;
                    p.rect(Rect { x: gx + o, y: gy - o, w: glyph, h: 1 }, fg, 1.0);
                    p.rect(Rect { x: gx + o + glyph - 1, y: gy - o, w: 1, h: glyph }, fg, 1.0);
                    outline(&mut p, gx, gy);
                }
                Caption::Maximize => outline(&mut p, gx, gy),
                Caption::Close => {
                    p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + (r.w - m.w as i32) / 2, r.y + (r.h - m.h as i32) / 2, "✕", fg, 1.0);
                }
            }
        }
        // Sidebar: docked, or floating over the panes while the pointer rests on the left edge.
        if let Some(s) = self.view.sidebar_rect() {
            p.rect(s, col.bar_bg, 1.0);
            if self.view.floating_sidebar().is_some() {
                p.rect(Rect { x: s.x + s.w - 1, y: s.y, w: 1, h: s.h }, col.divider, 1.0);
            }
            p.text(&self.gpu, &mut self.atlas, &self.fonts, s.x + pad_x, 6, "WORKSPACES", col.bar_fg_dim, 1.0);
            let (rects, new_row) = self.view.workspace_rects();
            let rows = self.view.sidebar_rows();
            let chip = m.h as i32 / 2;
            for ((_, r), row) in rects.iter().zip(&rows) {
                if row.current {
                    p.rect(*r, col.tab_active_bg, 1.0);
                    p.rect(Rect { x: r.x, y: r.y, w: 2, h: r.h }, accent, 1.0);
                }
                let c = ((row.color >> 16) as u8, (row.color >> 8) as u8, row.color as u8);
                p.rect(Rect { x: r.x + pad_x, y: r.y + (r.h - chip) / 2, w: chip, h: chip }, c, 1.0);
                let tx = r.x + pad_x + chip + pad_x;
                let cells = ((r.w - (tx - r.x) - pad_x) / m.w as i32).max(0) as usize;
                let tc = if row.current { col.bar_fg } else { col.bar_fg_dim };
                let name = truncate_cells(&row.name, cells);
                p.text(&self.gpu, &mut self.atlas, &self.fonts, tx, r.y + (r.h - m.h as i32) / 2, &name, tc, if row.elsewhere { 0.6 } else { 1.0 });
                if row.attention {
                    p.rect(Rect { x: r.x + r.w - pad_x - chip, y: r.y + (r.h - chip) / 2, w: chip, h: chip }, accent, 1.0);
                }
            }
            // Each workspace's tabs, indented under it; the tab this window shows is highlighted.
            let shown = self.view.active_tab().map(|t| t.id);
            for (t, r) in self.view.sidebar_tab_rects() {
                let snap = self.view.snapshot();
                let agent = self.view.tab_agent(t).map(|a| (a.icon, a.attention));
                let current = Some(t) == shown;
                if current {
                    p.rect(r, col.tab_active_bg, 1.0);
                }
                let title = snap.tab_title(t);
                let label = agent.map_or(title.clone(), |(icon, _)| format!("{icon} {title}"));
                let cells = ((r.w - 2 * pad_x - chip - pad_x) / m.w as i32).max(0) as usize;
                let tc = if current { col.bar_fg } else { col.bar_fg_dim };
                p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + pad_x, r.y + (r.h - m.h as i32) / 2, &truncate_cells(&label, cells), tc, 1.0);
                if agent.is_some_and(|(_, attention)| attention) {
                    p.rect(Rect { x: r.x + r.w - pad_x - chip, y: r.y + (r.h - chip) / 2, w: chip, h: chip }, accent, 1.0);
                }
            }
            p.text(&self.gpu, &mut self.atlas, &self.fonts, new_row.x + pad_x, new_row.y + (new_row.h - m.h as i32) / 2, "+ New workspace", col.bar_fg_dim, 1.0);
            // Agents: waiting first (oldest first), then working, then idle (spec §4.2).
            let agents = self.view.agent_rects();
            if !agents.is_empty() {
                let hy = agents[0].1.y - new_row.h + (new_row.h - m.h as i32) / 2;
                p.text(&self.gpu, &mut self.atlas, &self.fonts, s.x + pad_x, hy, "AGENTS", col.bar_fg_dim, 1.0);
            }
            let rows = self.view.agent_rows();
            let snap = self.view.snapshot();
            for ((_, r), row) in agents.iter().zip(&rows) {
                let a = &row.agent;
                let (state, color) = match a.state {
                    AgentState::Asking => ("needs input", col.bar_fg),
                    AgentState::Done => ("done", col.bar_fg),
                    AgentState::Working => ("working", col.bar_fg_dim),
                    AgentState::Idle => ("idle", col.bar_fg_dim),
                };
                let ws = snap.workspace(row.workspace).map_or("", |w| w.name.as_str());
                let cells = ((r.w - 2 * pad_x - chip - pad_x) / m.w as i32).max(0) as usize;
                let text = truncate_cells(&format!("{} {} · {state} · {ws}", a.icon, a.name), cells);
                p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + pad_x, r.y + (r.h - m.h as i32) / 2, &text, color, 1.0);
                if a.attention {
                    p.rect(Rect { x: r.x + r.w - pad_x - chip, y: r.y + (r.h - chip) / 2, w: chip, h: chip }, accent, 1.0);
                }
            }
        }
        // Find bar: top right of the pane it searches, with the match count.
        if let Some(f) = self.view.find()
            && let Some((_, r)) = panes.iter().find(|(id, _)| *id == f.pane)
        {
            let count = self.grids.get(&f.pane).and_then(GridRenderer::search).map_or(String::new(), |s| match s.selected {
                Some(i) => format!("{}/{}", i + 1, s.total),
                None => format!("{}", s.total),
            });
            let (cw, row_h) = (m.w as i32, m.h as i32 + 8);
            let w = (34 * cw).min(r.w - 2 * cw).max(12 * cw);
            let bar = Rect { x: r.x + r.w - w - cw, y: r.y + 4, w, h: row_h };
            p.rect(Rect { x: bar.x - 1, y: bar.y - 1, w: bar.w + 2, h: bar.h + 2 }, accent, 1.0);
            p.rect(bar, col.bar_bg, 1.0);
            let ty = bar.y + (bar.h - m.h as i32) / 2;
            let count_w = crate::paint::text_width(&self.fonts, &count);
            let room = ((bar.w - count_w - 3 * pad_x) / cw).max(0) as usize;
            p.text(&self.gpu, &mut self.atlas, &self.fonts, bar.x + pad_x, ty, &truncate_cells(&format!("Find: {}▏", f.query), room), col.bar_fg, 1.0);
            p.text(&self.gpu, &mut self.atlas, &self.fonts, bar.x + bar.w - pad_x - count_w, ty, &count, col.bar_fg_dim, 1.0);
        }
        // Command palette / picker, centered over everything.
        if let Some(pal) = self.view.palette() {
            let (cw, row_h) = (m.w as i32, m.h as i32 + 6);
            let items = pal.visible();
            let shown = items.len().min(12);
            let w = (60 * cw).min(self.size_px.0 as i32 - 2 * cw).max(10 * cw);
            let panel = Rect { x: (self.size_px.0 as i32 - w) / 2, y: regions.tabbar.h + 12, w, h: (shown as i32 + 1) * row_h + 10 };
            p.rect(Rect { x: panel.x - 1, y: panel.y - 1, w: panel.w + 2, h: panel.h + 2 }, accent, 1.0);
            p.rect(panel, col.bar_bg, 1.0);
            let title = match pal.mode {
                Mode::Palette => "› ",
                Mode::Picker => "Workspace › ",
                Mode::MoveTab => "Move tab to › ",
            };
            p.text(&self.gpu, &mut self.atlas, &self.fonts, panel.x + pad_x, panel.y + 5, &format!("{title}{}▏", pal.query), col.bar_fg, 1.0);
            let first = pal.selected.saturating_sub(shown.saturating_sub(1));
            for (n, item) in items.iter().skip(first).take(shown).enumerate() {
                let r = Rect { x: panel.x, y: panel.y + 5 + (n as i32 + 1) * row_h, w: panel.w, h: row_h };
                if first + n == pal.selected {
                    p.rect(r, col.tab_active_bg, 1.0);
                    p.rect(Rect { x: r.x, y: r.y, w: 2, h: r.h }, accent, 1.0);
                }
                let ty = r.y + (r.h - m.h as i32) / 2;
                let group = format!("{} › ", item.group);
                let gw = p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + pad_x, ty, &group, col.bar_fg_dim, 1.0);
                let detail_w = crate::paint::text_width(&self.fonts, &item.detail);
                let room = ((r.w - gw - detail_w - 3 * pad_x) / cw).max(0) as usize;
                p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + pad_x + gw, ty, &truncate_cells(&item.label, room), col.bar_fg, 1.0);
                p.text(&self.gpu, &mut self.atlas, &self.fonts, r.x + r.w - pad_x - detail_w, ty, &item.detail, col.bar_fg_dim, 1.0);
            }
        }
        // Notices.
        let notice = |p: &mut Painter, gpu: &Gpu, atlas: &mut GlyphAtlas, fonts: &FontStack, text: &str, at_bottom: bool, left: bool| {
            let w = crate::paint::text_width(fonts, text);
            let c = regions.content;
            let x = if left { c.x } else { (c.x + c.w - w).max(c.x) };
            let y = if at_bottom { c.y + c.h - m.h as i32 } else { c.y };
            p.rect(Rect { x, y, w, h: m.h as i32 }, fg, 1.0);
            p.text(gpu, atlas, fonts, x, y, text, bg, 1.0);
        };
        if self.server_stopped {
            notice(&mut p, &self.gpu, &mut self.atlas, &self.fonts, "[server stopped] Enter: reconnect", true, true);
        }
        if let Some(t) = &self.toast {
            notice(&mut p, &self.gpu, &mut self.atlas, &self.fonts, t, true, false);
        }
        if let Some(t) = &self.stats {
            notice(&mut p, &self.gpu, &mut self.atlas, &self.fonts, t, false, false);
        }
        batches.push(Batch { quads: q, texture: None });
        self.pipe.submit(&self.gpu, self.sc.rtv(), self.size_px, &self.atlas, &batches, [0.0; 4]);
        let _ = self.sc.present();
        tracing::debug!(seq = ?self.last_seq, total_us = t0.elapsed().as_micros() as u64, "presented");
        crate::reload::presented(self);
    }
}

/// `s` cut to `cells` terminal cells, ending in `…` when cut.
fn truncate_cells(s: &str, cells: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if s.width() <= cells {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if (out.as_str().width() + 1 + unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0)) > cells {
            break;
        }
        out.push(ch);
    }
    if cells > 0 {
        out.push('…');
    }
    out
}

pub fn clamp_font_size(pt: f32) -> f32 {
    pt.clamp(4.0, 72.0)
}

/// `wraith.exe -e <command...>` (Ghostty's flag): run that instead of the configured shell.
pub fn command_from_args(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "-e")?;
    let rest = &args[i + 1..];
    if rest.is_empty() {
        return None;
    }
    let quote = |a: &String| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() };
    Some(rest.iter().map(quote).collect::<Vec<_>>().join(" "))
}

/// Forward the server's messages to the UI thread and wake its message loop (coalesced). When the
/// server's pipe closes, flag `server_lost` and wake the loop once more.
fn start_forwarder(
    link: &ServerLink,
    tx: crossbeam_channel::Sender<ServerMsg>,
    woken: Arc<AtomicBool>,
    server_lost: Arc<AtomicBool>,
    hwnd: HWND,
) {
    let msgs = link.msgs.clone();
    let hwnd = hwnd.0 as usize;
    let wake = move || unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP, WPARAM(0), LPARAM(0));
    };
    let _ = std::thread::Builder::new().name("wraith-wake".into()).spawn(move || {
        while let Ok(m) = msgs.recv() {
            if tx.send(m).is_err() {
                return;
            }
            if !woken.swap(true, Ordering::AcqRel) {
                wake();
            }
        }
        server_lost.store(true, Ordering::Release);
        wake();
    });
}

/// Connect to the server, starting it if needed. On a protocol mismatch, offer to restart it.
/// `None` means the user gave up (or was told why it can't work); the app should exit.
fn attach_with_restart(hwnd: HWND, pipe: &str) -> Option<ServerLink> {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wraith.exe"));
    let ask = |text: &str, flags| unsafe {
        let w = wide(text);
        MessageBoxW(Some(hwnd), PCWSTR(w.as_ptr()), windows::core::w!("Wraith"), flags)
    };
    match ServerLink::connect(pipe, &exe) {
        Ok(l) => Some(l),
        Err(e @ AttachError::VersionMismatch { .. }) => {
            let q = format!("{e}\n\nRestart the server? Shells running in it will close.");
            if ask(&q, MB_YESNO | MB_ICONQUESTION) != IDYES {
                return None;
            }
            if let Err(err) = kill_server(pipe) {
                ask(&format!("Could not stop the old server: {err}"), MB_OK | MB_ICONERROR);
                return None;
            }
            match ServerLink::connect(pipe, &exe) {
                Ok(l) => Some(l),
                Err(e) => {
                    ask(&e.to_string(), MB_OK | MB_ICONERROR);
                    None
                }
            }
        }
        Err(e) => {
            tracing::error!("attach: {e}");
            ask(&e.to_string(), MB_OK | MB_ICONERROR);
            None
        }
    }
}

/// Detected profiles plus the configured ones; `-e <command>` or `command =` becomes the default.
fn build_spawn_ctx(config: &Config) -> SpawnCtx {
    let mut profiles = wraith_server::profile::detect();
    for (name, cmd) in &config.profiles {
        profiles.retain(|p| &p.name != name);
        profiles.push(wraith_server::profile::from_config(name, cmd));
    }
    let mut config = config.clone();
    if let Some(cmd) = command_from_args(&std::env::args().collect::<Vec<_>>()).or_else(|| config.command.clone()) {
        profiles.retain(|p| p.name != "command");
        profiles.push(wraith_server::profile::from_config("command", &cmd));
        config.default_profile = Some("command".into());
    }
    let scripts = wraith_server::shell::ensure_scripts().unwrap_or_else(|e| {
        tracing::warn!("shell integration scripts: {e}");
        PathBuf::new()
    });
    SpawnCtx { config, profiles, scripts, env: spawn_env() }
}

pub fn run() -> Result<()> {
    let path = wraith_config::default_path();
    let (config, diags) = wraith_config::load(&path, &themes_dir());
    // COM for the taskbar and toasts; the app id groups windows and names toasts. Both before the window.
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    crate::notify::set_app_id();
    let win = Window::create("Wraith", 1000, 640, config.background_opacity < 1.0)?;
    let notifier = crate::notify::Notifier::new(win.hwnd());
    let gpu = Gpu::new()?;
    let fonts = FontStack::new(&config.font_family, config.font_size, win.dpi(), &config.font_feature)?;
    let atlas = GlyphAtlas::new(&gpu, 2048)?;
    let pipe = Rc::new(Pipeline::new(&gpu)?);
    let (w, h) = win.client_size();
    let sc = gpu.create_swapchain(win.hwnd(), w, h)?;
    let pipe_name = wraith_ipc::pipe_name();
    let Some(link) = attach_with_restart(win.hwnd(), &pipe_name) else { return Ok(()) };

    let (tx, rx) = crossbeam_channel::unbounded();
    let woken = Arc::new(AtomicBool::new(false));
    let server_lost = Arc::new(AtomicBool::new(false));
    start_forwarder(&link, tx.clone(), woken.clone(), server_lost.clone(), win.hwnd());
    link.send(ClientMsg::AttachWindow { workspace: None });

    let double_click = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
    let font_size = config.font_size;
    let matcher = KeybindMatcher::new(&config.keybinds);
    let spawn_ctx = build_spawn_ctx(&config);
    let m = fonts.metrics();
    let s = win.dpi() as f32 / 96.0;
    let geom = Geometry {
        client: (w as i32, h as i32),
        cell: (m.w, m.h),
        pad: ((f32::from(config.window_padding_x) * s).round() as i32, (f32::from(config.window_padding_y) * s).round() as i32),
        bar_h: m.h as i32 + 10,
        sidebar_cols: None,
        caption_w: titlebar::caption_width(win.dpi()),
    };
    let mut view = WindowView::new(geom, double_click);
    view.set_dim_opacity(config.unfocused_split_opacity);
    view.set_sidebar(config.sidebar_visible, config.sidebar_width);
    let app = Rc::new(RefCell::new(App {
        win,
        gpu,
        sc,
        fonts,
        atlas,
        pipe,
        grids: HashMap::new(),
        images: ImageStore::default(),
        link,
        view,
        spawn_ctx,
        pipe_name,
        server_lost,
        server_stopped: false,
        forward_tx: tx,
        events: rx,
        woken,
        config,
        font_size,
        keys: KeyTranslator::default(),
        ime: Ime,
        matcher,
        rs: crate::reload::State::default(),
        // Set by WM_SETFOCUS: a window started minimized or behind others never had the focus.
        focused: false,
        dirty: true,
        last_seq: None,
        size_px: (w, h),
        blink_on: true,
        next_blink: Instant::now() + Duration::from_millis(600),
        started: Instant::now(),
        next_spin: Instant::now(),
        mouse_down: None,
        wheel_acc: 0,
        title: String::new(),
        present_token: false,
        toast: None,
        stats: None,
        quit: false,
        caption_hover: None,
        caption_down: None,
        notifier,
    }));
    crate::reload::start(&mut app.borrow_mut(), &path, diags);
    app.borrow().win.show();
    let a = app.clone();
    run_message_loop(
        move |hwnd, msg| a.try_borrow_mut().ok().and_then(|mut a| a.on_msg(hwnd, msg)),
        move |signaled| app.borrow_mut().idle(signaled),
    );
    Ok(())
}

/// The environment for a new shell: this window's own (the long-lived server's would be stale,
/// e.g. missing PATH entries added since it started), plus TERM_PROGRAM.
pub fn spawn_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().filter(|(k, _)| k != "TERM_PROGRAM").collect();
    env.push(("TERM_PROGRAM".into(), "wraith".into()));
    env
}
