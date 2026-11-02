//! The layout state machine (spec §4.1): workspaces, tabs and pane trees. It never touches a
//! process; every command returns the `Effect`s the session must carry out, so each rule is
//! testable without ConPTY.
use serde::{Deserialize, Serialize};
use wraith_proto::model::{AgentInfo, Axis, ModelCmd, ModelSnapshot, PaneMeta, PaneNode, Removal, Tab, TabId, Workspace, WorkspaceId};
use wraith_proto::pane::SpawnOpts;
use wraith_proto::wire::PaneId;

const FILE_VERSION: u32 = 1;
const COLORS: [u32; 8] = [0x3b82f6, 0x10b981, 0xf59e0b, 0xef4444, 0x8b5cf6, 0xec4899, 0x14b8a6, 0xf97316];

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Spawn { pane: PaneId, spawn: SpawnOpts },
    Kill(PaneId),
}

pub struct Model {
    rev: u64,
    workspaces: Vec<Workspace>,
    tabs: Vec<Tab>,
    panes: Vec<PaneMeta>,
    next_ws: WorkspaceId,
    next_tab: TabId,
    next_pane: PaneId,
    clock: u64,
}

/// What `layout.json` holds.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    workspaces: Vec<Workspace>,
    tabs: Vec<Tab>,
    panes: Vec<PaneMeta>,
    clock: u64,
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

impl Model {
    pub fn new() -> Model {
        let mut m = Model { rev: 0, workspaces: vec![], tabs: vec![], panes: vec![], next_ws: 1, next_tab: 1, next_pane: 1, clock: 0 };
        m.add_workspace(None, None, "");
        m
    }

    pub fn snapshot(&self) -> ModelSnapshot {
        ModelSnapshot { rev: self.rev, workspaces: self.workspaces.clone(), tabs: self.tabs.clone(), panes: self.panes.clone() }
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.panes.iter().map(|p| p.id).collect()
    }

    pub fn workspace_of(&self, pane: PaneId) -> Option<WorkspaceId> {
        self.tabs.iter().find(|t| t.root.contains(pane)).map(|t| t.workspace)
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn has_workspace(&self, id: WorkspaceId) -> bool {
        self.workspaces.iter().any(|w| w.id == id)
    }

    pub fn workspace_of_tab(&self, tab: TabId) -> Option<WorkspaceId> {
        self.tabs.iter().find(|t| t.id == tab).map(|t| t.workspace)
    }

    /// A workspace with no tabs; the window that gets it opens the first tab.
    pub fn new_empty_workspace(&mut self) -> WorkspaceId {
        let id = self.add_workspace(None, None, "");
        self.touch();
        id
    }

    fn touch(&mut self) {
        self.rev += 1;
    }

    fn add_workspace(&mut self, name: Option<String>, root_dir: Option<String>, profile: &str) -> WorkspaceId {
        let id = self.next_ws;
        self.next_ws += 1;
        self.workspaces.push(Workspace {
            id,
            name: name.unwrap_or_else(|| format!("Workspace {id}")),
            root_dir,
            color: COLORS[id as usize % COLORS.len()],
            tabs: vec![],
            active_tab: None,
            profile: profile.to_string(),
            attached: false,
            last_used: 0,
        });
        id
    }

    fn add_pane(&mut self, profile: &str, spawn: &SpawnOpts) -> PaneId {
        let id = self.next_pane;
        self.next_pane += 1;
        self.panes.push(PaneMeta { id, title: String::new(), cwd: spawn.cwd.clone(), exit: None, profile: profile.into(), cmdline: spawn.cmdline.clone(), agent: None });
        id
    }

    fn add_tab(&mut self, workspace: WorkspaceId, profile: &str, spawn: &SpawnOpts, fx: &mut Vec<Effect>) -> (TabId, PaneId) {
        let pane = self.add_pane(profile, spawn);
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(Tab { id, workspace, title_override: None, root: PaneNode::Leaf(pane), focused: pane, zoomed: None });
        let ws = self.ws_mut(workspace).expect("checked by caller");
        ws.tabs.push(id);
        ws.active_tab = Some(id);
        fx.push(Effect::Spawn { pane, spawn: spawn.clone() });
        (id, pane)
    }

    fn ws_mut(&mut self, id: WorkspaceId) -> Result<&mut Workspace, String> {
        self.workspaces.iter_mut().find(|w| w.id == id).ok_or_else(|| format!("no workspace {id}"))
    }

    fn tab_idx(&self, id: TabId) -> Result<usize, String> {
        self.tabs.iter().position(|t| t.id == id).ok_or_else(|| format!("no tab {id}"))
    }

    fn tab_of_pane(&self, pane: PaneId) -> Result<usize, String> {
        self.tabs.iter().position(|t| t.root.contains(pane)).ok_or_else(|| format!("no pane {pane}"))
    }

    fn check_ws(&self, id: WorkspaceId) -> Result<(), String> {
        self.workspaces.iter().any(|w| w.id == id).then_some(()).ok_or_else(|| format!("no workspace {id}"))
    }

    pub fn apply(&mut self, cmd: ModelCmd) -> Result<Vec<Effect>, String> {
        let mut fx = Vec::new();
        match cmd {
            ModelCmd::NewWorkspace { name, root_dir, profile, layout, spawn } => {
                if !["single", "cols", "rows", "grid"].contains(&layout.as_str()) {
                    return Err(format!("unknown layout `{layout}`"));
                }
                let ws = self.add_workspace(name, root_dir, &profile);
                let (tab, p1) = self.add_tab(ws, &profile, &spawn, &mut fx);
                let idx = self.tab_idx(tab)?;
                let grow = |this: &mut Model, target: PaneId, axis: Axis, fx: &mut Vec<Effect>| {
                    let new = this.add_pane(&profile, &spawn);
                    this.tabs[idx].root.split(target, new, axis, false);
                    fx.push(Effect::Spawn { pane: new, spawn: spawn.clone() });
                    new
                };
                match layout.as_str() {
                    "cols" => {
                        grow(self, p1, Axis::Horizontal, &mut fx);
                    }
                    "rows" => {
                        grow(self, p1, Axis::Vertical, &mut fx);
                    }
                    "grid" => {
                        let p2 = grow(self, p1, Axis::Horizontal, &mut fx);
                        grow(self, p1, Axis::Vertical, &mut fx);
                        grow(self, p2, Axis::Vertical, &mut fx);
                    }
                    _ => {}
                }
            }
            ModelCmd::CloseWorkspace(id) => {
                self.check_ws(id)?;
                self.remove_workspace(id, &mut fx);
            }
            ModelCmd::RenameWorkspace(id, name) => self.ws_mut(id)?.name = name,
            ModelCmd::NewTab { workspace, profile, spawn } => {
                self.check_ws(workspace)?;
                self.add_tab(workspace, &profile, &spawn, &mut fx);
            }
            ModelCmd::CloseTab(tab) => {
                self.tab_idx(tab)?;
                self.remove_tab(tab, &mut fx);
            }
            ModelCmd::SelectTab(tab) => {
                let ws = self.tabs[self.tab_idx(tab)?].workspace;
                self.ws_mut(ws)?.active_tab = Some(tab);
            }
            ModelCmd::MoveTab { tab, to, index } => {
                let from = self.tabs[self.tab_idx(tab)?].workspace;
                self.check_ws(to)?;
                self.move_tab(tab, from, to, index);
            }
            ModelCmd::RenameTab(tab, title) => {
                let i = self.tab_idx(tab)?;
                self.tabs[i].title_override = title.filter(|t| !t.is_empty());
            }
            ModelCmd::Split { pane, axis, new_first, profile, spawn } => {
                let i = self.tab_of_pane(pane)?;
                let new = self.add_pane(&profile, &spawn);
                self.tabs[i].root.split(pane, new, axis, new_first);
                self.tabs[i].focused = new;
                self.tabs[i].zoomed = None;
                fx.push(Effect::Spawn { pane: new, spawn });
            }
            ModelCmd::ClosePane(pane) => {
                let i = self.tab_of_pane(pane)?;
                match self.tabs[i].root.remove(pane) {
                    Removal::Last => {
                        let tab = self.tabs[i].id;
                        self.remove_tab(tab, &mut fx);
                    }
                    Removal::Removed { focus } => {
                        let t = &mut self.tabs[i];
                        if t.focused == pane {
                            t.focused = focus;
                        }
                        t.zoomed = None;
                        self.panes.retain(|p| p.id != pane);
                        fx.push(Effect::Kill(pane));
                    }
                    Removal::NotFound => return Err(format!("no pane {pane}")),
                }
            }
            ModelCmd::FocusPane(pane) => {
                let i = self.tab_of_pane(pane)?;
                self.tabs[i].focused = pane;
                let (tab, ws) = (self.tabs[i].id, self.tabs[i].workspace);
                self.ws_mut(ws)?.active_tab = Some(tab);
            }
            ModelCmd::SetRatio { tab, path, ratio } => {
                let i = self.tab_idx(tab)?;
                if !self.tabs[i].root.set_ratio(&path, ratio) {
                    return Err("no split at that path".into());
                }
            }
            ModelCmd::NudgeSplit { pane, dir, delta } => {
                let i = self.tab_of_pane(pane)?;
                if !self.tabs[i].root.nudge(pane, dir, delta) {
                    return Ok(fx); // no split on that axis: nothing changed, nothing to announce
                }
            }
            ModelCmd::Equalize(tab) => {
                let i = self.tab_idx(tab)?;
                self.tabs[i].root.equalize();
            }
            ModelCmd::ToggleZoom(tab) => {
                let i = self.tab_idx(tab)?;
                let t = &mut self.tabs[i];
                t.zoomed = if t.zoomed.is_some() { None } else { Some(t.focused) };
            }
        }
        self.touch();
        Ok(fx)
    }

    fn remove_tab(&mut self, tab: TabId, fx: &mut Vec<Effect>) {
        let i = self.tab_idx(tab).expect("checked by caller");
        let t = self.tabs.remove(i);
        for p in t.root.leaves() {
            self.panes.retain(|m| m.id != p);
            fx.push(Effect::Kill(p));
        }
        let ws = self.ws_mut(t.workspace).expect("tab belongs to a workspace");
        let at = ws.tabs.iter().position(|&x| x == tab).unwrap_or(0);
        ws.tabs.retain(|&x| x != tab);
        if ws.active_tab == Some(tab) {
            ws.active_tab = ws.tabs.get(at).or_else(|| at.checked_sub(1).and_then(|p| ws.tabs.get(p))).copied();
        }
        if ws.tabs.is_empty() {
            let id = ws.id;
            self.remove_workspace(id, fx);
        }
    }

    fn remove_workspace(&mut self, id: WorkspaceId, fx: &mut Vec<Effect>) {
        let tabs: Vec<TabId> = self.tabs.iter().filter(|t| t.workspace == id).map(|t| t.id).collect();
        for tab in tabs {
            let i = self.tab_idx(tab).expect("listed above");
            let t = self.tabs.remove(i);
            for p in t.root.leaves() {
                self.panes.retain(|m| m.id != p);
                fx.push(Effect::Kill(p));
            }
        }
        self.workspaces.retain(|w| w.id != id);
        if self.workspaces.is_empty() {
            self.add_workspace(None, None, "");
        }
    }

    fn move_tab(&mut self, tab: TabId, from: WorkspaceId, to: WorkspaceId, index: Option<usize>) {
        let i = self.tab_idx(tab).expect("checked by caller");
        self.tabs[i].workspace = to;
        let src = self.ws_mut(from).expect("tab belongs to a workspace");
        let at = src.tabs.iter().position(|&x| x == tab).unwrap_or(0);
        src.tabs.retain(|&x| x != tab);
        if src.active_tab == Some(tab) {
            src.active_tab = src.tabs.get(at).or_else(|| at.checked_sub(1).and_then(|p| src.tabs.get(p))).copied();
        }
        let emptied = src.tabs.is_empty();
        let dst = self.ws_mut(to).expect("checked by caller");
        let at = index.unwrap_or(dst.tabs.len()).min(dst.tabs.len());
        dst.tabs.insert(at, tab);
        dst.active_tab = Some(tab);
        if emptied && from != to && self.workspaces.len() > 1 {
            self.workspaces.retain(|w| w.id != from);
        }
    }

    fn pane_mut(&mut self, pane: PaneId) -> Option<&mut PaneMeta> {
        self.panes.iter_mut().find(|p| p.id == pane)
    }

    /// Record a title change; true when the snapshot changed.
    pub fn note_title(&mut self, pane: PaneId, title: String) -> bool {
        self.edit_meta(pane, |p| (p.title != title).then(|| p.title = title))
    }

    pub fn note_cwd(&mut self, pane: PaneId, cwd: String) -> bool {
        self.edit_meta(pane, |p| (p.cwd.as_deref() != Some(cwd.as_str())).then(|| p.cwd = Some(cwd)))
    }

    pub fn note_exit(&mut self, pane: PaneId, code: u32) -> bool {
        self.edit_meta(pane, |p| (p.exit != Some(code)).then(|| p.exit = Some(code)))
    }

    pub fn note_agent(&mut self, pane: PaneId, agent: Option<AgentInfo>) -> bool {
        self.edit_meta(pane, |p| (p.agent != agent).then(|| p.agent = agent))
    }

    pub fn clear_exit(&mut self, pane: PaneId) -> bool {
        self.edit_meta(pane, |p| p.exit.take().map(drop))
    }

    fn edit_meta(&mut self, pane: PaneId, f: impl FnOnce(&mut PaneMeta) -> Option<()>) -> bool {
        let changed = self.pane_mut(pane).is_some_and(|p| f(p).is_some());
        if changed {
            self.touch();
        }
        changed
    }

    pub fn set_attached(&mut self, ws: WorkspaceId, attached: bool) {
        self.clock += 1;
        let stamp = self.clock;
        if let Ok(w) = self.ws_mut(ws) {
            w.attached = attached;
            w.last_used = stamp;
            self.touch();
        }
    }

    /// The detached workspace used most recently (highest `last_used`, then highest id).
    pub fn most_recent_detached(&self) -> Option<WorkspaceId> {
        self.workspaces.iter().filter(|w| !w.attached).max_by_key(|w| (w.last_used, w.id)).map(|w| w.id)
    }

    pub fn to_json(&self) -> String {
        let mut workspaces = self.workspaces.clone();
        workspaces.iter_mut().for_each(|w| w.attached = false);
        let mut panes = self.panes.clone();
        panes.iter_mut().for_each(|p| p.agent = None); // agents are live state, gone after a restart
        let saved = Saved { version: FILE_VERSION, workspaces, tabs: self.tabs.clone(), panes, clock: self.clock };
        serde_json::to_string_pretty(&saved).expect("model serializes")
    }

    /// Load a saved layout. Every reference must resolve, or the file is rejected whole.
    pub fn from_json(s: &str) -> Result<Model, String> {
        let mut saved: Saved = serde_json::from_str(s).map_err(|e| e.to_string())?;
        if saved.version != FILE_VERSION {
            return Err(format!("layout file version {} is not supported", saved.version));
        }
        for w in &saved.workspaces {
            for t in &w.tabs {
                if !saved.tabs.iter().any(|x| x.id == *t && x.workspace == w.id) {
                    return Err(format!("workspace {} names a missing tab {t}", w.id));
                }
            }
        }
        for t in &saved.tabs {
            let leaves = t.root.leaves();
            if !saved.workspaces.iter().any(|w| w.id == t.workspace) || !leaves.contains(&t.focused) {
                return Err(format!("tab {} is inconsistent", t.id));
            }
            if let Some(p) = leaves.iter().find(|p| !saved.panes.iter().any(|m| m.id == **p)) {
                return Err(format!("tab {} names a missing pane {p}", t.id));
            }
        }
        let in_tree = |id: PaneId| saved.tabs.iter().any(|t| t.root.contains(id));
        saved.panes.retain(|p| in_tree(p.id));
        saved.workspaces.iter_mut().for_each(|w| w.attached = false);
        saved.panes.iter_mut().for_each(|p| p.exit = None);
        let max = |it: &mut dyn Iterator<Item = u64>| it.max().unwrap_or(0) + 1;
        let mut m = Model {
            rev: 0,
            next_ws: max(&mut saved.workspaces.iter().map(|w| w.id)),
            next_tab: max(&mut saved.tabs.iter().map(|t| t.id)),
            next_pane: max(&mut saved.panes.iter().map(|p| p.id)),
            workspaces: saved.workspaces,
            tabs: saved.tabs,
            panes: saved.panes,
            clock: saved.clock,
        };
        if m.workspaces.is_empty() {
            m.add_workspace(None, None, "");
        }
        Ok(m)
    }

    /// One `Spawn` per saved pane, with fresh shells at a default size (the window resizes them).
    pub fn restore_effects(&self) -> Vec<Effect> {
        self.panes
            .iter()
            .map(|p| Effect::Spawn {
                pane: p.id,
                spawn: SpawnOpts {
                    cmdline: p.cmdline.clone(),
                    // The last cwd may be a WSL path (its cmdline has its own --cd) or since deleted:
                    // CreateProcess fails on either, so such a pane starts in the server's directory.
                    cwd: p.cwd.clone().filter(|c| std::path::Path::new(c).is_absolute() && std::path::Path::new(c).is_dir()),
                    env: vec![],
                    cols: 80,
                    rows: 24,
                    cell_w_px: 8,
                    cell_h_px: 16,
                    scrollback_lines: 10_000,
                    default_fg: (255, 255, 255),
                    default_bg: (0, 0, 0),
                    palette: vec![],
                    image_storage_limit: 320 * 1024 * 1024,
                    agent_detect: vec![],
                },
            })
            .collect()
    }
}
