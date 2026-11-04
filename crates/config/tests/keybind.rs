use std::path::{Path, PathBuf};

use wraith_config::{Action, TriggerKey, parse_keybind, parse_str};
use wraith_proto::Mods;

fn p() -> &'static Path {
    Path::new("C:/cfg/config")
}
fn themes() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/themes")
}

#[test]
fn simple() {
    let k = parse_keybind("ctrl+shift+c=copy_to_clipboard").unwrap();
    assert_eq!(k.trigger.len(), 1);
    assert_eq!(k.trigger[0].mods, Mods(Mods::CTRL | Mods::SHIFT));
    assert_eq!(k.trigger[0].key, TriggerKey::Char('c'));
    assert_eq!(k.action, Action::Copy);
}

#[test]
fn sequence() {
    assert_eq!(parse_keybind("ctrl+a>c=reset_font_size").unwrap().trigger.len(), 2);
}

#[test]
fn mod_aliases_and_order() {
    assert_eq!(
        parse_keybind("shift+ctrl+c=copy_to_clipboard").unwrap().trigger,
        parse_keybind("control+shift+C=copy_to_clipboard").unwrap().trigger
    );
}

#[test]
fn named_and_physical_keys() {
    assert_eq!(parse_keybind("shift+page_up=scroll_page_up").unwrap().trigger[0].key, TriggerKey::Named("page_up".into()));
    assert_eq!(parse_keybind("ctrl+[KeyA]=copy_to_clipboard").unwrap().trigger[0].key, TriggerKey::Physical("KeyA".into()));
}

#[test]
fn action_param() {
    assert_eq!(parse_keybind("ctrl+equal=increase_font_size:2").unwrap().action, Action::IncreaseFontSize(2.0));
}

#[test]
fn unbind_removes_default() {
    let (c, d) = parse_str("keybind = ctrl+shift+c=unbind\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert!(!c.keybinds.iter().any(|k| k.action == Action::Copy));
}

#[test]
fn later_binding_replaces_earlier() {
    let (c, _) = parse_str("keybind = ctrl+shift+c=reset_font_size\n", p(), &themes());
    let hits: Vec<_> = c.keybinds.iter().filter(|k| k.trigger == parse_keybind("ctrl+shift+c=copy_to_clipboard").unwrap().trigger).collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].action, Action::ResetFontSize);
}

#[test]
fn unknown_action_is_error() {
    assert!(parse_keybind("ctrl+x=launch_rockets").is_err());
    assert!(parse_keybind("ctrl+x").is_err());
    assert!(parse_keybind("hyper+x=copy_to_clipboard").is_err());
}

#[test]
fn clear_resets_all() {
    let (c, _) = parse_str("keybind = clear\n", p(), &themes());
    assert!(c.keybinds.is_empty());
}

#[test]
fn defaults_present() {
    let (c, _) = parse_str("", p(), &themes());
    for a in [Action::Copy, Action::Paste, Action::ReloadConfig, Action::ToggleFrameOverlay, Action::CloseSurface, Action::ScrollPageUp] {
        assert!(c.keybinds.iter().any(|k| k.action == a), "missing default for {a:?}");
    }
}

#[test]
fn empty_value_restores_defaults() {
    let (c, d) = parse_str("keybind = clear\nkeybind =\n", p(), &themes());
    assert!(d.is_empty(), "{d:?}");
    assert!(c.keybinds.iter().any(|k| k.action == Action::Copy));
}

use wraith_config::{SplitTarget, all_actions};
use wraith_proto::model::Dir;

#[test]
fn new_actions_parse() {
    let act = |s: &str| parse_keybind(&format!("ctrl+shift+x={s}")).unwrap().action;
    assert_eq!(act("new_tab"), Action::NewTab);
    assert_eq!(act("close_tab"), Action::CloseTab);
    assert_eq!(act("next_tab"), Action::NextTab);
    assert_eq!(act("previous_tab"), Action::PreviousTab);
    assert_eq!(act("goto_tab:3"), Action::GotoTab(3));
    assert_eq!(act("new_split:right"), Action::NewSplit(Dir::Right));
    assert_eq!(act("new_split:down"), Action::NewSplit(Dir::Down));
    assert_eq!(act("goto_split:previous"), Action::GotoSplit(SplitTarget::Previous));
    assert_eq!(act("goto_split:next"), Action::GotoSplit(SplitTarget::Next));
    assert_eq!(act("goto_split:left"), Action::GotoSplit(SplitTarget::Dir(Dir::Left)));
    assert_eq!(act("toggle_split_zoom"), Action::ToggleSplitZoom);
    assert_eq!(act("resize_split:left,10"), Action::ResizeSplit(Dir::Left, 10));
    assert_eq!(act("equalize_splits"), Action::EqualizeSplits);
    assert_eq!(act("new_workspace"), Action::NewWorkspace);
    assert_eq!(act("close_workspace"), Action::CloseWorkspace);
    assert_eq!(act("next_workspace"), Action::NextWorkspace);
    assert_eq!(act("previous_workspace"), Action::PreviousWorkspace);
    assert_eq!(act("command_palette"), Action::CommandPalette);
    assert_eq!(act("workspace_picker"), Action::WorkspacePicker);
    assert_eq!(act("move_tab_to_workspace"), Action::MoveTabToWorkspace);
    assert_eq!(act("toggle_sidebar"), Action::ToggleSidebar);
    assert_eq!(act("new_window"), Action::NewWindow);
}

#[test]
fn all_actions_round_trip_through_parse() {
    let all = all_actions();
    assert!(all.len() >= 30, "{}", all.len());
    for (a, name) in all {
        assert_eq!(parse_keybind(&format!("ctrl+shift+x={name}")).unwrap().action, a, "{name}");
        assert!(name.starts_with(a.name()), "{name} vs {}", a.name());
    }
}

#[test]
fn default_binds_include_m3_chords() {
    let (c, d) = parse_str("", p(), &themes());
    assert!(d.is_empty());
    let has = |chord: &str, a: Action| {
        let want = parse_keybind(&format!("{chord}=copy_to_clipboard")).unwrap().trigger;
        c.keybinds.iter().any(|k| k.trigger == want && k.action == a)
    };
    assert!(has("ctrl+shift+t", Action::NewTab));
    assert!(has("ctrl+tab", Action::NextTab));
    assert!(has("ctrl+shift+tab", Action::PreviousTab));
    assert!(has("alt+shift+plus", Action::NewSplit(Dir::Right)));
    assert!(has("alt+shift+minus", Action::NewSplit(Dir::Down)));
    assert!(has("alt+left", Action::GotoSplit(SplitTarget::Dir(Dir::Left))));
    assert!(has("alt+down", Action::GotoSplit(SplitTarget::Dir(Dir::Down))));
    assert!(has("ctrl+shift+p", Action::CommandPalette));
    assert!(has("ctrl+shift+o", Action::WorkspacePicker));
    assert!(has("ctrl+shift+b", Action::ToggleSidebar));
    assert!(has("ctrl+shift+n", Action::NewWindow));
}

#[test]
fn goto_tab_zero_is_rejected() {
    assert!(parse_keybind("ctrl+x=goto_tab:0").is_err());
    assert!(parse_keybind("ctrl+x=goto_tab:300").is_err());
    assert!(parse_keybind("ctrl+x=new_split:sideways").is_err());
}

#[test]
fn ctrl_comma_opens_settings() {
    let c = wraith_config::Config::default();
    let kb = c.keybinds.iter().find(|k| k.action == Action::OpenSettings).expect("default binding");
    assert_eq!(kb.trigger, parse_keybind("ctrl+comma=open_settings").unwrap().trigger);
    assert!(wraith_config::all_actions().iter().any(|(a, n)| *a == Action::OpenSettings && *n == "open_settings"));
}

#[test]
fn new_tab_profile_parses() {
    let act = |s: &str| parse_keybind(&format!("ctrl+shift+x={s}")).map(|k| k.action);
    assert_eq!(act("new_tab_profile:3"), Ok(Action::NewTabProfile(3)));
    assert!(act("new_tab_profile:0").is_err());
    assert!(act("new_tab_profile").is_err());
    assert_eq!(act("open_new_tab_menu"), Ok(Action::OpenNewTabMenu));
    assert_eq!(act("import_windows_terminal_profiles"), Ok(Action::ImportWtProfiles));
}

#[test]
fn new_defaults_bound() {
    let (c, _) = parse_str("", p(), &themes());
    let bound = |t: &str| c.keybinds.iter().find(|k| k.trigger == parse_keybind(&format!("{t}=new_tab")).unwrap().trigger).map(|k| k.action);
    assert_eq!(bound("ctrl+shift+space"), Some(Action::OpenNewTabMenu));
    for n in 1..=9u8 {
        assert_eq!(bound(&format!("ctrl+shift+{n}")), Some(Action::NewTabProfile(n)));
    }
    let names: Vec<&str> = all_actions().into_iter().map(|(_, s)| s).collect();
    assert!(names.contains(&"open_new_tab_menu") && names.contains(&"import_windows_terminal_profiles"));
}

#[test]
fn parse_global_rejects_sequences() {
    assert!(wraith_config::parse_global("ctrl+a>ctrl+b=toggle_quick_terminal").is_err());
    assert!(wraith_config::parse_global("super+backquote=toggle_quick_terminal").unwrap().is_some());
    assert_eq!(wraith_config::parse_global("super+backquote=unbind"), Ok(None));
}
