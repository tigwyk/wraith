//! Config hot reload, diagnostics toasts and the frame-time overlay.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use wraith_config::{Action, Config, Diagnostic};
use wraith_server::profile::Profile;

use crate::app::{App, KeybindMatcher, themes_dir};

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
    /// Diagnostics of the last load, for the settings panel's warning marks.
    pub(crate) diags: Vec<Diagnostic>,
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
            if let Some(dir) = path.parent().filter(|d| d.is_dir())
                && let Err(e) = w.watch(dir, RecursiveMode::NonRecursive)
            {
                tracing::warn!("config watch {}: {e}", dir.display());
            }
            app.rs._watcher = Some(w);
        }
        Err(e) => tracing::warn!("config watcher: {e}"),
    }
    toast(app, "config", format_diagnostics(&diags));
    app.rs.diags = diags;
}

fn toast(app: &mut App, kind: &'static str, text: Option<String>) {
    if let Some(text) = text {
        toast_text(app, kind, text);
    }
}

/// Show `text` bottom-right for a few seconds, at most one toast per kind every 3 s.
pub(crate) fn toast_text(app: &mut App, kind: &'static str, text: String) {
    if app.rs.toasts.allow(kind, Instant::now()) {
        app.toast = Some(text);
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
                app.stats = None;
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
        app.toast = None;
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
    app.stats = Some(text);
}

pub(crate) fn path(app: &App) -> &Path {
    &app.rs.path
}

/// Per settings row: where its value comes from, and the diagnostic on its line in this file.
pub fn row_states(path: &Path, diags: &[Diagnostic]) -> Vec<crate::settings::RowState> {
    use wraith_config::settings::all;
    let keys: Vec<_> = all().iter().map(|s| (s.key, s.which)).collect();
    wraith_config::edit::origins(path, &keys)
        .into_iter()
        .map(|origin| {
            let warning = match origin {
                wraith_config::edit::Origin::Here { line } => diags.iter().find(|d| d.file == path && d.line == line).map(|d| d.message.clone()),
                _ => None,
            };
            crate::settings::RowState { origin, warning }
        })
        .collect()
}

pub(crate) fn reload(app: &mut App) {
    match wraith_config::load_checked(&app.rs.path, &themes_dir()) {
        Ok((new, d)) => {
            let (new, msg) = log_and_format(new, &d);
            apply_config(app, new);
            toast(app, "config", msg);
            app.rs.diags = d;
            app.settings_reloaded();
        }
        // Unreadable (e.g. caught mid-save): keep running with the current config.
        Err(d) => toast(app, "config", format_diagnostics(&[d])),
    }
}

/// Apply a freshly loaded config to the running app.
pub(crate) fn apply_config(app: &mut App, new: Config) {
    let old = std::mem::replace(&mut app.config, new);
    if app.config.keybinds != old.keybinds {
        app.matcher = KeybindMatcher::new(&app.config.keybinds);
    }
    let schemes = |a: &App| (a.spawn_ctx.profiles.iter().map(|p| (p.name.clone(), p.scheme.clone())).collect::<Vec<_>>(), a.spawn_ctx.schemes.clone());
    let old_schemes = schemes(app);
    let old_profiles = app.spawn_ctx.profiles.clone();
    // Rebuilt first: resend_colors reads each pane's colours from it.
    app.rebuild_spawn_ctx();
    if (app.config.foreground, app.config.background, app.config.palette) != (old.foreground, old.background, old.palette) || schemes(app) != old_schemes {
        app.resend_colors();
    }
    // shortcut: `command` changes apply on the next app start, not on pane restart (pane keeps its options).
    let opacities = |c: &Config, ps: &[Profile]| (c.background_opacity, ps.iter().map(|p| p.opacity).collect::<Vec<_>>());
    if fonts_changed(&old, &app.config, &old_profiles, &app.spawn_ctx.profiles) {
        app.rebuild_fonts();
    } else if opacities(&old, &old_profiles) != opacities(&app.config, &app.spawn_ctx.profiles) {
        app.rebuild_looks();
    }
    app.sync_swapchain(crate::app::needs_transparency(&old, &old_profiles));
    app.request_redraw();
}

/// Whether a reload needs a new font table and atlas: the window font, padding, or any profile's
/// font, size or padding changed (spec §3.5).
pub fn fonts_changed(old: &Config, new: &Config, old_p: &[Profile], new_p: &[Profile]) -> bool {
    let profiles = |ps: &[Profile]| ps.iter().map(|p| (p.name.clone(), p.font.clone(), p.font_size, p.padding)).collect::<Vec<_>>();
    (&new.font_family, &new.font_feature) != (&old.font_family, &old.font_feature)
        || new.font_size != old.font_size
        || (new.window_padding_x, new.window_padding_y) != (old.window_padding_x, old.window_padding_y)
        || profiles(old_p) != profiles(new_p)
}
