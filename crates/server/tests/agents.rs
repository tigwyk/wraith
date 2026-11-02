//! Toast text built from untrusted terminal titles (spec §5.5).
use wraith_server::agents::notify_title;

#[test]
fn notify_title_strips_control_characters_from_titles() {
    let t0 = std::time::Instant::now();
    let a = wraith_agent::step(None, &wraith_agent::Signal::Attached { harness: "claude".into() }, wraith_agent::Source::Hook, &wraith_agent::Ctx { visible: false, now: t0 })
        .and_then(|a| wraith_agent::step(Some(a), &wraith_agent::Signal::Ask { message: String::new() }, wraith_agent::Source::Hook, &wraith_agent::Ctx { visible: false, now: t0 }))
        .unwrap();
    let t = notify_title(&a, "ws\u{1b}[31m", "tab\u{7}\u{0}name");
    assert_eq!(t, "Claude Code needs input — ws[31m/tabname");
}
