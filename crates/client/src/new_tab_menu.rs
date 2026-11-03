//! The `⌄` menu of the new-tab button (wt-profiles spec §5.2): one row per profile, then Settings and
//! the command palette. Pure view state, owned by `WindowView` like the palette.
use wraith_config::{Action, Keybind};
use wraith_proto::model::Rect;
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};
use wraith_server::profile::Profile;

use crate::palette::trigger_text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuRow {
    Profile { index: usize, name: String, icon: Option<String>, binding: Option<String>, default: bool },
    Separator,
    /// With its binding text.
    Settings(Option<String>),
    Palette(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOutcome {
    /// A new tab with profile `i`.
    Open(usize),
    /// Split the focused pane with profile `i`.
    Split(usize),
    Settings,
    Palette,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuHit {
    Row(usize),
    /// Inside the menu, not on a row.
    Panel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuLayout {
    pub panel: Rect,
    /// Shown rows: index into `rows`, and where.
    pub rows: Vec<(usize, Rect)>,
}

pub struct NewTabMenu {
    pub rows: Vec<MenuRow>,
    pub selected: usize,
}

/// Vertical padding inside the panel, top and bottom.
const PAD: i32 = 4;

impl NewTabMenu {
    pub fn new(profiles: &[Profile], default: &str, keybinds: &[Keybind]) -> NewTabMenu {
        let bound = |a: Action| keybinds.iter().find(|k| k.action == a).map(trigger_text);
        let mut rows: Vec<MenuRow> = profiles
            .iter()
            .enumerate()
            .map(|(i, p)| MenuRow::Profile {
                index: i,
                name: p.name.clone(),
                icon: p.icon.clone(),
                binding: u8::try_from(i + 1).ok().and_then(|n| bound(Action::NewTabProfile(n))),
                default: p.name.eq_ignore_ascii_case(default),
            })
            .collect();
        let selected = rows.iter().position(|r| matches!(r, MenuRow::Profile { default: true, .. })).unwrap_or(0);
        rows.extend([MenuRow::Separator, MenuRow::Settings(bound(Action::OpenSettings)), MenuRow::Palette(bound(Action::CommandPalette))]);
        NewTabMenu { rows, selected }
    }

    fn selectable(&self, i: usize) -> bool {
        self.rows.get(i).is_some_and(|r| *r != MenuRow::Separator)
    }

    /// Move the selection `dir` (±1) to the next selectable row, wrapping.
    fn step(&mut self, dir: isize) {
        let n = self.rows.len() as isize;
        let mut i = self.selected as isize;
        for _ in 0..n {
            i = (i + dir).rem_euclid(n);
            if self.selectable(i as usize) {
                self.selected = i as usize;
                return;
            }
        }
    }

    fn outcome(&self, i: usize, alt: bool) -> Option<MenuOutcome> {
        Some(match self.rows.get(i)? {
            MenuRow::Profile { index, .. } if alt => MenuOutcome::Split(*index),
            MenuRow::Profile { index, .. } => MenuOutcome::Open(*index),
            MenuRow::Separator => return None,
            MenuRow::Settings(_) => MenuOutcome::Settings,
            MenuRow::Palette(_) => MenuOutcome::Palette,
        })
    }

    /// A key while the menu is open. Every key is the menu's; `None` means it only moved the selection.
    pub fn key(&mut self, ev: &KeyEvent) -> Option<MenuOutcome> {
        if ev.action == KeyAction::Release {
            return None;
        }
        match ev.code {
            KeyCode::ArrowDown => self.step(1),
            KeyCode::ArrowUp => self.step(-1),
            KeyCode::Home => self.selected = (0..self.rows.len()).find(|&i| self.selectable(i)).unwrap_or(0),
            KeyCode::End => self.selected = (0..self.rows.len()).rev().find(|&i| self.selectable(i)).unwrap_or(0),
            KeyCode::Enter | KeyCode::NumpadEnter => return self.outcome(self.selected, ev.mods.has(Mods::ALT)),
            KeyCode::Escape => return Some(MenuOutcome::Close),
            _ => {}
        }
        None
    }

    /// The pointer moved onto row `i`: the selection follows it, as in menus (the separator is skipped).
    pub fn hover(&mut self, i: usize) {
        if self.selectable(i) {
            self.selected = i;
        }
    }

    pub fn click(&mut self, hit: &MenuHit, alt: bool) -> Option<MenuOutcome> {
        let MenuHit::Row(i) = *hit else { return None };
        if self.selectable(i) {
            self.selected = i;
        }
        self.outcome(i, alt)
    }

    fn label_cols(r: &MenuRow) -> usize {
        let n = |s: &str| s.chars().count();
        match r {
            MenuRow::Profile { name, binding, .. } => 3 + n(name) + binding.as_deref().map_or(0, |b| 3 + n(b)),
            MenuRow::Separator => 0,
            MenuRow::Settings(b) => 3 + n("Settings") + b.as_deref().map_or(0, |b| 3 + n(b)),
            MenuRow::Palette(b) => 3 + n("Command palette") + b.as_deref().map_or(0, |b| 3 + n(b)),
        }
    }

    /// Below `anchor` (the new-tab button), moved left to stay inside the window, and scrolled so
    /// the selection shows when the rows do not all fit.
    pub fn layout(&self, anchor: Rect, client: (i32, i32), cell: (i32, i32), bar_h: i32) -> MenuLayout {
        let (cw, row_h, sep_h) = (cell.0, cell.1 + 6, 7);
        let cols = self.rows.iter().map(Self::label_cols).max().unwrap_or(0) as i32 + 2;
        let w = (cols * cw).max(16 * cw).min(client.0 - 2 * cw).max(0);
        let x = anchor.x.min(client.0 - cw - w).max(0);
        let room = client.1 - bar_h - cell.1 - 2 * PAD;
        let fit = ((room / row_h).max(1) as usize).min(self.rows.len());
        let top = (self.selected + 1).saturating_sub(fit);
        let mut y = bar_h + PAD;
        let mut rows = Vec::new();
        for (i, r) in self.rows.iter().enumerate().skip(top) {
            let h = if *r == MenuRow::Separator { sep_h } else { row_h };
            if y + h > bar_h + PAD + room {
                break;
            }
            rows.push((i, Rect { x, y, w, h }));
            y += h;
        }
        MenuLayout { panel: Rect { x, y: bar_h, w, h: y - bar_h + PAD }, rows }
    }

    pub fn hit(l: &MenuLayout, x: i32, y: i32) -> Option<MenuHit> {
        if !l.panel.contains(x, y) {
            return None;
        }
        Some(l.rows.iter().find(|(_, r)| r.contains(x, y)).map_or(MenuHit::Panel, |(i, _)| MenuHit::Row(*i)))
    }
}

/// What a menu outcome asks the app to do. A split is not an `Action`: it carries a profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuDo {
    Action(Action),
    /// Split the focused pane with profile `i`.
    Split(usize),
}

pub fn menu_action(o: MenuOutcome) -> Option<MenuDo> {
    match o {
        MenuOutcome::Open(i) => u8::try_from(i + 1).ok().map(|n| MenuDo::Action(Action::NewTabProfile(n))),
        MenuOutcome::Split(i) => Some(MenuDo::Split(i)),
        MenuOutcome::Settings => Some(MenuDo::Action(Action::OpenSettings)),
        MenuOutcome::Palette => Some(MenuDo::Action(Action::CommandPalette)),
        MenuOutcome::Close => None,
    }
}

/// The stand-in icon for a profile without a drawable one: its name's first letter or digit, on a
/// colour picked from the name so it stays the same from run to run.
pub fn letter_tile(name: &str) -> (char, (u8, u8, u8)) {
    const COLORS: [(u8, u8, u8); 8] =
        [(0x3b, 0x82, 0xf6), (0x10, 0xb9, 0x81), (0xf5, 0x9e, 0x0b), (0xef, 0x44, 0x44), (0x8b, 0x5c, 0xf6), (0x06, 0xb6, 0xd4), (0xec, 0x48, 0x99), (0x84, 0xcc, 0x16)];
    let letter = name.chars().find(|c| c.is_alphanumeric()).map_or('?', |c| c.to_uppercase().next().unwrap_or(c));
    // FNV-1a: stable across runs, unlike std's hasher.
    let h = name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    (letter, COLORS[h as usize % COLORS.len()])
}
