//! The state engine (spec §5.3). Pure: the caller supplies time and visibility.
use std::time::{Duration, Instant};

/// Something an agent did, from a hook or seen in the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    Attached { harness: String },
    Detached,
    Working,
    Ask { message: String },
    TurnEnd { message: String },
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Hook,
    Passive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// The agent asked a question or for permission.
    Ask,
    /// The agent finished its turn.
    TurnEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Waiting(Wait),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub harness: String,
    pub state: State,
    /// The user should look at this pane.
    pub attention: bool,
    pub message: String,
    /// When `state` last changed.
    pub since: Instant,
    last_hook: Option<Instant>,
}

pub struct Ctx {
    /// Some window shows the pane right now.
    pub visible: bool,
    pub now: Instant,
}

/// After a hook event, passive state signals are ignored this long.
const HOOK_PRIORITY: Duration = Duration::from_secs(600);

/// The agent after `sig`. `None` means no agent in the pane.
pub fn step(a: Option<Agent>, sig: &Signal, src: Source, ctx: &Ctx) -> Option<Agent> {
    match sig {
        Signal::Detached => return None,
        Signal::Attached { harness } => {
            let mut a = a.unwrap_or_else(|| Agent {
                harness: String::new(),
                state: State::Idle,
                attention: false,
                message: String::new(),
                since: ctx.now,
                last_hook: None,
            });
            a.harness.clone_from(harness);
            if src == Source::Hook {
                a.last_hook = Some(ctx.now);
            }
            return Some(a);
        }
        _ => {}
    }
    // State signals need an agent: a bell or a notification alone is not one.
    let mut a = a?;
    if src == Source::Passive && a.last_hook.is_some_and(|t| ctx.now.saturating_duration_since(t) < HOOK_PRIORITY) {
        return Some(a);
    }
    if src == Source::Hook {
        a.last_hook = Some(ctx.now);
    }
    let (state, attention, message) = match sig {
        Signal::Working => (State::Working, false, String::new()),
        Signal::Ask { message } => (State::Waiting(Wait::Ask), true, message.clone()),
        Signal::TurnEnd { message } => (State::Waiting(Wait::TurnEnd), !ctx.visible, message.clone()),
        _ => (State::Idle, false, String::new()),
    };
    if state != a.state {
        a.since = ctx.now;
    }
    (a.state, a.attention, a.message) = (state, attention, message);
    Some(a)
}

/// The user typed in the pane: an answered question or a seen result is no longer waiting.
pub fn typed(a: &mut Agent) {
    if matches!(a.state, State::Waiting(_)) {
        (a.state, a.attention) = (State::Idle, false);
        a.message.clear();
    }
}

/// The pane came into view: a finished turn has been seen. An Ask stays until answered.
pub fn became_visible(a: &mut Agent) {
    if a.state == State::Waiting(Wait::TurnEnd) {
        (a.state, a.attention) = (State::Idle, false);
        a.message.clear();
    }
}
