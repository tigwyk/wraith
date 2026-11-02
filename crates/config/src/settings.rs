//! The settings panel's rows (settings spec §2.1) as data: which key, which control, and the current
//! value as the text it would be written as. Values are interpreted only by the parser.
use crate::edit::Which;
use crate::{CloseOnExit, Config, CursorStyle, RightClick};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Appearance,
    Behaviour,
    Agents,
}

pub const PAGES: [Page; 3] = [Page::Appearance, Page::Behaviour, Page::Agents];

impl Page {
    pub fn label(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Behaviour => "Behaviour",
            Page::Agents => "Agents",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListSource {
    Themes,
    Fonts,
    Profiles,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Toggle,
    /// (value written, label shown)
    Choice(&'static [(&'static str, &'static str)]),
    Stepper { min: i32, max: i32 },
    Slider { min: f32, max: f32, step: f32 },
    List(ListSource),
    /// `agent-notify`: four toggles written as one comma list.
    NotifySet,
    /// `agent-notify-quiet`: a toggle and two HH:MM fields.
    QuietHours,
}

pub struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    pub page: Page,
    pub which: Which,
    pub kind: Kind,
    pub get: fn(&Config) -> String,
}

/// The four `agent-notify` ways, in the order they are written.
pub const NOTIFY_WAYS: [&str; 4] = ["toast", "flash", "badge", "sound"];

fn on_off(b: bool) -> String {
    b.to_string()
}

use Page::*;

const fn row(key: &'static str, label: &'static str, page: Page, kind: Kind, get: fn(&Config) -> String) -> Setting {
    Setting { key, label, page, which: Which::Last, kind, get }
}

static SETTINGS: [Setting; 20] = [
    row("theme", "Theme", Appearance, Kind::List(ListSource::Themes), |c| c.theme.clone().unwrap_or_default()),
    Setting {
        key: "font-family",
        label: "Font",
        page: Appearance,
        which: Which::First,
        kind: Kind::List(ListSource::Fonts),
        get: |c| c.font_family.first().cloned().unwrap_or_default(),
    },
    row("font-size", "Font size", Appearance, Kind::Stepper { min: 6, max: 36 }, |c| c.font_size.to_string()),
    row("background-opacity", "Background opacity", Appearance, Kind::Slider { min: 0.30, max: 1.00, step: 0.05 }, |c| format!("{:.2}", c.background_opacity)),
    row(
        "cursor-style",
        "Cursor",
        Appearance,
        Kind::Choice(&[("block", "block"), ("bar", "bar"), ("underline", "underline"), ("block_hollow", "block hollow")]),
        |c| {
            match c.cursor_style {
                CursorStyle::Block => "block",
                CursorStyle::Bar => "bar",
                CursorStyle::Underline => "underline",
                CursorStyle::BlockHollow => "block_hollow",
            }
            .into()
        },
    ),
    row("cursor-style-blink", "Cursor blink", Appearance, Kind::Toggle, |c| on_off(c.cursor_style_blink)),
    row("window-padding-x", "Padding, horizontal", Appearance, Kind::Stepper { min: 0, max: 32 }, |c| c.window_padding_x.to_string()),
    row("window-padding-y", "Padding, vertical", Appearance, Kind::Stepper { min: 0, max: 32 }, |c| c.window_padding_y.to_string()),
    row("unfocused-split-opacity", "Dim unfocused splits", Appearance, Kind::Slider { min: 0.15, max: 1.00, step: 0.05 }, |c| {
        format!("{:.2}", c.unfocused_split_opacity)
    }),
    row("copy-on-select", "Copy on select", Behaviour, Kind::Toggle, |c| on_off(c.copy_on_select)),
    row("right-click-action", "Right-click", Behaviour, Kind::Choice(&[("paste", "paste"), ("menu", "menu")]), |c| {
        match c.right_click_action {
            RightClick::Paste => "paste",
            RightClick::Menu => "menu",
        }
        .into()
    }),
    row("close-on-exit", "When a shell exits", Behaviour, Kind::Choice(&[("clean", "close if clean"), ("always", "always close"), ("never", "keep open")]), |c| {
        match c.close_on_exit {
            CloseOnExit::Clean => "clean",
            CloseOnExit::Always => "always",
            CloseOnExit::Never => "never",
        }
        .into()
    }),
    row("default-profile", "Default profile", Behaviour, Kind::List(ListSource::Profiles), |c| c.default_profile.clone().unwrap_or_default()),
    row(
        "scrollback-limit",
        "Scrollback",
        Behaviour,
        Kind::Choice(&[("10000000", "10 MB"), ("50000000", "50 MB"), ("100000000", "100 MB"), ("500000000", "500 MB")]),
        |c| c.scrollback_limit.to_string(),
    ),
    row("sidebar-visible", "Show sidebar", Behaviour, Kind::Toggle, |c| on_off(c.sidebar_visible)),
    row("sidebar-width", "Sidebar width", Behaviour, Kind::Stepper { min: 10, max: 60 }, |c| c.sidebar_width.to_string()),
    row("agent-notify", "Notify by", Agents, Kind::NotifySet, |c| {
        let n = c.agent_notify;
        let on: Vec<&str> = NOTIFY_WAYS.into_iter().zip([n.toast, n.flash, n.badge, n.sound]).filter(|(_, b)| *b).map(|(w, _)| w).collect();
        if on.is_empty() { "none".into() } else { on.join(",") }
    }),
    row("agent-notify-when", "Notify", Agents, Kind::Choice(&[("unfocused", "when unfocused"), ("always", "always")]), |c| {
        (if c.agent_notify.when_always { "always" } else { "unfocused" }).into()
    }),
    row("agent-notify-quiet", "Quiet hours", Agents, Kind::QuietHours, |c| {
        c.agent_notify.quiet.map_or_else(String::new, |(a, b)| format!("{:02}:{:02}-{:02}:{:02}", a / 60, a % 60, b / 60, b % 60))
    }),
    row("agent-tab-pulse", "Pulse tabs that need me", Agents, Kind::Toggle, |c| on_off(c.agent_tab_pulse)),
];

pub fn all() -> &'static [Setting] {
    &SETTINGS
}

/// Indexes into `all()` of the rows on page `p`.
pub fn on_page(p: Page) -> Vec<usize> {
    (0..SETTINGS.len()).filter(|&i| SETTINGS[i].page == p).collect()
}
