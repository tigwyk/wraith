//! The settings panel (settings spec §2, §5): pages, rows, lists and the quiet-hours fields. Pure
//! state like the palette; the window draws it, routes keys and clicks here, and turns the `Edit`s it
//! returns into config writes or previews.
use std::path::PathBuf;

use wraith_config::Config;
use wraith_config::edit::{Origin, Which};
use wraith_config::settings::{Kind, ListSource, NOTIFY_WAYS, PAGES, Page, Setting, all, on_page};
use wraith_proto::model::Rect;
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};

use crate::palette::fuzzy;

/// What the panel asks the window to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Set { key: &'static str, value: String, which: Which },
    Unset { key: &'static str },
    /// Show this value without saving it (moving through the theme or font list).
    Preview { key: &'static str, value: String, which: Which },
    /// Put back the config from before the preview.
    Revert,
    Close,
    /// Open the config file (`None`), or the included file a read-only row is set in.
    OpenFile(Option<PathBuf>),
}

/// The choices for the list rows, gathered when the panel opens.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub themes: Vec<String>,
    pub fonts: Vec<String>,
    pub profiles: Vec<String>,
}

/// Per row of `all()`: where its value comes from, and the parser's complaint about its line.
#[derive(Clone, Debug, PartialEq)]
pub struct RowState {
    pub origin: Origin,
    pub warning: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsHit {
    Page(Page),
    Row(usize),
    Dec(usize),
    Inc(usize),
    /// One of the four `agent-notify` toggles.
    Way(usize, u8),
    /// Quiet hours start (0) or end (1).
    Time(usize, u8),
    Reset(usize),
    /// An index into the open list's `items()`.
    ListItem(usize),
    OpenFile,
    /// Inside the panel, on nothing.
    Panel,
}

pub struct ListState {
    pub row: usize,
    pub query: String,
    pub selected: usize,
    previewed: bool,
    choices: Vec<String>,
}

impl ListState {
    /// The choices matching the query, best first (original order for an empty query).
    pub fn items(&self) -> Vec<&str> {
        let mut scored: Vec<(i32, usize, &str)> =
            self.choices.iter().enumerate().filter_map(|(i, c)| fuzzy(&self.query, c).map(|s| (s, i, c.as_str()))).collect();
        scored.sort_by_key(|&(s, i, _)| (-s, i));
        scored.into_iter().map(|(_, _, c)| c).collect()
    }
}

pub struct TimeEdit {
    pub row: usize,
    /// 0: start, 1: end.
    pub field: u8,
    pub start: String,
    pub end: String,
    /// The last Enter found the times bad or equal.
    pub invalid: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsLayout {
    pub panel: Rect,
    pub pages: Vec<(Page, Rect)>,
    /// Rows shown, as indexes into `all()`.
    pub rows: Vec<(usize, Rect)>,
    /// The open list's dropdown and its shown items (indexes into `ListState::items()`).
    pub list: Option<(Rect, Vec<(usize, Rect)>)>,
    pub open_file: Rect,
    /// Every clickable part, front first.
    pub targets: Vec<(SettingsHit, Rect)>,
}

/// Items an open list shows at once.
const LIST_SHOWN: usize = 10;

pub struct SettingsView {
    pub page: Page,
    /// An index into `all()`.
    pub focus: usize,
    list: Option<ListState>,
    time: Option<TimeEdit>,
    sources: Sources,
    rows: Vec<RowState>,
}

fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60 && s.len() == 5).then_some(h * 60 + m)
}

impl SettingsView {
    pub fn new(sources: Sources, rows: Vec<RowState>) -> SettingsView {
        let page = Page::Appearance;
        SettingsView { page, focus: on_page(page)[0], list: None, time: None, sources, rows }
    }

    pub fn set_rows(&mut self, rows: Vec<RowState>) {
        self.rows = rows;
    }

    pub fn rows(&self) -> &[RowState] {
        &self.rows
    }

    pub fn list(&self) -> Option<&ListState> {
        self.list.as_ref()
    }

    pub fn time(&self) -> Option<&TimeEdit> {
        self.time.as_ref()
    }

    pub fn read_only(&self, i: usize) -> bool {
        matches!(self.rows.get(i).map(|r| &r.origin), Some(Origin::Included { .. }))
    }

    fn set_page(&mut self, p: Page) {
        self.page = p;
        self.focus = on_page(p)[0];
        self.list = None;
        self.time = None;
    }

    fn move_focus(&mut self, by: i32) {
        let rows = on_page(self.page);
        let at = rows.iter().position(|&r| r == self.focus).unwrap_or(0);
        self.focus = rows[at.saturating_add_signed(by as isize).min(rows.len() - 1)];
    }

    /// The wheel over the panel moves the focus, as it moves the palette's selection.
    pub fn scroll(&mut self, rows: i32) {
        self.move_focus(rows);
    }

    pub fn key(&mut self, ev: &KeyEvent, cfg: &Config) -> Option<Edit> {
        if ev.action == KeyAction::Release {
            return None;
        }
        if self.list.is_some() {
            return self.list_key(ev);
        }
        if self.time.is_some() {
            return self.time_key(ev);
        }
        let shift = ev.mods.has(Mods::SHIFT);
        match ev.code {
            KeyCode::Tab => {
                let at = PAGES.iter().position(|&p| p == self.page).unwrap_or(0);
                self.set_page(PAGES[(at + if shift { PAGES.len() - 1 } else { 1 }) % PAGES.len()]);
                None
            }
            KeyCode::ArrowUp => {
                self.move_focus(-1);
                None
            }
            KeyCode::ArrowDown => {
                self.move_focus(1);
                None
            }
            KeyCode::Escape => Some(Edit::Close),
            _ if self.read_only(self.focus) => None,
            KeyCode::ArrowLeft => self.step(self.focus, -1, cfg),
            KeyCode::ArrowRight => self.step(self.focus, 1, cfg),
            KeyCode::Delete => Some(Edit::Unset { key: all()[self.focus].key }),
            KeyCode::Space => match all()[self.focus].kind {
                Kind::Toggle => self.step(self.focus, 1, cfg),
                Kind::NotifySet => self.flip_way(self.focus, 0, cfg),
                Kind::QuietHours => self.quiet_switch(self.focus, cfg),
                _ => None,
            },
            KeyCode::Enter | KeyCode::NumpadEnter => match all()[self.focus].kind {
                Kind::List(src) => {
                    self.open_list(self.focus, src, cfg);
                    None
                }
                Kind::QuietHours => {
                    self.open_time(self.focus, 0, cfg);
                    None
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn list_key(&mut self, ev: &KeyEvent) -> Option<Edit> {
        let l = self.list.as_mut()?;
        match ev.code {
            KeyCode::Escape => {
                let previewed = l.previewed;
                self.list = None;
                previewed.then_some(Edit::Revert)
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown => {
                let n = l.items().len();
                let next = if ev.code == KeyCode::ArrowUp { l.selected.saturating_sub(1) } else { (l.selected + 1).min(n.saturating_sub(1)) };
                if n == 0 || next == l.selected {
                    return None;
                }
                l.selected = next;
                let s = &all()[l.row];
                if !matches!(s.kind, Kind::List(ListSource::Themes | ListSource::Fonts)) {
                    return None;
                }
                l.previewed = true;
                Some(Edit::Preview { key: s.key, value: l.items()[next].to_string(), which: s.which })
            }
            KeyCode::Backspace => {
                l.query.pop();
                l.selected = 0;
                None
            }
            KeyCode::Enter | KeyCode::NumpadEnter => self.commit_list(),
            _ => ev.text.clone().filter(|_| !ev.mods.has(Mods::CTRL | Mods::ALT)).and_then(|t| self.text(&t)),
        }
    }

    fn commit_list(&mut self) -> Option<Edit> {
        let l = self.list.take()?;
        let s = &all()[l.row];
        let value = l.items().get(l.selected)?.to_string();
        Some(Edit::Set { key: s.key, value, which: s.which })
    }

    fn time_key(&mut self, ev: &KeyEvent) -> Option<Edit> {
        let t = self.time.as_mut()?;
        match ev.code {
            KeyCode::Escape => self.time = None,
            KeyCode::Backspace => {
                (if t.field == 0 { &mut t.start } else { &mut t.end }).pop();
            }
            KeyCode::Enter | KeyCode::NumpadEnter if t.field == 0 => t.field = 1,
            KeyCode::Enter | KeyCode::NumpadEnter => match (parse_hm(&t.start), parse_hm(&t.end)) {
                (Some(a), Some(b)) if a != b => {
                    let t = self.time.take()?;
                    return Some(Edit::Set { key: all()[t.row].key, value: format!("{}-{}", t.start, t.end), which: Which::Last });
                }
                _ => t.invalid = true,
            },
            _ => return ev.text.clone().filter(|_| !ev.mods.has(Mods::CTRL | Mods::ALT)).and_then(|s| self.text(&s)),
        }
        None
    }

    /// Typed or IME text: filters an open list, or edits the focused time field.
    pub fn text(&mut self, t: &str) -> Option<Edit> {
        if let Some(l) = &mut self.list {
            l.query.extend(t.chars().filter(|c| !c.is_control()));
            l.selected = 0;
        } else if let Some(e) = &mut self.time {
            let f = if e.field == 0 { &mut e.start } else { &mut e.end };
            for c in t.chars().filter(|c| c.is_ascii_digit() || *c == ':') {
                if f.len() < 5 {
                    f.push(c);
                }
            }
        }
        None
    }

    pub fn click(&mut self, hit: &SettingsHit, cfg: &Config) -> Option<Edit> {
        if let Some(i) = match *hit {
            SettingsHit::Row(i) | SettingsHit::Dec(i) | SettingsHit::Inc(i) | SettingsHit::Way(i, _) | SettingsHit::Time(i, _) | SettingsHit::Reset(i) => Some(i),
            _ => None,
        } {
            self.focus = i;
            if !matches!(hit, SettingsHit::Time(..)) {
                self.time = None;
            }
            if self.read_only(i) {
                return match (&self.rows[i].origin, hit) {
                    (Origin::Included { file, .. }, SettingsHit::Row(_)) => Some(Edit::OpenFile(Some(file.clone()))),
                    _ => None,
                };
            }
        }
        match *hit {
            SettingsHit::Page(p) => {
                self.set_page(p);
                None
            }
            SettingsHit::Row(i) => match all()[i].kind {
                Kind::Toggle | Kind::Choice(_) => self.step(i, 1, cfg),
                Kind::List(src) => {
                    self.open_list(i, src, cfg);
                    None
                }
                Kind::QuietHours => self.quiet_switch(i, cfg),
                _ => None,
            },
            SettingsHit::Dec(i) => self.step(i, -1, cfg),
            SettingsHit::Inc(i) => self.step(i, 1, cfg),
            SettingsHit::Way(i, w) => self.flip_way(i, w, cfg),
            SettingsHit::Time(i, f) => {
                match &mut self.time {
                    Some(t) if t.row == i => t.field = f,
                    _ => self.open_time(i, f, cfg),
                }
                None
            }
            SettingsHit::Reset(i) => Some(Edit::Unset { key: all()[i].key }),
            SettingsHit::ListItem(k) => {
                self.list.as_mut()?.selected = k;
                self.commit_list()
            }
            SettingsHit::OpenFile => Some(Edit::OpenFile(None)),
            SettingsHit::Panel => None,
        }
    }

    fn open_list(&mut self, row: usize, src: ListSource, cfg: &Config) {
        let choices = match src {
            ListSource::Themes => &self.sources.themes,
            ListSource::Fonts => &self.sources.fonts,
            ListSource::Profiles => &self.sources.profiles,
        }
        .clone();
        let cur = (all()[row].get)(cfg);
        let selected = choices.iter().position(|c| *c == cur).unwrap_or(0);
        self.list = Some(ListState { row, query: String::new(), selected, previewed: false, choices });
    }

    fn open_time(&mut self, row: usize, field: u8, cfg: &Config) {
        let cur = (all()[row].get)(cfg);
        let (start, end) = cur.split_once('-').map_or(("22:00", "07:00"), |(a, b)| (a, b));
        self.time = Some(TimeEdit { row, field, start: start.into(), end: end.into(), invalid: false });
    }

    /// Space or a click on quiet hours: on switches it off; off opens the time fields.
    fn quiet_switch(&mut self, row: usize, cfg: &Config) -> Option<Edit> {
        if (all()[row].get)(cfg).is_empty() {
            self.open_time(row, 0, cfg);
            None
        } else {
            Some(Edit::Unset { key: all()[row].key })
        }
    }

    fn flip_way(&mut self, row: usize, way: u8, cfg: &Config) -> Option<Edit> {
        let s = &all()[row];
        let cur = (s.get)(cfg);
        let on: Vec<&str> = NOTIFY_WAYS
            .iter()
            .enumerate()
            .filter(|&(k, w)| cur.split(',').any(|c| c == *w) != (k == usize::from(way)))
            .map(|(_, w)| *w)
            .collect();
        let value = if on.is_empty() { "none".to_string() } else { on.join(",") };
        Some(Edit::Set { key: s.key, value, which: s.which })
    }

    /// ←/→ (or a click): step a stepper or slider within range, move a choice (wrapping), flip a toggle.
    fn step(&mut self, row: usize, by: i32, cfg: &Config) -> Option<Edit> {
        let s: &Setting = &all()[row];
        let cur = (s.get)(cfg);
        let value = match s.kind {
            Kind::Toggle => (cur != "true").to_string(),
            Kind::Choice(cs) => {
                let n = cs.len() as i32;
                let at = cs.iter().position(|(v, _)| *v == cur).map_or(if by > 0 { -1 } else { 0 }, |p| p as i32);
                cs[(at + by).rem_euclid(n) as usize].0.to_string()
            }
            Kind::Stepper { min, max } => {
                let v = cur.parse::<f32>().map_or(min, |f| f.round() as i32);
                (v + by).clamp(min, max).to_string()
            }
            Kind::Slider { min, max, step } => {
                let v = cur.parse::<f32>().unwrap_or(max);
                format!("{:.2}", (((v / step).round() + by as f32) * step).clamp(min, max))
            }
            _ => return None,
        };
        Some(Edit::Set { key: s.key, value, which: s.which })
    }

    /// Centred under the tab bar; the pages on the left, the rows on the right, the footer below.
    pub fn layout(&self, client: (i32, i32), cell: (i32, i32), bar_h: i32) -> SettingsLayout {
        let (cw, ch) = cell;
        let row_h = ch + 10;
        let page_rows = on_page(self.page);
        let w = 640.min(client.0 - 2 * cw).max(0);
        let top = (bar_h + 12).min(client.1);
        let need = 16 + (page_rows.len() as i32 + 1) * row_h;
        let h = need.min(client.1 * 8 / 10).min(client.1 - top - 4).max(0);
        let panel = Rect { x: (client.0 - w) / 2, y: top, w, h };
        let bottom = panel.y + panel.h;
        let pw = (14 * cw).min(w / 3);
        let pages: Vec<(Page, Rect)> = PAGES
            .iter()
            .enumerate()
            .map(|(k, &p)| (p, Rect { x: panel.x + 4, y: panel.y + 8 + k as i32 * row_h, w: (pw - 8).max(0), h: row_h }))
            .filter(|(_, r)| r.y + r.h <= bottom)
            .collect();
        let (rx, rw) = (panel.x + pw, (w - pw - 8).max(0));
        let shown = (((h - 16) / row_h - 1).max(1) as usize).min(page_rows.len());
        let at = page_rows.iter().position(|&r| r == self.focus).unwrap_or(0);
        let first = at.saturating_sub(shown - 1);
        let rows: Vec<(usize, Rect)> = page_rows[first..first + shown]
            .iter()
            .enumerate()
            .map(|(k, &i)| (i, Rect { x: rx, y: panel.y + 8 + k as i32 * row_h, w: rw, h: row_h }))
            .filter(|(_, r)| r.y + r.h <= bottom)
            .collect();
        let open_file = Rect { x: rx, y: (bottom - 8 - row_h).max(panel.y), w: (20 * cw).min(rw), h: row_h.min(h) };

        let ctl_w = (24 * cw).min(rw / 2);
        let reset_w = (2 * cw).min(rw);
        let mut targets = Vec::new();
        let mut list = None;
        if let Some(l) = &self.list
            && let Some((_, r)) = rows.iter().find(|(i, _)| *i == l.row)
        {
            let n = l.items().len().min(LIST_SHOWN);
            let start = l.selected.saturating_sub(LIST_SHOWN - 1);
            let x = r.x + r.w - reset_w - ctl_w;
            let room = ((client.1 - (r.y + r.h)) / row_h).max(0) as usize;
            let items: Vec<(usize, Rect)> =
                (0..n.min(room)).map(|k| (start + k, Rect { x, y: r.y + r.h + k as i32 * row_h, w: ctl_w, h: row_h })).collect();
            targets.extend(items.iter().map(|&(k, r)| (SettingsHit::ListItem(k), r)));
            let drop = Rect { x, y: r.y + r.h, w: ctl_w, h: items.len() as i32 * row_h };
            list = Some((drop, items));
        }
        targets.extend(pages.iter().map(|&(p, r)| (SettingsHit::Page(p), r)));
        for &(i, r) in &rows {
            let ctl = Rect { x: r.x + r.w - reset_w - ctl_w, y: r.y, w: ctl_w, h: r.h };
            if !self.read_only(i) {
                let arrow = (3 * cw).min(ctl_w / 2);
                match all()[i].kind {
                    Kind::Stepper { .. } | Kind::Slider { .. } => {
                        targets.push((SettingsHit::Dec(i), Rect { w: arrow, ..ctl }));
                        targets.push((SettingsHit::Inc(i), Rect { x: ctl.x + ctl.w - arrow, w: arrow, ..ctl }));
                    }
                    Kind::NotifySet => {
                        let q = ctl.w / 4;
                        targets.extend((0..4).map(|k| (SettingsHit::Way(i, k as u8), Rect { x: ctl.x + k * q, w: q, ..ctl })));
                    }
                    Kind::QuietHours => {
                        let half = ctl.w / 2;
                        targets.push((SettingsHit::Time(i, 0), Rect { w: half, ..ctl }));
                        targets.push((SettingsHit::Time(i, 1), Rect { x: ctl.x + half, w: half, ..ctl }));
                    }
                    _ => {}
                }
                targets.push((SettingsHit::Reset(i), Rect { x: r.x + r.w - reset_w, w: reset_w, ..r }));
            }
            targets.push((SettingsHit::Row(i), r));
        }
        targets.push((SettingsHit::OpenFile, open_file));
        SettingsLayout { panel, pages, rows, list, open_file, targets }
    }

    pub fn hit(l: &SettingsLayout, x: i32, y: i32) -> Option<SettingsHit> {
        if let Some((h, _)) = l.targets.iter().find(|(_, r)| r.contains(x, y)) {
            return Some(*h);
        }
        l.panel.contains(x, y).then_some(SettingsHit::Panel)
    }
}
