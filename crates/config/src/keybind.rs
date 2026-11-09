//! Ghostty keybind syntax: `mods+key[>mods+key...]=action[:param]`.
use wraith_proto::Mods;
use wraith_proto::model::Dir;

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
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    /// 1-based.
    GotoTab(u8),
    NewSplit(Dir),
    GotoSplit(SplitTarget),
    ToggleSplitZoom,
    ResizeSplit(Dir, u16),
    EqualizeSplits,
    NewWorkspace,
    CloseWorkspace,
    RenameWorkspace,
    NextWorkspace,
    PreviousWorkspace,
    CommandPalette,
    /// Search the focused pane's scrollback.
    Find,
    WorkspacePicker,
    MoveTabToWorkspace,
    ToggleSidebar,
    NewWindow,
    OpenSettings,
    /// 1-based index into the profile list.
    NewTabProfile(u8),
    OpenNewTabMenu,
    ImportWtProfiles,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplitTarget {
    Previous,
    Next,
    Dir(Dir),
}

fn parse_dir(s: &str) -> Result<Dir, String> {
    Ok(match s {
        "left" => Dir::Left,
        "right" => Dir::Right,
        "up" => Dir::Up,
        "down" => Dir::Down,
        _ => return Err(format!("invalid direction `{s}`")),
    })
}

impl Action {
    /// The config spelling without its parameter.
    pub fn name(&self) -> &'static str {
        match self {
            Action::Copy => "copy_to_clipboard",
            Action::Paste => "paste_from_clipboard",
            Action::IncreaseFontSize(_) => "increase_font_size",
            Action::DecreaseFontSize(_) => "decrease_font_size",
            Action::ResetFontSize => "reset_font_size",
            Action::ScrollPageUp => "scroll_page_up",
            Action::ScrollPageDown => "scroll_page_down",
            Action::ScrollToTop => "scroll_to_top",
            Action::ScrollToBottom => "scroll_to_bottom",
            Action::ClearScreen => "clear_screen",
            Action::ReloadConfig => "reload_config",
            Action::ToggleFrameOverlay => "toggle_frame_overlay",
            Action::CloseSurface => "close_surface",
            Action::Unbind => "unbind",
            Action::NewTab => "new_tab",
            Action::CloseTab => "close_tab",
            Action::NextTab => "next_tab",
            Action::PreviousTab => "previous_tab",
            Action::GotoTab(_) => "goto_tab",
            Action::NewSplit(_) => "new_split",
            Action::GotoSplit(_) => "goto_split",
            Action::ToggleSplitZoom => "toggle_split_zoom",
            Action::ResizeSplit(..) => "resize_split",
            Action::EqualizeSplits => "equalize_splits",
            Action::NewWorkspace => "new_workspace",
            Action::CloseWorkspace => "close_workspace",
            Action::RenameWorkspace => "rename_workspace",
            Action::NextWorkspace => "next_workspace",
            Action::PreviousWorkspace => "previous_workspace",
            Action::CommandPalette => "command_palette",
            Action::Find => "find",
            Action::WorkspacePicker => "workspace_picker",
            Action::MoveTabToWorkspace => "move_tab_to_workspace",
            Action::ToggleSidebar => "toggle_sidebar",
            Action::NewWindow => "new_window",
            Action::OpenSettings => "open_settings",
            Action::NewTabProfile(_) => "new_tab_profile",
            Action::OpenNewTabMenu => "open_new_tab_menu",
            Action::ImportWtProfiles => "import_windows_terminal_profiles",
        }
    }
}

/// One entry per palette row: the action and its full config spelling (`new_split:right`).
pub fn all_actions() -> Vec<(Action, &'static str)> {
    use Action::*;
    vec![
        (NewWorkspace, "new_workspace"),
        (CloseWorkspace, "close_workspace"),
        (RenameWorkspace, "rename_workspace"),
        (NextWorkspace, "next_workspace"),
        (PreviousWorkspace, "previous_workspace"),
        (WorkspacePicker, "workspace_picker"),
        (NewTab, "new_tab"),
        (OpenNewTabMenu, "open_new_tab_menu"),
        (CloseTab, "close_tab"),
        (NextTab, "next_tab"),
        (PreviousTab, "previous_tab"),
        (GotoTab(1), "goto_tab:1"),
        (MoveTabToWorkspace, "move_tab_to_workspace"),
        (NewSplit(Dir::Right), "new_split:right"),
        (NewSplit(Dir::Down), "new_split:down"),
        (NewSplit(Dir::Left), "new_split:left"),
        (NewSplit(Dir::Up), "new_split:up"),
        (GotoSplit(SplitTarget::Previous), "goto_split:previous"),
        (GotoSplit(SplitTarget::Next), "goto_split:next"),
        (GotoSplit(SplitTarget::Dir(Dir::Left)), "goto_split:left"),
        (GotoSplit(SplitTarget::Dir(Dir::Right)), "goto_split:right"),
        (GotoSplit(SplitTarget::Dir(Dir::Up)), "goto_split:up"),
        (GotoSplit(SplitTarget::Dir(Dir::Down)), "goto_split:down"),
        (ToggleSplitZoom, "toggle_split_zoom"),
        (ResizeSplit(Dir::Left, 10), "resize_split:left,10"),
        (ResizeSplit(Dir::Right, 10), "resize_split:right,10"),
        (ResizeSplit(Dir::Up, 10), "resize_split:up,10"),
        (ResizeSplit(Dir::Down, 10), "resize_split:down,10"),
        (EqualizeSplits, "equalize_splits"),
        (CloseSurface, "close_surface"),
        (NewWindow, "new_window"),
        (ToggleSidebar, "toggle_sidebar"),
        (OpenSettings, "open_settings"),
        (ImportWtProfiles, "import_windows_terminal_profiles"),
        (CommandPalette, "command_palette"),
        (Find, "find"),
        (Copy, "copy_to_clipboard"),
        (Paste, "paste_from_clipboard"),
        (IncreaseFontSize(1.0), "increase_font_size:1"),
        (DecreaseFontSize(1.0), "decrease_font_size:1"),
        (ResetFontSize, "reset_font_size"),
        (ScrollPageUp, "scroll_page_up"),
        (ScrollPageDown, "scroll_page_down"),
        (ScrollToTop, "scroll_to_top"),
        (ScrollToBottom, "scroll_to_bottom"),
        (ClearScreen, "clear_screen"),
        (ReloadConfig, "reload_config"),
        (ToggleFrameOverlay, "toggle_frame_overlay"),
    ]
}

pub fn parse_keybind(value: &str) -> Result<Keybind, String> {
    // Split on the last `=` so `ctrl+equal` style names never collide; `=` itself is written `equal`.
    let (trig, action) = value.rsplit_once('=').ok_or_else(|| format!("expected `trigger=action`, got `{value}`"))?;
    let trigger = trig.split('>').map(parse_trigger).collect::<Result<Vec<_>, _>>()?;
    Ok(Keybind { trigger, action: parse_action(action.trim())? })
}

/// The part of `keybind = global:...` after `global:`: its key, and true for `toggle_quick_terminal`,
/// false for `unbind` (quick-terminal spec §2); a global binding takes one key, not a sequence.
pub fn parse_global(value: &str) -> Result<(Trigger, bool), String> {
    let (trig, action) = value.rsplit_once('=').ok_or_else(|| format!("expected `trigger=action`, got `{value}`"))?;
    if trig.contains('>') {
        return Err("a global keybind takes one key".into());
    }
    let trigger = parse_trigger(trig)?;
    match action.trim() {
        "toggle_quick_terminal" => Ok((trigger, true)),
        "unbind" => Ok((trigger, false)),
        _ => Err("`global:` only supports toggle_quick_terminal".into()),
    }
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
        "new_tab" => Action::NewTab,
        "close_tab" => Action::CloseTab,
        "next_tab" => Action::NextTab,
        "previous_tab" => Action::PreviousTab,
        "goto_tab" => {
            let n: u8 = param.ok_or("goto_tab needs a number")?.parse().map_err(|_| "invalid tab number".to_string())?;
            if n == 0 {
                return Err("tab numbers start at 1".into());
            }
            Action::GotoTab(n)
        }
        "new_split" => Action::NewSplit(parse_dir(param.ok_or("new_split needs a direction")?)?),
        "goto_split" => Action::GotoSplit(match param.ok_or("goto_split needs a target")? {
            "previous" => SplitTarget::Previous,
            "next" => SplitTarget::Next,
            d => SplitTarget::Dir(parse_dir(d)?),
        }),
        "toggle_split_zoom" => Action::ToggleSplitZoom,
        "resize_split" => {
            let (d, n) = param.and_then(|p| p.split_once(',')).ok_or("resize_split needs `direction,amount`")?;
            Action::ResizeSplit(parse_dir(d)?, n.parse().map_err(|_| format!("invalid amount `{n}`"))?)
        }
        "equalize_splits" => Action::EqualizeSplits,
        "new_workspace" => Action::NewWorkspace,
        "close_workspace" => Action::CloseWorkspace,
        "rename_workspace" => Action::RenameWorkspace,
        "next_workspace" => Action::NextWorkspace,
        "previous_workspace" => Action::PreviousWorkspace,
        "command_palette" => Action::CommandPalette,
        "find" => Action::Find,
        "workspace_picker" => Action::WorkspacePicker,
        "move_tab_to_workspace" => Action::MoveTabToWorkspace,
        "toggle_sidebar" => Action::ToggleSidebar,
        "new_window" => Action::NewWindow,
        "open_settings" => Action::OpenSettings,
        "new_tab_profile" => {
            let n: u8 = param.ok_or("new_tab_profile needs a number")?.parse().map_err(|_| "invalid profile number".to_string())?;
            if n == 0 {
                return Err("profile numbers start at 1".into());
            }
            Action::NewTabProfile(n)
        }
        "open_new_tab_menu" => Action::OpenNewTabMenu,
        "import_windows_terminal_profiles" => Action::ImportWtProfiles,
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
        "ctrl+shift+t=new_tab",
        "ctrl+tab=next_tab",
        "ctrl+shift+tab=previous_tab",
        "alt+shift+plus=new_split:right",
        "alt+shift+minus=new_split:down",
        "alt+left=goto_split:left",
        "alt+right=goto_split:right",
        "alt+up=goto_split:up",
        "alt+down=goto_split:down",
        "ctrl+shift+b=toggle_sidebar",
        "ctrl+shift+p=command_palette",
        "ctrl+shift+f=find",
        "ctrl+shift+o=workspace_picker",
        "ctrl+shift+n=new_window",
        "ctrl+comma=open_settings",
        "ctrl+shift+space=open_new_tab_menu",
        "ctrl+shift+1=new_tab_profile:1",
        "ctrl+shift+2=new_tab_profile:2",
        "ctrl+shift+3=new_tab_profile:3",
        "ctrl+shift+4=new_tab_profile:4",
        "ctrl+shift+5=new_tab_profile:5",
        "ctrl+shift+6=new_tab_profile:6",
        "ctrl+shift+7=new_tab_profile:7",
        "ctrl+shift+8=new_tab_profile:8",
        "ctrl+shift+9=new_tab_profile:9",
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
