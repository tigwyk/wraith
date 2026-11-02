//! The agent state engine (spec §5.3), as event sequences and the state they must end in.
use std::time::{Duration, Instant};

use wraith_agent::{Agent, Ctx, Signal, Source, State, Wait, became_visible, step, typed};

fn ctx(visible: bool, now: Instant) -> Ctx {
    Ctx { visible, now }
}

fn run(events: &[(Signal, Source, bool)]) -> Option<Agent> {
    let t0 = Instant::now();
    events.iter().enumerate().fold(None, |a, (i, (sig, src, visible))| step(a, sig, *src, &ctx(*visible, t0 + Duration::from_secs(i as u64))))
}

fn attached() -> (Signal, Source, bool) {
    (Signal::Attached { harness: "claude".into() }, Source::Hook, false)
}

fn ask(src: Source, visible: bool) -> (Signal, Source, bool) {
    (Signal::Ask { message: "Allow Bash?".into() }, src, visible)
}

#[test]
fn ask_latches_until_typed() {
    let mut a = run(&[attached(), ask(Source::Hook, true), (Signal::Working, Source::Passive, true)]).unwrap();
    assert_eq!(a.state, State::Waiting(Wait::Ask), "a passive signal does not clear a hook Ask");
    assert!(a.attention);
    assert_eq!(a.message, "Allow Bash?");
    became_visible(&mut a);
    assert!(a.attention, "viewing does not clear an Ask");
    typed(&mut a);
    assert_eq!((a.state, a.attention), (State::Idle, false));
}

#[test]
fn turn_end_attention_only_when_hidden_and_clears_on_view() {
    let shown = run(&[attached(), (Signal::TurnEnd { message: "done".into() }, Source::Hook, true)]).unwrap();
    assert_eq!(shown.state, State::Waiting(Wait::TurnEnd));
    assert!(!shown.attention, "the user is looking at it");
    let mut hidden = run(&[attached(), (Signal::TurnEnd { message: "done".into() }, Source::Hook, false)]).unwrap();
    assert!(hidden.attention);
    became_visible(&mut hidden);
    assert_eq!((hidden.state, hidden.attention), (State::Idle, false));
}

#[test]
fn passive_signals_ignored_within_10_min_of_hook() {
    let t0 = Instant::now();
    let a = step(None, &Signal::Attached { harness: "claude".into() }, Source::Hook, &ctx(false, t0));
    let a = step(a, &Signal::Working, Source::Hook, &ctx(false, t0));
    let a = step(a, &Signal::Ask { message: "bell".into() }, Source::Passive, &ctx(false, t0 + Duration::from_secs(599)));
    assert_eq!(a.as_ref().unwrap().state, State::Working, "hooks win for 10 minutes");
    let a = step(a, &Signal::Ask { message: "bell".into() }, Source::Passive, &ctx(false, t0 + Duration::from_secs(601)));
    assert_eq!(a.unwrap().state, State::Waiting(Wait::Ask), "then passive signals count again");
}

#[test]
fn passive_attach_still_sets_harness_after_hook() {
    let t0 = Instant::now();
    let a = step(None, &Signal::Attached { harness: "agent".into() }, Source::Hook, &ctx(false, t0));
    let a = step(a, &Signal::Working, Source::Hook, &ctx(false, t0));
    let a = step(a, &Signal::Attached { harness: "codex".into() }, Source::Passive, &ctx(false, t0)).unwrap();
    assert_eq!(a.harness, "codex", "icon detection keeps running");
    assert_eq!(a.state, State::Working, "attaching does not reset the state");
}

#[test]
fn unknown_or_empty_message_defaults_idle() {
    let a = run(&[attached()]).unwrap();
    assert_eq!((a.state, a.attention), (State::Idle, false), "a fresh agent is idle");
    let a = run(&[attached(), ask(Source::Hook, false), (Signal::Idle, Source::Hook, false)]).unwrap();
    assert_eq!((a.state, a.attention), (State::Idle, false));
}

#[test]
fn detached_clears_everything() {
    assert!(run(&[attached(), ask(Source::Hook, false), (Signal::Detached, Source::Passive, false)]).is_none());
}

#[test]
fn state_signals_without_an_agent_create_nothing() {
    assert!(run(&[ask(Source::Passive, false)]).is_none(), "a bell with no agent attached is just a bell");
    assert!(run(&[(Signal::Working, Source::Hook, false)]).is_none(), "the server attaches first");
}

#[test]
fn working_clears_attention_and_message() {
    let a = run(&[attached(), ask(Source::Hook, false), (Signal::Working, Source::Hook, false)]).unwrap();
    assert_eq!((a.state, a.attention, a.message.as_str()), (State::Working, false, ""));
}
