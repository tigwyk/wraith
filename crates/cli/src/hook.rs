//! `wr hook <harness> [event]` (spec §5.1): tell the server what the agent in this pane is doing.
//! Always exits 0, within 200 ms, and never writes to stdout (harnesses may parse it).
use std::io::Read;
use std::time::Duration;

use wraith_ipc::connect;
use wraith_proto::pane::AgentSignal;
use wraith_proto::wire::{ClientKind, ClientMsg, PROTO_VERSION, PaneId, ServerMsg, read_frame, write_frame};

/// Harness payloads are capped (spec §5.1).
const MAX_PAYLOAD: u64 = 64 * 1024;
const DEADLINE: Duration = Duration::from_millis(200);

/// Run the hook and exit the process with 0, whatever happens.
pub fn run(args: &[&str]) -> ! {
    // A hung pipe or a slow server must never hold up the agent.
    std::thread::spawn(|| {
        std::thread::sleep(DEADLINE);
        std::process::exit(0);
    });
    let _ = send(args);
    std::process::exit(0)
}

fn send(args: &[&str]) -> Option<()> {
    let pane: PaneId = std::env::var("WRAITH_PANE_ID").ok()?.parse().ok()?;
    let pipe = std::env::var("WRAITH_PIPE").ok()?;
    let (harness, signal) = parse(args)?;
    let (mut r, mut w) = connect(&pipe).ok()?.split();
    write_frame(&mut w, &ClientMsg::Hello { proto_version: PROTO_VERSION, kind: ClientKind::Hook }).ok()?;
    matches!(read_frame::<ServerMsg>(&mut r).ok()?, ServerMsg::Welcome { .. }).then_some(())?;
    write_frame(&mut w, &ClientMsg::AgentEvent { pane, harness: Some(harness), signal }).ok()
}

fn stdin_json() -> serde_json::Value {
    let mut buf = Vec::new();
    let _ = std::io::stdin().take(MAX_PAYLOAD).read_to_end(&mut buf);
    serde_json::from_slice(&buf).unwrap_or(serde_json::Value::Null)
}

fn text(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|m| m.as_str()).unwrap_or_default().to_string()
}

/// The harness name and the signal a hook invocation means.
fn parse(args: &[&str]) -> Option<(String, AgentSignal)> {
    match args {
        ["claude", rest @ ..] => {
            let v = stdin_json();
            let event = rest.first().map_or_else(|| text(&v, "hook_event_name"), |e| (*e).to_string());
            let signal = match event.as_str() {
                "SessionStart" => AgentSignal::Attached,
                "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => AgentSignal::Working,
                "Notification" => AgentSignal::Ask { message: text(&v, "message") },
                "Stop" => AgentSignal::TurnEnd { message: String::new() },
                "SessionEnd" => AgentSignal::Detached,
                _ => return None,
            };
            Some(("claude".into(), signal))
        }
        // Codex passes its JSON as the last argument (`notify`).
        ["codex", .., payload] => {
            let v: serde_json::Value = serde_json::from_str(payload).ok()?;
            (text(&v, "type") == "agent-turn-complete")
                .then(|| ("codex".into(), AgentSignal::TurnEnd { message: text(&v, "last-assistant-message") }))
        }
        ["generic", rest @ ..] => {
            let opt = |name: &str| rest.iter().position(|a| *a == name).and_then(|i| rest.get(i + 1)).map(|s| (*s).to_string());
            let message = opt("--message").unwrap_or_default();
            let signal = match opt("--state")?.as_str() {
                "working" => AgentSignal::Working,
                "ask" => AgentSignal::Ask { message },
                "turn-end" => AgentSignal::TurnEnd { message },
                "idle" => AgentSignal::Idle,
                "detached" => AgentSignal::Detached,
                _ => return None,
            };
            Some((opt("--name").unwrap_or_else(|| "agent".into()), signal))
        }
        _ => None,
    }
}
