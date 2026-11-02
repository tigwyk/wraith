//! `wr events` (spec §3.1): the server sends a model snapshot after every change; the difference
//! between two snapshots becomes JSON events, one per line. Text in them is untrusted (§5.5) and
//! leaves only through serde_json, which escapes it.
use serde_json::{Value, json};
use wraith_proto::model::{AgentState, ModelSnapshot, PaneMeta};

/// Events that turn `old` into `new`: workspaces, then panes, then agents.
pub fn diff(old: &ModelSnapshot, new: &ModelSnapshot) -> Vec<Value> {
    let mut out = Vec::new();
    for w in &new.workspaces {
        match old.workspace(w.id) {
            None => out.push(json!({"event": "workspace_opened", "workspace": w.id, "name": w.name})),
            Some(o) if o.name != w.name => out.push(json!({"event": "workspace_renamed", "workspace": w.id, "name": w.name})),
            Some(_) => {}
        }
    }
    for w in old.workspaces.iter().filter(|w| new.workspace(w.id).is_none()) {
        out.push(json!({"event": "workspace_closed", "workspace": w.id}));
    }
    let mut agents = Vec::new();
    for p in &new.panes {
        let Some(o) = old.pane(p.id) else {
            let ws = new.tabs.iter().find(|t| t.root.contains(p.id)).map(|t| t.workspace);
            out.push(json!({"event": "pane_opened", "pane": p.id, "workspace": ws, "title": p.title, "cwd": p.cwd}));
            if p.agent.is_some() {
                agents.push(agent(p));
            }
            continue;
        };
        if o.title != p.title {
            out.push(json!({"event": "title", "pane": p.id, "title": p.title}));
        }
        if o.cwd != p.cwd {
            out.push(json!({"event": "cwd", "pane": p.id, "cwd": p.cwd}));
        }
        if o.exit != p.exit
            && let Some(code) = p.exit
        {
            out.push(json!({"event": "exited", "pane": p.id, "code": code}));
        }
        let key = |m: &PaneMeta| m.agent.as_ref().map(|a| (a.harness.clone(), a.state, a.attention, a.message.clone()));
        if key(o) != key(p) {
            agents.push(agent(p));
        }
    }
    for p in old.panes.iter().filter(|p| new.pane(p.id).is_none()) {
        out.push(json!({"event": "pane_closed", "pane": p.id}));
    }
    out.extend(agents);
    out
}

fn agent(p: &PaneMeta) -> Value {
    let a = p.agent.as_ref();
    let state = a.map(|a| match a.state {
        AgentState::Idle => "idle",
        AgentState::Working => "working",
        AgentState::Asking => "asking",
        AgentState::Done => "done",
    });
    json!({
        "event": "agent",
        "pane": p.id,
        "harness": a.map(|a| &a.harness),
        "state": state,
        "attention": a.is_some_and(|a| a.attention),
        "message": a.map(|a| &a.message),
    })
}
