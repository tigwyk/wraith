//! The model state machine: every layout rule, with no processes.
use wraith_proto::model::*;
use wraith_proto::pane::SpawnOpts;
use wraith_server::model::{Effect, Model};

fn sp(cwd: Option<&str>) -> SpawnOpts {
    SpawnOpts {
        cmdline: "pwsh.exe".into(),
        cwd: cwd.map(Into::into),
        env: vec![],
        cols: 80,
        rows: 24,
        cell_w_px: 8,
        cell_h_px: 16,
        scrollback_lines: 1000,
        default_fg: (255, 255, 255),
        default_bg: (0, 0, 0),
        palette: vec![],
        image_storage_limit: 1 << 20,
        agent_detect: vec![],
    }
}

fn spawned(fx: &[Effect]) -> PaneId {
    let Effect::Spawn { pane, .. } = fx[0] else { panic!("{fx:?}") };
    pane
}

use wraith_proto::wire::PaneId;

fn new_tab(m: &mut Model, ws: WorkspaceId) -> (TabId, PaneId) {
    let fx = m.apply(ModelCmd::NewTab { workspace: ws, profile: "pwsh".into(), spawn: sp(None) }).unwrap();
    let pane = spawned(&fx);
    (m.snapshot().workspace(ws).unwrap().active_tab.unwrap(), pane)
}

fn new_tab0(m: &mut Model) -> (TabId, PaneId) {
    let ws = ws0(m);
    new_tab(m, ws)
}

fn split(m: &mut Model, pane: PaneId, axis: Axis) -> PaneId {
    let fx = m.apply(ModelCmd::Split { pane, axis, new_first: false, profile: "pwsh".into(), spawn: sp(None) }).unwrap();
    spawned(&fx)
}

fn ws0(m: &Model) -> WorkspaceId {
    m.snapshot().workspaces[0].id
}

fn new_ws(m: &mut Model, name: &str, layout: &str) -> Vec<Effect> {
    m.apply(ModelCmd::NewWorkspace { name: Some(name.into()), root_dir: None, profile: "pwsh".into(), layout: layout.into(), spawn: sp(None) })
        .unwrap()
}

#[test]
fn new_has_one_empty_workspace() {
    let s = Model::new().snapshot();
    assert_eq!(s.workspaces.len(), 1);
    assert!(s.workspaces[0].tabs.is_empty() && s.workspaces[0].active_tab.is_none() && !s.workspaces[0].attached);
    assert!(s.tabs.is_empty() && s.panes.is_empty());
}

#[test]
fn new_tab_creates_pane_effect_and_selects_tab() {
    let mut m = Model::new();
    let ws = ws0(&m);
    let rev = m.snapshot().rev;
    let (t1, p1) = new_tab(&mut m, ws);
    let (t2, p2) = new_tab(&mut m, ws);
    let s = m.snapshot();
    assert!(s.rev > rev);
    assert_eq!(s.workspace(ws).unwrap().tabs, vec![t1, t2]);
    assert_eq!(s.workspace(ws).unwrap().active_tab, Some(t2));
    assert_eq!(s.tab(t2).unwrap().focused, p2);
    assert_ne!(p1, p2);
    assert_eq!(s.pane(p1).unwrap().cmdline, "pwsh.exe");
    assert_eq!(s.pane(p1).unwrap().profile, "pwsh");
}

#[test]
fn split_adds_leaf_and_focuses_new_pane() {
    let mut m = Model::new();
    let (t, p1) = new_tab0(&mut m);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    let tab = m.snapshot().tab(t).unwrap().clone();
    assert_eq!(tab.root.leaves(), vec![p1, p2]);
    assert_eq!(tab.focused, p2);
}

#[test]
fn close_pane_focuses_sibling() {
    let mut m = Model::new();
    let (t, p1) = new_tab0(&mut m);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    let fx = m.apply(ModelCmd::ClosePane(p2)).unwrap();
    assert_eq!(fx, vec![Effect::Kill(p2)]);
    let tab = m.snapshot().tab(t).unwrap().clone();
    assert_eq!(tab.root, PaneNode::Leaf(p1));
    assert_eq!(tab.focused, p1);
    assert!(m.snapshot().pane(p2).is_none());
}

#[test]
fn close_last_pane_closes_tab_and_selects_neighbor_tab() {
    let mut m = Model::new();
    let ws = ws0(&m);
    let (t1, _) = new_tab(&mut m, ws);
    let (t2, p2) = new_tab(&mut m, ws);
    let (t3, _) = new_tab(&mut m, ws);
    m.apply(ModelCmd::SelectTab(t2)).unwrap();
    assert_eq!(m.apply(ModelCmd::ClosePane(p2)).unwrap(), vec![Effect::Kill(p2)]);
    let s = m.snapshot();
    assert_eq!(s.workspace(ws).unwrap().tabs, vec![t1, t3]);
    assert_eq!(s.workspace(ws).unwrap().active_tab, Some(t3), "the tab that slid into its place");
    assert!(s.tab(t2).is_none());
}

#[test]
fn close_last_tab_closes_workspace() {
    let mut m = Model::new();
    let first = ws0(&m);
    new_ws(&mut m, "api", "single");
    let second = m.snapshot().workspaces.iter().find(|w| w.name == "api").unwrap().id;
    let t = m.snapshot().workspace(second).unwrap().tabs[0];
    m.apply(ModelCmd::CloseTab(t)).unwrap();
    let s = m.snapshot();
    assert!(s.workspace(second).is_none());
    assert!(s.workspace(first).is_some());
}

#[test]
fn default_names_take_the_lowest_free_number() {
    let mut m = Model::new();
    m.quick_workspace();
    let names = |m: &Model| m.snapshot().workspaces.iter().map(|w| w.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&m), ["Workspace 1", "Quick"]);
    let b = m.new_empty_workspace();
    m.new_empty_workspace();
    assert_eq!(names(&m), ["Workspace 1", "Quick", "Workspace 2", "Workspace 3"]);
    m.apply(ModelCmd::CloseWorkspace(b)).unwrap();
    m.new_empty_workspace();
    assert_eq!(names(&m), ["Workspace 1", "Quick", "Workspace 3", "Workspace 2"]);
}

#[test]
fn close_last_workspace_leaves_empty_default() {
    let mut m = Model::new();
    let ws = ws0(&m);
    let (_, p1) = new_tab(&mut m, ws);
    let p2 = split(&mut m, p1, Axis::Vertical);
    let mut fx = m.apply(ModelCmd::CloseWorkspace(ws)).unwrap();
    fx.sort_by_key(|e| match e {
        Effect::Kill(p) => *p,
        Effect::Spawn { pane, .. } => *pane,
    });
    assert_eq!(fx, vec![Effect::Kill(p1), Effect::Kill(p2)]);
    let s = m.snapshot();
    assert_eq!(s.workspaces.len(), 1);
    assert_ne!(s.workspaces[0].id, ws);
    assert!(s.workspaces[0].tabs.is_empty() && s.tabs.is_empty() && s.panes.is_empty());
}

#[test]
fn exit_keeps_pane_in_tree() {
    let mut m = Model::new();
    let (t, p1) = new_tab0(&mut m);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    assert!(m.note_exit(p2, 3));
    let s = m.snapshot();
    assert_eq!(s.pane(p2).unwrap().exit, Some(3));
    assert_eq!(s.tab(t).unwrap().root.leaves(), vec![p1, p2]);
    assert!(m.clear_exit(p2));
    assert_eq!(m.snapshot().pane(p2).unwrap().exit, None);
    m.apply(ModelCmd::ClosePane(p2)).unwrap();
    assert_eq!(m.snapshot().tab(t).unwrap().root.leaves(), vec![p1]);
}

#[test]
fn move_tab_to_workspace_keeps_panes() {
    let mut m = Model::new();
    let a = ws0(&m);
    let (t, p) = new_tab(&mut m, a);
    new_tab(&mut m, a);
    new_ws(&mut m, "b", "single");
    let b = m.snapshot().workspaces.iter().find(|w| w.name == "b").unwrap().id;
    let fx = m.apply(ModelCmd::MoveTab { tab: t, to: b, index: Some(0) }).unwrap();
    assert!(fx.is_empty());
    let s = m.snapshot();
    assert_eq!(s.tab(t).unwrap().workspace, b);
    assert_eq!(s.workspace(b).unwrap().tabs[0], t);
    assert_eq!(s.workspace(b).unwrap().active_tab, Some(t));
    assert!(!s.workspace(a).unwrap().tabs.contains(&t));
    assert_eq!(m.workspace_of(p), Some(b));
}

#[test]
fn new_workspace_applies_layout_presets() {
    let mut m = Model::new();
    for (layout, panes) in [("single", 1), ("cols", 2), ("rows", 2), ("grid", 4)] {
        let fx = m
            .apply(ModelCmd::NewWorkspace {
                name: Some(layout.into()),
                root_dir: Some("C:/src".into()),
                profile: "pwsh".into(),
                layout: layout.into(),
                spawn: sp(Some("C:/src")),
            })
            .unwrap();
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Spawn { .. })).count(), panes, "{layout}");
        let s = m.snapshot();
        let w = s.workspaces.iter().find(|w| w.name == layout).unwrap();
        let tab = s.tab(w.tabs[0]).unwrap();
        assert_eq!(tab.root.leaves().len(), panes, "{layout}");
        assert_eq!(w.root_dir.as_deref(), Some("C:/src"));
        match (layout, &tab.root) {
            ("cols", PaneNode::Split { axis, .. }) => assert_eq!(*axis, Axis::Horizontal),
            ("rows", PaneNode::Split { axis, .. }) => assert_eq!(*axis, Axis::Vertical),
            ("grid", PaneNode::Split { axis: Axis::Horizontal, a, b, .. }) => {
                assert!(matches!(**a, PaneNode::Split { axis: Axis::Vertical, .. }));
                assert!(matches!(**b, PaneNode::Split { axis: Axis::Vertical, .. }));
            }
            ("single", PaneNode::Leaf(_)) => {}
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn zoom_toggles_and_split_close_clears_zoom() {
    let mut m = Model::new();
    let (t, p1) = new_tab0(&mut m);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    m.apply(ModelCmd::ToggleZoom(t)).unwrap();
    assert_eq!(m.snapshot().tab(t).unwrap().zoomed, Some(p2));
    m.apply(ModelCmd::ToggleZoom(t)).unwrap();
    assert_eq!(m.snapshot().tab(t).unwrap().zoomed, None);
    m.apply(ModelCmd::ToggleZoom(t)).unwrap();
    m.apply(ModelCmd::ClosePane(p2)).unwrap();
    assert_eq!(m.snapshot().tab(t).unwrap().zoomed, None);
}

#[test]
fn unknown_ids_are_errors_not_panics() {
    let mut m = Model::new();
    let ws = ws0(&m);
    let (t, _) = new_tab(&mut m, ws);
    let before = m.snapshot();
    let cmds = vec![
        ModelCmd::CloseWorkspace(999),
        ModelCmd::RenameWorkspace(999, "x".into()),
        ModelCmd::NewTab { workspace: 999, profile: "p".into(), spawn: sp(None) },
        ModelCmd::CloseTab(999),
        ModelCmd::SelectTab(999),
        ModelCmd::MoveTab { tab: 999, to: ws, index: None },
        ModelCmd::MoveTab { tab: t, to: 999, index: None },
        ModelCmd::RenameTab(999, None),
        ModelCmd::Split { pane: 999, axis: Axis::Horizontal, new_first: false, profile: "p".into(), spawn: sp(None) },
        ModelCmd::ClosePane(999),
        ModelCmd::FocusPane(999),
        ModelCmd::SetRatio { tab: 999, path: vec![], ratio: 0.3 },
        ModelCmd::SetRatio { tab: t, path: vec![true, true], ratio: 0.3 },
        ModelCmd::NudgeSplit { pane: 999, dir: Dir::Left, delta: 0.1 },
        ModelCmd::Equalize(999),
        ModelCmd::ToggleZoom(999),
        ModelCmd::NewWorkspace { name: None, root_dir: None, profile: "p".into(), layout: "diagonal".into(), spawn: sp(None) },
    ];
    for c in cmds {
        assert!(m.apply(c.clone()).is_err(), "{c:?}");
    }
    assert_eq!(m.snapshot(), before);
}

#[test]
fn json_roundtrip_restores_tree_and_spawns_fresh_shells() {
    let mut m = Model::new();
    let ws = ws0(&m);
    m.apply(ModelCmd::RenameWorkspace(ws, "api".into())).unwrap();
    let (t, p1) = new_tab(&mut m, ws);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    m.apply(ModelCmd::RenameTab(t, Some("build".into()))).unwrap();
    let work = std::env::temp_dir().display().to_string();
    m.note_cwd(p2, work.clone());
    m.note_title(p2, "vim".into());
    m.note_exit(p1, 1);
    m.set_attached(ws, true);
    let m2 = Model::from_json(&m.to_json()).unwrap();
    let mut want = m.snapshot();
    want.workspaces.iter_mut().for_each(|w| w.attached = false);
    want.panes.iter_mut().for_each(|p| p.exit = None);
    let mut got = m2.snapshot();
    got.rev = want.rev;
    assert_eq!(got, want);
    let fx = m2.restore_effects();
    assert_eq!(fx.len(), 2);
    let spawn_of = |id| {
        fx.iter()
            .find_map(|e| match e {
                Effect::Spawn { pane, spawn } if *pane == id => Some(spawn.clone()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(spawn_of(p2).cwd.as_deref(), Some(work.as_str()));
    assert_eq!(spawn_of(p1).cmdline, "pwsh.exe");
}

#[test]
fn restore_never_hands_createprocess_a_directory_it_cannot_enter() {
    let mut m = Model::new();
    let (_, p1) = new_tab0(&mut m);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    let p3 = split(&mut m, p2, Axis::Vertical);
    m.note_cwd(p1, "/mnt/c/Users/x".into()); // a WSL shell's OSC 7 cwd; its cmdline carries --cd
    m.note_cwd(p2, r"C:\definitely\not\here\wraith".into()); // deleted since
    m.note_cwd(p3, r"\\?\".into());
    let fx = Model::from_json(&m.to_json()).unwrap().restore_effects();
    for e in &fx {
        let Effect::Spawn { pane, spawn } = e else { continue };
        assert_eq!(spawn.cwd, None, "pane {pane}: {:?}", spawn.cwd);
    }
}

#[test]
fn from_json_rejects_garbage_and_unknown_version() {
    assert!(Model::from_json("").is_err());
    assert!(Model::from_json("{").is_err());
    assert!(Model::from_json(r#"{"version":99,"workspaces":[],"tabs":[],"panes":[],"clock":0}"#).is_err());
    let mut m = Model::new();
    let (_, p) = new_tab0(&mut m);
    let bad = m.to_json().replace(&format!("\"Leaf\": {p}"), "\"Leaf\": 4242");
    assert_ne!(bad, m.to_json(), "the replacement must hit");
    assert!(Model::from_json(&bad).is_err(), "a tree naming a missing pane must not load");
}

#[test]
fn ids_resume_above_loaded_max() {
    let mut m = Model::new();
    let (t0, p1) = new_tab0(&mut m);
    let p2 = split(&mut m, p1, Axis::Horizontal);
    let mut m2 = Model::from_json(&m.to_json()).unwrap();
    let ws = m2.snapshot().workspaces[0].id;
    let (t, p3) = new_tab(&mut m2, ws);
    assert!(p3 > p2);
    assert!(t > t0);
}

#[test]
fn ids_are_never_reused_within_a_run() {
    let mut m = Model::new();
    let ws = ws0(&m);
    let (_, p1) = new_tab(&mut m, ws);
    m.apply(ModelCmd::ClosePane(p1)).unwrap();
    // The workspace went with its last tab; a fresh empty one took its place.
    let (_, p2) = new_tab0(&mut m);
    assert!(p2 > p1);
}

#[test]
fn most_recent_detached_follows_last_used() {
    let mut m = Model::new();
    let a = ws0(&m);
    new_ws(&mut m, "b", "single");
    let b = m.snapshot().workspaces.iter().find(|w| w.name == "b").unwrap().id;
    m.set_attached(a, true);
    m.set_attached(b, true);
    m.set_attached(b, false);
    m.set_attached(a, false);
    assert_eq!(m.most_recent_detached(), Some(a));
    m.set_attached(a, true);
    assert_eq!(m.most_recent_detached(), Some(b));
    m.set_attached(b, true);
    assert_eq!(m.most_recent_detached(), None);
}

// Quick terminal (quick-terminal spec §4).
#[test]
fn quick_workspace_is_created_once_and_skipped_by_reattach() {
    let mut m = Model::default();
    let q = m.quick_workspace();
    assert_eq!(m.quick_workspace(), q);
    let ws = m.snapshot().workspaces.into_iter().find(|w| w.id == q).unwrap();
    assert!(ws.quick);
    assert_eq!(ws.name, "Quick");
    assert_ne!(m.most_recent_detached(), Some(q), "a normal window never picks the quick workspace");
    let only_normal = m.snapshot().workspaces.iter().filter(|w| !w.quick).count();
    assert_eq!(only_normal, 1, "the model's first workspace is still the only normal one");
}

#[test]
fn quick_flag_survives_the_layout_and_old_layouts_load() {
    let mut m = Model::default();
    let q = m.quick_workspace();
    let back = Model::from_json(&m.to_json()).unwrap();
    assert!(back.snapshot().workspaces.iter().any(|w| w.id == q && w.quick));
    let mut v: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
    for w in v["workspaces"].as_array_mut().unwrap() {
        assert!(w.as_object_mut().unwrap().remove("quick").is_some());
    }
    let old = Model::from_json(&v.to_string()).expect("a layout from before the flag still loads");
    assert!(old.snapshot().workspaces.iter().all(|w| !w.quick));
}
