//! `wraith_proto` input → libghostty-vt key/mouse types.
use libghostty_vt::key::{self, Key};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};

/// Fill `ev` (a reusable libghostty event) from `e`.
pub fn to_ghostty_key(e: &KeyEvent, ev: &mut key::Event<'_>) {
    ev.set_key(code(e.code))
        .set_mods(mods(e.mods))
        .set_action(match e.action {
            KeyAction::Press => key::Action::Press,
            KeyAction::Repeat => key::Action::Repeat,
            KeyAction::Release => key::Action::Release,
        })
        .set_composing(e.composing)
        // Control characters must not be passed as text; the encoder derives them from key + mods.
        .set_utf8(e.text.as_deref().filter(|t| !t.chars().any(char::is_control)));
}

pub fn mods(m: Mods) -> key::Mods {
    let mut out = key::Mods::empty();
    for (bit, flag) in [
        (Mods::SHIFT, key::Mods::SHIFT),
        (Mods::CTRL, key::Mods::CTRL),
        (Mods::ALT, key::Mods::ALT),
        (Mods::SUPER, key::Mods::SUPER),
        (Mods::CAPS, key::Mods::CAPS_LOCK),
        (Mods::NUM, key::Mods::NUM_LOCK),
    ] {
        if m.has(bit) {
            out |= flag;
        }
    }
    out
}

pub fn code(c: KeyCode) -> Key {
    use KeyCode as K;
    match c {
        K::KeyA => Key::A,
        K::KeyB => Key::B,
        K::KeyC => Key::C,
        K::KeyD => Key::D,
        K::KeyE => Key::E,
        K::KeyF => Key::F,
        K::KeyG => Key::G,
        K::KeyH => Key::H,
        K::KeyI => Key::I,
        K::KeyJ => Key::J,
        K::KeyK => Key::K,
        K::KeyL => Key::L,
        K::KeyM => Key::M,
        K::KeyN => Key::N,
        K::KeyO => Key::O,
        K::KeyP => Key::P,
        K::KeyQ => Key::Q,
        K::KeyR => Key::R,
        K::KeyS => Key::S,
        K::KeyT => Key::T,
        K::KeyU => Key::U,
        K::KeyV => Key::V,
        K::KeyW => Key::W,
        K::KeyX => Key::X,
        K::KeyY => Key::Y,
        K::KeyZ => Key::Z,
        K::Digit0 => Key::Digit0,
        K::Digit1 => Key::Digit1,
        K::Digit2 => Key::Digit2,
        K::Digit3 => Key::Digit3,
        K::Digit4 => Key::Digit4,
        K::Digit5 => Key::Digit5,
        K::Digit6 => Key::Digit6,
        K::Digit7 => Key::Digit7,
        K::Digit8 => Key::Digit8,
        K::Digit9 => Key::Digit9,
        K::Enter => Key::Enter,
        K::Escape => Key::Escape,
        K::Backspace => Key::Backspace,
        K::Tab => Key::Tab,
        K::Space => Key::Space,
        K::ArrowUp => Key::ArrowUp,
        K::ArrowDown => Key::ArrowDown,
        K::ArrowLeft => Key::ArrowLeft,
        K::ArrowRight => Key::ArrowRight,
        K::Home => Key::Home,
        K::End => Key::End,
        K::PageUp => Key::PageUp,
        K::PageDown => Key::PageDown,
        K::Insert => Key::Insert,
        K::Delete => Key::Delete,
        K::F1 => Key::F1,
        K::F2 => Key::F2,
        K::F3 => Key::F3,
        K::F4 => Key::F4,
        K::F5 => Key::F5,
        K::F6 => Key::F6,
        K::F7 => Key::F7,
        K::F8 => Key::F8,
        K::F9 => Key::F9,
        K::F10 => Key::F10,
        K::F11 => Key::F11,
        K::F12 => Key::F12,
        K::F13 => Key::F13,
        K::F14 => Key::F14,
        K::F15 => Key::F15,
        K::F16 => Key::F16,
        K::F17 => Key::F17,
        K::F18 => Key::F18,
        K::F19 => Key::F19,
        K::F20 => Key::F20,
        K::F21 => Key::F21,
        K::F22 => Key::F22,
        K::F23 => Key::F23,
        K::F24 => Key::F24,
        K::Numpad0 => Key::Numpad0,
        K::Numpad1 => Key::Numpad1,
        K::Numpad2 => Key::Numpad2,
        K::Numpad3 => Key::Numpad3,
        K::Numpad4 => Key::Numpad4,
        K::Numpad5 => Key::Numpad5,
        K::Numpad6 => Key::Numpad6,
        K::Numpad7 => Key::Numpad7,
        K::Numpad8 => Key::Numpad8,
        K::Numpad9 => Key::Numpad9,
        K::NumpadEnter => Key::NumpadEnter,
        K::NumpadAdd => Key::NumpadAdd,
        K::NumpadSubtract => Key::NumpadSubtract,
        K::NumpadMultiply => Key::NumpadMultiply,
        K::NumpadDivide => Key::NumpadDivide,
        K::NumpadDecimal => Key::NumpadDecimal,
        K::Minus => Key::Minus,
        K::Equal => Key::Equal,
        K::BracketLeft => Key::BracketLeft,
        K::BracketRight => Key::BracketRight,
        K::Backslash => Key::Backslash,
        K::Semicolon => Key::Semicolon,
        K::Quote => Key::Quote,
        K::Backquote => Key::Backquote,
        K::Comma => Key::Comma,
        K::Period => Key::Period,
        K::Slash => Key::Slash,
        K::IntlBackslash => Key::IntlBackslash,
        K::ShiftLeft => Key::ShiftLeft,
        K::ShiftRight => Key::ShiftRight,
        K::ControlLeft => Key::ControlLeft,
        K::ControlRight => Key::ControlRight,
        K::AltLeft => Key::AltLeft,
        K::AltRight => Key::AltRight,
        K::MetaLeft => Key::MetaLeft,
        K::MetaRight => Key::MetaRight,
        K::CapsLock => Key::CapsLock,
        K::NumLock => Key::NumLock,
        K::ContextMenu => Key::ContextMenu,
        K::Unidentified => Key::Unidentified,
    }
}
