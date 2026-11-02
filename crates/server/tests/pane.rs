//! Pane engine: ConPTY + libghostty Terminal on its own thread, emitting FrameDeltas.
#![cfg(windows)]
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use wraith_proto::{FrameDelta, KeyAction, KeyCode, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent};
use wraith_server::pane::{PaneCmd, PaneEvent, PaneHandle, PaneOptions, ScrollTo, WheelAction, spawn_pane, wheel_action};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn emit(args: &str) -> String {
    format!("\"{}\" {args}", env!("CARGO_BIN_EXE_emit-seq"))
}

fn opts(cmdline: &str) -> PaneOptions {
    PaneOptions {
        cmdline: cmdline.into(),
        cwd: None,
        env: vec![],
        cols: 80,
        rows: 24,
        cell_w_px: 8,
        cell_h_px: 16,
        scrollback_lines: 1000,
        default_fg: (255, 255, 255),
        default_bg: (0, 0, 0),
        palette: vec![(1, 2, 3); 256],
        image_storage_limit: 64 << 20,
        agent_detect: vec![],
        restore: None,
    }
}

/// Client-side mirror of the screen, fed by frames.
#[derive(Default)]
struct Mirror {
    rows: BTreeMap<u16, String>,
    last: Option<FrameDelta>,
    first: Option<FrameDelta>,
}

impl Mirror {
    fn apply(&mut self, f: FrameDelta) {
        if f.full {
            self.rows.clear();
        }
        for r in &f.dirty_rows {
            self.rows.insert(r.y, r.runs.iter().map(|s| s.text.as_str()).collect());
        }
        if self.first.is_none() {
            self.first = Some(f.clone());
        }
        self.last = Some(f);
    }
    fn contains(&self, needle: &str) -> bool {
        self.rows.values().any(|r| r.contains(needle))
    }
    /// Highest `line-N` visible; floods skip intermediate screens, so wait on this, not an exact line.
    fn max_line(&self) -> u64 {
        self.rows
            .values()
            .filter_map(|r| r.trim_start().strip_prefix("line-")?.split(' ').next()?.parse().ok())
            .max()
            .unwrap_or(0)
    }
}

/// Pump events, acking every frame, until `pred` holds or `timeout`. Returns non-frame events seen.
fn pump(
    h: &PaneHandle,
    m: &mut Mirror,
    timeout: Duration,
    mut pred: impl FnMut(&Mirror, &[PaneEvent]) -> bool,
) -> (bool, Vec<PaneEvent>) {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match h.events.recv_timeout(left) {
            Ok(PaneEvent::Frame(f)) => {
                let seq = f.seq;
                m.apply(f);
                h.send(PaneCmd::Ready(seq)).unwrap();
            }
            Ok(ev) => seen.push(ev),
            Err(_) => break,
        }
        if pred(m, &seen) {
            return (true, seen);
        }
    }
    (false, seen)
}

fn wait_text(h: &PaneHandle, needle: &str, timeout: Duration) -> bool {
    pump(h, &mut Mirror::default(), timeout, |m, _| m.contains(needle)).0
}

fn key(code: KeyCode, text: Option<&str>) -> KeyEvent {
    KeyEvent { code, mods: Mods(0), text: text.map(Into::into), action: KeyAction::Press, composing: false }
}

fn waits(ev: Vec<PaneEvent>) -> impl Iterator<Item = Duration> {
    ev.into_iter().filter_map(|e| match e {
        PaneEvent::InputWritten { waited } => Some(waited),
        _ => None,
    })
}

#[test]
fn echo_reaches_frame() {
    let h = spawn_pane(opts(&emit("print pane-ok"))).unwrap();
    assert!(wait_text(&h, "pane-ok", secs(5)));
}

#[test]
fn image_upload_precedes_the_frame_that_places_it() {
    let h = spawn_pane(opts(&emit("kitty"))).unwrap();
    let deadline = Instant::now() + secs(5);
    let mut uploaded = false;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match h.events.recv_timeout(left) {
            Ok(PaneEvent::ImageUpload { width: 1, height: 1, rgba, .. }) => {
                assert_eq!(rgba, vec![0, 0, 0, 255]);
                uploaded = true;
            }
            Ok(PaneEvent::Frame(f)) => {
                h.send(PaneCmd::Ready(f.seq)).unwrap();
                if !f.images.is_empty() {
                    assert!(uploaded, "the frame placing an image came before its upload");
                    return;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    panic!("no frame with the image placement");
}

#[test]
fn typed_keys_reach_shell() {
    let h = spawn_pane(opts("cmd.exe /k")).unwrap();
    h.send(PaneCmd::Text("echo typed-".into())).unwrap();
    h.send(PaneCmd::Key(key(KeyCode::KeyO, Some("o")))).unwrap();
    h.send(PaneCmd::Key(key(KeyCode::KeyK, Some("k")))).unwrap();
    h.send(PaneCmd::Key(key(KeyCode::Enter, None))).unwrap();
    // cmd echoes the command line too, so require the output line on its own.
    let (ok, _) = pump(&h, &mut Mirror::default(), secs(5), |m, _| m.rows.values().any(|r| r.trim_end() == "typed-ok"));
    assert!(ok);
}

#[test]
fn flood_coalesces_to_one_pending_frame() {
    let h = spawn_pane(opts(&emit("flood 20000"))).unwrap();
    std::thread::sleep(secs(3)); // never send Ready
    let mut seqs = Vec::new();
    while let Ok(ev) = h.events.try_recv() {
        if let PaneEvent::Frame(f) = ev {
            seqs.push(f.seq);
        }
    }
    assert_eq!(seqs.len(), 1, "only one frame may be sent without credit");
    h.send(PaneCmd::Ready(seqs[0])).unwrap();
    assert!(wait_text(&h, "line-20000 ", secs(10)), "merged frame must carry the final state");
}

#[test]
fn first_frame_is_full_and_has_palette() {
    let h = spawn_pane(opts(&emit("print x"))).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(5), |m, _| m.first.is_some()).0);
    let first = m.first.unwrap();
    assert!(first.full);
    assert_eq!(first.colors.palette.len(), 256);
    assert_eq!((first.cols, first.rows), (80, 24));
}

#[test]
fn resize_zero_is_clamped() {
    let h = spawn_pane(opts("cmd.exe /k")).unwrap();
    let mut m = Mirror::default();
    pump(&h, &mut m, secs(5), |m, _| m.last.is_some());
    h.send(PaneCmd::Resize { cols: 0, rows: 0, cell_w_px: 8, cell_h_px: 16 }).unwrap();
    let (ok, _) = pump(&h, &mut m, secs(5), |m, _| {
        m.last.as_ref().is_some_and(|f| f.full && (f.cols, f.rows) == (2, 1))
    });
    assert!(ok);
}

#[test]
fn exit_is_reported_and_restart_works() {
    let exited = |_: &Mirror, ev: &[PaneEvent]| ev.iter().any(|e| matches!(e, PaneEvent::Exited(3)));
    let h = spawn_pane(opts("cmd.exe /c exit 3")).unwrap();
    assert!(pump(&h, &mut Mirror::default(), secs(5), exited).0, "expected Exited(3)");
    h.send(PaneCmd::Restart).unwrap();
    assert!(pump(&h, &mut Mirror::default(), secs(5), exited).0, "restarted process must run and exit again");
}

#[test]
fn multiline_paste_needs_confirm() {
    let h = spawn_pane(opts("cmd.exe /k")).unwrap();
    h.send(PaneCmd::Paste("echo a\necho b".into())).unwrap();
    let (ok, _) = pump(&h, &mut Mirror::default(), secs(3), |_, ev| {
        ev.iter().any(|e| matches!(e, PaneEvent::PasteNeedsConfirm(t) if t == "echo a\necho b"))
    });
    assert!(ok);
}

#[test]
fn wide_char_run_text() {
    let h = spawn_pane(opts(&emit("print 日本x"))).unwrap();
    let mut m = Mirror::default();
    let (ok, _) = pump(&h, &mut m, secs(5), |m, _| m.contains("日本x"));
    assert!(ok, "rows: {:?}", m.rows);
}

#[test]
fn title_event() {
    let h = spawn_pane(opts(&emit("title"))).unwrap();
    let (ok, _) = pump(&h, &mut Mirror::default(), secs(5), |_, ev| {
        ev.iter().any(|e| matches!(e, PaneEvent::Title(t) if t == "wraith-title"))
    });
    assert!(ok);
}

#[test]
fn copy_selection_returns_text() {
    let h = spawn_pane(opts(&emit("print hello-copy"))).unwrap();
    let mut m = Mirror::default();
    pump(&h, &mut m, secs(5), |m, _| m.contains("hello-copy"));
    let y = *m.rows.iter().find(|(_, r)| r.contains("hello-copy")).unwrap().0;
    h.send(PaneCmd::SelectLine { y }).unwrap();
    h.send(PaneCmd::CopySelection).unwrap();
    let (ok, ev) = pump(&h, &mut m, secs(3), |_, ev| ev.iter().any(|e| matches!(e, PaneEvent::Copied(_))));
    assert!(ok);
    assert!(ev.iter().any(|e| matches!(e, PaneEvent::Copied(t) if t.trim_end() == "hello-copy")), "{ev:?}");
}

#[test]
fn wheel_scrolls_viewport_on_primary_screen() {
    let h = spawn_pane(opts(&emit("flood 200"))).unwrap();
    let mut m = Mirror::default();
    pump(&h, &mut m, secs(5), |m, _| m.contains("line-200 "));
    let before = m.last.as_ref().unwrap().scrollbar.offset;
    let wheel = MouseEvent {
        x_px: 1.0,
        y_px: 1.0,
        button: Some(MouseButton::WheelUp),
        action: MouseAction::Press,
        mods: Mods(0),
    };
    h.send(PaneCmd::Mouse(wheel)).unwrap();
    let (ok, _) = pump(&h, &mut m, secs(3), |m, _| m.last.as_ref().unwrap().scrollbar.offset + 3 == before);
    assert!(ok, "offset before {before}, after {}", m.last.unwrap().scrollbar.offset);
}

#[test]
fn wheel_action_rules() {
    // (alt_screen, mouse_tracking, up)
    assert_eq!(wheel_action(false, false, true), WheelAction::Scroll(-3));
    assert_eq!(wheel_action(false, false, false), WheelAction::Scroll(3));
    assert_eq!(wheel_action(true, false, true), WheelAction::Arrows { up: true, count: 3 });
    assert_eq!(wheel_action(false, true, true), WheelAction::Report);
    assert_eq!(wheel_action(true, true, false), WheelAction::Report);
}

#[test]
fn input_not_starved_by_flood() {
    let h = spawn_pane(opts(&emit("flood"))).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(5), |m, _| m.max_line() >= 1000).0);
    let mut all = Vec::new();
    for _ in 0..50 {
        h.send(PaneCmd::Text("x".into())).unwrap();
        all.extend(waits(pump(&h, &mut m, Duration::from_millis(20), |_, _| false).1));
    }
    all.extend(waits(pump(&h, &mut m, Duration::from_millis(300), |_, _| false).1));
    assert_eq!(all.len(), 50);
    all.sort();
    // Shared 2-vCPU CI runners preempt us now and then (stalls of 65 ms seen, sometimes two in 50
    // keys), so the latency gate only holds on a real machine. The rest of the test runs everywhere.
    if std::env::var_os("CI").is_none() {
        assert!(all[49] < Duration::from_millis(5), "max wait {:?}", all[49]);
        assert!(all[48] < Duration::from_millis(2), "p99 wait {:?}", all[48]);
    }
}

fn working_set() -> usize {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS::default();
    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut c, size_of::<PROCESS_MEMORY_COUNTERS>() as u32).unwrap();
    }
    c.WorkingSetSize
}

#[test]
fn memory_flat_after_scrollback_full() {
    let mut o = opts(&emit("flood 200000"));
    o.scrollback_lines = 10_000;
    let h = spawn_pane(o).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(60), |m, _| m.max_line() >= 50_000).0);
    let at_50k = working_set();
    assert!(pump(&h, &mut m, secs(120), |m, _| m.contains("line-200000 ")).0);
    let growth = working_set().saturating_sub(at_50k);
    assert!(growth < 16 << 20, "grew {} MiB after scrollback was full", growth >> 20);
}

#[test]
fn selection_marks_cells_in_frames() {
    let h = spawn_pane(opts(&emit("print hello-sel"))).unwrap();
    let mut m = Mirror::default();
    pump(&h, &mut m, secs(5), |m, _| m.contains("hello-sel"));
    let y = *m.rows.iter().find(|(_, r)| r.contains("hello-sel")).unwrap().0;
    h.send(PaneCmd::SelectLine { y }).unwrap();
    let (ok, _) = pump(&h, &mut m, secs(3), |m, _| {
        m.last.as_ref().is_some_and(|f| {
            f.dirty_rows.iter().any(|r| r.y == y && r.runs.iter().any(|s| s.flags.has(wraith_proto::StyleFlags::SELECTED) && s.text.contains("hello-sel")))
        })
    });
    assert!(ok, "a frame must mark the selected row's cells");
}

#[test]
fn redraw_forces_full_frame_without_size_change() {
    let h = spawn_pane(opts(&emit("print x"))).unwrap();
    let mut m = Mirror::default();
    pump(&h, &mut m, secs(5), |m, _| m.contains("x"));
    let before = m.last.as_ref().unwrap().seq;
    h.send(PaneCmd::Redraw).unwrap();
    let (ok, _) = pump(&h, &mut m, secs(3), |m, _| m.last.as_ref().is_some_and(|f| f.seq > before && f.full));
    assert!(ok, "font changes must get every row re-sent");
}

#[test]
fn reattach_resends_full_frame_without_ack() {
    let h = spawn_pane(opts(&emit("print hi"))).unwrap();
    // Take the first frame but never ack it, like a client that disconnected.
    let first = loop {
        if let PaneEvent::Frame(f) = h.events.recv_timeout(secs(5)).unwrap() {
            break f;
        }
    };
    h.send(PaneCmd::Reattach).unwrap();
    let deadline = Instant::now() + secs(3);
    let again = loop {
        match h.events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(PaneEvent::Frame(f)) => break Some(f),
            Ok(_) => continue,
            Err(_) => break None,
        }
    };
    let again = again.expect("reattach must produce a frame even though the last one was never acked");
    assert!(again.full && again.seq > first.seq);
}

#[test]
fn cwd_from_uri_converts_drive_and_posix_paths() {
    use wraith_server::pane::cwd_from_uri;
    assert_eq!(cwd_from_uri("file://host/C:/Users/Lee").as_deref(), Some(r"C:\Users\Lee"));
    assert_eq!(cwd_from_uri("file:///C:/x%20y").as_deref(), Some(r"C:\x y"));
    assert_eq!(cwd_from_uri("file://wsl/home/lee/app").as_deref(), Some("/home/lee/app"));
    assert_eq!(cwd_from_uri("file://host/C:/bad%zz").as_deref(), Some(r"C:\bad%zz"));
    assert_eq!(cwd_from_uri("http://x/y"), None);
    assert_eq!(cwd_from_uri("file://nohostpath"), None);
}

#[test]
fn osc7_sends_cwd_event() {
    let h = spawn_pane(opts(&emit("osc7"))).unwrap();
    let deadline = Instant::now() + secs(5);
    let mut got = None;
    while got.is_none() && Instant::now() < deadline {
        match h.events.recv_timeout(Duration::from_millis(100)) {
            Ok(PaneEvent::Cwd(c)) => got = Some(c),
            Ok(PaneEvent::Frame(f)) => {
                let _ = h.send(PaneCmd::Ready(f.seq));
            }
            _ => {}
        }
    }
    assert_eq!(got.as_deref(), Some(r"C:\tmp"));
}

// ---- Passive agent signals (spec §5.2) ----

use wraith_proto::pane::AgentSignal;

/// Agent events from a pane running `cmdline`, until `want` returns true or 5 s pass.
fn agent_events(cmdline: &str, mut want: impl FnMut(&[PaneEvent]) -> bool) -> Vec<PaneEvent> {
    let h = spawn_pane(opts(cmdline)).unwrap();
    let mut got = Vec::new();
    let deadline = Instant::now() + secs(5);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match h.events.recv_timeout(left) {
            Ok(PaneEvent::Frame(f)) => h.send(PaneCmd::Ready(f.seq)).unwrap(),
            Ok(ev @ (PaneEvent::Agent(_) | PaneEvent::AgentProcess(_))) => {
                got.push(ev);
                if want(&got) {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    got
}

fn has(evs: &[PaneEvent], sig: &AgentSignal) -> bool {
    evs.iter().any(|e| matches!(e, PaneEvent::Agent(s) if s == sig))
}

#[test]
fn osc9_and_osc777_notifications_become_ask() {
    let want = AgentSignal::Ask { message: "wraith-osc9".into() };
    assert!(has(&agent_events(&emit("osc9"), |e| has(e, &want)), &want));
    let want = AgentSignal::Ask { message: "Claude: needs you".into() };
    let got = agent_events(&emit("osc777-notify"), |e| has(e, &want));
    assert!(has(&got, &want), "{got:?}");
}

#[test]
fn osc7501_states_map() {
    let want = AgentSignal::Ask { message: "Allow Bash?".into() };
    let got = agent_events(&emit("osc7501-blocked"), |e| has(e, &want));
    assert!(has(&got, &want), "{got:?}");
    assert!(has(&agent_events(&emit("osc7501-working"), |e| has(e, &AgentSignal::Working)), &AgentSignal::Working));
    let done = AgentSignal::TurnEnd { message: String::new() };
    assert!(has(&agent_events(&emit("osc7501-done"), |e| has(e, &done)), &done));
}

#[test]
fn progress_and_bel_signals() {
    assert!(has(&agent_events(&emit("progress"), |e| has(e, &AgentSignal::Working)), &AgentSignal::Working));
    let bell = AgentSignal::Ask { message: String::new() };
    assert!(has(&agent_events(&emit("bel"), |e| has(e, &bell)), &bell));
}

#[test]
fn process_scan_attaches_and_detaches() {
    // A copy of emit-seq named like a known agent stands in for one.
    let dir = std::env::temp_dir().join(format!("wraith-scan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("codex.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_emit-seq"), &fake).unwrap();
    let cmd = format!("cmd.exe /c \"\"{}\" stream 40 50 & ping -n 3 127.0.0.1 >nul\"", fake.display());
    let got = agent_events(&cmd, |e| e.iter().any(|x| matches!(x, PaneEvent::AgentProcess(None))));
    let seen: Vec<Option<String>> = got.iter().filter_map(|e| if let PaneEvent::AgentProcess(p) = e { Some(p.clone()) } else { None }).collect();
    assert_eq!(seen, vec![Some("codex".to_string()), None], "attached while it runs, detached after");
}

// ---- Scrollback search (M5) ----

use wraith_proto::pane::SearchCmd;
use wraith_proto::SearchView;

/// Pump until a frame's search view satisfies `pred`; returns that view.
fn until_search(h: &PaneHandle, m: &mut Mirror, t: Duration, mut pred: impl FnMut(&Option<SearchView>) -> bool) -> Option<Option<SearchView>> {
    let mut got = None;
    pump(h, m, t, |m, _| {
        let s = m.last.as_ref().map(|f| f.search.clone()).unwrap_or(None);
        if pred(&s) {
            got = Some(s);
            true
        } else {
            false
        }
    });
    got
}

#[test]
fn search_finds_and_highlights_viewport_matches() {
    let h = spawn_pane(opts(&emit("print xx needle yy NEEDLE"))).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(5), |m, _| m.contains("needle")).0);
    h.send(PaneCmd::Search(SearchCmd::Set("needle".into()))).unwrap();
    let s = until_search(&h, &mut m, secs(5), |s| s.as_ref().is_some_and(|s| s.total == 2)).expect("two matches, ASCII case-insensitive").unwrap();
    let mut hits: Vec<(u16, u16, u16)> = s.hits.iter().map(|h| (h.y, h.x0, h.x1)).collect();
    hits.sort();
    assert_eq!(hits, vec![(0, 3, 8), (0, 13, 18)], "cell ranges, end inclusive");
}

#[test]
fn next_scrolls_to_older_match() {
    let h = spawn_pane(opts(&emit("flood 300"))).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(5), |m, _| m.max_line() >= 300).0);
    h.send(PaneCmd::Search(SearchCmd::Set("line-150 ".into()))).unwrap();
    assert!(until_search(&h, &mut m, secs(5), |s| s.as_ref().is_some_and(|s| s.total == 1)).is_some());
    h.send(PaneCmd::Search(SearchCmd::Next)).unwrap();
    let s = until_search(&h, &mut m, secs(5), |s| s.as_ref().is_some_and(|s| s.selected == Some(0) && s.hits.iter().any(|h| h.selected))).expect("the match scrolled into view, selected");
    let y = s.unwrap().hits.iter().find(|h| h.selected).unwrap().y;
    assert!(m.rows.get(&y).is_some_and(|r| r.contains("line-150 ")), "highlight row shows the match: {:?}", m.rows.get(&y));
}

#[test]
fn close_clears_highlights() {
    let h = spawn_pane(opts(&emit("print a needle"))).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(5), |m, _| m.contains("needle")).0);
    h.send(PaneCmd::Search(SearchCmd::Set("needle".into()))).unwrap();
    assert!(until_search(&h, &mut m, secs(5), |s| s.as_ref().is_some_and(|s| s.total == 1)).is_some());
    h.send(PaneCmd::Search(SearchCmd::Close)).unwrap();
    assert!(until_search(&h, &mut m, secs(5), Option::is_none).is_some());
}

#[test]
fn search_keeps_input_fast_while_flooding() {
    let h = spawn_pane(opts(&emit("flood"))).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(5), |m, _| m.max_line() >= 1000).0);
    h.send(PaneCmd::Search(SearchCmd::Set("fox".into()))).unwrap();
    let mut all = Vec::new();
    for _ in 0..50 {
        h.send(PaneCmd::Text("x".into())).unwrap();
        all.extend(waits(pump(&h, &mut m, Duration::from_millis(20), |_, _| false).1));
    }
    all.extend(waits(pump(&h, &mut m, Duration::from_millis(300), |_, _| false).1));
    assert_eq!(all.len(), 50);
    all.sort();
    // Shared 2-vCPU CI runners preempt us now and then (stalls of 65 ms seen, sometimes two in 50
    // keys), so the latency gate only holds on a real machine. The rest of the test runs everywhere.
    if std::env::var_os("CI").is_none() {
        assert!(all[49] < Duration::from_millis(5), "max wait {:?}", all[49]);
        assert!(all[48] < Duration::from_millis(2), "p99 wait {:?}", all[48]);
    }
    assert!(m.last.as_ref().and_then(|f| f.search.as_ref()).is_some_and(|s| s.total > 0), "matches keep coming");
}

#[test]
fn scrollback_keeps_the_configured_lines_not_a_byte_default() {
    // libghostty defaults to a ~10 KB byte cap; Wraith's limit is in lines (from scrollback-limit).
    let mut o = opts(&emit("flood 5000"));
    o.scrollback_lines = 10_000;
    let h = spawn_pane(o).unwrap();
    let mut m = Mirror::default();
    assert!(pump(&h, &mut m, secs(10), |m, _| m.max_line() >= 5000).0);
    h.send(PaneCmd::Scroll(ScrollTo::Top)).unwrap();
    let top = pump(&h, &mut m, secs(5), |m, _| m.rows.get(&0).is_some_and(|r| r.starts_with("line-1 "))).0;
    assert!(top, "line-1 is still the oldest line; top is {:?}", m.rows.get(&0));
}
