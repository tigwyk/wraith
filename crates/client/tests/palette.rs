use wraith_client::palette::{Choice, Mode, Palette, fuzzy, move_tab_items, palette_items, picker_items, trigger_text};
use wraith_config::{Action, WorkspaceDef, all_actions, parse_keybind};
use wraith_proto::model::{ModelSnapshot, PaneNode, PaneMeta, Tab, Workspace};

#[test]
fn fuzzy_requires_subsequence() {
    assert!(fuzzy("nt", "new tab").is_some());
    assert!(fuzzy("NT", "new tab").is_some(), "case-insensitive");
    assert!(fuzzy("tn", "new tab").is_none(), "order matters");
    assert!(fuzzy("xyz", "new tab").is_none());
    assert_eq!(fuzzy("", "anything"), Some(0));
    assert!(fuzzy("longer than text", "short").is_none());
}

#[test]
fn fuzzy_prefers_consecutive_and_word_start() {
    let consecutive = fuzzy("tab", "new tab").unwrap();
    let scattered = fuzzy("tab", "toggle a bar").unwrap();
    assert!(consecutive > scattered, "{consecutive} vs {scattered}");
    let word_start = fuzzy("sp", "new split").unwrap();
    let mid_word = fuzzy("sp", "whisper").unwrap();
    assert!(word_start > mid_word, "{word_start} vs {mid_word}");
    assert!(fuzzy("new", "new tab").unwrap() > fuzzy("new", "renew").unwrap(), "prefix beats the middle of a word");
}

fn binds() -> Vec<wraith_config::Keybind> {
    wraith_config::Config::default().keybinds
}

#[test]
fn empty_query_lists_everything_in_group_order() {
    let items = palette_items(&binds());
    let p = Palette::new(Mode::Palette, items);
    let groups: Vec<&str> = p.visible().iter().map(|i| i.group).collect();
    let mut order: Vec<&str> = Vec::new();
    for g in &groups {
        if order.last() != Some(g) {
            order.push(g);
        }
    }
    assert_eq!(order, vec!["Workspaces", "Tabs", "Panes", "Window"], "each group appears once, in this order");
}

#[test]
fn palette_lists_every_action_in_all_actions() {
    let items = palette_items(&binds());
    for (a, name) in all_actions() {
        assert!(items.iter().any(|i| i.choice == Choice::Action(a)), "{name} missing from the palette");
    }
}

#[test]
fn palette_shows_current_keybinding() {
    let items = palette_items(&binds());
    let tab = items.iter().find(|i| i.choice == Choice::Action(Action::NewTab)).unwrap();
    assert_eq!(tab.detail, "Ctrl+Shift+T");
    let mut b = binds();
    wraith_config::Config::default(); // defaults untouched
    b.retain(|k| k.action != Action::NewTab);
    b.push(parse_keybind("ctrl+alt+n=new_tab").unwrap());
    let items = palette_items(&b);
    assert_eq!(items.iter().find(|i| i.choice == Choice::Action(Action::NewTab)).unwrap().detail, "Ctrl+Alt+N");
    let unbound = items.iter().find(|i| i.choice == Choice::Action(Action::EqualizeSplits)).unwrap();
    assert_eq!(unbound.detail, "");
    assert_eq!(trigger_text(&parse_keybind("ctrl+a>c=new_tab").unwrap()), "Ctrl+A > C");
    assert_eq!(trigger_text(&parse_keybind("alt+shift+plus=new_tab").unwrap()), "Alt+Shift++");
    assert_eq!(trigger_text(&parse_keybind("shift+page_up=new_tab").unwrap()), "Shift+Page Up");
}

#[test]
fn typing_filters_and_selection_clamps_when_filter_shrinks() {
    let mut p = Palette::new(Mode::Palette, palette_items(&binds()));
    let all = p.visible().len();
    for _ in 0..5 {
        p.down();
    }
    assert_eq!(p.selected, 5);
    p.type_text("split");
    assert!(p.visible().len() < all && !p.visible().is_empty());
    assert!(p.selected < p.visible().len(), "selection stays on a visible row");
    assert!(p.visible().iter().all(|i| fuzzy("split", &i.label).is_some()));
    p.type_text("zzzz");
    assert!(p.visible().is_empty());
    assert_eq!(p.accept(), None, "nothing to accept");
    p.backspace();
    p.backspace();
    p.backspace();
    p.backspace();
    assert!(!p.visible().is_empty());
    p.up();
    p.up();
    p.up();
    assert_eq!(p.selected, 0, "up stops at the first row");
}

#[test]
fn accept_returns_the_selected_choice() {
    let mut p = Palette::new(Mode::Palette, palette_items(&binds()));
    p.type_text("new tab");
    assert_eq!(p.accept(), Some(Choice::Action(Action::NewTab)));
}

fn snap() -> ModelSnapshot {
    let w = |id, name: &str, root: Option<&str>, attached| Workspace {
        id,
        name: name.into(),
        root_dir: root.map(Into::into),
        color: 0,
        tabs: vec![id * 10],
        active_tab: Some(id * 10),
        profile: "pwsh".into(),
        attached,
        last_used: id,
        quick: false,
    };
    let t = |id: u64, ws, pane| Tab { id, workspace: ws, title_override: None, root: PaneNode::Leaf(pane), focused: pane, zoomed: None };
    let p = |id, cwd: Option<&str>| PaneMeta { id, title: String::new(), cwd: cwd.map(Into::into), exit: None, profile: "pwsh".into(), cmdline: String::new(), agent: None };
    ModelSnapshot {
        rev: 1,
        workspaces: vec![w(1, "api", Some(r"C:\src\api"), true), w(2, "web", None, false)],
        tabs: vec![t(10, 1, 100), t(20, 2, 200)],
        panes: vec![p(100, Some(r"C:\src\api")), p(200, Some(r"C:\Work\Web")), p(201, Some(r"c:\work\web")), p(202, Some(r"D:\notes"))],
    }
}

#[test]
fn picker_orders_open_then_configured_then_recent() {
    let configured = vec![
        WorkspaceDef { name: "web".into(), dir: Some("C:/web".into()), layout: "single".into() }, // already open
        WorkspaceDef { name: "infra".into(), dir: Some("C:/infra".into()), layout: "grid".into() },
    ];
    let items = picker_items(&snap(), &configured);
    let kinds: Vec<(&str, String)> = items.iter().map(|i| (i.group, i.label.clone())).collect();
    assert_eq!(kinds[0], ("Open", "api".into()));
    assert_eq!(kinds[1], ("Open", "web".into()));
    assert_eq!(kinds[2], ("Configured", "infra".into()));
    assert!(kinds[3..].iter().all(|(g, _)| *g == "Recent"), "{kinds:?}");
    assert_eq!(items[0].choice, Choice::Workspace(1));
    assert_eq!(items[2].choice, Choice::ConfiguredWorkspace(1));
    assert!(items[0].detail.contains("this window") || items[0].detail.contains("open"), "{}", items[0].detail);
}

#[test]
fn picker_dedups_directories_case_insensitively() {
    let items = picker_items(&snap(), &[]);
    let dirs: Vec<&str> = items.iter().filter_map(|i| if let Choice::Directory(d) = &i.choice { Some(d.as_str()) } else { None }).collect();
    assert_eq!(dirs, vec![r"C:\src\api", r"C:\Work\Web", r"D:\notes"], "one entry for C:\\Work\\Web / c:\\work\\web");
    let web = items.iter().find(|i| i.detail == r"C:\Work\Web").unwrap();
    assert_eq!(web.label, "Web");
}

#[test]
fn move_tab_lists_other_workspaces() {
    let items = move_tab_items(&snap(), 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].choice, Choice::Workspace(2));
}

#[test]
fn select_clamps_to_items() {
    let mut p = Palette::new(Mode::Palette, palette_items(&binds()));
    p.select(3);
    assert_eq!(p.selected, 3);
    p.select(10_000);
    assert_eq!(p.selected, p.visible().len() - 1);
    p.type_text("zzzz");
    p.select(2);
    assert_eq!(p.selected, 0, "nothing visible: stays at 0");
}

#[test]
fn scroll_moves_selection_by_rows_and_clamps() {
    use wraith_client::palette::SHOWN;
    let mut p = Palette::new(Mode::Palette, palette_items(&binds()));
    let n = p.visible().len();
    assert!(n > SHOWN + 3, "enough actions to scroll");
    p.scroll(3);
    assert_eq!((p.selected, p.top), (3, 0), "still on screen: the list does not move");
    p.scroll(20);
    assert_eq!(p.selected, 23.min(n - 1));
    assert!(p.top <= p.selected && p.selected < p.top + SHOWN, "selection kept on screen");
    p.scroll(-1000);
    assert_eq!((p.selected, p.top), (0, 0));
    p.scroll(1000);
    assert_eq!(p.selected, n - 1);
    assert_eq!(p.top, n - SHOWN);
}

#[test]
fn new_tab_rows_per_profile() {
    let binds = wraith_config::Config::default().keybinds;
    let items = wraith_client::palette::profile_items(&["pwsh".to_string(), "Ubuntu 22.04".to_string()], &binds);
    let rows: Vec<(&str, &str, &str)> = items.iter().map(|i| (i.label.as_str(), i.detail.as_str(), i.group)).collect();
    assert_eq!(rows, [("New tab: pwsh", "Ctrl+Shift+1", "Tabs"), ("New tab: Ubuntu 22.04", "Ctrl+Shift+2", "Tabs")]);
    assert_eq!(items[1].choice, Choice::Action(Action::NewTabProfile(2)));
}
