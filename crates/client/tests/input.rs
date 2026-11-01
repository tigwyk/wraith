#![cfg(windows)]
//! Keyboard translation against real Windows layouts (loaded without activating them).
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayoutList, HKL, KLF_NOTELLSHELL, LoadKeyboardLayoutW, UnloadKeyboardLayout, VIRTUAL_KEY, VK_CONTROL,
    VK_LCONTROL, VK_MENU, VK_OEM_6, VK_RMENU, VK_SHIFT,
};
use windows::core::PCWSTR;
use wraith_client::input::{KeyTranslator, scancode_to_code};
use wraith_proto::{KeyAction, KeyCode, Mods};

/// Loads a layout for the test and unloads it afterwards unless the user already had it.
/// Holds a global lock: tests sharing one HKL would otherwise unload it under each other.
struct Layout {
    hkl: HKL,
    unload: bool,
    _lock: std::sync::MutexGuard<'static, ()>,
}

static LAYOUTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Layout {
    fn load(klid: &str) -> Layout {
        let lock = LAYOUTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = list();
        let w: Vec<u16> = klid.encode_utf16().chain([0]).collect();
        let hkl = unsafe { LoadKeyboardLayoutW(PCWSTR(w.as_ptr()), KLF_NOTELLSHELL) }.unwrap();
        Layout { unload: !before.contains(&hkl.0), hkl, _lock: lock }
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if self.unload {
            unsafe {
                let _ = UnloadKeyboardLayout(self.hkl);
            }
        }
    }
}

fn list() -> Vec<*mut core::ffi::c_void> {
    let mut buf = [HKL::default(); 64];
    let n = unsafe { GetKeyboardLayoutList(Some(&mut buf)) } as usize;
    buf[..n].iter().map(|h| h.0).collect()
}

fn state(down: &[VIRTUAL_KEY]) -> [u8; 256] {
    let mut s = [0u8; 256];
    for k in down {
        s[k.0 as usize] = 0x80;
    }
    s
}

const VK_A: u16 = 0x41;
const VK_C: u16 = 0x43;
const VK_E: u16 = 0x45;
const VK_Q: u16 = 0x51;
const VK_X: u16 = 0x58;

#[test]
fn plain_letter() {
    let us = Layout::load("00000409");
    let e = KeyTranslator::default().on_keydown(VK_A, 0x1E, false, false, &state(&[]), us.hkl).unwrap();
    assert_eq!((e.code, e.text.as_deref(), e.mods, e.action), (KeyCode::KeyA, Some("a"), Mods(0), KeyAction::Press));
}

#[test]
fn shift_letter() {
    let us = Layout::load("00000409");
    let e = KeyTranslator::default().on_keydown(VK_A, 0x1E, false, false, &state(&[VK_SHIFT]), us.hkl).unwrap();
    assert_eq!((e.text.as_deref(), e.mods), (Some("A"), Mods(Mods::SHIFT)));
}

#[test]
fn ctrl_c_has_no_text() {
    let us = Layout::load("00000409");
    let e = KeyTranslator::default().on_keydown(VK_C, 0x2E, false, false, &state(&[VK_CONTROL, VK_LCONTROL]), us.hkl).unwrap();
    assert_eq!((e.code, e.text.as_deref(), e.mods), (KeyCode::KeyC, None, Mods(Mods::CTRL)));
}

#[test]
fn alt_letter() {
    let us = Layout::load("00000409");
    let e = KeyTranslator::default().on_keydown(VK_X, 0x2D, false, false, &state(&[VK_MENU]), us.hkl).unwrap();
    assert_eq!((e.text.as_deref(), e.mods), (Some("x"), Mods(Mods::ALT)));
}

#[test]
fn altgr_produces_text_not_ctrl_alt() {
    let de = Layout::load("00000407");
    let s = state(&[VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU]);
    let e = KeyTranslator::default().on_keydown(VK_Q, 0x10, false, false, &s, de.hkl).unwrap();
    assert_eq!(e.text.as_deref(), Some("@"));
    assert!(!e.mods.has(Mods::CTRL) && !e.mods.has(Mods::ALT), "{:?}", e.mods);
}

#[test]
fn dead_key_composes() {
    let de = Layout::load("00000407");
    let mut t = KeyTranslator::default();
    assert!(t.on_keydown(VK_OEM_6.0, 0x0D, false, false, &state(&[]), de.hkl).is_none(), "dead key alone emits nothing");
    let e = t.on_keydown(VK_E, 0x12, false, false, &state(&[]), de.hkl).unwrap();
    assert_eq!(e.text.as_deref(), Some("é"));
}

#[test]
fn repeat_and_release() {
    let us = Layout::load("00000409");
    let mut t = KeyTranslator::default();
    assert_eq!(t.on_keydown(VK_A, 0x1E, false, true, &state(&[]), us.hkl).unwrap().action, KeyAction::Repeat);
    let up = t.on_keyup(VK_A, 0x1E, false, &state(&[]));
    assert_eq!((up.code, up.action, up.text), (KeyCode::KeyA, KeyAction::Release, None));
}

#[test]
fn arrows_and_function_keys_by_scancode() {
    assert_eq!(scancode_to_code(0x48, true), KeyCode::ArrowUp);
    assert_eq!(scancode_to_code(0x48, false), KeyCode::Numpad8);
    assert_eq!(scancode_to_code(0x3B, false), KeyCode::F1);
    assert_eq!(scancode_to_code(0x58, false), KeyCode::F12);
    assert_eq!(scancode_to_code(0x1C, true), KeyCode::NumpadEnter);
    assert_eq!(scancode_to_code(0x1C, false), KeyCode::Enter);
    assert_eq!(scancode_to_code(0x1D, true), KeyCode::ControlRight);
    assert_eq!(scancode_to_code(0x02, false), KeyCode::Digit1);
    assert_eq!(scancode_to_code(0x0B, false), KeyCode::Digit0);
}

#[test]
fn ime_process_key_is_not_sent_to_the_pane() {
    let us = Layout::load("00000409");
    // VK_PROCESSKEY: the IME owns this keystroke; it must not become a KeyEvent.
    assert!(KeyTranslator::default().on_keydown(0xE5, 0x1E, false, false, &state(&[]), us.hkl).is_none());
}
