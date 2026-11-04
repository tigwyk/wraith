//! The quick terminal (quick-terminal spec): a hidden `--quick-terminal` window that a global
//! hotkey slides in from a screen edge. The pure parts are here first; the Win32 side follows them.
use std::time::Duration;

use wraith_config::{Dim, QuickPosition, QuickSize, Trigger, TriggerKey};
use wraith_proto::model::Rect;
use wraith_proto::{KeyCode, Mods};

use crate::input::scancode_to_code;

/// `RegisterHotKey` modifier flags.
pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;
pub const MOD_NOREPEAT: u32 = 0x4000;

/// Every physical key with its set-1 scancode, non-extended first.
fn scancodes() -> impl Iterator<Item = (u16, bool, KeyCode)> {
    [false, true].into_iter().flat_map(|ext| (0x01..=0x7Fu16).map(move |sc| (sc, ext, scancode_to_code(sc, ext))))
}

/// The set-1 scancode of a physical key, and whether it is an extended key.
pub fn scancode_of(code: KeyCode) -> Option<(u16, bool)> {
    scancodes().find(|(_, _, c)| *c == code).map(|(sc, ext, _)| (sc, ext))
}

/// The Win32 modifiers (with `MOD_NOREPEAT`) and the physical key for a `global:` trigger.
pub fn hotkey_code(t: &Trigger) -> Option<(u32, KeyCode)> {
    let code = match &t.key {
        TriggerKey::Named(n) => crate::app::named_key(n)?,
        TriggerKey::Char(c) => scancodes().map(|(_, _, k)| k).find(|k| crate::app::key_char(*k) == Some(*c))?,
        TriggerKey::Physical(n) => scancodes().map(|(_, _, k)| k).find(|k| format!("{k:?}") == *n)?,
    };
    let m = t.mods.0;
    let mut mods = MOD_NOREPEAT;
    for (bit, flag) in [(Mods::SUPER, MOD_WIN), (Mods::CTRL, MOD_CONTROL), (Mods::ALT, MOD_ALT), (Mods::SHIFT, MOD_SHIFT)] {
        if m & bit != 0 {
            mods |= flag;
        }
    }
    Some((mods, code))
}

/// The window's place on a monitor's work area (spec §3.2). Pixel sizes are at 96 DPI.
pub fn quick_rect(work: Rect, pos: QuickPosition, size: &QuickSize, dpi: u32) -> Rect {
    let len = |d: Dim, axis: i32| -> i32 {
        let v = match d {
            Dim::Percent(p) => (axis as f32 * p / 100.0).round() as i32,
            Dim::Px(px) => (px as f32 * dpi as f32 / 96.0).round() as i32,
        };
        v.clamp(1, axis.max(1))
    };
    let h = len(size.primary, work.h);
    let w = size.secondary.map_or(work.w, |d| len(d, work.w));
    let x = work.x + (work.w - w) / 2;
    let y = match pos {
        QuickPosition::Top => work.y,
        QuickPosition::Bottom => work.y + work.h - h,
    };
    Rect { x, y, w, h }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Show,
    Hide,
    Front,
}

/// What the hotkey does (spec §3.3).
pub fn toggle(visible: bool, foreground: bool) -> Toggle {
    match (visible, foreground) {
        (false, _) => Toggle::Show,
        (true, true) => Toggle::Hide,
        (true, false) => Toggle::Front,
    }
}

/// The window's top edge `progress` (0..1) into a slide in (`showing`) or out.
pub fn slide_y(target: Rect, pos: QuickPosition, progress: f32, showing: bool) -> i32 {
    let off = match pos {
        QuickPosition::Top => target.y - target.h,
        QuickPosition::Bottom => target.y + target.h,
    } as f32;
    let to = target.y as f32;
    let e = crate::motion::ease_out(progress);
    (if showing { off + (to - off) * e } else { to + (off - to) * e }).round() as i32
}

/// How far a slide of `duration_s` seconds is after `elapsed`; a zero duration is already done.
pub fn progress(elapsed: Duration, duration_s: f32) -> f32 {
    if duration_s <= 0.0 { 1.0 } else { (elapsed.as_secs_f32() / duration_s).clamp(0.0, 1.0) }
}

/// The single-instance mutex for the quick terminal of `pipe` (FNV-1a 64, stable across builds).
pub fn mutex_name(pipe: &str) -> String {
    let h = pipe.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    format!(r"Local\wraith-quick-{h:016x}")
}

// ---- The Win32 side: the `--quick-terminal` process (spec §3). Free functions on the app, as in reload.rs.

use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, POINT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MAPVK_VSC_TO_VK_EX, MapVirtualKeyW, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOTOWNER, GetAncestor, GetCursorPos, GetForegroundWindow, HWND_TOPMOST, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SetForegroundWindow, SetWindowPos, ShowWindow,
};
use windows::core::HSTRING;
use wraith_config::{Config, QuickScreen};

use crate::app::App;

const HOTKEY_ID: i32 = 1;
/// The slide's frame interval, ms.
const SLIDE_MS: u32 = 8;

pub(crate) struct State {
    pub(crate) shown: bool,
    /// A slide in progress: when it started, and whether it shows the window.
    slide: Option<(Instant, bool)>,
    target: Rect,
    registered: Option<Trigger>,
    dpi: u32,
    /// Held for the process's life: one quick terminal per user and pipe.
    _mutex: HANDLE,
}

impl State {
    pub(crate) fn new(mutex: HANDLE, dpi: u32) -> State {
        State { shown: false, slide: None, target: Rect::default(), registered: None, dpi, _mutex: mutex }
    }

    /// Hidden with nothing moving: the process may sleep until a message arrives.
    pub(crate) fn idle(&self) -> bool {
        !self.shown && self.slide.is_none()
    }
}

/// Take the quick terminal's single-instance mutex; `None` when another process holds it.
pub(crate) fn take_instance(pipe: &str) -> Option<HANDLE> {
    let h = unsafe { CreateMutexW(None, false, &HSTRING::from(mutex_name(pipe))) }.ok()?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(h);
        }
        return None;
    }
    Some(h)
}

/// A quick terminal is running for `pipe`.
pub(crate) fn running(pipe: &str) -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(mutex_name(pipe))) } {
        Ok(h) => {
            unsafe {
                let _ = CloseHandle(h);
            }
            true
        }
        Err(_) => false,
    }
}

fn hwnd(app: &App) -> HWND {
    app.win.hwnd()
}

/// (Re)register the configured hotkey; one toast when it cannot be had (spec §3.1).
pub(crate) fn register(app: &mut App) {
    let h = hwnd(app);
    unsafe {
        let _ = UnregisterHotKey(Some(h), HOTKEY_ID);
    }
    let want = app.config.quick_terminal_hotkey.clone();
    let Some(st) = app.quick.as_mut() else { return };
    st.registered = None;
    let Some(t) = want else { return };
    let vk = hotkey_code(&t).and_then(|(mods, code)| {
        let (sc, ext) = scancode_of(code)?;
        let vk = unsafe { MapVirtualKeyW(u32::from(sc) | if ext { 0xE000 } else { 0 }, MAPVK_VSC_TO_VK_EX) };
        (vk != 0).then_some((mods, vk))
    });
    let ok = vk.is_some_and(|(mods, vk)| unsafe { RegisterHotKey(Some(h), HOTKEY_ID, HOT_KEY_MODIFIERS(mods), vk) }.is_ok());
    let key = crate::palette::trigger_text(&wraith_config::Keybind { trigger: vec![t.clone()], action: wraith_config::Action::Unbind });
    if ok {
        tracing::info!("quick terminal: hotkey {key}");
        st.registered = Some(t);
    } else {
        tracing::warn!("quick terminal: cannot register {key}");
        let why = if vk.is_some() { "is in use by another app" } else { "cannot be used as a global key" };
        app.notify_text("Wraith quick terminal", &format!("{key} {why}. Set another with keybind = global:...=toggle_quick_terminal"));
    }
}

/// The work area and DPI of the monitor the quick terminal goes on.
fn work_area(screen: QuickScreen) -> (Rect, u32) {
    unsafe {
        let mon = match screen {
            QuickScreen::Main => MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
            QuickScreen::Mouse => {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
            }
        };
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let r = mi.rcWork;
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }, dx.max(96))
    }
}

fn target(app: &App) -> Rect {
    let (work, dpi) = work_area(app.config.quick_terminal_screen);
    quick_rect(work, app.config.quick_terminal_position, &app.config.quick_terminal_size, dpi)
}

/// Move and size the window, then catch up on what its nested WM_SIZE / WM_DPICHANGED could not reach:
/// the app is borrowed while a message is handled, so those fall through to the default handler.
fn place(app: &mut App, r: Rect, y: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd(app), Some(HWND_TOPMOST), r.x, y, r.w, r.h, SWP_NOACTIVATE);
    }
    catch_up(app);
    if let Some(st) = app.quick.as_mut() {
        st.target = r;
    }
}

/// Size and DPI as Windows now has them, for when the messages carrying them were lost.
fn catch_up(app: &mut App) {
    let dpi = app.win.dpi();
    let size = app.win.client_size();
    let dpi_changed = app.quick.as_mut().is_some_and(|st| std::mem::replace(&mut st.dpi, dpi) != dpi);
    app.sync_window_size(size, dpi_changed);
}

fn animated(c: &Config) -> bool {
    c.quick_terminal_animation_duration > 0.0 && crate::window::animations_enabled()
}

fn show(app: &mut App) {
    let r = target(app);
    let animate = animated(&app.config);
    // Placed on its own monitor first, so size and DPI are that monitor's; then moved off the edge.
    place(app, r, r.y);
    if animate {
        let y = slide_y(r, app.config.quick_terminal_position, 0.0, true);
        unsafe {
            let _ = SetWindowPos(hwnd(app), None, r.x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
    // Shown before the first frame: a hidden quick terminal renders nothing.
    if let Some(st) = app.quick.as_mut() {
        st.shown = true;
        st.slide = animate.then(|| (Instant::now(), true));
    }
    // The first visible frame is complete: drawn while the window is still hidden.
    app.render_first_frame();
    let h = hwnd(app);
    unsafe {
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
    }
    app.sync_focus();
}

/// Bring the quick terminal up, never hiding it (a toast click).
pub(crate) fn summon(app: &mut App) {
    if app.quick.as_ref().is_some_and(|st| st.shown) {
        front(app);
    } else {
        show(app);
    }
}

fn front(app: &mut App) {
    unsafe {
        let _ = SetForegroundWindow(hwnd(app));
    }
    app.sync_focus();
}

/// Slide the window out (or hide it at once); it keeps its workspace and shells.
pub(crate) fn hide(app: &mut App) {
    let animate = animated(&app.config);
    let h = hwnd(app);
    let Some(st) = app.quick.as_mut() else { return };
    if !st.shown {
        return;
    }
    st.shown = false;
    if animate {
        st.slide = Some((Instant::now(), false));
    } else {
        st.slide = None;
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
        app.sync_focus();
    }
}

/// `WM_HOTKEY` (spec §3.3).
pub(crate) fn on_hotkey(app: &mut App) {
    let Some(st) = app.quick.as_ref() else { return };
    let h = hwnd(app);
    match toggle(st.shown, unsafe { GetForegroundWindow() } == h) {
        Toggle::Show => show(app),
        Toggle::Hide => hide(app),
        Toggle::Front => front(app),
    }
}

/// `WM_ACTIVATE` / `WA_INACTIVE`: focus went to `other`. Our own popups (IME candidates, message
/// boxes) do not count, nor does the moment of a slide.
pub(crate) fn on_deactivate(app: &mut App, other: HWND) {
    let Some(st) = app.quick.as_ref() else { return };
    if !app.config.quick_terminal_autohide || !st.shown || st.slide.is_some() {
        return;
    }
    let h = hwnd(app);
    if !other.is_invalid() && unsafe { GetAncestor(other, GA_ROOTOWNER) } == h {
        return;
    }
    hide(app);
}

/// Advance a slide; the sleep it needs until its next step.
pub(crate) fn tick(app: &mut App, now: Instant) -> Option<u32> {
    let dur = app.config.quick_terminal_animation_duration;
    let pos = app.config.quick_terminal_position;
    let h = hwnd(app);
    let st = app.quick.as_mut()?;
    let (start, showing) = st.slide?;
    let p = progress(now.saturating_duration_since(start), dur);
    unsafe {
        let _ = SetWindowPos(h, None, st.target.x, slide_y(st.target, pos, p, showing), 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    if p < 1.0 {
        return Some(SLIDE_MS);
    }
    st.slide = None;
    if showing {
        // Crossing monitors during the slide sent WM_DPICHANGED from inside idle, which was lost.
        catch_up(app);
    } else {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
        app.sync_focus();
    }
    None
}

/// Put a shown window where the config and monitors now say (reload, display change).
pub(crate) fn replace(app: &mut App) {
    if app.quick.as_ref().is_some_and(|st| st.shown && st.slide.is_none()) {
        let r = target(app);
        place(app, r, r.y);
        app.request_redraw();
    }
}

/// After a config reload (spec §2): no hotkey left ends the process; a new one is registered.
pub(crate) fn on_reload(app: &mut App, old: &Config) {
    let Some(st) = app.quick.as_ref() else { return };
    if app.config.quick_terminal_hotkey.is_none() {
        tracing::info!("quick terminal: no global toggle left; exiting");
        app.quit = true;
        return;
    }
    if st.registered.as_ref() != app.config.quick_terminal_hotkey.as_ref() {
        register(app);
    }
    let placement = |c: &Config| (c.quick_terminal_position, c.quick_terminal_size.clone(), c.quick_terminal_screen);
    if placement(old) != placement(&app.config) {
        replace(app);
    }
}
