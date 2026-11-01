//! Ghostty keybind syntax: `mods+key[>mods+key...]=action[:param]`.
use wraith_proto::Mods;

#[derive(Clone, Debug, PartialEq)]
pub struct Keybind {
    /// More than one trigger means a sequence.
    pub trigger: Vec<Trigger>,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Trigger {
    pub mods: Mods,
    pub key: TriggerKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TriggerKey {
    /// A key by Ghostty name: `enter`, `page_up`, `equal`, `f1`, ...
    Named(String),
    /// A single character, lowercased.
    Char(char),
    /// A layout-independent W3C `KeyboardEvent.code`, written `[KeyA]`.
    Physical(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Copy,
    Paste,
    IncreaseFontSize(f32),
    DecreaseFontSize(f32),
    ResetFontSize,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,
    ClearScreen,
    ReloadConfig,
    ToggleFrameOverlay,
    CloseSurface,
    Unbind,
}

pub fn parse_keybind(value: &str) -> Result<Keybind, String> {
    // Split on the last `=` so `ctrl+equal` style names never collide; `=` itself is written `equal`.
    let (trig, action) = value.rsplit_once('=').ok_or_else(|| format!("expected `trigger=action`, got `{value}`"))?;
    let trigger = trig.split('>').map(parse_trigger).collect::<Result<Vec<_>, _>>()?;
    Ok(Keybind { trigger, action: parse_action(action.trim())? })
}

fn parse_trigger(s: &str) -> Result<Trigger, String> {
    let parts: Vec<&str> = s.trim().split('+').map(str::trim).collect();
    let (key, mod_parts) = parts.split_last().filter(|(k, _)| !k.is_empty()).ok_or_else(|| format!("empty trigger `{s}`"))?;
    let mut mods = 0u8;
    for m in mod_parts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Mods::CTRL,
            "shift" => Mods::SHIFT,
            "alt" | "opt" | "option" => Mods::ALT,
            "super" | "cmd" | "command" | "win" | "meta" => Mods::SUPER,
            other => return Err(format!("unknown modifier `{other}`")),
        };
    }
    let key = if let Some(code) = key.strip_prefix('[').and_then(|k| k.strip_suffix(']')) {
        TriggerKey::Physical(code.to_string())
    } else {
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => TriggerKey::Char(c.to_ascii_lowercase()),
            _ => TriggerKey::Named(key.to_ascii_lowercase()),
        }
    };
    Ok(Trigger { mods: Mods(mods), key })
}

fn parse_action(s: &str) -> Result<Action, String> {
    let (name, param) = match s.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (s, None),
    };
    let amount = || -> Result<f32, String> {
        param.map_or(Ok(1.0), |p| p.parse().map_err(|_| format!("invalid parameter `{p}`")))
    };
    Ok(match name {
        "copy_to_clipboard" => Action::Copy,
        "paste_from_clipboard" => Action::Paste,
        "increase_font_size" => Action::IncreaseFontSize(amount()?),
        "decrease_font_size" => Action::DecreaseFontSize(amount()?),
        "reset_font_size" => Action::ResetFontSize,
        "scroll_page_up" => Action::ScrollPageUp,
        "scroll_page_down" => Action::ScrollPageDown,
        "scroll_to_top" => Action::ScrollToTop,
        "scroll_to_bottom" => Action::ScrollToBottom,
        "clear_screen" => Action::ClearScreen,
        "reload_config" => Action::ReloadConfig,
        "toggle_frame_overlay" => Action::ToggleFrameOverlay,
        "close_surface" => Action::CloseSurface,
        "unbind" => Action::Unbind,
        _ => return Err(format!("unknown action `{name}`")),
    })
}

pub fn defaults() -> Vec<Keybind> {
    [
        "ctrl+shift+c=copy_to_clipboard",
        "ctrl+shift+v=paste_from_clipboard",
        "ctrl+equal=increase_font_size:1",
        "ctrl+minus=decrease_font_size:1",
        "ctrl+zero=reset_font_size",
        "shift+page_up=scroll_page_up",
        "shift+page_down=scroll_page_down",
        "ctrl+shift+home=scroll_to_top",
        "ctrl+shift+end=scroll_to_bottom",
        "ctrl+shift+comma=reload_config",
        "ctrl+shift+f12=toggle_frame_overlay",
        "ctrl+shift+w=close_surface",
    ]
    .iter()
    .map(|s| parse_keybind(s).expect("default keybind"))
    .collect()
}

/// Apply one `keybind = ...` value: `clear`, an unbind, or a (re)binding.
pub fn apply(list: &mut Vec<Keybind>, value: &str) -> Result<(), String> {
    if value.is_empty() {
        *list = defaults();
        return Ok(());
    }
    if value == "clear" {
        list.clear();
        return Ok(());
    }
    let kb = parse_keybind(value)?;
    list.retain(|k| k.trigger != kb.trigger);
    if kb.action != Action::Unbind {
        list.push(kb);
    }
    Ok(())
}
