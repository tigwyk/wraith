//! Config hot reload, diagnostics toasts and the frame-time overlay.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use wraith_config::{Action, Config, Diagnostic};
use wraith_server::pane::PaneCmd;

use crate::app::{App, KeybindMatcher, themes_dir};
use crate::grid::Overlay;

const DEBOUNCE: Duration = Duration::from_millis(200);
const TOAST_FOR: Duration = Duration::from_secs(5);
const TOAST_THROTTLE: Duration = Duration::from_secs(3);

/// At most one toast per kind every 3 s (held keys or rapid saves must not spam).
#[derive(Default)]
pub struct Toasts {
    last: HashMap<&'static str, Instant>,
}

impl Toasts {
    pub fn allow(&mut self, kind: &'static str, now: Instant) -> bool {
        if self.last.get(kind).is_some_and(|t| now.duration_since(*t) < TOAST_THROTTLE) {
            return false;
        }
        self.last.insert(kind, now);
        true
    }
}

/// `config: file:line: message` for the first diagnostic, plus how many more there are.
pub fn format_diagnostics(d: &[Diagnostic]) -> Option<String> {
    let first = d.first()?;
    let mut s = format!("config: {}:{}: {}", first.file.display(), first.line, first.message);
    if d.len() > 1 {
        s.push_str(&format!(" (and {} more)", d.len() - 1));
    }
    Some(s)
}

/// Load `path`; bad lines keep their defaults and come back as one toast line.
pub fn reload_file(path: &Path, themes: &Path) -> (Config, Option<String>) {
    let (c, d) = wraith_config::load(path, themes);
    log_and_format(c, &d)
}

fn log_and_format(c: Config, d: &[Diagnostic]) -> (Config, Option<String>) {
    for diag in d {
        tracing::warn!(file = %diag.file.display(), line = diag.line, "{}", diag.message);
    }
    (c, format_diagnostics(d))
}

#[derive(Default)]
pub(crate) struct State {
    _watcher: Option<notify::RecommendedWatcher>,
    changed_at: Arc<Mutex<Option<Instant>>>,
    path: PathBuf,
    toasts: Toasts,
    toast_until: Option<Instant>,
    overlay: bool,
    last_input: Option<Instant>,
    render_start: Option<Instant>,
}

pub(crate) fn start(app: &mut App, path: &Path, diags: Vec<Diagnostic>) {
    app.rs.path = path.to_path_buf();
    let changed = app.rs.changed_at.clone();
    let hwnd = app.win.hwnd().0 as usize;
    let name = path.file_name().map(|n| n.to_os_string());
    // shortcut: watches only the main config file, not `config-file` includes; add include paths when needed.
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if ev.paths.iter().any(|p| p.file_name().map(|n| n.to_os_string()) == name) {
            *changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP, WPARAM(0), LPARAM(0));
            }
        }
    });
    match watcher {
        Ok(mut w) => {
            if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
                if let Err(e) = w.watch(dir, RecursiveMode::NonRecursive) {
                    tracing::warn!("config watch {}: {e}", dir.display());
                }
            }
            app.rs._watcher = Some(w);
        }
        Err(e) => tracing::warn!("config watcher: {e}"),
    }
    toast(app, "config", format_diagnostics(&diags));
}

fn toast(app: &mut App, kind: &'static str, text: Option<String>) {
    let Some(text) = text else { return };
    if app.rs.toasts.allow(kind, Instant::now()) {
        app.grid.set_overlay(Overlay::Toast, Some(&text), &app.fonts);
        app.rs.toast_until = Some(Instant::now() + TOAST_FOR);
        app.request_redraw();
    }
}

pub(crate) fn action(app: &mut App, a: Action) {
    match a {
        Action::ReloadConfig => reload(app),
        Action::ToggleFrameOverlay => {
            app.rs.overlay = !app.rs.overlay;
            if !app.rs.overlay {
                app.grid.set_overlay(Overlay::Stats, None, &app.fonts);
            }
            app.request_redraw();
        }
        _ => {}
    }
}

pub(crate) fn note_input(app: &mut App) {
    app.rs.last_input = Some(Instant::now());
}

pub(crate) fn render_start(app: &mut App) {
    app.rs.render_start = Some(Instant::now());
}

pub(crate) fn poll(app: &mut App) {
    let due = {
        let mut c = app.rs.changed_at.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match *c {
            Some(t) if t.elapsed() >= DEBOUNCE => c.take().is_some(),
            _ => false,
        }
    };
    if due {
        reload(app);
    }
    if app.rs.toast_until.is_some_and(|t| Instant::now() >= t) {
        app.rs.toast_until = None;
        app.grid.set_overlay(Overlay::Toast, None, &app.fonts);
        app.request_redraw();
    }
}

/// Wake-up budget for pending debounce/toast timers, if any.
pub(crate) fn next_deadline(app: &App) -> Option<Instant> {
    let changed = app.rs.changed_at.lock().map(|c| c.map(|t| t + DEBOUNCE)).unwrap_or(None);
    [changed, app.rs.toast_until].into_iter().flatten().min()
}

pub(crate) fn presented(app: &mut App) {
    if !app.rs.overlay {
        return;
    }
    let now = Instant::now();
    let latency = app.rs.last_input.take().map(|t| now.duration_since(t).as_secs_f32() * 1000.0);
    let frame = app.rs.render_start.map_or(0.0, |t| now.duration_since(t).as_secs_f32() * 1000.0);
    let text = match latency {
        Some(l) => format!(" input→present {l:.1} ms · frame {frame:.1} ms · atlas {:.0}% ", app.atlas.fill_ratio() * 100.0),
        None => format!(" frame {frame:.1} ms · atlas {:.0}% ", app.atlas.fill_ratio() * 100.0),
    };
    app.grid.set_overlay(Overlay::Stats, Some(&text), &app.fonts);
}

fn reload(app: &mut App) {
    match wraith_config::load_checked(&app.rs.path, &themes_dir()) {
        Ok((new, d)) => {
            let (new, msg) = log_and_format(new, &d);
            apply_config(app, new);
            toast(app, "config", msg);
        }
        // Unreadable (e.g. caught mid-save): keep running with the current config.
        Err(d) => toast(app, "config", format_diagnostics(&[d])),
    }
}

/// Apply a freshly loaded config to the running app.
fn apply_config(app: &mut App, new: Config) {
    let old = std::mem::replace(&mut app.config, new);
    let c = &app.config;
    if c.keybinds != old.keybinds {
        app.matcher = KeybindMatcher::new(&c.keybinds);
    }
    if (c.foreground, c.background, c.palette) != (old.foreground, old.background, old.palette) {
        let rgb = |p: wraith_config::Rgb| (p.0, p.1, p.2);
        let _ = app.pane.send(PaneCmd::SetColors {
            fg: rgb(c.foreground),
            bg: rgb(c.background),
            palette: c.palette.iter().copied().map(rgb).collect(),
        });
    }
    // shortcut: `command` changes apply on the next app start, not on pane restart (pane keeps its options).
    let fonts_changed = (&c.font_family, &c.font_feature) != (&old.font_family, &old.font_feature)
        || c.font_size != old.font_size
        || (c.window_padding_x, c.window_padding_y) != (old.window_padding_x, old.window_padding_y);
    if c.font_size != old.font_size {
        app.font_size = c.font_size;
    }
    if fonts_changed {
        app.rebuild_fonts();
    }
    app.request_redraw();
}
