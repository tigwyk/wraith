use wraith_client::settings::{Edit, RowState, SettingsHit, SettingsView, Sources};
use wraith_config::Config;
use wraith_config::edit::{Origin, Which};
use wraith_config::settings::{Page, all};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};

fn rows() -> Vec<RowState> {
    all().iter().map(|_| RowState { origin: Origin::Default, warning: None }).collect()
}

fn sources() -> Sources {
    Sources { themes: vec!["Dracula".into(), "Nord".into()], fonts: vec!["Cascadia Mono".into(), "Consolas".into()], profiles: vec!["pwsh".into()] }
}

fn view() -> SettingsView {
    SettingsView::new(sources(), rows())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, mods: Mods(0), text: None, action: KeyAction::Press, composing: false }
}

fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent { mods: Mods(Mods::SHIFT), ..key(code) }
}

fn idx(k: &str) -> usize {
    all().iter().position(|s| s.key == k).unwrap()
}

/// Move the focus to `k` with Tab and ↓, as a user would.
fn focus(v: &mut SettingsView, k: &str) {
    let c = Config::default();
    let target = &all()[idx(k)];
    while v.page != target.page {
        v.key(&key(KeyCode::Tab), &c);
    }
    while v.focus != idx(k) {
        v.key(&key(KeyCode::ArrowDown), &c);
    }
}

fn set(key: &'static str, value: &str) -> Option<Edit> {
    Some(Edit::Set { key, value: value.into(), which: Which::Last })
}

#[test]
fn tab_changes_page_and_wraps() {
    let mut v = view();
    let c = Config::default();
    assert_eq!(v.page, Page::Appearance);
    assert_eq!(v.focus, idx("theme"));
    v.key(&key(KeyCode::Tab), &c);
    assert_eq!(v.page, Page::Behaviour);
    assert_eq!(v.focus, idx("copy-on-select"), "a new page focuses its first row");
    v.key(&key(KeyCode::Tab), &c);
    v.key(&key(KeyCode::Tab), &c);
    assert_eq!(v.page, Page::Appearance);
    v.key(&shift(KeyCode::Tab), &c);
    assert_eq!(v.page, Page::Agents);
}

#[test]
fn arrows_move_rows_and_stop_at_ends() {
    let mut v = view();
    let c = Config::default();
    v.key(&key(KeyCode::ArrowUp), &c);
    assert_eq!(v.focus, idx("theme"));
    for _ in 0..20 {
        v.key(&key(KeyCode::ArrowDown), &c);
    }
    assert_eq!(v.focus, idx("unfocused-split-opacity"), "the last Appearance row");
}

#[test]
fn stepper_clamps() {
    let mut v = view();
    focus(&mut v, "font-size");
    let mut c = Config::default();
    assert_eq!(v.key(&key(KeyCode::ArrowRight), &c), set("font-size", "13"));
    c.font_size = 36.0;
    assert_eq!(v.key(&key(KeyCode::ArrowRight), &c), set("font-size", "36"));
    c.font_size = 50.0; // from the file, out of range
    assert_eq!(v.key(&key(KeyCode::ArrowLeft), &c), set("font-size", "36"));
}

#[test]
fn slider_steps_by_005() {
    let mut v = view();
    focus(&mut v, "background-opacity");
    let mut c = Config::default();
    assert_eq!(v.key(&key(KeyCode::ArrowLeft), &c), set("background-opacity", "0.95"));
    c.background_opacity = 0.30;
    assert_eq!(v.key(&key(KeyCode::ArrowLeft), &c), set("background-opacity", "0.30"));
}

#[test]
fn choice_wraps() {
    let mut v = view();
    focus(&mut v, "cursor-style");
    let mut c = Config::default();
    c.cursor_style = wraith_config::CursorStyle::BlockHollow;
    assert_eq!(v.key(&key(KeyCode::ArrowRight), &c), set("cursor-style", "block"));
    c.cursor_style = wraith_config::CursorStyle::Block;
    assert_eq!(v.key(&key(KeyCode::ArrowLeft), &c), set("cursor-style", "block_hollow"));
}

#[test]
fn space_toggles() {
    let mut v = view();
    focus(&mut v, "copy-on-select");
    assert_eq!(v.key(&key(KeyCode::Space), &Config::default()), set("copy-on-select", "true"));
}

#[test]
fn delete_resets() {
    let mut v = view();
    focus(&mut v, "font-size");
    assert_eq!(v.key(&key(KeyCode::Delete), &Config::default()), Some(Edit::Unset { key: "font-size" }));
    assert_eq!(v.click(&SettingsHit::Reset(idx("font-size")), &Config::default()), Some(Edit::Unset { key: "font-size" }));
}

#[test]
fn esc_closes_the_panel() {
    let mut v = view();
    assert_eq!(v.key(&key(KeyCode::Escape), &Config::default()), Some(Edit::Close));
}

#[test]
fn list_filters_previews_and_commits() {
    let mut v = view();
    let c = Config::default();
    assert_eq!(v.key(&key(KeyCode::Enter), &c), None);
    assert_eq!(v.list().unwrap().items(), vec!["Dracula", "Nord"]);
    v.text("no");
    assert_eq!(v.list().unwrap().items(), vec!["Nord"]);
    v.text("\u{8}"); // control characters are not typed
    assert_eq!(v.list().unwrap().query, "no");
    v.key(&key(KeyCode::Backspace), &c);
    v.key(&key(KeyCode::Backspace), &c);
    assert_eq!(v.key(&key(KeyCode::ArrowDown), &c), Some(Edit::Preview { key: "theme", value: "Nord".into(), which: Which::Last }));
    assert_eq!(v.key(&key(KeyCode::ArrowUp), &c), Some(Edit::Preview { key: "theme", value: "Dracula".into(), which: Which::Last }));
    assert_eq!(v.key(&key(KeyCode::Enter), &c), set("theme", "Dracula"));
    assert!(v.list().is_none());
}

#[test]
fn list_opens_on_the_current_value() {
    let mut v = view();
    focus(&mut v, "font-family");
    let mut c = Config::default();
    c.font_family = vec!["Consolas".into()];
    v.key(&key(KeyCode::Enter), &c);
    assert_eq!(v.list().unwrap().selected, 1);
    assert_eq!(v.key(&key(KeyCode::Enter), &c), Some(Edit::Set { key: "font-family", value: "Consolas".into(), which: Which::First }));
}

#[test]
fn profiles_do_not_preview() {
    let mut v = view();
    focus(&mut v, "default-profile");
    let c = Config::default();
    v.key(&key(KeyCode::Enter), &c);
    assert_eq!(v.key(&key(KeyCode::ArrowDown), &c), None);
}

#[test]
fn list_esc_reverts() {
    let mut v = view();
    let c = Config::default();
    v.key(&key(KeyCode::Enter), &c);
    v.key(&key(KeyCode::ArrowDown), &c);
    assert_eq!(v.key(&key(KeyCode::Escape), &c), Some(Edit::Revert));
    assert!(v.list().is_none());
    assert_eq!(v.key(&key(KeyCode::Escape), &c), Some(Edit::Close));
    let mut v = view();
    v.key(&key(KeyCode::Enter), &c);
    assert_eq!(v.key(&key(KeyCode::Escape), &c), None, "nothing was previewed");
    assert!(v.list().is_none());
}

#[test]
fn notify_all_off_writes_none() {
    let mut v = view();
    let mut c = Config::default();
    c.agent_notify.flash = false;
    c.agent_notify.badge = false;
    assert_eq!(v.click(&SettingsHit::Way(idx("agent-notify"), 0), &c), set("agent-notify", "none"));
    assert_eq!(v.click(&SettingsHit::Way(idx("agent-notify"), 3), &c), set("agent-notify", "toast,sound"));
}

#[test]
fn quiet_hours_rejects_equal_times() {
    let mut v = view();
    focus(&mut v, "agent-notify-quiet");
    let c = Config::default();
    v.key(&key(KeyCode::Enter), &c);
    let clear = |v: &mut SettingsView| {
        for _ in 0..5 {
            v.key(&key(KeyCode::Backspace), &c);
        }
    };
    clear(&mut v);
    v.text("22:00");
    assert_eq!(v.key(&key(KeyCode::Enter), &c), None);
    clear(&mut v);
    v.text("22:00x99"); // only digits and `:`, at most 5
    assert_eq!(v.time().unwrap().end, "22:00");
    assert_eq!(v.key(&key(KeyCode::Enter), &c), None);
    assert!(v.time().unwrap().invalid);
    clear(&mut v);
    v.text("07:00");
    assert_eq!(v.key(&key(KeyCode::Enter), &c), set("agent-notify-quiet", "22:00-07:00"));
    assert!(v.time().is_none());
}

#[test]
fn quiet_hours_space_switches_off() {
    let mut v = view();
    focus(&mut v, "agent-notify-quiet");
    let mut c = Config::default();
    c.agent_notify.quiet = Some((22 * 60, 7 * 60));
    assert_eq!(v.key(&key(KeyCode::Space), &c), Some(Edit::Unset { key: "agent-notify-quiet" }));
    c.agent_notify.quiet = None;
    assert_eq!(v.key(&key(KeyCode::Space), &c), None);
    assert_eq!(v.time().map(|t| (t.start.as_str(), t.end.as_str())), Some(("22:00", "07:00")));
}

#[test]
fn read_only_row_gives_no_edit() {
    let mut rows = rows();
    rows[idx("font-size")].origin = Origin::Included { file: "inc".into(), line: 1 };
    let mut v = SettingsView::new(sources(), rows);
    focus(&mut v, "font-size");
    let c = Config::default();
    for k in [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::Space, KeyCode::Delete, KeyCode::Enter] {
        assert_eq!(v.key(&key(k), &c), None, "{k:?}");
    }
    assert_eq!(v.click(&SettingsHit::Inc(idx("font-size")), &c), None);
    assert_eq!(v.click(&SettingsHit::Reset(idx("font-size")), &c), None);
    assert_eq!(v.click(&SettingsHit::Row(idx("font-size")), &c), Some(Edit::OpenFile(Some("inc".into()))), "Open shows the file it is set in");
}

#[test]
fn layout_fits_small_window() {
    let mut v = view();
    for client in [(1600, 900), (500, 300), (200, 120)] {
        let l = v.layout(client, (8, 16), 26);
        for r in l.targets.iter().map(|(_, r)| r).chain([&l.panel]) {
            assert!(r.w >= 0 && r.h >= 0 && r.x >= 0 && r.y >= 0 && r.x + r.w <= client.0 && r.y + r.h <= client.1, "{client:?} {r:?}");
        }
        assert!(!l.rows.is_empty(), "{client:?}");
    }
    // The focused row stays on screen.
    focus(&mut v, "unfocused-split-opacity");
    let l = v.layout((200, 120), (8, 16), 26);
    assert!(l.rows.iter().any(|(i, _)| *i == idx("unfocused-split-opacity")));
}

#[test]
fn hit_finds_controls() {
    let v = view();
    let l = v.layout((1600, 900), (8, 16), 26);
    let (h, r) = &l.targets[0];
    assert_eq!(SettingsView::hit(&l, r.x + 1, r.y + 1).as_ref(), Some(h));
    assert_eq!(SettingsView::hit(&l, l.panel.x + l.panel.w - 1, l.panel.y + 1), Some(SettingsHit::Panel));
    assert_eq!(SettingsView::hit(&l, 0, 0), None);
    assert!(l.targets.iter().any(|(h, _)| *h == SettingsHit::OpenFile));
    assert!(l.targets.iter().any(|(h, _)| *h == SettingsHit::Inc(idx("font-size"))));
}

#[test]
fn page_click_switches_and_wheel_moves_focus() {
    let mut v = view();
    let c = Config::default();
    assert_eq!(v.click(&SettingsHit::Page(Page::Agents), &c), None);
    assert_eq!((v.page, v.focus), (Page::Agents, idx("agent-notify")));
    v.scroll(2);
    assert_eq!(v.focus, idx("agent-notify-quiet"));
    v.scroll(-9);
    assert_eq!(v.focus, idx("agent-notify"));
}

#[test]
fn control_text_shows_values() {
    use wraith_client::settings::control_text;
    let s = |k: &str| &all()[idx(k)];
    assert_eq!(control_text(s("font-size"), "12", None), "‹ 12 ›");
    assert_eq!(control_text(s("close-on-exit"), "never", None), "‹ keep open ›");
    assert_eq!(control_text(s("copy-on-select"), "true", None), "● on");
    assert_eq!(control_text(s("theme"), "", None), "(default) ▾");
    assert_eq!(control_text(s("agent-notify"), "toast,sound", None), "■toast□flash□badge■sound");
    assert_eq!(control_text(s("agent-notify-quiet"), "22:00-07:00", None), "22:00 – 07:00");
    assert_eq!(control_text(s("agent-notify-quiet"), "", None), "○ off");
}
