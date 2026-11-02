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
    PrimaryDeviceAttributes, ProgramStatusState, ProgressState, ScrollViewport, SecondaryDeviceAttributes, SemanticPromptKind, Terminal,
};
pub use wraith_proto::pane::{AgentSignal, PaneCmd, PaneEvent, ScrollTo, SearchCmd};
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
    /// Kitty graphics storage per screen, in bytes (0 turns the protocol off).
    pub image_storage_limit: u64,
    /// `agent-detect` entries: extra (name, regex) process matchers.
    pub agent_detect: Vec<(String, String)>,
}

pub struct PaneHandle {
    cmd: Sender<(Instant, PaneCmd)>,
    pub events: Receiver<PaneEvent>,
    pid: u32,
}

impl PaneHandle {
    /// Queue a command; the send time is recorded for input-latency reporting.
    /// Process id of the shell started with the pane (not updated by `Restart`).
    pub fn pid(&self) -> u32 {
        self.pid
    }

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
    let (init_tx, init_rx) = bounded::<io::Result<u32>>(1);
    thread::Builder::new().name("wraith-pane".into()).spawn(move || {
        let mut pane = match Pane::new(opts, ev_tx) {
            Ok(p) => p,
            Err(e) => {
                let _ = init_tx.send(Err(e));
                return;
            }
        };
        let _ = init_tx.send(Ok(pane.pty.pid()));
        pane.run(&cmd_rx);
    })?;
    let pid = init_rx.recv().map_err(|_| io::Error::other("pane thread died during init"))??;
    Ok(PaneHandle { cmd: cmd_tx, events: ev_rx, pid })
}

/// Side effects raised inside libghostty callbacks, drained after each `vt_write`.
#[derive(Default)]
struct Outbox {
    pty: Vec<u8>,
    title: bool,
    bell: bool,
    clipboard: Option<String>,
    /// Agent signals seen in the output (spec §5.2).
    agent: Vec<AgentSignal>,
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
    /// The last working directory reported to the session (OSC 7).
    pwd: String,
    images: crate::images::ImageSync,
    /// Process scan: next run, the agent it last reported, and the user's extra matchers.
    next_scan: Instant,
    agent_proc: Option<String>,
    detect: Vec<(String, regex::Regex)>,
    /// The open scrollback search (M5).
    search: Option<crate::search::PaneSearch>,
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
        // Before this thread's first libghostty call: the decoder hook is process-global.
        crate::images::install_png_decoder();
        let mut term = Terminal::new(cols, rows).map_err(io_err)?;
        term.set_scrollback_max_lines(Some(opts.scrollback_lines)).map_err(io_err)?;
        term.resize(cols, rows, opts.cell_w_px, opts.cell_h_px).map_err(io_err)?;
        term.set_kitty_image_storage_limit(opts.image_storage_limit).map_err(io_err)?;
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
            pwd: String::new(),
            images: crate::images::ImageSync::default(),
            next_scan: Instant::now(),
            agent_proc: None,
            detect: Vec::new(),
            search: None,
        };
        pane.detect = crate::agentscan::compile(&pane.opts.agent_detect);
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
            self.scan_agents();
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
                self.images.reset();
                self.outstanding = false;
                self.force_full = true;
                self.changed = true;
                // A new view starts with mouse tracking off; tell it what is true now.
                let _ = self.events.send(PaneEvent::MouseTracking(self.mouse_tracking));
            }
            PaneCmd::Search(c) => {
                match c {
                    SearchCmd::Set(needle) => {
                        if self.search.is_none() {
                            self.search = crate::search::PaneSearch::new(&mut self.term);
                        }
                        if let Some(s) = &mut self.search {
                            s.set_needle(&mut self.term, &needle);
                        }
                    }
                    SearchCmd::Next | SearchCmd::Prev => {
                        if let Some(s) = &mut self.search {
                            s.select(&mut self.term, c == SearchCmd::Next);
                        }
                    }
                    SearchCmd::Close => self.search = None,
                }
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

    /// Once a second: is a known agent running in this pane's job? Reports changes only.
    fn scan_agents(&mut self) {
        const EVERY: Duration = Duration::from_secs(1);
        let now = Instant::now();
        if now < self.next_scan {
            return;
        }
        self.next_scan = now + EVERY;
        let found = crate::agentscan::scan(&self.pty.job_pids(), &self.detect);
        if found != self.agent_proc {
            self.agent_proc.clone_from(&found);
            let _ = self.events.send(PaneEvent::AgentProcess(found));
        }
    }

    fn flush_outbox(&mut self) {
        let mut ob = self.outbox.borrow_mut();
        for sig in ob.agent.drain(..) {
            let _ = self.events.send(PaneEvent::Agent(sig));
        }
        if !ob.pty.is_empty() {
            let _ = self.pty.write(&ob.pty);
            ob.pty.clear();
        }
        if std::mem::take(&mut ob.title)
            && let Ok(t) = self.term.title()
        {
            let _ = self.events.send(PaneEvent::Title(String::from_utf8_lossy(t).into_owned()));
        }
        if std::mem::take(&mut ob.bell) {
            let _ = self.events.send(PaneEvent::Bell);
        }
        if let Some(c) = ob.clipboard.take() {
            let _ = self.events.send(PaneEvent::Clipboard(c));
        }
        drop(ob);
        if let Ok(p) = self.term.pwd().map(String::from_utf8_lossy)
            && p != self.pwd
        {
            self.pwd = p.into_owned();
            if let Some(dir) = cwd_from_uri(&self.pwd) {
                let _ = self.events.send(PaneEvent::Cwd(dir));
            }
        }
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
        // Pixels go ahead of the frame that places them; 1 MiB of the frame cap is left for the rest.
        let max_image = (wraith_proto::wire::MAX_FRAME as usize).saturating_sub(1 << 20);
        let (uploads, images) = self.images.sync(&self.term, max_image);
        for ev in uploads {
            let _ = self.events.send(ev);
        }
        frame.images = images;
        if let Some(s) = &mut self.search {
            // More to search: keep frames coming until it has caught up.
            self.changed |= s.step(&mut self.term);
            frame.search = Some(s.view(&mut self.term));
        }
        let held_us = self.output_since.take().map_or(0, |t| t.elapsed().as_micros() as u64);
        tracing::debug!(seq = self.seq, rows = frame.dirty_rows.len(), held_us, "frame sent");
        let _ = self.events.send(PaneEvent::Frame(frame));
    }
}

/// The directory in an OSC 7 `file://host/path` URI, as a path for the shell's OS: a Windows drive path
/// becomes `C:\x`, anything else stays POSIX (a WSL or Git Bash shell). Percent escapes are decoded.
pub fn cwd_from_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let raw = path.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let escaped = (raw[i] == b'%')
            .then(|| raw.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match escaped {
            Some(v) => {
                bytes.push(v);
                i += 3;
            }
            None => {
                bytes.push(raw[i]);
                i += 1;
            }
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    let b = path.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        return Some(path[1..].replace('/', "\\"));
    }
    Some(path)
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
    term.on_bell(move |_t| {
        let mut ob = ob.borrow_mut();
        ob.bell = true;
        ob.agent.push(AgentSignal::Ask { message: String::new() });
    })?;
    // Agent signals (spec §5.2): notifications, progress, program status (OSC 7501), command end.
    let ob = outbox.clone();
    term.on_desktop_notification(move |_t, n| {
        let (title, body) = (String::from_utf8_lossy(n.title()), String::from_utf8_lossy(n.body()));
        let message = if title.is_empty() { body.into_owned() } else { format!("{title}: {body}") };
        ob.borrow_mut().agent.push(AgentSignal::Ask { message });
    })?;
    let ob = outbox.clone();
    term.on_progress_report(move |_t, p| {
        if matches!(p.state(), Ok(ProgressState::Set | ProgressState::Indeterminate)) {
            ob.borrow_mut().agent.push(AgentSignal::Working);
        }
    })?;
    let ob = outbox.clone();
    term.on_program_status(move |_t, s| {
        let message = String::from_utf8_lossy(s.message()).into_owned();
        let sig = match s.state() {
            Ok(ProgramStatusState::Working) => AgentSignal::Working,
            Ok(ProgramStatusState::Blocked) => AgentSignal::Ask { message },
            Ok(ProgramStatusState::Done) => AgentSignal::TurnEnd { message },
            _ => AgentSignal::Idle,
        };
        ob.borrow_mut().agent.push(sig);
    })?;
    let ob = outbox.clone();
    term.on_semantic_prompt(move |_t, e| {
        // The shell's command ended (OSC 133;D): an agent run as that command is gone.
        if matches!(e.kind(), Ok(SemanticPromptKind::CommandEnd)) {
            ob.borrow_mut().agent.push(AgentSignal::Detached);
        }
    })?;
    let ob = outbox.clone();
    term.on_clipboard_write(move |_t, w| {
        if let Some(c) = w.contents().find(|c| c.mime.starts_with(b"text/plain")) {
            ob.borrow_mut().clipboard = Some(String::from_utf8_lossy(c.data).into_owned());
        }
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
