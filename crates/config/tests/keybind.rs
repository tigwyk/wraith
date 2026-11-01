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
