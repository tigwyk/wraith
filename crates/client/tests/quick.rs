//! The quick terminal's pure parts (quick-terminal spec §2, §3): hotkey mapping, placement, toggle, slide.
use std::time::Duration;

use wraith_client::quick::*;
use wraith_config::{Dim, QuickPosition, QuickSize, Trigger, TriggerKey};
use wraith_proto::model::Rect;
use wraith_proto::{KeyCode, Mods};

fn trig(mods: u8, key: TriggerKey) -> Trigger {
    Trigger { mods: Mods(mods), key }
}
const WORK: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 };
fn pct(p: f32) -> QuickSize {
    QuickSize { primary: Dim::Percent(p), secondary: None }
}

#[test]
fn hotkeys_map_to_win32() {
    assert_eq!(hotkey_code(&trig(Mods::SUPER, TriggerKey::Named("backquote".into()))), Some((MOD_WIN | MOD_NOREPEAT, KeyCode::Backquote)));
    assert_eq!(hotkey_code(&trig(Mods::CTRL | Mods::SHIFT, TriggerKey::Named("f12".into()))), Some((MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, KeyCode::F12)));
    assert_eq!(hotkey_code(&trig(Mods::ALT, TriggerKey::Char('`'))).map(|h| h.1), Some(KeyCode::Backquote));
    assert_eq!(hotkey_code(&trig(0, TriggerKey::Physical("KeyQ".into()))).map(|h| h.1), Some(KeyCode::KeyQ));
    assert_eq!(hotkey_code(&trig(0, TriggerKey::Named("nope".into()))), None);
    assert_eq!(scancode_of(KeyCode::Backquote), Some((0x29, false)));
    assert_eq!(scancode_of(KeyCode::F12), Some((0x58, false)));
}

#[test]
fn rect_fills_the_work_area() {
    assert_eq!(quick_rect(WORK, QuickPosition::Top, &pct(40.0), 96), Rect { x: 0, y: 0, w: 1920, h: 416 });
    assert_eq!(quick_rect(WORK, QuickPosition::Bottom, &pct(40.0), 96), Rect { x: 0, y: 624, w: 1920, h: 416 });
    let px = QuickSize { primary: Dim::Px(300), secondary: Some(Dim::Px(800)) };
    assert_eq!(quick_rect(WORK, QuickPosition::Top, &px, 144), Rect { x: 360, y: 0, w: 1200, h: 450 });
    let offset = Rect { x: 100, y: 48, w: 1820, h: 1032 };
    assert_eq!(quick_rect(offset, QuickPosition::Top, &pct(50.0), 96), Rect { x: 100, y: 48, w: 1820, h: 516 });
    assert_eq!(quick_rect(WORK, QuickPosition::Top, &QuickSize { primary: Dim::Px(5000), secondary: None }, 96).h, 1040, "clamped to the work area");
}

#[test]
fn toggle_rules() {
    assert_eq!(toggle(false, false), Toggle::Show);
    assert_eq!(toggle(true, true), Toggle::Hide);
    assert_eq!(toggle(true, false), Toggle::Front);
}

#[test]
fn slide_runs_from_off_screen_to_place() {
    let r = Rect { x: 0, y: 48, w: 1920, h: 400 };
    assert_eq!(slide_y(r, QuickPosition::Top, 0.0, true), 48 - 400);
    assert_eq!(slide_y(r, QuickPosition::Top, 1.0, true), 48);
    assert_eq!(slide_y(r, QuickPosition::Top, 1.0, false), 48 - 400);
    assert_eq!(slide_y(r, QuickPosition::Bottom, 0.0, true), 48 + 400);
    let mid = slide_y(r, QuickPosition::Top, 0.5, true);
    assert!(mid > 48 - 200 && mid < 48, "eased out: past halfway at half time, got {mid}");
    assert_eq!(progress(Duration::from_millis(100), 0.2), 0.5);
    assert_eq!(progress(Duration::ZERO, 0.0), 1.0);
    assert_eq!(progress(Duration::from_secs(9), 0.2), 1.0);
}

#[test]
fn mutex_name_is_stable_and_per_pipe() {
    let a = mutex_name(r"\.\pipe\wraith-S-1-5-21-1");
    assert!(a.starts_with(r"Local\wraith-quick-") && a.len() == r"Local\wraith-quick-".len() + 16, "{a}");
    assert_eq!(a, mutex_name(r"\.\pipe\wraith-S-1-5-21-1"));
    assert_ne!(a, mutex_name(r"\.\pipe\wraith-test"));
}
