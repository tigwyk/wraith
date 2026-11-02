//! The window's model of what it shows (spec §4): a mirror of the server's snapshot, the pixel
//! rectangle of every visible pane, which panes it has subscribed to, and what each action means in
//! protocol messages. Pure state, no window or GPU, so it is tested directly.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use wraith_config::{Action, CloseOnExit, Config, SplitTarget};
use wraith_proto::model::{Axis, Dir, Divider, ModelCmd, ModelSnapshot, Rect, Tab, TabId, WorkspaceId, focus_neighbor};
use wraith_proto::pane::{PaneCmd, SpawnOpts};
use wraith_proto::wire::{ClientMsg, PaneId};
use wraith_proto::{KeyAction, KeyCode, KeyEvent};
use wraith_server::profile::{Profile, process_cwd, resolve, spawn_command};

use crate::chrome::{self, Hit, HitCtx, Regions};
use crate::palette::{Choice, Mode, Palette, move_tab_items, palette_items, picker_items};
use crate::selection::Selection;

/// Pixel space of the window: its client size, the font's cell, pane padding and chrome heights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub client: (i32, i32),
    pub cell: (u32, u32),
    pub pad: (i32, i32),
    pub bar_h: i32,
    pub sidebar_cols: Option<u16>,
    /// Width the caption buttons take at the right end of the tab bar.
    pub caption_w: i32,
}

/// What a new pane needs: its profile's command line, colors and environment.
pub struct SpawnCtx {
    pub config: Config,
    pub profiles: Vec<Profile>,
    /// Folder of the shell integration scripts.
    pub scripts: PathBuf,
    pub env: Vec<(String, String)>,
}

impl SpawnCtx {
    pub fn profile(&self, name: Option<&str>) -> &Profile {
        resolve(&self.profiles, name.filter(|n| !n.is_empty()).or(self.config.default_profile.as_deref()))
    }

    pub fn spawn(&self, p: &Profile, cwd: Option<&str>, cells: (u16, u16), cell_px: (u32, u32)) -> SpawnOpts {
        let c = &self.config;
        let rgb = |p: wraith_config::Rgb| (p.0, p.1, p.2);
        SpawnOpts {
            cmdline: spawn_command(p, cwd, &self.scripts, c.shell_integration),
            cwd: process_cwd(p, cwd),
            env: self.env.clone(),
            cols: cells.0,
            rows: cells.1,
            cell_w_px: cell_px.0,
            cell_h_px: cell_px.1,
            scrollback_lines: c.scrollback_lines(cells.0),
            default_fg: rgb(c.foreground),
            default_bg: rgb(c.background),
            palette: c.palette.iter().copied().map(rgb).collect(),
            image_storage_limit: c.image_storage_limit,
        }
    }
}

/// Things an action asks the window (not the server) to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ui {
    Palette,
    Picker,
    MoveTab,
    NewWindow,
}

/// What a shell exit does (see `WindowView::on_exit`).
#[derive(Debug, PartialEq)]
pub enum ExitOut {
    Keep,
    ClosePane(Vec<ClientMsg>),
    CloseWindow(Vec<ClientMsg>),
}

/// What an action produced: messages for the server, a close awaiting the user's confirmation, or a UI request.
#[derive(Debug, Default, PartialEq)]
pub struct Out {
    pub msgs: Vec<ClientMsg>,
    pub confirm_close: Option<PaneId>,
    pub confirm_close_workspace: Option<WorkspaceId>,
    pub ui: Option<Ui>,
}

impl Out {
    fn msgs(msgs: Vec<ClientMsg>) -> Out {
        Out { msgs, ..Out::default() }
    }

    fn model(cmd: ModelCmd) -> Out {
        Out::msgs(vec![ClientMsg::Model(cmd)])
    }
}

pub struct PaneView {
    pub rect: Rect,
    pub cells: (u16, u16),
    pub mouse_tracking: bool,
    pub sel: Selection,
}

struct Drag {
    tab: TabId,
    divider: Divider,
}

/// One workspace row of the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub id: WorkspaceId,
    pub name: String,
    pub color: u32,
    /// An agent in the workspace needs input (always false until agent state exists, M4).
    pub attention: bool,
    pub current: bool,
    /// Another window shows it.
    pub elsewhere: bool,
}

/// How long the hover-revealed sidebar lingers after the pointer leaves it (spec §4.2).
const HOVER_GRACE: Duration = Duration::from_millis(300);
/// Width of the left-edge strip that reveals a hidden sidebar.
const HOVER_STRIP: i32 = 8;

#[derive(Default)]
struct Hover {
    revealed: bool,
    leave_at: Option<Instant>,
}

pub struct WindowView {
    geom: Geometry,
    snap: ModelSnapshot,
    workspace: Option<WorkspaceId>,
    pub panes: HashMap<PaneId, PaneView>,
    subscribed: Vec<PaneId>,
    /// The size last sent for each pane: hidden panes keep it (spec §4.1).
    sent_size: HashMap<PaneId, (u16, u16, u32, u32)>,
    colored: HashSet<PaneId>,
    requested_tab_for: Option<WorkspaceId>,
    dim_opacity: f32,
    double_click: Duration,
    drag: Option<Drag>,
    /// The tab being dragged along the tab bar.
    tab_drag: Option<TabId>,
    sidebar_visible: bool,
    sidebar_cols: u16,
    hover: Hover,
    /// We asked for a new workspace and will attach it when it shows up.
    awaiting_new_ws: bool,
    known_ws: HashSet<WorkspaceId>,
    configured_done: bool,
    palette: Option<Palette>,
}

impl WindowView {
    pub fn new(geom: Geometry, double_click: Duration) -> WindowView {
        WindowView {
            geom,
            snap: ModelSnapshot::default(),
            workspace: None,
            panes: HashMap::new(),
            subscribed: Vec::new(),
            sent_size: HashMap::new(),
            colored: HashSet::new(),
            requested_tab_for: None,
            dim_opacity: 1.0,
            double_click,
            drag: None,
            tab_drag: None,
            sidebar_visible: false,
            sidebar_cols: 24,
            hover: Hover::default(),
            awaiting_new_ws: false,
            known_ws: HashSet::new(),
            configured_done: false,
            palette: None,
        }
    }

    pub fn palette(&self) -> Option<&Palette> {
        self.palette.as_ref()
    }

    pub fn open_palette(&mut self, mode: Mode, ctx: &SpawnCtx) {
        let items = match mode {
            Mode::Palette => palette_items(&ctx.config.keybinds),
            Mode::Picker => picker_items(&self.snap, &ctx.config.workspaces),
            Mode::MoveTab => self.workspace.map(|w| move_tab_items(&self.snap, w)).unwrap_or_default(),
        };
        self.palette = Some(Palette::new(mode, items));
    }

    /// IME or pasted text typed into the open palette.
    pub fn palette_text(&mut self, text: &str) {
        if let Some(p) = &mut self.palette {
            p.type_text(text);
        }
    }

    /// A key while the palette may be open. `None`: not open, the key is not consumed.
    pub fn palette_key(&mut self, ev: &KeyEvent, ctx: &SpawnCtx) -> Option<Out> {
        use wraith_proto::Mods;
        let p = self.palette.as_mut()?;
        if ev.action == KeyAction::Release {
            return Some(Out::default());
        }
        match ev.code {
            KeyCode::Escape => self.palette = None,
            KeyCode::ArrowUp => p.up(),
            KeyCode::ArrowDown => p.down(),
            KeyCode::Backspace => p.backspace(),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let (choice, mode) = (p.accept(), p.mode);
                self.palette = None;
                return Some(choice.map_or_else(Out::default, |c| self.run_choice(c, mode, ctx)));
            }
            _ => {
                if let Some(t) = ev.text.as_deref().filter(|_| !ev.mods.has(Mods::CTRL | Mods::ALT)) {
                    p.type_text(t);
                }
            }
        }
        Some(Out::default())
    }

    fn run_choice(&mut self, choice: Choice, mode: Mode, ctx: &SpawnCtx) -> Out {
        let new_workspace = |this: &mut WindowView, name: Option<String>, dir: Option<String>, layout: String| {
            this.awaiting_new_ws = true;
            let prof = ctx.profile(None);
            let cwd = dir.clone().or_else(|| std::env::var("USERPROFILE").ok());
            Out::model(ModelCmd::NewWorkspace {
                name,
                root_dir: dir,
                profile: prof.name.clone(),
                layout,
                spawn: ctx.spawn(prof, cwd.as_deref(), this.est_cells(), this.cell_px()),
            })
        };
        match choice {
            Choice::Action(a) => self.action(a, ctx),
            Choice::Workspace(id) if mode == Mode::MoveTab => match self.active_tab().map(|t| t.id) {
                Some(tab) => Out::model(ModelCmd::MoveTab { tab, to: id, index: None }),
                None => Out::default(),
            },
            Choice::Workspace(id) => Out::msgs(self.workspace_click(id)),
            Choice::ConfiguredWorkspace(i) => match ctx.config.workspaces.get(i) {
                Some(def) => new_workspace(self, Some(def.name.clone()), def.dir.clone(), def.layout.clone()),
                None => Out::default(),
            },
            Choice::Directory(path) => {
                let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().map(str::to_string);
                new_workspace(self, name, Some(path), "single".into())
            }
        }
    }

    /// Dock the sidebar (or hide it); panes are resized to the space left.
    pub fn set_sidebar(&mut self, visible: bool, cols: u16) -> Vec<ClientMsg> {
        self.sidebar_visible = visible;
        self.sidebar_cols = cols;
        self.geom.sidebar_cols = visible.then_some(cols);
        self.hover = Hover::default();
        self.relayout()
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    fn sidebar_px(&self) -> i32 {
        (i32::from(self.sidebar_cols) * self.geom.cell.0 as i32).clamp(0, self.geom.client.0.max(0) / 2)
    }

    /// The hover-revealed sidebar: drawn over the panes, never resizing them.
    pub fn floating_sidebar(&self) -> Option<Rect> {
        (!self.sidebar_visible && self.hover.revealed).then(|| Rect { x: 0, y: 0, w: self.sidebar_px(), h: self.geom.client.1.max(0) })
    }

    /// The sidebar being shown, docked or floating.
    pub fn sidebar_rect(&self) -> Option<Rect> {
        self.regions().sidebar.or_else(|| self.floating_sidebar())
    }

    pub fn pointer_moved(&mut self, x: i32, _y: i32, now: Instant) {
        if self.sidebar_visible {
            return;
        }
        if x < HOVER_STRIP {
            self.hover = Hover { revealed: true, leave_at: None };
        } else if self.hover.revealed {
            self.hover.leave_at = if x < self.sidebar_px() { None } else { self.hover.leave_at.or(Some(now)) };
        }
    }

    pub fn pointer_left(&mut self, now: Instant) {
        if self.hover.revealed {
            self.hover.leave_at.get_or_insert(now);
        }
    }

    /// Hide the floating sidebar once its grace is over; true when that changed what is drawn.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.hover.leave_at {
            Some(t) if self.hover.revealed && now >= t + HOVER_GRACE => {
                self.hover = Hover::default();
                true
            }
            _ => false,
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.hover.leave_at.filter(|_| self.hover.revealed).map(|t| t + HOVER_GRACE)
    }

    pub fn sidebar_rows(&self) -> Vec<SidebarRow> {
        self.snap
            .workspaces
            .iter()
            .map(|w| SidebarRow {
                id: w.id,
                name: w.name.clone(),
                color: w.color,
                attention: false,
                current: Some(w.id) == self.workspace,
                elsewhere: w.attached && Some(w.id) != self.workspace,
            })
            .collect()
    }

    /// Row rectangles of the sidebar and its trailing "new workspace" row.
    pub fn workspace_rects(&self) -> (Vec<(WorkspaceId, Rect)>, Rect) {
        let Some(s) = self.sidebar_rect() else { return (Vec::new(), Rect::default()) };
        let cell_h = self.geom.cell.1 as i32;
        let (row_h, y0) = (cell_h + 8, 6 + cell_h + 6);
        let rows: Vec<(WorkspaceId, Rect)> =
            self.snap.workspaces.iter().enumerate().map(|(i, w)| (w.id, Rect { x: s.x, y: y0 + i as i32 * row_h, w: s.w, h: row_h })).collect();
        let new = Rect { x: s.x, y: y0 + rows.len() as i32 * row_h, w: s.w, h: row_h };
        (rows, new)
    }

    /// A click on a workspace row: show it here (taking it from another window if needed).
    pub fn workspace_click(&self, id: WorkspaceId) -> Vec<ClientMsg> {
        if self.workspace == Some(id) { Vec::new() } else { vec![ClientMsg::AttachWindow { workspace: Some(id) }] }
    }

    /// Another window took our workspace: ask for a free one (or a new one); the window stays open.
    pub fn on_workspace_taken(&mut self, ws: WorkspaceId) -> Vec<ClientMsg> {
        if self.workspace != Some(ws) {
            return Vec::new();
        }
        self.workspace = None;
        self.requested_tab_for = None;
        self.subscribed.clear();
        self.panes.clear();
        self.drag = None;
        vec![ClientMsg::AttachWindow { workspace: None }]
    }

    /// The user confirmed closing a workspace (or it had nothing running).
    pub fn close_workspace(&self, ws: WorkspaceId) -> Vec<ClientMsg> {
        vec![ClientMsg::Model(ModelCmd::CloseWorkspace(ws))]
    }

    pub fn snapshot(&self) -> &ModelSnapshot {
        &self.snap
    }

    pub fn workspace(&self) -> Option<WorkspaceId> {
        self.workspace
    }

    /// The server gave this window a workspace; nothing is subscribed on its side yet.
    pub fn set_workspace(&mut self, ws: WorkspaceId) {
        self.workspace = Some(ws);
        self.requested_tab_for = None;
        self.subscribed.clear();
        self.panes.clear();
        self.drag = None;
    }

    /// A new connection: the server knows nothing about this window.
    pub fn on_reconnect(&mut self) {
        self.workspace = None;
        self.requested_tab_for = None;
        self.subscribed.clear();
        self.sent_size.clear();
        self.colored.clear();
        self.panes.clear();
        self.drag = None;
        self.known_ws.clear();
        self.awaiting_new_ws = false;
        self.configured_done = false;
    }

    pub fn set_dim_opacity(&mut self, opacity: f32) {
        self.dim_opacity = opacity;
    }

    pub fn geometry(&self) -> Geometry {
        self.geom
    }

    pub fn regions(&self) -> Regions {
        let g = &self.geom;
        chrome::regions(g.client, (g.cell.0 as i32, g.cell.1 as i32), g.sidebar_cols, g.bar_h)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        let ws = self.snap.workspace(self.workspace?)?;
        self.snap.tab(ws.active_tab?)
    }

    pub fn focused(&self) -> Option<PaneId> {
        self.active_tab().map(|t| t.focused)
    }

    /// Smallest pane: 2x1 cells plus padding (spec §4.1).
    fn min_px(&self) -> (i32, i32) {
        let g = &self.geom;
        (2 * g.cell.0 as i32 + 2 * g.pad.0, g.cell.1 as i32 + 2 * g.pad.1)
    }

    /// Pixel rectangle of every pane of the active tab (only the zoomed pane when zoomed).
    pub fn visible(&self) -> Vec<(PaneId, Rect)> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        let content = self.regions().content;
        match tab.zoomed.filter(|z| tab.root.contains(*z)) {
            Some(z) => vec![(z, content)],
            None => tab.root.layout(content, 1, self.min_px()),
        }
    }

    pub fn dividers(&self) -> Vec<Divider> {
        match self.active_tab() {
            Some(t) if t.zoomed.is_none() => t.root.dividers(self.regions().content, 1, self.min_px()),
            _ => Vec::new(),
        }
    }

    fn cells_for(&self, r: Rect) -> (u16, u16) {
        let g = &self.geom;
        let cols = ((r.w - 2 * g.pad.0) / g.cell.0.max(1) as i32).clamp(2, i32::from(u16::MAX));
        let rows = ((r.h - 2 * g.pad.1) / g.cell.1.max(1) as i32).clamp(1, i32::from(u16::MAX));
        (cols as u16, rows as u16)
    }

    /// Recompute pane rectangles; resize the shells whose cell grid changed.
    fn relayout(&mut self) -> Vec<ClientMsg> {
        let content = self.regions().content;
        if content.w <= 0 || content.h <= 0 {
            return Vec::new(); // minimized: leave every shell at its size
        }
        let rects = self.visible();
        self.panes.retain(|id, _| rects.iter().any(|(p, _)| p == id));
        let (cw, ch) = self.geom.cell;
        let mut msgs = Vec::new();
        for (id, rect) in rects {
            let cells = self.cells_for(rect);
            let dbl = self.double_click;
            let v = self.panes.entry(id).or_insert_with(|| PaneView { rect, cells, mouse_tracking: false, sel: Selection::new(dbl) });
            v.rect = rect;
            v.cells = cells;
            let want = (cells.0, cells.1, cw, ch);
            if self.sent_size.get(&id) != Some(&want) {
                self.sent_size.insert(id, want);
                msgs.push(ClientMsg::Cmd { pane: id, cmd: PaneCmd::Resize { cols: cells.0, rows: cells.1, cell_w_px: cw, cell_h_px: ch } });
            }
        }
        msgs
    }

    /// Colors for newly visible panes (once each), then the subscription for what is visible now.
    fn sync_subscription(&mut self, ctx: &SpawnCtx, resizes: Vec<ClientMsg>) -> Vec<ClientMsg> {
        let mut visible: Vec<PaneId> = self.visible().iter().map(|(p, _)| *p).collect();
        visible.sort_unstable();
        let mut msgs = Vec::new();
        let c = &ctx.config;
        let rgb = |p: wraith_config::Rgb| (p.0, p.1, p.2);
        for &p in &visible {
            if self.colored.insert(p) {
                msgs.push(ClientMsg::Cmd {
                    pane: p,
                    cmd: PaneCmd::SetColors { fg: rgb(c.foreground), bg: rgb(c.background), palette: c.palette.iter().copied().map(rgb).collect() },
                });
            }
        }
        msgs.extend(resizes);
        if visible != self.subscribed {
            self.subscribed = visible.clone();
            msgs.push(ClientMsg::Subscribe { panes: visible });
        }
        msgs
    }

    pub fn set_geometry(&mut self, g: Geometry) -> Vec<ClientMsg> {
        self.geom = g;
        self.relayout()
    }

    pub fn on_snapshot(&mut self, snap: ModelSnapshot, ctx: &SpawnCtx) -> Vec<ClientMsg> {
        self.snap = snap;
        let live: HashSet<PaneId> = self.snap.panes.iter().map(|p| p.id).collect();
        self.sent_size.retain(|p, _| live.contains(p));
        self.colored.retain(|p| live.contains(p));
        let mut msgs = Vec::new();
        let ids: HashSet<WorkspaceId> = self.snap.workspaces.iter().map(|w| w.id).collect();
        if self.awaiting_new_ws
            && let Some(new) = ids.iter().filter(|i| !self.known_ws.contains(i)).max()
        {
            self.awaiting_new_ws = false;
            msgs.push(ClientMsg::AttachWindow { workspace: Some(*new) });
        }
        if !self.configured_done {
            self.configured_done = true;
            for def in &ctx.config.workspaces {
                if !self.snap.workspaces.iter().any(|w| w.name == def.name) {
                    let prof = ctx.profile(None);
                    msgs.push(ClientMsg::Model(ModelCmd::NewWorkspace {
                        name: Some(def.name.clone()),
                        root_dir: def.dir.clone(),
                        profile: prof.name.clone(),
                        layout: def.layout.clone(),
                        spawn: ctx.spawn(prof, def.dir.as_deref(), self.est_cells(), self.cell_px()),
                    }));
                }
            }
        }
        self.known_ws = ids;
        if let Some(ws) = self.workspace.and_then(|w| self.snap.workspace(w))
            && ws.tabs.is_empty()
        {
            if self.requested_tab_for != Some(ws.id) {
                self.requested_tab_for = Some(ws.id);
                msgs.push(self.new_tab_msg(ws.id, ctx));
            }
        } else {
            self.requested_tab_for = None;
        }
        let resizes = self.relayout();
        msgs.extend(self.sync_subscription(ctx, resizes));
        msgs
    }

    fn est_cells(&self) -> (u16, u16) {
        self.cells_for(self.regions().content)
    }

    fn cell_px(&self) -> (u32, u32) {
        self.geom.cell
    }

    fn new_tab_msg(&self, ws: WorkspaceId, ctx: &SpawnCtx) -> ClientMsg {
        let w = self.snap.workspace(ws);
        let focused_cwd = self.focused().and_then(|p| self.snap.pane(p)).and_then(|p| p.cwd.clone());
        let cwd = w.and_then(|w| w.root_dir.clone()).or(focused_cwd).or_else(|| std::env::var("USERPROFILE").ok());
        let prof = ctx.profile(w.map(|w| w.profile.as_str()));
        ClientMsg::Model(ModelCmd::NewTab { workspace: ws, profile: prof.name.clone(), spawn: ctx.spawn(prof, cwd.as_deref(), self.est_cells(), self.cell_px()) })
    }

    pub fn action(&mut self, a: Action, ctx: &SpawnCtx) -> Out {
        if a == Action::ToggleSidebar {
            let visible = !self.sidebar_visible;
            return Out::msgs(self.set_sidebar(visible, self.sidebar_cols));
        }
        let Some(ws) = self.workspace else { return Out::default() };
        let tab = self.active_tab().cloned();
        let tabs = self.snap.workspace(ws).map(|w| w.tabs.clone()).unwrap_or_default();
        let active_idx = tab.as_ref().and_then(|t| tabs.iter().position(|&x| x == t.id));
        let select = |i: usize| Out::model(ModelCmd::SelectTab(tabs[i]));
        match a {
            Action::NewTab => Out::msgs(vec![self.new_tab_msg(ws, ctx)]),
            Action::CloseTab => tab.map_or_else(Out::default, |t| Out::model(ModelCmd::CloseTab(t.id))),
            Action::NextTab | Action::PreviousTab if tabs.len() > 1 => {
                let i = active_idx.unwrap_or(0);
                select(if a == Action::NextTab { (i + 1) % tabs.len() } else { (i + tabs.len() - 1) % tabs.len() })
            }
            Action::GotoTab(n) if (1..=tabs.len()).contains(&usize::from(n)) => select(usize::from(n) - 1),
            Action::NewSplit(dir) => {
                let Some(t) = tab else { return Out::default() };
                let meta = self.snap.pane(t.focused);
                let prof = ctx.profile(meta.map(|m| m.profile.as_str()));
                let cwd = meta.and_then(|m| m.cwd.clone());
                let (axis, new_first) = match dir {
                    Dir::Right => (Axis::Horizontal, false),
                    Dir::Left => (Axis::Horizontal, true),
                    Dir::Down => (Axis::Vertical, false),
                    Dir::Up => (Axis::Vertical, true),
                };
                let cells = self.panes.get(&t.focused).map_or_else(|| self.est_cells(), |p| p.cells);
                let cells = if axis == Axis::Horizontal { (cells.0 / 2, cells.1) } else { (cells.0, cells.1 / 2) };
                Out::model(ModelCmd::Split {
                    pane: t.focused,
                    axis,
                    new_first,
                    profile: prof.name.clone(),
                    spawn: ctx.spawn(prof, cwd.as_deref(), (cells.0.max(2), cells.1.max(1)), self.cell_px()),
                })
            }
            Action::GotoSplit(target) => {
                let Some(t) = tab else { return Out::default() };
                let rects = self.visible();
                let next = match target {
                    SplitTarget::Dir(d) => focus_neighbor(&rects, t.focused, d),
                    SplitTarget::Next | SplitTarget::Previous => {
                        let ids: Vec<PaneId> = t.root.leaves();
                        let i = ids.iter().position(|&p| p == t.focused).unwrap_or(0);
                        let n = ids.len();
                        Some(ids[if target == SplitTarget::Next { (i + 1) % n } else { (i + n - 1) % n }])
                    }
                };
                next.filter(|&p| p != t.focused).map_or_else(Out::default, |p| Out::model(ModelCmd::FocusPane(p)))
            }
            Action::ToggleSplitZoom => tab.map_or_else(Out::default, |t| Out::model(ModelCmd::ToggleZoom(t.id))),
            Action::ResizeSplit(dir, amount) => {
                tab.map_or_else(Out::default, |t| Out::model(ModelCmd::NudgeSplit { pane: t.focused, dir, delta: f32::from(amount) / 200.0 }))
            }
            Action::EqualizeSplits => tab.map_or_else(Out::default, |t| Out::model(ModelCmd::Equalize(t.id))),
            Action::CloseSurface => {
                let Some(t) = tab else { return Out::default() };
                if self.pane_exit(t.focused).is_some() {
                    Out::msgs(self.close_pane(t.focused))
                } else {
                    Out { confirm_close: Some(t.focused), ..Out::default() }
                }
            }
            Action::NewWorkspace => {
                self.awaiting_new_ws = true;
                let prof = ctx.profile(None);
                let home = std::env::var("USERPROFILE").ok();
                Out::model(ModelCmd::NewWorkspace {
                    name: None,
                    root_dir: None,
                    profile: prof.name.clone(),
                    layout: "single".into(),
                    spawn: ctx.spawn(prof, home.as_deref(), self.est_cells(), self.cell_px()),
                })
            }
            Action::CloseWorkspace => {
                let running = self.snap.tabs.iter().filter(|t| t.workspace == ws).flat_map(|t| t.root.leaves()).any(|p| self.pane_exit(p).is_none());
                if running { Out { confirm_close_workspace: Some(ws), ..Out::default() } } else { Out::msgs(self.close_workspace(ws)) }
            }
            Action::NextWorkspace | Action::PreviousWorkspace if self.snap.workspaces.len() > 1 => {
                let ids: Vec<WorkspaceId> = self.snap.workspaces.iter().map(|w| w.id).collect();
                let i = ids.iter().position(|&w| w == ws).unwrap_or(0);
                let n = ids.len();
                let to = ids[if a == Action::NextWorkspace { (i + 1) % n } else { (i + n - 1) % n }];
                Out::msgs(vec![ClientMsg::AttachWindow { workspace: Some(to) }])
            }
            Action::CommandPalette => Out { ui: Some(Ui::Palette), ..Out::default() },
            Action::WorkspacePicker => Out { ui: Some(Ui::Picker), ..Out::default() },
            Action::MoveTabToWorkspace => Out { ui: Some(Ui::MoveTab), ..Out::default() },
            Action::NewWindow => Out { ui: Some(Ui::NewWindow), ..Out::default() },
            _ => Out::default(),
        }
    }

    /// The user confirmed ending a running shell (or it had already exited).
    pub fn close_pane(&self, pane: PaneId) -> Vec<ClientMsg> {
        vec![ClientMsg::Model(ModelCmd::ClosePane(pane))]
    }

    /// What a shell exit does under `mode`: nothing, close the pane, or, when it was the last pane
    /// of this window's workspace, close the window too (else the empty workspace would get a new shell).
    pub fn on_exit(&self, pane: PaneId, code: u32, mode: CloseOnExit) -> ExitOut {
        let close = match mode {
            CloseOnExit::Never => false,
            CloseOnExit::Clean => code == 0,
            CloseOnExit::Always => true,
        };
        if !close {
            return ExitOut::Keep;
        }
        let msgs = self.close_pane(pane);
        let ws = self.workspace.and_then(|w| self.snap.workspace(w));
        let panes: usize = ws.map_or(0, |w| w.tabs.iter().filter_map(|&t| self.snap.tab(t)).map(|t| t.root.leaves().len()).sum());
        let mine = ws.is_some_and(|w| w.tabs.iter().filter_map(|&t| self.snap.tab(t)).any(|t| t.root.contains(pane)));
        if mine && panes == 1 { ExitOut::CloseWindow(msgs) } else { ExitOut::ClosePane(msgs) }
    }

    pub fn pane_exit(&self, pane: PaneId) -> Option<u32> {
        self.snap.pane(pane).and_then(|p| p.exit)
    }

    /// Enter on an exited focused pane restarts its shell (spec §8).
    pub fn restart_key(&self, ev: &KeyEvent) -> Option<ClientMsg> {
        let pane = self.focused()?;
        (ev.code == KeyCode::Enter && ev.action != KeyAction::Release && self.pane_exit(pane).is_some())
            .then_some(ClientMsg::Cmd { pane, cmd: PaneCmd::Restart })
    }

    /// Input for the focused pane.
    pub fn input(&self, cmd: PaneCmd) -> Option<ClientMsg> {
        Some(ClientMsg::Cmd { pane: self.focused()?, cmd })
    }

    /// How much an unfocused pane is dimmed (0 = not at all); a lone pane never is.
    pub fn dim_alpha(&self, pane: PaneId) -> f32 {
        let Some(t) = self.active_tab() else { return 0.0 };
        if t.focused == pane || self.visible().len() < 2 {
            0.0
        } else {
            1.0 - self.dim_opacity
        }
    }

    pub fn tab_layout(&self) -> (Vec<(TabId, Rect)>, Rect) {
        let tabs: Vec<TabId> = self.workspace.and_then(|w| self.snap.workspace(w)).map(|w| w.tabs.clone()).unwrap_or_default();
        let (rects, plus) = chrome::tab_rects(tabs.len(), self.regions().tabbar, self.geom.cell.0 as i32, self.geom.caption_w);
        (tabs.into_iter().zip(rects).collect(), plus)
    }

    pub fn hit(&self, x: i32, y: i32) -> Hit {
        let mut regions = self.regions();
        regions.sidebar = self.sidebar_rect();
        let (tabs, new_tab) = self.tab_layout();
        let (workspaces, new_workspace) = self.workspace_rects();
        let panes = self.visible();
        let dividers = self.dividers();
        let c = HitCtx {
            regions: &regions,
            tabs: &tabs,
            new_tab,
            workspaces: &workspaces,
            new_workspace,
            panes: &panes,
            dividers: &dividers,
            cell_w: self.geom.cell.0 as i32,
        };
        chrome::hit(&c, x, y)
    }

    /// A press inside `pane`: focus it if it is not already.
    pub fn pane_click(&self, pane: PaneId) -> Vec<ClientMsg> {
        if self.focused() == Some(pane) { Vec::new() } else { vec![ClientMsg::Model(ModelCmd::FocusPane(pane))] }
    }

    pub fn start_divider_drag(&mut self, index: usize) {
        let (Some(tab), Some(divider)) = (self.active_tab().map(|t| t.id), self.dividers().into_iter().nth(index)) else { return };
        self.drag = Some(Drag { tab, divider });
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
        self.tab_drag = None;
    }

    pub fn start_tab_drag(&mut self, tab: TabId) {
        self.tab_drag = Some(tab);
    }

    pub fn dragging_tab(&self) -> bool {
        self.tab_drag.is_some()
    }

    /// Move the dragged tab to the slot under `x`; the bar shows it at once and the server is told.
    pub fn drag_tab(&mut self, x: i32) -> Vec<ClientMsg> {
        let (Some(tab), Some(ws)) = (self.tab_drag, self.workspace) else { return Vec::new() };
        let (rects, _) = self.tab_layout();
        let Some(from) = rects.iter().position(|(t, _)| *t == tab) else { return Vec::new() };
        let to = rects.iter().position(|(_, r)| x < r.x + r.w).unwrap_or(rects.len() - 1);
        if to == from {
            return Vec::new();
        }
        if let Some(w) = self.snap.workspaces.iter_mut().find(|w| w.id == ws) {
            w.tabs.retain(|&t| t != tab);
            w.tabs.insert(to.min(w.tabs.len()), tab);
        }
        vec![ClientMsg::Model(ModelCmd::MoveTab { tab, to: ws, index: Some(to) })]
    }

    /// Move the grabbed divider to the pointer; the panes follow immediately and the server is told.
    pub fn drag_divider(&mut self, x: i32, y: i32) -> Vec<ClientMsg> {
        let Some(d) = &self.drag else { return Vec::new() };
        let p = &d.divider;
        let ratio = match p.axis {
            Axis::Horizontal => (x - p.parent.x) as f32 / p.parent.w.max(1) as f32,
            Axis::Vertical => (y - p.parent.y) as f32 / p.parent.h.max(1) as f32,
        };
        let (tab, path) = (d.tab, p.path.clone());
        let Some(t) = self.snap.tabs.iter_mut().find(|t| t.id == tab) else { return Vec::new() };
        if !t.root.set_ratio(&path, ratio) {
            return Vec::new();
        }
        let ratio = ratio.clamp(0.05, 0.95);
        let mut msgs = vec![ClientMsg::Model(ModelCmd::SetRatio { tab, path, ratio })];
        msgs.extend(self.relayout());
        msgs
    }
}
