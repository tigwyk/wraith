//! Glue between the `agent` crate's engine and the wire model (spec §5.3, §5.4).
use std::time::Instant;

use wraith_agent::{Agent, Signal, State, Wait, harness, sanitize};
use wraith_proto::model::{AgentInfo, AgentState};
use wraith_proto::pane::AgentSignal;

/// The engine's signal for a wire signal; `harness` names a newly attached agent.
pub fn signal(s: AgentSignal, harness: Option<&str>) -> Signal {
    match s {
        AgentSignal::Attached => Signal::Attached { harness: harness.unwrap_or("agent").to_string() },
        AgentSignal::Detached => Signal::Detached,
        AgentSignal::Working => Signal::Working,
        AgentSignal::Ask { message } => Signal::Ask { message: sanitize(&message) },
        AgentSignal::TurnEnd { message } => Signal::TurnEnd { message: sanitize(&message) },
        AgentSignal::Idle => Signal::Idle,
    }
}

/// Display name of a harness id (an unknown id, e.g. from `agent-detect` or a generic hook, as is).
pub fn name(id: &str) -> String {
    harness(id).map_or_else(|| id.to_string(), |h| h.name.to_string())
}

pub fn info(a: &Agent, epoch: Instant) -> AgentInfo {
    AgentInfo {
        harness: a.harness.clone(),
        name: name(&a.harness),
        icon: harness(&a.harness).map_or('●', |h| h.icon),
        state: match a.state {
            State::Idle => AgentState::Idle,
            State::Working => AgentState::Working,
            State::Waiting(Wait::Ask) => AgentState::Asking,
            State::Waiting(Wait::TurnEnd) => AgentState::Done,
        },
        attention: a.attention,
        message: a.message.clone(),
        since: a.since.saturating_duration_since(epoch).as_millis() as u64,
    }
}

/// Toast title for an agent that wants the user: "<Agent> needs input — <workspace>/<tab>".
pub fn notify_title(a: &Agent, workspace: &str, tab: &str) -> String {
    let what = if a.state == State::Waiting(Wait::Ask) { "needs input" } else { "finished" };
    format!("{} {what} — {workspace}/{tab}", name(&a.harness))
}
