//! The workspace/tab/pane model shared by server and client (spec §4.1): ids, the split tree
//! with its pixel layout, and the snapshot the server mirrors to a window.
use serde::{Deserialize, Serialize};

use crate::pane::SpawnOpts;
use crate::wire::PaneId;

pub type WorkspaceId = u64;
pub type TabId = u64;

/// `Horizontal`: `a` left of `b`. `Vertical`: `a` above `b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaneNode {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, a: Box<PaneNode>, b: Box<PaneNode> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removal {
    NotFound,
    /// The target is the only pane: the caller closes the whole tab.
    Last,
    /// Removed; `focus` is the pane that takes its place in visual order.
    Removed { focus: PaneId },
}

/// A draggable split line. `path` selects the split from the root (`false` = `a`, `true` = `b`).
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub path: Vec<bool>,
    pub axis: Axis,
    pub parent: Rect,
    pub line: Rect,
}

const MIN_RATIO: f32 = 0.05;

fn clamp_ratio(r: f32) -> f32 {
    r.clamp(MIN_RATIO, 1.0 - MIN_RATIO)
}

impl PaneNode {
    pub fn leaves(&self) -> Vec<PaneId> {
        match self {
            PaneNode::Leaf(id) => vec![*id],
            PaneNode::Split { a, b, .. } => {
                let mut v = a.leaves();
                v.extend(b.leaves());
                v
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            PaneNode::Leaf(l) => *l == id,
            PaneNode::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Replace leaf `target` with a split holding it and `new` (ratio 0.5).
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis, new_first: bool) -> bool {
        match self {
            PaneNode::Leaf(id) if *id == target => {
                let (a, b) = if new_first { (new, target) } else { (target, new) };
                *self = PaneNode::Split { axis, ratio: 0.5, a: Box::new(PaneNode::Leaf(a)), b: Box::new(PaneNode::Leaf(b)) };
                true
            }
            PaneNode::Leaf(_) => false,
            PaneNode::Split { a, b, .. } => a.split(target, new, axis, new_first) || b.split(target, new, axis, new_first),
        }
    }

    pub fn remove(&mut self, target: PaneId) -> Removal {
        let PaneNode::Split { a, b, .. } = self else {
            return if matches!(self, PaneNode::Leaf(id) if *id == target) { Removal::Last } else { Removal::NotFound };
        };
        let sibling_of = |keep: &mut Box<PaneNode>| std::mem::replace(&mut **keep, PaneNode::Leaf(0));
        if matches!(**a, PaneNode::Leaf(id) if id == target) {
            let rest = sibling_of(b);
            let focus = rest.leaves()[0];
            *self = rest;
            return Removal::Removed { focus };
        }
        if matches!(**b, PaneNode::Leaf(id) if id == target) {
            let rest = sibling_of(a);
            let focus = *rest.leaves().last().unwrap();
            *self = rest;
            return Removal::Removed { focus };
        }
        match a.remove(target) {
            Removal::NotFound => b.remove(target),
            r => r,
        }
    }

    /// Pixel rectangles of every leaf, in leaf order. Panes keep at least `min` while the area allows it.
    pub fn layout(&self, area: Rect, gap: i32, min: (i32, i32)) -> Vec<(PaneId, Rect)> {
        let (mut leaves, mut divs) = (Vec::new(), Vec::new());
        self.walk(area, gap, min, &mut Vec::new(), &mut leaves, &mut divs);
        leaves
    }

    pub fn dividers(&self, area: Rect, gap: i32, min: (i32, i32)) -> Vec<Divider> {
        let (mut leaves, mut divs) = (Vec::new(), Vec::new());
        self.walk(area, gap, min, &mut Vec::new(), &mut leaves, &mut divs);
        divs
    }

    fn walk(&self, area: Rect, gap: i32, min: (i32, i32), path: &mut Vec<bool>, leaves: &mut Vec<(PaneId, Rect)>, divs: &mut Vec<Divider>) {
        let area = Rect { w: area.w.max(0), h: area.h.max(0), ..area };
        let PaneNode::Split { axis, ratio, a, b } = self else {
            let PaneNode::Leaf(id) = self else { unreachable!() };
            leaves.push((*id, area));
            return;
        };
        let (total, min_side) = match axis {
            Axis::Horizontal => (area.w, min.0),
            Axis::Vertical => (area.h, min.1),
        };
        let usable = (total - gap).max(0);
        let first = if usable >= 2 * min_side {
            ((usable as f32 * ratio).round() as i32).clamp(min_side, usable - min_side)
        } else {
            min_side.min(usable)
        };
        let second = usable - first;
        let (ra, rb, line) = match axis {
            Axis::Horizontal => (
                Rect { w: first, ..area },
                Rect { x: area.x + (first + gap).min(total), w: second, ..area },
                Rect { x: area.x + first, w: gap.min(total), ..area },
            ),
            Axis::Vertical => (
                Rect { h: first, ..area },
                Rect { y: area.y + (first + gap).min(total), h: second, ..area },
                Rect { y: area.y + first, h: gap.min(total), ..area },
            ),
        };
        divs.push(Divider { path: path.clone(), axis: *axis, parent: area, line });
        path.push(false);
        a.walk(ra, gap, min, path, leaves, divs);
        path.pop();
        path.push(true);
        b.walk(rb, gap, min, path, leaves, divs);
        path.pop();
    }

    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) -> bool {
        match (self, path.split_first()) {
            (PaneNode::Split { ratio: r, .. }, None) => {
                *r = clamp_ratio(ratio);
                true
            }
            (PaneNode::Split { a, b, .. }, Some((&to_b, rest))) => (if to_b { b } else { a }).set_ratio(rest, ratio),
            (PaneNode::Leaf(_), _) => false,
        }
    }

    /// Move the nearest enclosing split on `dir`'s axis: Right/Down grow `a`, Left/Up shrink it.
    pub fn nudge(&mut self, target: PaneId, dir: Dir, delta: f32) -> bool {
        let want = if matches!(dir, Dir::Left | Dir::Right) { Axis::Horizontal } else { Axis::Vertical };
        let signed = if matches!(dir, Dir::Right | Dir::Down) { delta } else { -delta };
        let PaneNode::Split { axis, ratio, a, b } = self else { return false };
        if !a.contains(target) && !b.contains(target) {
            return false;
        }
        // Nearest first: a deeper split on the same axis wins over this one.
        if a.nudge(target, dir, delta) || b.nudge(target, dir, delta) {
            return true;
        }
        if *axis == want {
            *ratio = clamp_ratio(*ratio + signed);
            return true;
        }
        false
    }

    pub fn equalize(&mut self) {
        if let PaneNode::Split { ratio, a, b, .. } = self {
            *ratio = 0.5;
            a.equalize();
            b.equalize();
        }
    }
}

/// The neighbor of `from` in direction `dir`: the nearest rectangle beyond its edge that overlaps it on the other axis.
pub fn focus_neighbor(rects: &[(PaneId, Rect)], from: PaneId, dir: Dir) -> Option<PaneId> {
    let f = rects.iter().find(|(id, _)| *id == from)?.1;
    let overlap = |lo1: i32, hi1: i32, lo2: i32, hi2: i32| hi1.min(hi2) - lo1.max(lo2);
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|&(id, r)| {
            let (dist, ov) = match dir {
                Dir::Right => (r.x - (f.x + f.w), overlap(f.y, f.y + f.h, r.y, r.y + r.h)),
                Dir::Left => (f.x - (r.x + r.w), overlap(f.y, f.y + f.h, r.y, r.y + r.h)),
                Dir::Down => (r.y - (f.y + f.h), overlap(f.x, f.x + f.w, r.x, r.x + r.w)),
                Dir::Up => (f.y - (r.y + r.h), overlap(f.x, f.x + f.w, r.x, r.x + r.w)),
            };
            (dist >= 0 && ov > 0).then_some((id, dist, ov))
        })
        .min_by_key(|&(_, dist, ov)| (dist, -ov))
        .map(|(id, ..)| id)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root_dir: Option<String>,
    pub color: u32,
    pub tabs: Vec<TabId>,
    pub active_tab: Option<TabId>,
    pub profile: String,
    /// A window currently shows this workspace.
    pub attached: bool,
    /// Monotonic stamp of the last attach/detach; the highest detached one is reattached first.
    pub last_used: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub workspace: WorkspaceId,
    pub title_override: Option<String>,
    pub root: PaneNode,
    pub focused: PaneId,
    pub zoomed: Option<PaneId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneMeta {
    pub id: PaneId,
    pub title: String,
    pub cwd: Option<String>,
    pub exit: Option<u32>,
    pub profile: String,
    pub cmdline: String,
    /// The coding agent running here, if any (live only; never saved; absent in older layout files).
    #[serde(default)]
    pub agent: Option<AgentInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Working,
    /// Waiting for the user to answer.
    Asking,
    /// Finished its turn.
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Harness id (`claude`, `codex`, ...).
    pub harness: String,
    pub name: String,
    pub icon: char,
    pub state: AgentState,
    /// The user should look at this pane.
    pub attention: bool,
    /// Sanitized message from the agent.
    pub message: String,
    /// Model clock when `state` last changed: lower is older.
    pub since: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelSnapshot {
    pub rev: u64,
    pub workspaces: Vec<Workspace>,
    pub tabs: Vec<Tab>,
    pub panes: Vec<PaneMeta>,
}

impl ModelSnapshot {
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn pane(&self, id: PaneId) -> Option<&PaneMeta> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// The tab's name: its override, else the focused pane's title, else "shell".
    pub fn tab_title(&self, id: TabId) -> String {
        let Some(t) = self.tab(id) else { return "shell".into() };
        t.title_override
            .clone()
            .or_else(|| self.pane(t.focused).map(|p| p.title.clone()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "shell".into())
    }
}

/// Edits a window asks the server to make (spec §3.4 `Command`). New panes carry what the client
/// resolved for them: profile command line, cwd, environment, colors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModelCmd {
    NewWorkspace { name: Option<String>, root_dir: Option<String>, profile: String, layout: String, spawn: SpawnOpts },
    CloseWorkspace(WorkspaceId),
    RenameWorkspace(WorkspaceId, String),
    NewTab { workspace: WorkspaceId, profile: String, spawn: SpawnOpts },
    CloseTab(TabId),
    SelectTab(TabId),
    MoveTab { tab: TabId, to: WorkspaceId, index: Option<usize> },
    RenameTab(TabId, Option<String>),
    Split { pane: PaneId, axis: Axis, new_first: bool, profile: String, spawn: SpawnOpts },
    ClosePane(PaneId),
    FocusPane(PaneId),
    SetRatio { tab: TabId, path: Vec<bool>, ratio: f32 },
    NudgeSplit { pane: PaneId, dir: Dir, delta: f32 },
    Equalize(TabId),
    ToggleZoom(TabId),
}
