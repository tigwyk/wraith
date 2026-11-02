//! The window view: snapshot mirror, pane rectangles, subscriptions and actions → protocol messages.
//! No window or GPU involved.
use std::path::PathBuf;
use std::time::Duration;

use wraith_client::chrome::Hit;
use wraith_client::view::{Geometry, SpawnCtx, Ui, WindowView};
use wraith_config::{Action, Config, SplitTarget};
use wraith_proto::model::*;
use wraith_proto::pane::PaneCmd;
use wraith_proto::wire::{ClientMsg, PaneId};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};
use wraith_server::profile::{Profile, ShellKind};

fn geom() -> Geometry {
    Geometry { client: (1000, 600), cell: (8, 16), pad: (2, 2), bar_h: 26, sidebar_cols: None }
}

fn ctx() -> SpawnCtx {
    SpawnCtx {
        config: Config::default(),
        profiles: vec![
            Profile { name: "pwsh".into(), kind: ShellKind::Pwsh, command: "pwsh.exe".into() },
            Profile { name: "cmd".into(), kind: ShellKind::Cmd, command: "cmd.exe".into() },
        ],
        scripts: PathBuf::from(r"C:\s"),
        env: vec![],
    }
}

fn split(axis: Axis, a: PaneNode, b: PaneNode) -> PaneNode {
    PaneNode::Split { axis, ratio: 0.5, a: Box::new(a), b: Box::new(b) }
}

fn leaf(id: PaneId) -> PaneNode {
    PaneNode::Leaf(id)
}

fn meta(id: PaneId, cwd: Option<&str>, profile: &str, exit: Option<u32>) -> PaneMeta {
    PaneMeta { id, title: format!("t{id}"), cwd: cwd.map(Into::into), exit, profile: profile.into(), cmdline: "x".into() }
}

fn tab(id: TabId, root: PaneNode, focused: PaneId) -> Tab {
    Tab { id, workspace: 1, title_override: None, root, focused, zoomed: None }
}

/// Workspace 1 with tab 10 = [1|2] (focus 2, pane 2 in C:\work under cmd) and tab 11 = [3].
fn snap(active: TabId) -> ModelSnapshot {
    ModelSnapshot {
        rev: 1,
        workspaces: vec![Workspace {
            id: 1,
            name: "w".into(),
            root_dir: None,
            color: 0,
            tabs: vec![10, 11],
            active_tab: Some(active),
            profile: "pwsh".into(),
            attached: true,
            last_used: 1,
        }],
        tabs: vec![tab(10, split(Axis::Horizontal, leaf(1), leaf(2)), 2), tab(11, leaf(3), 3)],
        panes: vec![meta(1, None, "pwsh", None), meta(2, Some(r"C:\work"), "cmd", None), meta(3, None, "pwsh", None)],
    }
}

fn view() -> WindowView {
    let mut v = WindowView::new(geom(), Duration::from_millis(500));
    v.set_workspace(1);
    v
}

fn cmd(pane: PaneId, c: PaneCmd) -> ClientMsg {
    ClientMsg::Cmd { pane, cmd: c }
}

fn resizes(msgs: &[ClientMsg]) -> Vec<(PaneId, u16, u16)> {
    msgs.iter()
        .filter_map(|m| match m {
            ClientMsg::Cmd { pane, cmd: PaneCmd::Resize { cols, rows, .. } } => Some((*pane, *cols, *rows)),
            _ => None,
        })
        .collect()
}

fn subscribes(msgs: &[ClientMsg]) -> Vec<Vec<PaneId>> {
    msgs.iter().filter_map(|m| if let ClientMsg::Subscribe { panes } = m { Some(panes.clone()) } else { None }).collect()
}

#[test]
fn snapshot_creates_views_and_subscribes_only_visible_tab() {
    let mut v = view();
    let msgs = v.on_snapshot(snap(10), &ctx());
    assert_eq!(subscribes(&msgs), vec![vec![1, 2]]);
    // 1000x574 content, one 1 px divider: panes 500 and 499 px wide, minus 2 px padding a side.
    assert_eq!(resizes(&msgs), vec![(1, 62, 35), (2, 61, 35)]);
    let colors: Vec<PaneId> = msgs
        .iter()
        .filter_map(|m| match m {
            ClientMsg::Cmd { pane, cmd: PaneCmd::SetColors { .. } } => Some(*pane),
            _ => None,
        })
        .collect();
    assert_eq!(colors, vec![1, 2]);
    assert!(matches!(msgs.last(), Some(ClientMsg::Subscribe { .. })), "subscribe last, so the first frame has the right size and colors");
    let ids: Vec<PaneId> = v.visible().iter().map(|(p, _)| *p).collect();
    assert_eq!(ids, vec![1, 2]);
}

#[test]
fn tab_switch_subscribes_new_and_leaves_old_size_untouched() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    let msgs = v.on_snapshot(snap(11), &ctx());
    assert_eq!(subscribes(&msgs), vec![vec![3]]);
    assert_eq!(resizes(&msgs), vec![(3, 124, 35)], "only the newly visible pane is sized (full width: 1000/8 - 1)");
    let back = v.on_snapshot(snap(10), &ctx());
    assert_eq!(subscribes(&back), vec![vec![1, 2]]);
    assert!(resizes(&back).is_empty(), "hidden panes kept their size");
    assert!(!back.iter().any(|m| matches!(m, ClientMsg::Cmd { cmd: PaneCmd::SetColors { .. }, .. })), "colors are sent once");
}

#[test]
fn relayout_sends_resize_only_on_cell_change() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    let mut g = geom();
    g.client.1 += 5; // not enough for another row
    assert!(resizes(&v.set_geometry(g)).is_empty());
    g.client.0 += 16;
    assert_eq!(resizes(&v.set_geometry(g)), vec![(1, 63, 35), (2, 62, 35)]);
    let mut tiny = geom();
    tiny.client = (0, 0);
    assert!(v.set_geometry(tiny).is_empty(), "a minimized window must not resize shells");
}

#[test]
fn zoomed_tab_shows_only_the_zoomed_pane() {
    let mut v = view();
    let mut s = snap(10);
    s.tabs[0].zoomed = Some(1);
    let msgs = v.on_snapshot(s, &ctx());
    assert_eq!(subscribes(&msgs), vec![vec![1]]);
    assert_eq!(v.visible(), vec![(1, Rect { x: 0, y: 26, w: 1000, h: 574 })]);
    assert!(v.dividers().is_empty());
}

#[test]
fn new_workspace_snapshot_without_tabs_requests_new_tab() {
    let mut v = view();
    let mut s = snap(10);
    s.workspaces[0].tabs.clear();
    s.workspaces[0].active_tab = None;
    s.tabs.clear();
    s.panes.clear();
    let msgs = v.on_snapshot(s.clone(), &ctx());
    let ClientMsg::Model(ModelCmd::NewTab { workspace, profile, spawn }) = &msgs[0] else { panic!("{msgs:?}") };
    assert_eq!((*workspace, profile.as_str()), (1, "pwsh"));
    assert!(spawn.cmdline.starts_with("pwsh.exe -NoExit"), "shell integration is on by default: {}", spawn.cmdline);
    assert!(v.on_snapshot(s, &ctx()).is_empty(), "asked once, not on every snapshot");
}

#[test]
fn new_tab_starts_in_root_dir_else_focused_cwd() {
    let mut v = view();
    let mut s = snap(10);
    v.on_snapshot(s.clone(), &ctx());
    let out = v.action(Action::NewTab, &ctx());
    let ClientMsg::Model(ModelCmd::NewTab { spawn, .. }) = &out.msgs[0] else { panic!("{out:?}") };
    assert_eq!(spawn.cwd.as_deref(), Some(r"C:\work"), "the focused pane's directory");
    s.workspaces[0].root_dir = Some(r"C:\proj".into());
    v.on_snapshot(s, &ctx());
    let out = v.action(Action::NewTab, &ctx());
    let ClientMsg::Model(ModelCmd::NewTab { spawn, .. }) = &out.msgs[0] else { panic!("{out:?}") };
    assert_eq!(spawn.cwd.as_deref(), Some(r"C:\proj"), "the workspace root wins");
}

#[test]
fn new_split_action_uses_focused_pane_cwd_and_profile() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    for (dir, axis, first) in [(Dir::Right, Axis::Horizontal, false), (Dir::Left, Axis::Horizontal, true), (Dir::Down, Axis::Vertical, false), (Dir::Up, Axis::Vertical, true)] {
        let out = v.action(Action::NewSplit(dir), &ctx());
        let [ClientMsg::Model(ModelCmd::Split { pane, axis: a, new_first, profile, spawn })] = out.msgs.as_slice() else { panic!("{out:?}") };
        assert_eq!((*pane, *a, *new_first, profile.as_str()), (2, axis, first, "cmd"));
        assert_eq!((spawn.cmdline.as_str(), spawn.cwd.as_deref()), ("cmd.exe", Some(r"C:\work")));
    }
}

#[test]
fn goto_split_uses_geometry() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    // Focus is on pane 2 (right): left reaches pane 1, right is the edge.
    assert_eq!(v.action(Action::GotoSplit(SplitTarget::Dir(Dir::Left)), &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::FocusPane(1))]);
    assert!(v.action(Action::GotoSplit(SplitTarget::Dir(Dir::Right)), &ctx()).msgs.is_empty());
    assert_eq!(v.action(Action::GotoSplit(SplitTarget::Next), &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::FocusPane(1))], "next wraps");
    assert_eq!(v.action(Action::GotoSplit(SplitTarget::Previous), &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::FocusPane(1))]);
}

#[test]
fn tab_navigation_actions() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    assert_eq!(v.action(Action::GotoTab(2), &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::SelectTab(11))]);
    assert!(v.action(Action::GotoTab(5), &ctx()).msgs.is_empty(), "out of range is a no-op");
    assert_eq!(v.action(Action::NextTab, &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::SelectTab(11))]);
    assert_eq!(v.action(Action::PreviousTab, &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::SelectTab(11))], "previous wraps");
    assert_eq!(v.action(Action::CloseTab, &ctx()).msgs, vec![ClientMsg::Model(ModelCmd::CloseTab(10))]);
}

#[test]
fn close_surface_on_running_shell_asks_confirm() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    let out = v.action(Action::CloseSurface, &ctx());
    assert!(out.msgs.is_empty());
    assert_eq!(out.confirm_close, Some(2));
    assert_eq!(v.close_pane(2), vec![ClientMsg::Model(ModelCmd::ClosePane(2))]);
    let mut s = snap(10);
    s.panes[1].exit = Some(0);
    v.on_snapshot(s, &ctx());
    let out = v.action(Action::CloseSurface, &ctx());
    assert_eq!((out.msgs, out.confirm_close), (vec![ClientMsg::Model(ModelCmd::ClosePane(2))], None), "an exited shell closes without asking");
}

#[test]
fn exited_pane_restarts_on_enter() {
    let mut v = view();
    let mut s = snap(10);
    s.panes[1].exit = Some(3);
    v.on_snapshot(s, &ctx());
    let key = |code, action| KeyEvent { code, mods: Mods(0), text: None, action, composing: false };
    assert_eq!(v.pane_exit(2), Some(3));
    assert_eq!(v.restart_key(&key(KeyCode::Enter, KeyAction::Press)), Some(cmd(2, PaneCmd::Restart)));
    assert_eq!(v.restart_key(&key(KeyCode::Enter, KeyAction::Release)), None);
    assert_eq!(v.restart_key(&key(KeyCode::KeyA, KeyAction::Press)), None);
    let mut running = view();
    running.on_snapshot(snap(10), &ctx());
    assert_eq!(running.restart_key(&key(KeyCode::Enter, KeyAction::Press)), None);
}

#[test]
fn unfocused_panes_get_dim_only_when_split() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    v.set_dim_opacity(0.85);
    assert_eq!(v.dim_alpha(2), 0.0);
    assert!((v.dim_alpha(1) - 0.15).abs() < 1e-6);
    v.on_snapshot(snap(11), &ctx());
    assert_eq!(v.dim_alpha(3), 0.0, "a lone pane is never dimmed");
}

#[test]
fn keys_route_to_focused_pane_only() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    assert_eq!(v.input(PaneCmd::Text("a".into())), Some(cmd(2, PaneCmd::Text("a".into()))));
    v.action(Action::GotoSplit(SplitTarget::Dir(Dir::Left)), &ctx());
    let mut s = snap(10);
    s.tabs[0].focused = 1;
    v.on_snapshot(s, &ctx());
    assert_eq!(v.input(PaneCmd::Text("b".into())), Some(cmd(1, PaneCmd::Text("b".into()))));
    assert_eq!(view().input(PaneCmd::Text("c".into())), None, "no pane, no input");
}

#[test]
fn clicking_and_dragging() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    assert_eq!(v.hit(100, 300), Hit::Pane(1));
    assert_eq!(v.hit(700, 300), Hit::Pane(2));
    assert_eq!(v.pane_click(1), vec![ClientMsg::Model(ModelCmd::FocusPane(1))]);
    assert!(v.pane_click(2).is_empty(), "already focused");
    let Hit::Divider(i) = v.hit(500, 300) else { panic!("{:?}", v.hit(500, 300)) };
    v.start_divider_drag(i);
    let msgs = v.drag_divider(250, 300);
    assert!(matches!(&msgs[0], ClientMsg::Model(ModelCmd::SetRatio { tab: 10, path, ratio }) if path.is_empty() && (*ratio - 0.25).abs() < 0.01), "{msgs:?}");
    assert!(!resizes(&msgs).is_empty(), "the panes are resized as the divider moves");
    let rects = v.visible();
    assert!(rects[0].1.w < 300 && rects[1].1.w > 600, "{rects:?}");
}

#[test]
fn palette_and_window_actions_are_ui_requests() {
    let mut v = view();
    v.on_snapshot(snap(10), &ctx());
    assert_eq!(v.action(Action::CommandPalette, &ctx()).ui, Some(Ui::Palette));
    assert_eq!(v.action(Action::WorkspacePicker, &ctx()).ui, Some(Ui::Picker));
    assert_eq!(v.action(Action::NewWindow, &ctx()).ui, Some(Ui::NewWindow));
}

// ---- Task 9: sidebar and workspace switching ----

use std::time::Instant;
use wraith_config::WorkspaceDef;

fn ws(id: WorkspaceId, name: &str, attached: bool) -> Workspace {
    Workspace { id, name: name.into(), root_dir: None, color: 0x3366cc, tabs: vec![], active_tab: None, profile: "pwsh".into(), attached, last_used: id }
}

/// `snap(10)` plus workspaces 2 and 3 (both empty, detached).
fn three_workspaces() -> ModelSnapshot {
    let mut s = snap(10);
    s.workspaces.push(ws(2, "web", false));
    s.workspaces.push(ws(3, "infra", false));
    s
}

fn with_sidebar() -> WindowView {
    let mut v = view();
    v.set_sidebar(true, 24);
    v
}

#[test]
fn toggle_sidebar_changes_regions() {
    let mut v = with_sidebar();
    v.on_snapshot(three_workspaces(), &ctx());
    assert_eq!(v.regions().sidebar, Some(Rect { x: 0, y: 0, w: 192, h: 600 }));
    assert_eq!(v.regions().content.x, 192);
    let docked_cols = v.visible()[0].1.w;
    let out = v.action(Action::ToggleSidebar, &ctx());
    assert_eq!(v.regions().sidebar, None);
    assert_eq!(v.regions().content.x, 0);
    assert!(v.visible()[0].1.w > docked_cols);
    assert!(!resizes(&out.msgs).is_empty(), "panes grow into the freed space");
    v.action(Action::ToggleSidebar, &ctx());
    assert!(v.regions().sidebar.is_some());
}

#[test]
fn hover_strip_reveals_after_grace_and_hides_after_300ms() {
    let mut v = with_sidebar();
    v.on_snapshot(three_workspaces(), &ctx());
    v.action(Action::ToggleSidebar, &ctx());
    let t0 = Instant::now();
    assert!(v.floating_sidebar().is_none());
    v.pointer_moved(3, 100, t0);
    assert_eq!(v.floating_sidebar(), Some(Rect { x: 0, y: 0, w: 192, h: 600 }));
    assert_eq!(v.regions().content.x, 0, "revealing floats over the panes, it does not resize them");
    v.pointer_moved(100, 100, t0 + Duration::from_millis(100));
    assert!(v.floating_sidebar().is_some(), "inside the sidebar it stays");
    v.pointer_moved(600, 100, t0 + Duration::from_millis(200));
    assert!(!v.tick(t0 + Duration::from_millis(300)), "300 ms grace not over yet");
    assert!(v.floating_sidebar().is_some());
    assert!(v.next_deadline().is_some());
    assert!(v.tick(t0 + Duration::from_millis(520)));
    assert!(v.floating_sidebar().is_none());
    v.pointer_moved(3, 100, t0 + Duration::from_secs(1));
    v.pointer_left(t0 + Duration::from_secs(1));
    assert!(v.tick(t0 + Duration::from_millis(1400)), "leaving the window starts the grace too");
    assert!(v.floating_sidebar().is_none());
}

#[test]
fn clicking_workspace_sends_attach_window() {
    let mut v = with_sidebar();
    v.on_snapshot(three_workspaces(), &ctx());
    let (rows, _) = v.workspace_rects();
    assert_eq!(rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 2, 3]);
    let r2 = rows[1].1;
    assert_eq!(v.hit(r2.x + 5, r2.y + 5), Hit::Workspace(2));
    assert_eq!(v.workspace_click(2), vec![ClientMsg::AttachWindow { workspace: Some(2) }]);
    assert!(v.workspace_click(1).is_empty(), "already showing it");
}

#[test]
fn next_previous_workspace_wrap() {
    let mut v = view();
    v.on_snapshot(three_workspaces(), &ctx());
    let attach = |id| vec![ClientMsg::AttachWindow { workspace: Some(id) }];
    assert_eq!(v.action(Action::NextWorkspace, &ctx()).msgs, attach(2));
    assert_eq!(v.action(Action::PreviousWorkspace, &ctx()).msgs, attach(3));
}

#[test]
fn workspace_taken_attaches_another_without_closing_window() {
    let mut v = view();
    v.on_snapshot(three_workspaces(), &ctx());
    assert!(v.on_workspace_taken(9).is_empty(), "not our workspace");
    assert_eq!(v.workspace(), Some(1));
    assert_eq!(v.on_workspace_taken(1), vec![ClientMsg::AttachWindow { workspace: None }]);
    assert_eq!(v.workspace(), None);
    assert!(v.visible().is_empty(), "nothing is drawn for a workspace we no longer hold");
}

#[test]
fn new_workspace_is_attached_when_it_appears() {
    let mut v = view();
    v.on_snapshot(three_workspaces(), &ctx());
    let out = v.action(Action::NewWorkspace, &ctx());
    let [ClientMsg::Model(ModelCmd::NewWorkspace { name, root_dir, profile, layout, .. })] = out.msgs.as_slice() else { panic!("{out:?}") };
    assert_eq!((name.as_deref(), root_dir.as_deref(), profile.as_str(), layout.as_str()), (None, None, "pwsh", "single"));
    let mut s = three_workspaces();
    s.workspaces.push(ws(4, "Workspace 4", false));
    let msgs = v.on_snapshot(s.clone(), &ctx());
    assert!(msgs.contains(&ClientMsg::AttachWindow { workspace: Some(4) }), "{msgs:?}");
    s.workspaces.push(ws(5, "someone else's", false));
    assert!(!v.on_snapshot(s, &ctx()).iter().any(|m| matches!(m, ClientMsg::AttachWindow { .. })), "only the workspace we asked for");
}

#[test]
fn configured_workspaces_missing_from_snapshot_are_created_once() {
    let mut c = ctx();
    c.config.workspaces = vec![
        WorkspaceDef { name: "api".into(), dir: Some("C:/src".into()), layout: "cols".into() },
        WorkspaceDef { name: "web".into(), dir: None, layout: "single".into() },
    ];
    let mut v = view();
    let msgs = v.on_snapshot(three_workspaces(), &c); // "web" exists already
    let made: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            ClientMsg::Model(ModelCmd::NewWorkspace { name, root_dir, layout, spawn, .. }) => Some((name.clone(), root_dir.clone(), layout.clone(), spawn.cwd.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(made, vec![(Some("api".to_string()), Some("C:/src".to_string()), "cols".to_string(), Some("C:/src".to_string()))]);
    let again = v.on_snapshot(three_workspaces(), &c);
    assert!(!again.iter().any(|m| matches!(m, ClientMsg::Model(ModelCmd::NewWorkspace { .. }))), "created once per window");
}

#[test]
fn close_workspace_asks_when_shells_run() {
    let mut v = view();
    v.on_snapshot(three_workspaces(), &ctx());
    let out = v.action(Action::CloseWorkspace, &ctx());
    assert!(out.msgs.is_empty());
    assert_eq!(out.confirm_close_workspace, Some(1));
    assert_eq!(v.close_workspace(1), vec![ClientMsg::Model(ModelCmd::CloseWorkspace(1))]);
}

#[test]
fn attention_dot_is_false_in_m3() {
    let mut v = with_sidebar();
    v.on_snapshot(three_workspaces(), &ctx());
    let rows = v.sidebar_rows();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| !r.attention));
    assert!(rows[0].current && !rows[1].current);
    assert_eq!(rows[1].name, "web");
}
