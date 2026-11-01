//! One pane: ConPTY + libghostty Terminal on a dedicated thread (libghostty types are !Send).
//! Input is always written before more output is parsed (spec §6.3).
use std::cell::RefCell;
use std::io::{self, Read};
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, bounded, select, unbounded};
use libghostty_vt::key;
use libghostty_vt::mouse;
use libghostty_vt::paste;
use libghostty_vt::render::RenderState;
use libghostty_vt::screen::Screen;
use libghostty_vt::selection::{FormatOptions, SelectLineOptions, SelectWordOptions, Selection};
use libghostty_vt::style::{Palette, RgbColor};
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode, Point, PointCoordinate,
    PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes, Terminal,
};
use libghostty_vt::TerminalOptions;
pub use wraith_proto::pane::{PaneCmd, PaneEvent, ScrollTo};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, SelRange};

use crate::frame_build::build_frame;
use crate::keymap::{self, to_ghostty_key};
use crate::pty::{Pty, clamp_size};

pub struct PaneOptions {
    pub cmdline: String,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
    pub cell_w_px: u32,
    pub cell_h_px: u32,
    pub scrollback_lines: usize,
    pub default_fg: (u8, u8, u8),
    pub default_bg: (u8, u8, u8),
    pub palette: Vec<(u8, u8, u8)>,
}

pub struct PaneHandle {
    cmd: Sender<(Instant, PaneCmd)>,
    pub events: Receiver<PaneEvent>,
}

impl PaneHandle {
    /// Queue a command; the send time is recorded for input-latency reporting.
    pub fn send(&self, cmd: PaneCmd) -> Result<(), PaneCmd> {
        self.cmd.send((Instant::now(), cmd)).map_err(|e| e.into_inner().1)
    }
}

impl Drop for PaneHandle {
    fn drop(&mut self) {
        let _ = self.cmd.send((Instant::now(), PaneCmd::Shutdown));
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WheelAction {
    /// Scroll the viewport by this many rows (negative = up).
    Scroll(isize),
    /// Alternate screen without mouse tracking: send cursor keys.
    Arrows { up: bool, count: u8 },
    /// The program tracks the mouse: encode a wheel report.
    Report,
}

pub fn wheel_action(alt_screen: bool, mouse_tracking: bool, up: bool) -> WheelAction {
    const NOTCH: u8 = 3;
    if mouse_tracking {
        WheelAction::Report
    } else if alt_screen {
        WheelAction::Arrows { up, count: NOTCH }
    } else {
        WheelAction::Scroll(if up { -(NOTCH as isize) } else { NOTCH as isize })
    }
}

/// At most this much output is parsed before queued input is serviced again.
const CHUNK: usize = 64 * 1024;

pub fn spawn_pane(opts: PaneOptions) -> io::Result<PaneHandle> {
    let (cmd_tx, cmd_rx) = unbounded();
    let (ev_tx, ev_rx) = unbounded();
    let (init_tx, init_rx) = bounded::<io::Result<()>>(1);
    thread::Builder::new().name("wraith-pane".into()).spawn(move || {
        let mut pane = match Pane::new(opts, ev_tx) {
            Ok(p) => p,
            Err(e) => {
                let _ = init_tx.send(Err(e));
                return;
            }
        };
        let _ = init_tx.send(Ok(()));
        pane.run(&cmd_rx);
    })?;
    init_rx.recv().map_err(|_| io::Error::other("pane thread died during init"))??;
    Ok(PaneHandle { cmd: cmd_tx, events: ev_rx })
}

/// Side effects raised inside libghostty callbacks, drained after each `vt_write`.
#[derive(Default)]
struct Outbox {
    pty: Vec<u8>,
    title: bool,
    bell: bool,
    clipboard: Option<String>,
}

struct Pane {
    opts: PaneOptions,
    pty: Pty,
    chunks: Receiver<Vec<u8>>,
    term: Terminal<'static, 'static>,
    rs: RenderState<'static>,
    keys: key::Encoder<'static>,
    key_ev: key::Event<'static>,
    mice: mouse::Encoder<'static>,
    mouse_ev: mouse::Event<'static>,
    outbox: Rc<RefCell<Outbox>>,
    events: Sender<PaneEvent>,
    seq: u64,
    outstanding: bool,
    changed: bool,
    /// When output first changed the screen since the last frame (debug timing).
    output_since: Option<Instant>,
    force_full: bool,
    exited: bool,
    mouse_tracking: bool,
    size: (u16, u16, u32, u32),
}

fn io_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn rgb((r, g, b): (u8, u8, u8)) -> RgbColor {
    RgbColor { r, g, b }
}

impl Pane {
    fn new(opts: PaneOptions, events: Sender<PaneEvent>) -> io::Result<Pane> {
        let (cols, rows) = clamp_size(opts.cols, opts.rows);
        let (pty, chunks) = start_pty(&opts, cols, rows)?;
        let mut term = Terminal::new(TerminalOptions { cols, rows, max_scrollback: opts.scrollback_lines })
            .map_err(io_err)?;
        term.resize(cols, rows, opts.cell_w_px, opts.cell_h_px).map_err(io_err)?;
        let outbox = Rc::new(RefCell::new(Outbox::default()));
        install_callbacks(&mut term, &outbox).map_err(io_err)?;
        let mut pane = Pane {
            size: (cols, rows, opts.cell_w_px, opts.cell_h_px),
            opts,
            pty,
            chunks,
            term,
            rs: RenderState::new().map_err(io_err)?,
            keys: key::Encoder::new().map_err(io_err)?,
            key_ev: key::Event::new().map_err(io_err)?,
            mice: mouse::Encoder::new().map_err(io_err)?,
            mouse_ev: mouse::Event::new().map_err(io_err)?,
            outbox,
            events,
            seq: 0,
            outstanding: false,
            changed: true,
            output_since: None,
            force_full: true,
            exited: false,
            mouse_tracking: false,
        };
        let (fg, bg, pal) = (pane.opts.default_fg, pane.opts.default_bg, pane.opts.palette.clone());
        pane.set_colors(fg, bg, &pal);
        Ok(pane)
    }

    fn run(&mut self, cmds: &Receiver<(Instant, PaneCmd)>) {
        loop {
            // Input first: everything queued is handled before the next output chunk.
            while let Ok((t, cmd)) = cmds.try_recv() {
                if !self.handle(t, cmd) {
                    return;
                }
            }
            select! {
                recv(cmds) -> m => match m {
                    Ok((t, cmd)) => if !self.handle(t, cmd) { return },
                    Err(_) => return,
                },
                recv(self.chunks) -> c => if let Ok(c) = c {
                    tracing::trace!(bytes = c.len(), text = %String::from_utf8_lossy(&c[..c.len().min(200)]), "chunk");
                    self.term.vt_write(&c);
                    self.changed = true;
                    self.output_since.get_or_insert_with(Instant::now);
                },
                default(Duration::from_millis(16)) => {}
            }
            self.flush_outbox();
            self.maybe_send_frame();
            if !self.exited
                && let Some(code) = self.pty.try_wait()
                && self.chunks.is_empty()
            {
                self.exited = true;
                let _ = self.events.send(PaneEvent::Exited(code));
            }
        }
    }

    /// Returns false on shutdown.
    fn handle(&mut self, sent: Instant, cmd: PaneCmd) -> bool {
        match cmd {
            PaneCmd::Key(e) => {
                self.keys.set_options_from_terminal(&self.term);
                to_ghostty_key(&e, &mut self.key_ev);
                let mut out = Vec::new();
                if self.keys.encode_to_vec(&self.key_ev, &mut out).is_ok() && !out.is_empty() {
                    self.write_input(sent, &out);
                }
            }
            PaneCmd::Text(t) => self.write_input(sent, t.as_bytes()),
            PaneCmd::Paste(t) => {
                if paste::is_safe(&t) {
                    self.write_paste(sent, t);
                } else {
                    let _ = self.events.send(PaneEvent::PasteNeedsConfirm(t));
                }
            }
            PaneCmd::PasteConfirmed(t) => self.write_paste(sent, t),
            PaneCmd::Mouse(m) => self.mouse(sent, m),
            PaneCmd::Resize { cols, rows, cell_w_px, cell_h_px } => {
                let (cols, rows) = clamp_size(cols, rows);
                if (cols, rows, cell_w_px, cell_h_px) != self.size {
                    self.size = (cols, rows, cell_w_px, cell_h_px);
                    let _ = self.pty.resize(cols, rows);
                    let _ = self.term.resize(cols, rows, cell_w_px, cell_h_px);
                    self.force_full = true;
                    self.changed = true;
                }
            }
            PaneCmd::Scroll(s) => {
                let page = self.size.1 as isize;
                self.scroll(match s {
                    ScrollTo::Delta(d) => ScrollViewport::Delta(d as isize),
                    ScrollTo::PageUp => ScrollViewport::Delta(-page),
                    ScrollTo::PageDown => ScrollViewport::Delta(page),
                    ScrollTo::Top => ScrollViewport::Top,
                    ScrollTo::Bottom => ScrollViewport::Bottom,
                });
            }
            PaneCmd::Ready(seq) => {
                if seq == self.seq {
                    self.outstanding = false;
                }
            }
            PaneCmd::SetSelection(range) => {
                self.set_selection(range);
                self.selection_changed();
            }
            PaneCmd::SelectWord { x, y } => {
                if let Ok(g) = self.term.grid_ref(viewport(x, y))
                    && let Ok(Some(sel)) = self.term.select_word(SelectWordOptions::new(g))
                {
                    let _ = self.term.set_selection(Some(&sel));
                }
                self.selection_changed();
            }
            PaneCmd::SelectLine { y } => {
                if let Ok(g) = self.term.grid_ref(viewport(0, y))
                    && let Ok(Some(sel)) = self.term.select_line(SelectLineOptions::new(g))
                {
                    let _ = self.term.set_selection(Some(&sel));
                }
                self.selection_changed();
            }
            PaneCmd::CopySelection => {
                if let Ok(Some(bytes)) = self.term.format_selection_alloc(None, FormatOptions::new()) {
                    let _ = self.events.send(PaneEvent::Copied(String::from_utf8_lossy(&bytes).into_owned()));
                }
            }
            PaneCmd::SetColors { fg, bg, palette } => {
                self.set_colors(fg, bg, &palette);
                self.force_full = true;
                self.changed = true;
            }
            PaneCmd::Redraw => {
                self.force_full = true;
                self.changed = true;
            }
            PaneCmd::Reattach => {
                self.outstanding = false;
                self.force_full = true;
                self.changed = true;
            }
            PaneCmd::Restart => self.restart(),
            PaneCmd::Shutdown => return false,
        }
        true
    }

    fn write_input(&mut self, sent: Instant, bytes: &[u8]) {
        // Typing returns the viewport to the live screen.
        self.term.scroll_viewport(ScrollViewport::Bottom);
        let _ = self.pty.write(bytes);
        let _ = self.events.send(PaneEvent::InputWritten { waited: sent.elapsed() });
    }

    fn write_paste(&mut self, sent: Instant, text: String) {
        let bracketed = self.term.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
        let mut data = text.into_bytes();
        let mut buf = vec![0u8; data.len() + 16];
        if let Ok(n) = paste::encode(&mut data, bracketed, &mut buf) {
            self.write_input(sent, &buf[..n]);
        }
    }

    fn mouse(&mut self, sent: Instant, m: MouseEvent) {
        let wheel_up = match m.button {
            Some(MouseButton::WheelUp) => Some(true),
            Some(MouseButton::WheelDown) => Some(false),
            _ => None,
        };
        let alt = matches!(self.term.active_screen(), Ok(Screen::Alternate));
        if let Some(up) = wheel_up {
            match wheel_action(alt, self.mouse_tracking, up) {
                WheelAction::Scroll(n) => return self.scroll(ScrollViewport::Delta(n)),
                WheelAction::Arrows { up, count } => {
                    let code = if up { KeyCode::ArrowUp } else { KeyCode::ArrowDown };
                    for _ in 0..count {
                        let ev = KeyEvent { code, mods: Mods(0), text: None, action: KeyAction::Press, composing: false };
                        self.handle(sent, PaneCmd::Key(ev));
                    }
                    return;
                }
                WheelAction::Report => {}
            }
        }
        if !self.mouse_tracking {
            return;
        }
        let (cols, rows, cw, ch) = self.size;
        self.mice.set_options_from_terminal(&self.term).set_size(mouse::EncoderSize {
            screen_width: u32::from(cols) * cw,
            screen_height: u32::from(rows) * ch,
            cell_width: cw,
            cell_height: ch,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        });
        self.mouse_ev
            .set_action(match m.action {
                MouseAction::Press => mouse::Action::Press,
                MouseAction::Release => mouse::Action::Release,
                MouseAction::Motion => mouse::Action::Motion,
            })
            .set_button(m.button.map(|b| match b {
                MouseButton::Left => mouse::Button::Left,
                MouseButton::Middle => mouse::Button::Middle,
                MouseButton::Right => mouse::Button::Right,
                MouseButton::WheelUp => mouse::Button::Four,
                MouseButton::WheelDown => mouse::Button::Five,
                MouseButton::WheelLeft => mouse::Button::Six,
                MouseButton::WheelRight => mouse::Button::Seven,
                MouseButton::X1 => mouse::Button::Eight,
                MouseButton::X2 => mouse::Button::Nine,
            }))
            .set_mods(keymap::mods(m.mods))
            .set_position(mouse::Position { x: m.x_px, y: m.y_px });
        let mut out = Vec::new();
        if self.mice.encode_to_vec(&self.mouse_ev, &mut out).is_ok() && !out.is_empty() {
            let _ = self.pty.write(&out);
            let _ = self.events.send(PaneEvent::InputWritten { waited: sent.elapsed() });
        }
    }

    /// Selection doesn't mark rows dirty in libghostty; resend the screen so SELECTED flags update.
    fn selection_changed(&mut self) {
        self.force_full = true;
        self.changed = true;
    }

    fn scroll(&mut self, s: ScrollViewport) {
        self.term.scroll_viewport(s);
        self.changed = true;
    }

    fn set_selection(&mut self, range: Option<SelRange>) {
        let Some(r) = range else {
            let _ = self.term.set_selection(None);
            return;
        };
        if let (Ok(a), Ok(b)) = (self.term.grid_ref(viewport(r.start.0, r.start.1)), self.term.grid_ref(viewport(r.end.0, r.end.1))) {
            let sel = Selection::new(a, b, r.rectangle);
            let _ = self.term.set_selection(Some(&sel));
        }
    }

    fn set_colors(&mut self, fg: (u8, u8, u8), bg: (u8, u8, u8), palette: &[(u8, u8, u8)]) {
        let _ = self.term.set_default_fg_color(Some(rgb(fg)));
        let _ = self.term.set_default_bg_color(Some(rgb(bg)));
        if palette.len() == 256 {
            let mut p = [RgbColor { r: 0, g: 0, b: 0 }; 256];
            for (dst, &src) in p.iter_mut().zip(palette) {
                *dst = rgb(src);
            }
            let _ = self.term.set_default_color_palette(Some(Palette(p)));
        }
    }

    fn restart(&mut self) {
        let (cols, rows, _, _) = self.size;
        match start_pty(&self.opts, cols, rows) {
            Ok((pty, chunks)) => {
                self.pty = pty;
                self.chunks = chunks;
                self.term.reset();
                self.exited = false;
                self.force_full = true;
                self.changed = true;
            }
            Err(e) => tracing::warn!("pane restart failed: {e}"),
        }
    }

    fn flush_outbox(&mut self) {
        let mut ob = self.outbox.borrow_mut();
        if !ob.pty.is_empty() {
            let _ = self.pty.write(&ob.pty);
            ob.pty.clear();
        }
        if std::mem::take(&mut ob.title)
            && let Ok(t) = self.term.title()
        {
            let _ = self.events.send(PaneEvent::Title(t.to_string()));
        }
        if std::mem::take(&mut ob.bell) {
            let _ = self.events.send(PaneEvent::Bell);
        }
        if let Some(c) = ob.clipboard.take() {
            let _ = self.events.send(PaneEvent::Clipboard(c));
        }
        drop(ob);
        let tracking = self.term.is_mouse_tracking().unwrap_or(false);
        if tracking != self.mouse_tracking {
            self.mouse_tracking = tracking;
            let _ = self.events.send(PaneEvent::MouseTracking(tracking));
        }
    }

    fn maybe_send_frame(&mut self) {
        if self.outstanding || !self.changed {
            return;
        }
        self.changed = false;
        // Sent even with no dirty rows: the cursor or colors may have changed.
        let frame = self.rs.update(&self.term).and_then(|snap| build_frame(&snap, self.seq + 1, self.force_full));
        let Ok(mut frame) = frame else { return };
        if let Ok(sb) = self.term.scrollbar() {
            frame.scrollbar = wraith_proto::Scrollbar { total: sb.total, offset: sb.offset, len: sb.len };
        }
        self.seq += 1;
        self.force_full = false;
        self.outstanding = true;
        let held_us = self.output_since.take().map_or(0, |t| t.elapsed().as_micros() as u64);
        tracing::debug!(seq = self.seq, rows = frame.dirty_rows.len(), held_us, "frame sent");
        let _ = self.events.send(PaneEvent::Frame(frame));
    }
}

fn viewport(x: u16, y: u16) -> Point {
    Point::Viewport(PointCoordinate { x, y: u32::from(y) })
}

fn start_pty(opts: &PaneOptions, cols: u16, rows: u16) -> io::Result<(Pty, Receiver<Vec<u8>>)> {
    let mut pty = Pty::spawn(&opts.cmdline, opts.cwd.as_deref(), &opts.env, cols, rows).map_err(io_err)?;
    let mut reader = pty.take_reader();
    // Bounded: when the pane falls behind, ConPTY backpressures the child instead of buffering here.
    let (tx, rx) = bounded::<Vec<u8>>(4);
    thread::Builder::new().name("wraith-pty-read".into()).spawn(move || {
        let mut buf = vec![0u8; CHUNK];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    })?;
    Ok((pty, rx))
}

fn install_callbacks(term: &mut Terminal<'static, 'static>, outbox: &Rc<RefCell<Outbox>>) -> libghostty_vt::error::Result<()> {
    let ob = outbox.clone();
    term.on_pty_write(move |_t, data| ob.borrow_mut().pty.extend_from_slice(data))?;
    let ob = outbox.clone();
    term.on_title_changed(move |_t| ob.borrow_mut().title = true)?;
    let ob = outbox.clone();
    term.on_bell(move |_t| ob.borrow_mut().bell = true)?;
    let ob = outbox.clone();
    term.on_clipboard_write(move |_t, w| {
        if let Some(c) = w.contents().find(|c| c.mime.starts_with("text/plain")) {
            ob.borrow_mut().clipboard = Some(c.data.to_string());
        }
        Ok(())
    })?;
    // The bundled OpenConsole sends DA1 at startup and waits for an answer.
    term.on_device_attributes(|_t| {
        Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(
                ConformanceLevel::VT220,
                &[DeviceAttributeFeature::COLUMNS_132, DeviceAttributeFeature::SELECTIVE_ERASE, DeviceAttributeFeature::ANSI_COLOR],
            ),
            secondary: SecondaryDeviceAttributes { device_type: DeviceType::VT220, firmware_version: 1, rom_cartridge: 0 },
            tertiary: Default::default(),
        })
    })?;
    Ok(())
}
