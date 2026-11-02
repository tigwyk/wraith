//! `wr events`: successive model snapshots become JSON event lines.
use serde_json::{Value, json};
use wraith_cli::events::diff;
use wraith_proto::model::{AgentInfo, AgentState, ModelSnapshot, PaneMeta, PaneNode, Tab, Workspace};

fn snap() -> ModelSnapshot {
    ModelSnapshot {
        rev: 1,
        workspaces: vec![Workspace {
            id: 1,
            name: "api".into(),
            root_dir: None,
            color: 0,
            tabs: vec![10],
            active_tab: Some(10),
            profile: "pwsh".into(),
            attached: true,
            last_used: 0,
        }],
        tabs: vec![Tab { id: 10, workspace: 1, title_override: None, root: PaneNode::Leaf(100), focused: 100, zoomed: None }],
        panes: vec![PaneMeta {
            id: 100,
            title: "pwsh".into(),
            cwd: Some(r"C:\src".into()),
            exit: None,
            profile: "pwsh".into(),
            cmdline: "pwsh".into(),
            agent: None,
        }],
    }
}

fn agent(state: AgentState, attention: bool) -> AgentInfo {
    AgentInfo { harness: "claude".into(), name: "Claude Code".into(), icon: '◆', state, attention, message: "May I?".into(), since: 0 }
}

fn kinds(v: &[Value]) -> Vec<&str> {
    v.iter().map(|e| e["event"].as_str().unwrap()).collect()
}

#[test]
fn first_snapshot_reports_everything_opened() {
    let ev = diff(&ModelSnapshot::default(), &snap());
    assert_eq!(kinds(&ev), ["workspace_opened", "pane_opened"]);
    assert_eq!(ev[0], json!({"event": "workspace_opened", "workspace": 1, "name": "api"}));
    assert_eq!(ev[1], json!({"event": "pane_opened", "pane": 100, "workspace": 1, "title": "pwsh", "cwd": r"C:\src"}));
}

#[test]
fn title_change_is_one_event() {
    let a = snap();
    let mut b = snap();
    b.panes[0].title = "vim".into();
    assert_eq!(diff(&a, &b), vec![json!({"event": "title", "pane": 100, "title": "vim"})]);
}

#[test]
fn agent_ask_then_detach() {
    let a = snap();
    let mut b = snap();
    b.panes[0].agent = Some(agent(AgentState::Asking, true));
    assert_eq!(
        diff(&a, &b),
        vec![json!({"event": "agent", "pane": 100, "harness": "claude", "state": "asking", "attention": true, "message": "May I?"})]
    );
    let ev = diff(&b, &a);
    assert_eq!(kinds(&ev), ["agent"]);
    assert_eq!(ev[0]["state"], Value::Null);
}

#[test]
fn closed_pane_and_workspace() {
    let ev = diff(&snap(), &ModelSnapshot::default());
    assert_eq!(ev, vec![json!({"event": "workspace_closed", "workspace": 1}), json!({"event": "pane_closed", "pane": 100})]);
}

#[test]
fn cwd_exit_and_rename() {
    let a = snap();
    let mut b = snap();
    b.workspaces[0].name = "web".into();
    b.panes[0].cwd = Some(r"D:\x".into());
    b.panes[0].exit = Some(3);
    assert_eq!(
        diff(&a, &b),
        vec![
            json!({"event": "workspace_renamed", "workspace": 1, "name": "web"}),
            json!({"event": "cwd", "pane": 100, "cwd": r"D:\x"}),
            json!({"event": "exited", "pane": 100, "code": 3}),
        ]
    );
}

#[test]
fn unchanged_snapshot_is_silent() {
    assert!(diff(&snap(), &snap()).is_empty());
}
