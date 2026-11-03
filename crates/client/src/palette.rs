//! Command palette and workspace picker (spec §4.2): fuzzy matching and the list state. Pure; the
//! window draws it and routes keys here while it is open.
use wraith_config::{Action, Keybind, TriggerKey, WorkspaceDef, all_actions};
use wraith_proto::model::{ModelSnapshot, WorkspaceId};

/// Subsequence match of `query` in `text`, case-insensitive. Higher is better: consecutive letters and
/// word starts score more, gaps cost a little. Empty query matches everything with 0.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut qi, mut score, mut prev) = (0, 0i32, None::<usize>);
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += 1;
            match prev {
                Some(p) if p + 1 == i => score += 8,
                Some(p) => score -= (i - p - 1) as i32,
                None => {}
            }
            if i == 0 || !t[i - 1].is_alphanumeric() {
                score += 6;
            }
            prev = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Palette,
    Picker,
    MoveTab,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Action(Action),
    Workspace(WorkspaceId),
    /// An index into the configured `workspace =` list.
    ConfiguredWorkspace(usize),
    Directory(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    /// The keybinding (palette), or where it lives (picker).
    pub detail: String,
    pub group: &'static str,
    pub choice: Choice,
}

/// Rows the palette shows at once.
pub const SHOWN: usize = 12;

pub struct Palette {
    pub mode: Mode,
    pub query: String,
    items: Vec<Item>,
    pub selected: usize,
    /// The first row on screen; it moves only to keep the selection visible, so hovering never scrolls.
    pub top: usize,
}

impl Palette {
    pub fn new(mode: Mode, items: Vec<Item>) -> Palette {
        Palette { mode, query: String::new(), items, selected: 0, top: 0 }
    }

    /// The rows that match the query, best first (original order for an empty query).
    pub fn visible(&self) -> Vec<&Item> {
        let mut scored: Vec<(i32, usize, &Item)> =
            self.items.iter().enumerate().filter_map(|(i, it)| fuzzy(&self.query, &it.label).map(|s| (s, i, it))).collect();
        scored.sort_by_key(|&(s, i, _)| (-s, i));
        scored.into_iter().map(|(_, _, it)| it).collect()
    }

    fn query_changed(&mut self) {
        self.selected = 0;
        self.top = 0;
    }

    fn keep_visible(&mut self) {
        let n = self.visible().len();
        self.top = self.top.min(self.selected).max((self.selected + 1).saturating_sub(SHOWN)).min(n.saturating_sub(SHOWN));
    }

    /// Select row `i` of `visible()` (clamped).
    pub fn select(&mut self, i: usize) {
        self.selected = i.min(self.visible().len().saturating_sub(1));
        self.keep_visible();
    }

    /// Move the selection `rows` down (negative: up), as the wheel does.
    pub fn scroll(&mut self, rows: i32) {
        self.select(self.selected.saturating_add_signed(rows as isize));
    }

    pub fn type_text(&mut self, t: &str) {
        self.query.extend(t.chars().filter(|c| !c.is_control()));
        self.query_changed();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.query_changed();
    }

    pub fn up(&mut self) {
        self.scroll(-1);
    }

    pub fn down(&mut self) {
        self.scroll(1);
    }

    pub fn accept(&self) -> Option<Choice> {
        self.visible().get(self.selected).map(|i| i.choice.clone())
    }
}

/// `Ctrl+Shift+T`, or `Ctrl+A > C` for a sequence.
pub fn trigger_text(kb: &Keybind) -> String {
    use wraith_proto::Mods;
    kb.trigger
        .iter()
        .map(|t| {
            let mut s = String::new();
            for (bit, name) in [(Mods::CTRL, "Ctrl"), (Mods::ALT, "Alt"), (Mods::SHIFT, "Shift"), (Mods::SUPER, "Super")] {
                if t.mods.has(bit) {
                    s.push_str(name);
                    s.push('+');
                }
            }
            match &t.key {
                TriggerKey::Char(c) => s.extend(c.to_uppercase()),
                TriggerKey::Physical(code) => s.push_str(code),
                TriggerKey::Named(n) => s.push_str(match n.as_str() {
                    "plus" => "+".to_string(),
                    "minus" => "-".to_string(),
                    "equal" => "=".to_string(),
                    "comma" => ",".to_string(),
                    "period" => ".".to_string(),
                    other => other
                        .split('_')
                        .map(|w| {
                            let mut c = w.chars();
                            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                }
                .as_str()),
            }
            s
        })
        .collect::<Vec<_>>()
        .join(" > ")
}

fn group_of(a: Action) -> (usize, &'static str) {
    use Action::*;
    match a {
        NewWorkspace | CloseWorkspace | NextWorkspace | PreviousWorkspace | WorkspacePicker => (0, "Workspaces"),
        NewTab | CloseTab | NextTab | PreviousTab | GotoTab(_) | MoveTabToWorkspace | NewTabProfile(_) | OpenNewTabMenu => (1, "Tabs"),
        NewSplit(_) | GotoSplit(_) | ToggleSplitZoom | ResizeSplit(..) | EqualizeSplits | CloseSurface | Find => (2, "Panes"),
        _ => (3, "Window"),
    }
}

fn humanize(name: &str) -> String {
    let s = name.replace(['_', ':', ','], " ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Every named action with its current keybinding, grouped Workspaces / Tabs / Panes / Window.
pub fn palette_items(binds: &[Keybind]) -> Vec<Item> {
    let mut items: Vec<(usize, Item)> = all_actions()
        .into_iter()
        .map(|(a, name)| {
            let (rank, group) = group_of(a);
            let detail = binds.iter().find(|k| k.action == a).map(trigger_text).unwrap_or_default();
            (rank, Item { label: humanize(name), detail, group, choice: Choice::Action(a) })
        })
        .collect();
    items.sort_by_key(|(rank, _)| *rank);
    items.into_iter().map(|(_, i)| i).collect()
}

fn last_component(path: &str) -> String {
    path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// Open workspaces, then configured ones not open yet, then directories recently used by a pane.
pub fn picker_items(snap: &ModelSnapshot, configured: &[WorkspaceDef]) -> Vec<Item> {
    let mut items = Vec::new();
    for w in &snap.workspaces {
        let detail = if w.attached { "open in a window" } else { "open" };
        items.push(Item { label: w.name.clone(), detail: detail.into(), group: "Open", choice: Choice::Workspace(w.id) });
    }
    for (i, def) in configured.iter().enumerate() {
        if !snap.workspaces.iter().any(|w| w.name == def.name) {
            items.push(Item { label: def.name.clone(), detail: def.dir.clone().unwrap_or_default(), group: "Configured", choice: Choice::ConfiguredWorkspace(i) });
        }
    }
    let mut seen = std::collections::HashSet::new();
    for dir in snap.panes.iter().filter_map(|p| p.cwd.as_deref()) {
        if seen.insert(dir.to_lowercase().trim_end_matches(['\\', '/']).to_string()) {
            items.push(Item { label: last_component(dir), detail: dir.to_string(), group: "Recent", choice: Choice::Directory(dir.to_string()) });
        }
    }
    items
}

/// Targets for `move_tab_to_workspace`: every workspace but the current one.
pub fn move_tab_items(snap: &ModelSnapshot, current: WorkspaceId) -> Vec<Item> {
    snap.workspaces
        .iter()
        .filter(|w| w.id != current)
        .map(|w| Item { label: w.name.clone(), detail: String::new(), group: "Workspaces", choice: Choice::Workspace(w.id) })
        .collect()
}
