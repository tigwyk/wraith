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
