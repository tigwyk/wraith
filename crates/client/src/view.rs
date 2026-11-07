//! The window's model of what it shows (spec §4): a mirror of the server's snapshot, the pixel
//! rectangle of every visible pane, which panes it has subscribed to, and what each action means in
//! protocol messages. Pure state, no window or GPU, so it is tested directly.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use wraith_config::{Action, CloseOnExit, Config, Scheme, SplitTarget};
use wraith_proto::model::{AgentInfo, AgentState, Axis, Dir, Divider, ModelCmd, ModelSnapshot, Rect, Tab, TabId, WorkspaceId, focus_neighbor};
use wraith_proto::pane::{PaneCmd, SearchCmd, SpawnOpts};
use wraith_proto::wire::{ClientMsg, PaneId};
use wraith_proto::{KeyAction, KeyCode, KeyEvent};
use wraith_server::profile::{Profile, process_cwd, resolve, spawn_command};

use crate::chrome::{self, Hit, HitCtx, Regions};
use crate::font_table::FontId;
use crate::new_tab_menu::{MenuLayout, NewTabMenu};
use crate::palette::{Choice, Mode, Palette, SHOWN, move_tab_items, palette_items, picker_items, profile_items};
use crate::selection::Selection;
use crate::settings::{SettingsLayout, SettingsView};

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

type Rgb8 = (u8, u8, u8);

/// How a pane is drawn and sized (spec §3.3): its font, cell, padding and background opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub font: FontId,
    pub cell: (u32, u32),
    pub pad: (i32, i32),
    pub opacity: f32,
}

/// What a new pane needs: its profile's command line, colors and environment.
pub struct SpawnCtx {
    pub config: Config,
    pub profiles: Vec<Profile>,
    /// The colour schemes the profiles name, by name.
    pub schemes: HashMap<String, Scheme>,
    /// Folder of the shell integration scripts.
    pub scripts: PathBuf,
    pub env: Vec<(String, String)>,
}

impl SpawnCtx {
    pub fn profile(&self, name: Option<&str>) -> &Profile {
        resolve(&self.profiles, name.filter(|n| !n.is_empty()).or(self.config.default_profile.as_deref()))
    }

    /// Foreground, background and 256-colour palette for a pane of `profile`: its scheme over the
    /// global colours, or the global colours when it has none (or no longer exists).
    pub fn colors(&self, profile: &str) -> (Rgb8, Rgb8, Vec<Rgb8>) {
        let c = &self.config;
        let rgb = |p: wraith_config::Rgb| (p.0, p.1, p.2);
        let mut palette = c.palette;
        let scheme = self.profiles.iter().find(|p| p.name == profile).and_then(|p| self.schemes.get(p.scheme.as_deref()?));
        let (fg, bg) = match scheme {
            Some(s) => {
                palette[..16].copy_from_slice(&s.palette);
                (s.fg, s.bg)
            }
            None => (c.foreground, c.background),
        };
        (rgb(fg), rgb(bg), palette.iter().copied().map(rgb).collect())
    }

    pub fn set_colors(&self, profile: &str) -> PaneCmd {
        let (fg, bg, palette) = self.colors(profile);
        PaneCmd::SetColors { fg, bg, palette }
    }

    pub fn spawn(&self, p: &Profile, cwd: Option<&str>, cells: (u16, u16), cell_px: (u32, u32)) -> SpawnOpts {
        let c = &self.config;
        let (default_fg, default_bg, palette) = self.colors(&p.name);
        SpawnOpts {
            cmdline: spawn_command(p, cwd, &self.scripts, c.shell_integration),
            cwd: process_cwd(p, cwd),
            env: self.env.clone(),
            cols: cells.0,
            rows: cells.1,
            cell_w_px: cell_px.0,
            cell_h_px: cell_px.1,
            scrollback_lines: c.scrollback_lines(cells.0),
            default_fg,
            default_bg,
            palette,
            image_storage_limit: c.image_storage_limit,
            agent_detect: c.agent_detect.clone(),
        }
    }
}

/// Where a new tab of `p` starts: its `dir`, with a leading `~` meaning the Windows profile folder for
/// native shells (WSL resolves `~` itself).
fn profile_dir(p: &Profile) -> Option<String> {
    let d = p.dir.as_deref()?;
    match (&p.kind, d.strip_prefix('~')) {
        (wraith_server::profile::ShellKind::Wsl { .. }, _) | (_, None) => Some(d.to_string()),
        (_, Some(rest)) => std::env::var("USERPROFILE").ok().map(|h| format!("{h}{}", rest.replace('/', "\\"))),
    }
}

/// Things an action asks the window (not the server) to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ui {
    Palette,
    Picker,
    MoveTab,
    NewWindow,
    Settings,
    NewTabMenu,
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
    /// An action picked from the palette: `App::action` runs it, passing what it doesn't handle back to the view.
    pub app: Option<Action>,
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
    /// An agent in the workspace needs the user (rolled up from its panes).
    pub attention: bool,
    pub current: bool,
    /// Another window shows it.
    pub elsewhere: bool,
}

/// Sidebar rows: workspaces, their tabs, and the trailing "new workspace" row.
type SidebarLayout = (Vec<(WorkspaceId, Rect)>, Vec<(TabId, Rect)>, Rect);

/// The find bar: which pane it searches and what was typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Find {
    pub pane: PaneId,
    pub query: String,
}

/// Where the open palette is drawn: its panel and each shown row (index into `Palette::visible()`).
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteLayout {
    pub panel: Rect,
    pub rows: Vec<(usize, Rect)>,
}

/// One row of the sidebar's Agents list.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRow {
    pub pane: PaneId,
    pub workspace: WorkspaceId,
    pub tab: TabId,
    pub agent: AgentInfo,
}

/// How long the hover-revealed sidebar lingers after the pointer leaves it (spec §4.2).
const HOVER_GRACE: Duration = Duration::from_millis(300);
/// How long the pointer rests on a button before its tooltip shows.
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
/// Width of the left-edge strip that reveals a hidden sidebar.
const HOVER_STRIP: i32 = 8;

#[derive(Default)]
struct Hover {
    revealed: bool,
    leave_at: Option<Instant>,
}

/// A zoom delta to 0.01 pt, so fractional steps return exactly to 0 (and to one font, not near-duplicates).
pub fn round_zoom(delta: f32) -> f32 {
    let d = (delta * 100.0).round() / 100.0;
    if d.abs() < 0.005 { 0.0 } else { d }
}

/// Where a tab dragged over the sidebar would land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabDrop {
    /// A workspace's row: at the end of its tabs.
    Workspace(WorkspaceId),
    /// A tab's row: just before that tab.
    Before(TabId),
}

struct TabDrag {
    tab: TabId,
    /// Its place in the bar when the drag began, restored when the drag leaves the bar or is cancelled.
    origin: usize,
    over: Option<TabDrop>,
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
    /// The tab being dragged: along the tab bar (reorder) or over the sidebar (move tab).
    tab_drag: Option<TabDrag>,
    sidebar_visible: bool,
    sidebar_cols: u16,
    hover: Hover,
    /// We asked for a new workspace and will attach it when it shows up.
    awaiting_new_ws: bool,
    known_ws: HashSet<WorkspaceId>,
    configured_done: bool,
    palette: Option<Palette>,
    settings: Option<SettingsView>,
    menu: Option<NewTabMenu>,
    /// The open find bar (scrollback search), if any.
    find: Option<Find>,
    /// The pointer's last position in the client area, and what is under it.
    pointer: Option<(i32, i32)>,
    hovered: Option<Hit>,
    /// The chrome target a held left button went down on.
    pressed: Option<Hit>,
    /// When the pointer came to rest on its current target, and whether that target's tooltip is up.
    tip_since: Option<Instant>,
    tip_shown: bool,
    /// The window look; follows `geom` until `set_looks` is called.
    window_look: Look,
    looks_set: bool,
    profile_looks: HashMap<String, Look>,
    /// Per-pane zoom delta (points) and the look the app built for it (spec §3.4).
    zooms: HashMap<PaneId, (f32, Look)>,
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
            settings: None,
            menu: None,
            find: None,
            pointer: None,
            hovered: None,
            pressed: None,
            tip_since: None,
            tip_shown: false,
            window_look: Look { font: 0, cell: geom.cell, pad: geom.pad, opacity: 1.0 },
            looks_set: false,
            profile_looks: HashMap::new(),
            zooms: HashMap::new(),
        }
    }

    pub fn settings(&self) -> Option<&SettingsView> {
        self.settings.as_ref()
    }

    pub fn settings_mut(&mut self) -> Option<&mut SettingsView> {
        self.settings.as_mut()
    }

    pub fn open_settings(&mut self, s: SettingsView) {
        self.menu = None;
        self.settings = Some(s);
        self.rehover();
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
        self.rehover();
    }

    pub fn settings_layout(&self) -> Option<SettingsLayout> {
        self.settings.as_ref().map(|s| s.layout(self.geom.client, (self.geom.cell.0 as i32, self.geom.cell.1 as i32), self.geom.bar_h))
    }

    pub fn menu(&self) -> Option<&NewTabMenu> {
        self.menu.as_ref()
    }

    pub fn menu_mut(&mut self) -> Option<&mut NewTabMenu> {
        self.menu.as_mut()
    }

    /// Open the new-tab menu over everything else; the palette and settings close.
    pub fn open_menu(&mut self, ctx: &SpawnCtx) {
        self.palette = None;
        self.settings = None;
        self.menu = Some(NewTabMenu::new(&ctx.profiles, &ctx.profile(None).name, &ctx.config.keybinds));
        self.rehover();
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
        self.rehover();
    }

    pub fn menu_layout(&self) -> Option<MenuLayout> {
        let (_, plus) = self.tab_layout();
        self.menu.as_ref().map(|m| m.layout(plus, self.geom.client, (self.geom.cell.0 as i32, self.geom.cell.1 as i32), self.geom.bar_h))
    }

    pub fn palette(&self) -> Option<&Palette> {
        self.palette.as_ref()
    }

    pub fn open_palette(&mut self, mode: Mode, ctx: &SpawnCtx) {
        let items = match mode {
            Mode::Palette => {
                let mut items = palette_items(&ctx.config.keybinds);
                let names: Vec<String> = ctx.profiles.iter().map(|p| p.name.clone()).collect();
                let at = items.iter().rposition(|i| i.group == "Tabs").map_or(items.len(), |i| i + 1);
                items.splice(at..at, profile_items(&names, &ctx.config.keybinds));
                items
            }
            Mode::Picker => picker_items(&self.snap, &ctx.config.workspaces),
            Mode::MoveTab => self.workspace.map(|w| move_tab_items(&self.snap, w)).unwrap_or_default(),
        };
        self.menu = None;
        self.palette = Some(Palette::new(mode, items));
    }

    /// IME or pasted text typed into the open palette.
    pub fn palette_text(&mut self, text: &str) {
        if let Some(p) = &mut self.palette {
            p.type_text(text);
        }
    }

    pub fn find(&self) -> Option<&Find> {
        self.find.as_ref()
    }

    /// IME or pasted text typed into the open find bar.
    pub fn find_text(&mut self, text: &str) -> Option<Out> {
        let f = self.find.as_mut()?;
        f.query.extend(text.chars().filter(|c| !c.is_control()));
        Some(Out::msgs(vec![ClientMsg::Cmd { pane: f.pane, cmd: PaneCmd::Search(SearchCmd::Set(f.query.clone())) }]))
    }

    /// A key while the find bar may be open: typing searches, Enter/Shift+Enter step through
    /// matches, Esc closes. `None`: not open, the key is not consumed.
    pub fn find_key(&mut self, ev: &KeyEvent) -> Option<Out> {
        use wraith_proto::Mods;
        let f = self.find.as_mut()?;
        if ev.action == KeyAction::Release {
            return Some(Out::default());
        }
        let pane = f.pane;
        let cmd = match ev.code {
            KeyCode::Escape => {
                self.find = None;
                SearchCmd::Close
            }
            KeyCode::Enter | KeyCode::NumpadEnter if ev.mods.has(Mods::SHIFT) => SearchCmd::Prev,
            KeyCode::Enter | KeyCode::NumpadEnter => SearchCmd::Next,
            KeyCode::Backspace => {
                f.query.pop();
                SearchCmd::Set(f.query.clone())
            }
            _ => match ev.text.as_deref().filter(|_| !ev.mods.has(Mods::CTRL | Mods::ALT)) {
                Some(t) => return self.find_text(t),
                None => return Some(Out::default()),
            },
        };
        Some(Out::msgs(vec![ClientMsg::Cmd { pane, cmd: PaneCmd::Search(cmd) }]))
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
            KeyCode::Enter | KeyCode::NumpadEnter => return Some(self.accept_palette(ctx)),
            _ => {
                if let Some(t) = ev.text.as_deref().filter(|_| !ev.mods.has(Mods::CTRL | Mods::ALT)) {
                    p.type_text(t);
                }
            }
        }
        Some(Out::default())
    }

    /// Run the selected row and close the palette.
    fn accept_palette(&mut self, ctx: &SpawnCtx) -> Out {
        let Some(p) = self.palette.take() else { return Out::default() };
        p.accept().map_or_else(Out::default, |c| self.run_choice(c, p.mode, ctx))
    }

    /// The wheel over the open palette: `rows` down (negative: up).
    pub fn scroll_palette(&mut self, rows: i32) {
        if let Some(p) = &mut self.palette {
            p.scroll(rows);
        }
    }

    /// Centered under the tab bar, at most `SHOWN` rows: the query line, then the rows.
    pub fn palette_layout(&self) -> Option<PaletteLayout> {
        let p = self.palette.as_ref()?;
        let (cw, row_h) = (self.geom.cell.0 as i32, self.geom.cell.1 as i32 + 6);
        let client_w = self.geom.client.0;
        let n = p.visible().len().saturating_sub(p.top).min(SHOWN);
        let w = (60 * cw).min(client_w - 2 * cw).max(10 * cw);
        let panel = Rect { x: (client_w - w) / 2, y: self.geom.bar_h + 12, w, h: (n as i32 + 1) * row_h + 10 };
        let rows = (0..n).map(|k| (p.top + k, Rect { x: panel.x, y: panel.y + 5 + (k as i32 + 1) * row_h, w: panel.w, h: row_h })).collect();
        Some(PaletteLayout { panel, rows })
    }

    /// A press while the palette may be open: a row runs, the panel keeps it open, anywhere else
    /// closes it (and the press goes no further). `None`: not open.
    pub fn palette_click(&mut self, hit: &Hit, ctx: &SpawnCtx) -> Option<Out> {
        self.palette.as_ref()?;
        match hit {
            Hit::PaletteRow(i) => {
                if let Some(p) = &mut self.palette {
                    p.select(*i);
                }
                Some(self.accept_palette(ctx))
            }
            Hit::Palette => Some(Out::default()),
            _ => {
                self.palette = None;
                Some(Out::default())
            }
        }
    }

    fn run_choice(&mut self, choice: Choice, mode: Mode, ctx: &SpawnCtx) -> Out {
        let new_workspace = |this: &mut WindowView, name: Option<String>, dir: Option<String>, layout: String| {
            this.awaiting_new_ws = true;
            let prof = ctx.profile(None);
            let cwd = dir.clone().or_else(|| std::env::var("USERPROFILE").ok());
            let est = this.est_size(&prof.name);
            Out::model(ModelCmd::NewWorkspace {
                name,
                root_dir: dir,
                profile: prof.name.clone(),
                layout,
                spawn: ctx.spawn(prof, cwd.as_deref(), est.0, est.1),
            })
        };
        match choice {
            Choice::Action(a) => Out { app: Some(a), ..Out::default() },
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

    /// The pointer is at `(x, y)`: true when that changed the target under it (so it needs a redraw).
    pub fn hover_at(&mut self, x: i32, y: i32) -> bool {
        self.pointer = Some((x, y));
        let changed = self.rehover();
        // The palette follows a moving pointer, as menus do (never a still one the panel opened under).
        if changed
            && let (Some(Hit::PaletteRow(i)), Some(p)) = (&self.hovered, &mut self.palette)
        {
            p.select(*i);
        }
        if changed
            && let (Some(Hit::Menu(crate::new_tab_menu::MenuHit::Row(i))), Some(m)) = (&self.hovered, &mut self.menu)
        {
            m.hover(*i);
        }
        changed
    }

    /// Recompute the hover target after the model or the geometry changed under a still pointer.
    fn rehover(&mut self) -> bool {
        let hit = self.pointer.map(|(x, y)| self.hit(x, y));
        let changed = hit != self.hovered;
        self.hovered = hit;
        if changed {
            self.restart_tooltip();
        }
        changed
    }

    /// The pointer moved to another target (a caption button counts): the tooltip wait starts over.
    pub fn restart_tooltip(&mut self) {
        self.tip_since = Some(Instant::now());
        self.tip_shown = false;
    }

    /// The pointer has rested long enough for a tooltip.
    pub fn tooltip_due(&self, now: Instant) -> bool {
        self.tip_since.is_some_and(|t| now >= t + TOOLTIP_DELAY)
    }

    pub fn hovered(&self) -> Option<&Hit> {
        self.hovered.as_ref()
    }

    pub fn pressed(&self) -> Option<&Hit> {
        self.pressed.as_ref()
    }

    pub fn press(&mut self, hit: Hit) {
        self.pressed = Some(hit);
    }

    /// The button came up, or capture or focus was lost.
    pub fn release(&mut self) {
        self.pressed = None;
    }

    pub fn pointer_left(&mut self, now: Instant) {
        self.pointer = None;
        self.hovered = None;
        // A button released outside the window sends us nothing else.
        self.pressed = None;
        if self.hover.revealed {
            self.hover.leave_at.get_or_insert(now);
        }
    }

    /// Hide the floating sidebar once its grace is over; true when that changed what is drawn.
    pub fn tick(&mut self, now: Instant) -> bool {
        if !self.tip_shown && self.tooltip_due(now) {
            self.tip_shown = true;
            return true;
        }
        match self.hover.leave_at {
            Some(t) if self.hover.revealed && now >= t + HOVER_GRACE => {
                self.hover = Hover::default();
                true
            }
            _ => false,
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let tip = self.tip_since.filter(|_| !self.tip_shown).map(|t| t + TOOLTIP_DELAY);
        let grace = self.hover.leave_at.filter(|_| self.hover.revealed).map(|t| t + HOVER_GRACE);
        tip.into_iter().chain(grace).min()
    }

    pub fn sidebar_rows(&self) -> Vec<SidebarRow> {
        self.snap
            .workspaces
            .iter()
            .map(|w| SidebarRow {
                id: w.id,
                name: w.name.clone(),
                color: w.color,
                attention: w.tabs.iter().filter_map(|&t| self.snap.tab(t)).flat_map(|t| t.root.leaves()).any(|p| {
                    self.snap.pane(p).and_then(|m| m.agent.as_ref()).is_some_and(|a| a.attention)
                }),
                current: Some(w.id) == self.workspace,
                elsewhere: w.attached && Some(w.id) != self.workspace,
            })
            .collect()
    }

    /// Every pane with an agent: waiting ones first (oldest first), then working, then idle.
    pub fn agent_rows(&self) -> Vec<AgentRow> {
        let mut rows: Vec<AgentRow> = self
            .snap
            .panes
            .iter()
            .filter_map(|p| {
                let a = p.agent.as_ref()?;
                let tab = self.snap.tabs.iter().find(|t| t.root.contains(p.id))?;
                Some(AgentRow { pane: p.id, workspace: tab.workspace, tab: tab.id, agent: a.clone() })
            })
            .collect();
        let rank = |s: AgentState| match s {
            AgentState::Asking | AgentState::Done => 0,
            AgentState::Working => 1,
            AgentState::Idle => 2,
        };
        rows.sort_by_key(|r| (rank(r.agent.state), r.agent.since, r.pane));
        rows
    }

    /// Rows of the sidebar's Agents list, below the workspaces (after a header row).
    pub fn agent_rects(&self) -> Vec<(PaneId, Rect)> {
        let (_, new_ws) = self.workspace_rects();
        if self.sidebar_rect().is_none() {
            return Vec::new();
        }
        let row_h = new_ws.h;
        let y0 = new_ws.y + 2 * row_h + 6;
        self.agent_rows().iter().enumerate().map(|(i, r)| (r.pane, Rect { x: new_ws.x, y: y0 + i as i32 * row_h, w: new_ws.w, h: row_h })).collect()
    }

    /// The agent a tab's badge shows: the one most in need of the user.
    pub fn tab_agent(&self, tab: TabId) -> Option<&AgentInfo> {
        let t = self.snap.tab(tab)?;
        let urgency = |a: &AgentInfo| (a.attention, matches!(a.state, AgentState::Asking), matches!(a.state, AgentState::Working));
        t.root.leaves().into_iter().filter_map(|p| self.snap.pane(p)?.agent.as_ref()).max_by_key(|a| urgency(a))
    }

    /// Show `pane`: its workspace in this window, its tab selected, the pane focused (one ordered batch).
    pub fn focus_pane(&self, pane: PaneId) -> Vec<ClientMsg> {
        let Some(tab) = self.snap.tabs.iter().find(|t| t.root.contains(pane)) else { return Vec::new() };
        let mut msgs = Vec::new();
        let here = self.workspace == Some(tab.workspace);
        if !here {
            msgs.push(ClientMsg::AttachWindow { workspace: Some(tab.workspace) });
        }
        if !here || self.active_tab().map(|t| t.id) != Some(tab.id) {
            msgs.push(ClientMsg::Model(ModelCmd::SelectTab(tab.id)));
        }
        msgs.push(ClientMsg::Model(ModelCmd::FocusPane(pane)));
        msgs
    }

    /// Row rectangles of the sidebar and its trailing "new workspace" row.
    pub fn workspace_rects(&self) -> (Vec<(WorkspaceId, Rect)>, Rect) {
        let (ws, _, new) = self.sidebar_layout();
        (ws, new)
    }

    /// Each workspace's tabs, listed (indented) under its row.
    pub fn sidebar_tab_rects(&self) -> Vec<(TabId, Rect)> {
        self.sidebar_layout().1
    }

    /// The sidebar tree: a workspace row, then a row per tab of it; then "new workspace".
    fn sidebar_layout(&self) -> SidebarLayout {
        let Some(s) = self.sidebar_rect() else { return (Vec::new(), Vec::new(), Rect::default()) };
        let (cell_w, cell_h) = (self.geom.cell.0 as i32, self.geom.cell.1 as i32);
        let row_h = cell_h + 8;
        let indent = 2 * cell_w;
        let (mut y, mut ws, mut tabs) = (6 + cell_h + 6, Vec::new(), Vec::new());
        for w in &self.snap.workspaces {
            ws.push((w.id, Rect { x: s.x, y, w: s.w, h: row_h }));
            y += row_h;
            for &t in w.tabs.iter().filter(|&&t| self.snap.tab(t).is_some()) {
                tabs.push((t, Rect { x: s.x + indent, y, w: s.w - indent, h: row_h }));
                y += row_h;
            }
        }
        (ws, tabs, Rect { x: s.x, y, w: s.w, h: row_h })
    }

    /// A click on a tab in the sidebar: show it here (its workspace too, if another).
    pub fn tab_click(&self, tab: TabId) -> Vec<ClientMsg> {
        let Some(t) = self.snap.tab(tab) else { return Vec::new() };
        let here = self.workspace == Some(t.workspace);
        let mut msgs = Vec::new();
        if !here {
            msgs.push(ClientMsg::AttachWindow { workspace: Some(t.workspace) });
        }
        if !here || self.active_tab().map(|a| a.id) != Some(tab) {
            msgs.push(ClientMsg::Model(ModelCmd::SelectTab(tab)));
        }
        msgs
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
        self.zooms.clear();
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
        self.zooms.clear();
        self.drag = None;
    }

    /// A new connection: the server knows nothing about this window.
    pub fn on_reconnect(&mut self) {
        self.workspace = None;
        self.requested_tab_for = None;
        self.subscribed.clear();
        self.sent_size.clear();
        self.zooms.clear();
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

    /// Smallest pane: 2x1 cells plus padding (spec §4.1), of the largest look in the active tab.
    fn min_px(&self) -> (i32, i32) {
        let looks = self.active_tab().map(|t| t.root.leaves()).unwrap_or_default().into_iter().map(|p| self.look(p));
        looks.fold((0, 0), |(w, h), l| (w.max(2 * l.cell.0 as i32 + 2 * l.pad.0), h.max(l.cell.1 as i32 + 2 * l.pad.1)))
    }

    /// Window look plus one per profile name (zoom 0); relayout follows.
    pub fn set_looks(&mut self, window: Look, by_profile: HashMap<String, Look>) -> Vec<ClientMsg> {
        self.window_look = window;
        self.looks_set = true;
        self.profile_looks = by_profile;
        self.relayout()
    }

    /// A zoomed pane's delta (points, through `round_zoom`) and the look the app built for it; delta 0
    /// clears both.
    pub fn set_zoom(&mut self, pane: PaneId, delta: f32, look: Look) -> Vec<ClientMsg> {
        let delta = round_zoom(delta);
        if delta == 0.0 {
            self.zooms.remove(&pane);
        } else {
            self.zooms.insert(pane, (delta, look));
        }
        self.relayout()
    }

    pub fn zoom(&self, pane: PaneId) -> f32 {
        self.zooms.get(&pane).map_or(0.0, |z| z.0)
    }

    /// The pane's zoom override, else its profile's look, else the window look.
    pub fn look(&self, pane: PaneId) -> Look {
        match self.zooms.get(&pane) {
            Some(z) => z.1,
            None => self.profile_look(self.snap.pane(pane).map_or("", |m| m.profile.as_str())),
        }
    }

    pub fn profile_look(&self, profile: &str) -> Look {
        self.profile_looks.get(profile).copied().unwrap_or(self.window_look)
    }

    /// (pane, profile, delta) of every zoomed pane, for rebuilding their looks.
    pub fn zoomed_panes(&self) -> Vec<(PaneId, String, f32)> {
        let mut v: Vec<_> = self.zooms.iter().map(|(&p, z)| (p, self.snap.pane(p).map_or_else(String::new, |m| m.profile.clone()), z.0)).collect();
        v.sort_by_key(|z| z.0);
        v
    }

    /// Every font a look refers to (window, each profile, each zoom): what the font table must keep.
    pub fn used_fonts(&self) -> HashSet<FontId> {
        let zooms = self.zooms.values().map(|z| &z.1);
        [&self.window_look].into_iter().chain(self.profile_looks.values()).chain(zooms).map(|l| l.font).collect()
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

    fn cells_for(r: Rect, l: &Look) -> (u16, u16) {
        let cols = ((r.w - 2 * l.pad.0) / l.cell.0.max(1) as i32).clamp(2, i32::from(u16::MAX));
        let rows = ((r.h - 2 * l.pad.1) / l.cell.1.max(1) as i32).clamp(1, i32::from(u16::MAX));
        (cols as u16, rows as u16)
    }

    /// Recompute pane rectangles; resize the shells whose cell grid changed.
    fn relayout(&mut self) -> Vec<ClientMsg> {
        self.rehover();
        let content = self.regions().content;
        if content.w <= 0 || content.h <= 0 {
            return Vec::new(); // minimized: leave every shell at its size
        }
        let rects = self.visible();
        self.panes.retain(|id, _| rects.iter().any(|(p, _)| p == id));
        let mut msgs = Vec::new();
        for (id, rect) in rects {
            let look = self.look(id);
            let (cw, ch) = look.cell;
            let cells = Self::cells_for(rect, &look);
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
        for &p in &visible {
            if self.colored.insert(p) {
                let profile = self.snap.pane(p).map_or("", |m| m.profile.as_str());
                msgs.push(ClientMsg::Cmd { pane: p, cmd: ctx.set_colors(profile) });
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
        if !self.looks_set {
            (self.window_look.cell, self.window_look.pad) = (g.cell, g.pad);
        }
        self.relayout()
    }

    pub fn on_snapshot(&mut self, snap: ModelSnapshot, ctx: &SpawnCtx) -> Vec<ClientMsg> {
        self.snap = snap;
        let live: HashSet<PaneId> = self.snap.panes.iter().map(|p| p.id).collect();
        self.sent_size.retain(|p, _| live.contains(p));
        self.zooms.retain(|p, _| live.contains(p));
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
                    let est = self.est_size(&prof.name);
                    msgs.push(ClientMsg::Model(ModelCmd::NewWorkspace {
                        name: Some(def.name.clone()),
                        root_dir: def.dir.clone(),
                        profile: prof.name.clone(),
                        layout: def.layout.clone(),
                        spawn: ctx.spawn(prof, def.dir.as_deref(), est.0, est.1),
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
                msgs.push(self.new_tab_msg(ws.id, ctx, None));
            }
        } else {
            self.requested_tab_for = None;
        }
        let resizes = self.relayout();
        msgs.extend(self.sync_subscription(ctx, resizes));
        msgs
    }

    /// Estimated cells and cell size of a new full-content pane of `profile`.
    fn est_size(&self, profile: &str) -> ((u16, u16), (u32, u32)) {
        let l = self.profile_look(profile);
        (Self::cells_for(self.regions().content, &l), l.cell)
    }

    /// Split the focused pane toward `dir`, running `prof`, else the focused pane's profile, in its directory.
    fn split_msg(&self, dir: Dir, prof: Option<&Profile>, ctx: &SpawnCtx) -> Out {
        let Some(t) = self.active_tab() else { return Out::default() };
        let meta = self.snap.pane(t.focused);
        let prof = prof.unwrap_or_else(|| ctx.profile(meta.map(|m| m.profile.as_str())));
        let cwd = meta.and_then(|m| m.cwd.clone());
        let (axis, new_first) = match dir {
            Dir::Right => (Axis::Horizontal, false),
            Dir::Left => (Axis::Horizontal, true),
            Dir::Down => (Axis::Vertical, false),
            Dir::Up => (Axis::Vertical, true),
        };
        // Half the focused pane's pixels, in the new profile's cells (cells_for keeps at least 2x1).
        let r = self.panes.get(&t.focused).map_or_else(|| self.regions().content, |p| p.rect);
        let half = if axis == Axis::Horizontal { Rect { w: r.w / 2, ..r } } else { Rect { h: r.h / 2, ..r } };
        let look = self.profile_look(&prof.name);
        let cells = Self::cells_for(half, &look);
        Out::model(ModelCmd::Split {
            pane: t.focused,
            axis,
            new_first,
            profile: prof.name.clone(),
            spawn: ctx.spawn(prof, cwd.as_deref(), cells, look.cell),
        })
    }

    /// Split the focused pane with profile `i`, along its longer side (the menu's Alt+click).
    pub fn split_profile(&self, i: usize, ctx: &SpawnCtx) -> Out {
        let Some(p) = ctx.profiles.get(i) else { return Out::default() };
        let wide = self.focused().and_then(|f| self.panes.get(&f)).is_none_or(|v| v.rect.w >= v.rect.h);
        self.split_msg(if wide { Dir::Right } else { Dir::Down }, Some(p), ctx)
    }

    /// A new tab in `ws` running `prof`, else the workspace's profile.
    fn new_tab_msg(&self, ws: WorkspaceId, ctx: &SpawnCtx, prof: Option<&Profile>) -> ClientMsg {
        let w = self.snap.workspace(ws);
        let prof = prof.unwrap_or_else(|| ctx.profile(w.map(|w| w.profile.as_str())));
        let focused_cwd = self.focused().and_then(|p| self.snap.pane(p)).and_then(|p| p.cwd.clone());
        let cwd = profile_dir(prof)
            .or_else(|| w.and_then(|w| w.root_dir.clone()))
            .or(focused_cwd)
            .or_else(|| std::env::var("USERPROFILE").ok());
        let est = self.est_size(&prof.name);
        ClientMsg::Model(ModelCmd::NewTab { workspace: ws, profile: prof.name.clone(), spawn: ctx.spawn(prof, cwd.as_deref(), est.0, est.1) })
    }

    pub fn action(&mut self, a: Action, ctx: &SpawnCtx) -> Out {
        if a == Action::OpenSettings {
            return Out { ui: Some(Ui::Settings), ..Out::default() };
        }
        if a == Action::OpenNewTabMenu {
            return Out { ui: Some(Ui::NewTabMenu), ..Out::default() };
        }
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
            Action::NewTab => Out::msgs(vec![self.new_tab_msg(ws, ctx, None)]),
            Action::NewTabProfile(n) => match ctx.profiles.get(usize::from(n).saturating_sub(1)) {
                Some(p) => Out::msgs(vec![self.new_tab_msg(ws, ctx, Some(p))]),
                None => Out::default(),
            },
            Action::CloseTab => tab.map_or_else(Out::default, |t| Out::model(ModelCmd::CloseTab(t.id))),
            Action::NextTab | Action::PreviousTab if tabs.len() > 1 => {
                let i = active_idx.unwrap_or(0);
                select(if a == Action::NextTab { (i + 1) % tabs.len() } else { (i + tabs.len() - 1) % tabs.len() })
            }
            Action::GotoTab(n) if (1..=tabs.len()).contains(&usize::from(n)) => select(usize::from(n) - 1),
            Action::NewSplit(dir) => self.split_msg(dir, None, ctx),
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
                let est = self.est_size(&prof.name);
                Out::model(ModelCmd::NewWorkspace {
                    name: None,
                    root_dir: None,
                    profile: prof.name.clone(),
                    layout: "single".into(),
                    spawn: ctx.spawn(prof, home.as_deref(), est.0, est.1),
                })
            }
            Action::CloseWorkspace => {
                let running = self.snap.tabs.iter().filter(|t| t.workspace == ws).flat_map(|t| t.root.leaves()).any(|p| self.pane_exit(p).is_none());
                if running { Out { confirm_close_workspace: Some(ws), ..Out::default() } } else { Out::msgs(self.close_workspace(ws)) }
            }
            Action::NextWorkspace | Action::PreviousWorkspace if self.snap.workspaces.len() > 1 => {
                // The quick terminal's workspace is not on the way (quick-terminal spec §4).
                let ids: Vec<WorkspaceId> = self.snap.workspaces.iter().filter(|w| !w.quick || w.id == ws).map(|w| w.id).collect();
                let i = ids.iter().position(|&w| w == ws).unwrap_or(0);
                let n = ids.len();
                let to = ids[if a == Action::NextWorkspace { (i + 1) % n } else { (i + n - 1) % n }];
                Out::msgs(vec![ClientMsg::AttachWindow { workspace: Some(to) }])
            }
            Action::CommandPalette => Out { ui: Some(Ui::Palette), ..Out::default() },
            Action::Find => {
                if let Some(pane) = self.focused() {
                    self.find = Some(Find { pane, query: String::new() });
                }
                Out::default()
            }
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

    /// Another workspace no window shows: when ours closes, the server moves us there instead.
    pub fn has_free_workspace(&self) -> bool {
        self.snap.workspaces.iter().any(|w| Some(w.id) != self.workspace && !w.attached && !w.quick)
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

    pub fn gear_rect(&self) -> Rect {
        chrome::gear_rect(self.regions().tabbar, self.geom.caption_w, self.geom.cell.0 as i32)
    }

    pub fn tab_layout(&self) -> (Vec<(TabId, Rect)>, Rect) {
        let tabs: Vec<TabId> = self.workspace.and_then(|w| self.snap.workspace(w)).map(|w| w.tabs.clone()).unwrap_or_default();
        let (rects, plus) = chrome::tab_rects(tabs.len(), self.regions().tabbar, self.geom.cell.0 as i32, self.geom.caption_w + self.gear_rect().w);
        (tabs.into_iter().zip(rects).collect(), plus)
    }

    pub fn hit(&self, x: i32, y: i32) -> Hit {
        // The palette opens over the panel, so it is checked first.
        if let Some(l) = self.palette_layout()
            && l.panel.contains(x, y)
        {
            return l.rows.iter().find(|(_, r)| r.contains(x, y)).map_or(Hit::Palette, |(i, _)| Hit::PaletteRow(*i));
        }
        if let Some(h) = self.menu_layout().and_then(|l| NewTabMenu::hit(&l, x, y)) {
            return Hit::Menu(h);
        }
        if let Some(h) = self.settings_layout().and_then(|l| SettingsView::hit(&l, x, y)) {
            return Hit::Settings(h);
        }
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
            new_tab_menu: chrome::chevron_rect(new_tab, self.geom.cell.0 as i32),
            workspaces: &workspaces,
            new_workspace,
            agents: &self.agent_rects(),
            sidebar_tabs: &self.sidebar_tab_rects(),
            panes: &panes,
            dividers: &dividers,
            gear: self.gear_rect(),
            cell_w: self.geom.cell.0 as i32,
        };
        chrome::hit(&c, x, y)
    }

    /// A press inside `pane`: focus it if it is not already.
    /// Focus `pane` in this view now, ahead of the server's snapshot (the FocusPane is already sent),
    /// so an action run straight away (the right-click menu) targets it.
    pub fn focus_local(&mut self, pane: PaneId) {
        if let Some(t) = self.snap.tabs.iter_mut().find(|t| t.root.contains(pane)) {
            t.focused = pane;
        }
    }

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
        let origin = self.workspace.and_then(|w| self.snap.workspace(w)).and_then(|w| w.tabs.iter().position(|&t| t == tab));
        self.tab_drag = origin.map(|origin| TabDrag { tab, origin, over: None });
    }

    /// A press on a tab listed in the sidebar; only this workspace's tabs are ours to move.
    pub fn start_sidebar_tab_drag(&mut self, tab: TabId) {
        if self.snap.tab(tab).is_some_and(|t| Some(t.workspace) == self.workspace) {
            self.start_tab_drag(tab);
        }
    }

    pub fn dragging_tab(&self) -> bool {
        self.tab_drag.is_some()
    }

    /// The pointer moved during a tab drag. Over the sidebar it picks a drop target (undoing any
    /// reorder made on the way); elsewhere the tab takes the bar slot under `x`, shown at once and
    /// sent to the server.
    pub fn drag_tab(&mut self, x: i32, y: i32) -> Vec<ClientMsg> {
        let Some(tab) = self.tab_drag.as_ref().map(|d| d.tab) else { return Vec::new() };
        let quick = self.workspace.and_then(|w| self.snap.workspace(w)).is_some_and(|w| w.quick);
        if !quick && self.sidebar_rect().is_some_and(|s| s.contains(x, y)) {
            let over = self.sidebar_target(tab, x, y);
            let msgs = self.restore_tab_order();
            if let Some(d) = &mut self.tab_drag {
                d.over = over;
            }
            return msgs;
        }
        if let Some(d) = &mut self.tab_drag {
            d.over = None;
        }
        let (rects, _) = self.tab_layout();
        let to = rects.iter().position(|(_, r)| x < r.x + r.w).unwrap_or(rects.len().saturating_sub(1));
        self.place_tab(tab, to)
    }

    /// The drop target under the pointer and whether it takes the tab (for the highlight and cursor).
    pub fn tab_drop(&self) -> Option<(TabDrop, bool)> {
        let d = self.tab_drag.as_ref()?;
        d.over.map(|o| (o, self.accepts(d.tab, o)))
    }

    /// The button went up: move the tab to the target under the pointer, if it takes it.
    pub fn drop_tab(&mut self) -> Vec<ClientMsg> {
        let Some(d) = self.tab_drag.take() else { return Vec::new() };
        let Some(over) = d.over.filter(|&o| self.accepts(d.tab, o)) else { return Vec::new() };
        let (to, index) = match over {
            TabDrop::Workspace(w) => (w, None),
            TabDrop::Before(t) => {
                let Some(to) = self.snap.tab(t).map(|t| t.workspace) else { return Vec::new() };
                let tabs = self.snap.workspace(to).map(|w| w.tabs.as_slice()).unwrap_or_default();
                let at = tabs.iter().position(|&x| x == t).unwrap_or(tabs.len());
                // Within one workspace the server takes the tab out first, so later slots shift down.
                let from = tabs.iter().position(|&x| x == d.tab);
                (to, Some(if from.is_some_and(|f| f < at) { at - 1 } else { at }))
            }
        };
        vec![ClientMsg::Model(ModelCmd::MoveTab { tab: d.tab, to, index })]
    }

    /// Esc during a tab drag: put the tab back where it started.
    pub fn cancel_tab_drag(&mut self) -> Vec<ClientMsg> {
        let msgs = self.restore_tab_order();
        self.tab_drag = None;
        msgs
    }

    fn sidebar_target(&self, tab: TabId, x: i32, y: i32) -> Option<TabDrop> {
        let (rows, _, _) = self.sidebar_layout();
        if let Some((w, _)) = rows.iter().find(|(_, r)| r.contains(x, y)) {
            return Some(TabDrop::Workspace(*w));
        }
        let tabs = self.sidebar_tab_rects();
        tabs.iter().find(|(t, r)| *t != tab && r.contains(x, y)).map(|(t, _)| TabDrop::Before(*t))
    }

    /// Refused: the quick terminal's workspace, one another window shows, and our own row when the
    /// tab is its only one (it would have nowhere to go back to).
    fn accepts(&self, tab: TabId, drop: TabDrop) -> bool {
        let dest = match drop {
            TabDrop::Workspace(w) => Some(w),
            TabDrop::Before(t) => self.snap.tab(t).map(|t| t.workspace),
        };
        let Some(w) = dest.and_then(|d| self.snap.workspace(d)) else { return false };
        let here = Some(w.id) == self.workspace;
        !w.quick && (here || !w.attached) && !(here && w.tabs == [tab])
    }

    fn restore_tab_order(&mut self) -> Vec<ClientMsg> {
        match self.tab_drag.as_ref() {
            Some(d) => self.place_tab(d.tab, d.origin),
            None => Vec::new(),
        }
    }

    /// Put `tab` at bar slot `to` here at once and tell the server; nothing if it is already there.
    fn place_tab(&mut self, tab: TabId, to: usize) -> Vec<ClientMsg> {
        let Some(ws) = self.workspace else { return Vec::new() };
        let Some(w) = self.snap.workspaces.iter_mut().find(|w| w.id == ws) else { return Vec::new() };
        let Some(from) = w.tabs.iter().position(|&t| t == tab) else { return Vec::new() };
        let to = to.min(w.tabs.len() - 1);
        if to == from {
            return Vec::new();
        }
        w.tabs.remove(from);
        w.tabs.insert(to, tab);
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
