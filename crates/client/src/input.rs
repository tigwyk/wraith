//! Win32 keyboard and IME → `wraith_proto::KeyEvent`.
//! We translate keys ourselves (no `TranslateMessage`), so dead-key state lives in exactly one place.
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, GCS_COMPSTR, GCS_RESULTSTR, ImmGetCompositionStringW,
    ImmGetContext, ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionWindow,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HKL, ToUnicodeEx, VK_CAPITAL, VK_PROCESSKEY, VK_CONTROL, VK_LCONTROL, VK_LWIN, VK_MENU, VK_NUMLOCK, VK_RMENU, VK_RWIN, VK_SHIFT,
};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};

#[derive(Default)]
pub struct KeyTranslator {
    _private: (),
}

fn down(s: &[u8; 256], vk: u16) -> bool {
    s[vk as usize] & 0x80 != 0
}

fn mods(s: &[u8; 256]) -> Mods {
    let mut m = 0;
    if down(s, VK_SHIFT.0) {
        m |= Mods::SHIFT;
    }
    if down(s, VK_CONTROL.0) {
        m |= Mods::CTRL;
    }
    if down(s, VK_MENU.0) {
        m |= Mods::ALT;
    }
    if down(s, VK_LWIN.0) || down(s, VK_RWIN.0) {
        m |= Mods::SUPER;
    }
    if s[VK_CAPITAL.0 as usize] & 1 != 0 {
        m |= Mods::CAPS;
    }
    if s[VK_NUMLOCK.0 as usize] & 1 != 0 {
        m |= Mods::NUM;
    }
    Mods(m)
}

/// Text `vk` produces with `state` on `hkl`. `Err(())` = a dead key was stored for the next key.
fn to_unicode(vk: u16, scancode: u16, state: &[u8; 256], hkl: HKL) -> Result<Option<String>, ()> {
    let mut buf = [0u16; 8];
    // Flags 0: let the kernel keep dead-key state; we are its only caller (no TranslateMessage).
    let n = unsafe { ToUnicodeEx(u32::from(vk), u32::from(scancode), state, &mut buf, 0, Some(hkl)) };
    match n {
        n if n < 0 => Err(()),
        0 => Ok(None),
        n => Ok(Some(String::from_utf16_lossy(&buf[..n as usize]))),
    }
}

impl KeyTranslator {
    /// `None` for a dead key (its text arrives with the next key) or a key that produces nothing to send.
    pub fn on_keydown(&mut self, vk: u16, scancode: u16, extended: bool, repeat: bool, state: &[u8; 256], hkl: HKL) -> Option<KeyEvent> {
        if vk == VK_PROCESSKEY.0 {
            return None; // The IME owns it; text arrives as WM_IME_COMPOSITION.
        }
        let code = scancode_to_code(scancode, extended);
        let mut m = mods(state);
        // Windows reports AltGr as LCtrl + RAlt.
        let altgr = down(state, VK_RMENU.0) && down(state, VK_LCONTROL.0);
        let text = match to_unicode(vk, scancode, state, hkl) {
            Err(()) => return None,
            Ok(t) => t,
        };
        let text = match text {
            Some(t) if t.chars().any(char::is_control) => None,
            Some(t) if altgr => {
                m.0 &= !(Mods::CTRL | Mods::ALT);
                Some(t)
            }
            // Ctrl(+Shift)+key: the encoder derives control codes from key + mods.
            Some(_) if m.has(Mods::CTRL) => None,
            t => t,
        };
        Some(KeyEvent { code, mods: m, text, action: if repeat { KeyAction::Repeat } else { KeyAction::Press }, composing: false })
    }

    pub fn on_keyup(&mut self, _vk: u16, scancode: u16, extended: bool, state: &[u8; 256]) -> KeyEvent {
        KeyEvent { code: scancode_to_code(scancode, extended), mods: mods(state), text: None, action: KeyAction::Release, composing: false }
    }
}

/// Physical key from a set-1 scancode (layout independent).
pub fn scancode_to_code(sc: u16, extended: bool) -> KeyCode {
    use KeyCode as K;
    if extended {
        return match sc {
            0x1C => K::NumpadEnter,
            0x1D => K::ControlRight,
            0x35 => K::NumpadDivide,
            0x38 => K::AltRight,
            0x47 => K::Home,
            0x48 => K::ArrowUp,
            0x49 => K::PageUp,
            0x4B => K::ArrowLeft,
            0x4D => K::ArrowRight,
            0x4F => K::End,
            0x50 => K::ArrowDown,
            0x51 => K::PageDown,
            0x52 => K::Insert,
            0x53 => K::Delete,
            0x5B => K::MetaLeft,
            0x5C => K::MetaRight,
            0x5D => K::ContextMenu,
            0x45 => K::NumLock,
            _ => K::Unidentified,
        };
    }
    const ROW: [KeyCode; 0x59] = {
        let mut t = [K::Unidentified; 0x59];
        t[0x01] = K::Escape;
        t[0x02] = K::Digit1;
        t[0x03] = K::Digit2;
        t[0x04] = K::Digit3;
        t[0x05] = K::Digit4;
        t[0x06] = K::Digit5;
        t[0x07] = K::Digit6;
        t[0x08] = K::Digit7;
        t[0x09] = K::Digit8;
        t[0x0A] = K::Digit9;
        t[0x0B] = K::Digit0;
        t[0x0C] = K::Minus;
        t[0x0D] = K::Equal;
        t[0x0E] = K::Backspace;
        t[0x0F] = K::Tab;
        t[0x10] = K::KeyQ;
        t[0x11] = K::KeyW;
        t[0x12] = K::KeyE;
        t[0x13] = K::KeyR;
        t[0x14] = K::KeyT;
        t[0x15] = K::KeyY;
        t[0x16] = K::KeyU;
        t[0x17] = K::KeyI;
        t[0x18] = K::KeyO;
        t[0x19] = K::KeyP;
        t[0x1A] = K::BracketLeft;
        t[0x1B] = K::BracketRight;
        t[0x1C] = K::Enter;
        t[0x1D] = K::ControlLeft;
        t[0x1E] = K::KeyA;
        t[0x1F] = K::KeyS;
        t[0x20] = K::KeyD;
        t[0x21] = K::KeyF;
        t[0x22] = K::KeyG;
        t[0x23] = K::KeyH;
        t[0x24] = K::KeyJ;
        t[0x25] = K::KeyK;
        t[0x26] = K::KeyL;
        t[0x27] = K::Semicolon;
        t[0x28] = K::Quote;
        t[0x29] = K::Backquote;
        t[0x2A] = K::ShiftLeft;
        t[0x2B] = K::Backslash;
        t[0x2C] = K::KeyZ;
        t[0x2D] = K::KeyX;
        t[0x2E] = K::KeyC;
        t[0x2F] = K::KeyV;
        t[0x30] = K::KeyB;
        t[0x31] = K::KeyN;
        t[0x32] = K::KeyM;
        t[0x33] = K::Comma;
        t[0x34] = K::Period;
        t[0x35] = K::Slash;
        t[0x36] = K::ShiftRight;
        t[0x37] = K::NumpadMultiply;
        t[0x38] = K::AltLeft;
        t[0x39] = K::Space;
        t[0x3A] = K::CapsLock;
        t[0x3B] = K::F1;
        t[0x3C] = K::F2;
        t[0x3D] = K::F3;
        t[0x3E] = K::F4;
        t[0x3F] = K::F5;
        t[0x40] = K::F6;
        t[0x41] = K::F7;
        t[0x42] = K::F8;
        t[0x43] = K::F9;
        t[0x44] = K::F10;
        t[0x45] = K::NumLock;
        t[0x47] = K::Numpad7;
        t[0x48] = K::Numpad8;
        t[0x49] = K::Numpad9;
        t[0x4A] = K::NumpadSubtract;
        t[0x4B] = K::Numpad4;
        t[0x4C] = K::Numpad5;
        t[0x4D] = K::Numpad6;
        t[0x4E] = K::NumpadAdd;
        t[0x4F] = K::Numpad1;
        t[0x50] = K::Numpad2;
        t[0x51] = K::Numpad3;
        t[0x52] = K::Numpad0;
        t[0x53] = K::NumpadDecimal;
        t[0x56] = K::IntlBackslash;
        t[0x57] = K::F11;
        t[0x58] = K::F12;
        t
    };
    match sc {
        0x64..=0x6E => [K::F13, K::F14, K::F15, K::F16, K::F17, K::F18, K::F19, K::F20, K::F21, K::F22, K::F23][(sc - 0x64) as usize],
        0x76 => K::F24,
        _ => ROW.get(sc as usize).copied().unwrap_or(K::Unidentified),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImeUpdate {
    Preedit(String),
    Commit(String),
    None,
}

#[derive(Default)]
pub struct Ime;

impl Ime {
    /// Handle `WM_IME_COMPOSITION`.
    pub fn on_composition(&mut self, hwnd: HWND, lparam: LPARAM) -> ImeUpdate {
        unsafe {
            let ctx = ImmGetContext(hwnd);
            if ctx.is_invalid() {
                return ImeUpdate::None;
            }
            let read = |kind| {
                let bytes = ImmGetCompositionStringW(ctx, kind, None, 0);
                if bytes <= 0 {
                    return String::new();
                }
                let mut buf = vec![0u16; bytes as usize / 2];
                ImmGetCompositionStringW(ctx, kind, Some(buf.as_mut_ptr().cast()), bytes as u32);
                String::from_utf16_lossy(&buf)
            };
            let r = if lparam.0 as u32 & GCS_RESULTSTR.0 != 0 {
                ImeUpdate::Commit(read(GCS_RESULTSTR))
            } else if lparam.0 as u32 & GCS_COMPSTR.0 != 0 {
                ImeUpdate::Preedit(read(GCS_COMPSTR))
            } else {
                ImeUpdate::None
            };
            let _ = ImmReleaseContext(hwnd, ctx);
            r
        }
    }

    /// Keep the composition and candidate windows at the terminal cursor.
    pub fn set_candidate_pos(hwnd: HWND, x_px: i32, y_px: i32, cell_h: i32) {
        unsafe {
            let ctx = ImmGetContext(hwnd);
            if ctx.is_invalid() {
                return;
            }
            let comp = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x: x_px, y: y_px }, rcArea: RECT::default() };
            let _ = ImmSetCompositionWindow(ctx, &comp);
            let cand = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_EXCLUDE,
                ptCurrentPos: POINT { x: x_px, y: y_px },
                rcArea: RECT { left: x_px, top: y_px, right: x_px + 1, bottom: y_px + cell_h },
            };
            let _ = ImmSetCandidateWindow(ctx, &cand);
            let _ = ImmReleaseContext(hwnd, ctx);
        }
    }
}
