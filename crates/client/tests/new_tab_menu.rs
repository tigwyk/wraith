//! The new-tab profile menu's view state (wt-profiles spec §5.2): rows, keys, clicks and layout.
use wraith_client::new_tab_menu::{MenuDo, MenuHit, MenuOutcome, MenuRow, NewTabMenu, letter_tile, menu_action};
use wraith_config::Action;
use wraith_config::Config;
use wraith_proto::model::Rect;
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};
use wraith_server::profile::Profile;

fn profiles(n: usize) -> Vec<Profile> {
    (1..=n).map(|i| Profile { name: format!("p{i}"), command: "x.exe".into(), ..Profile::default() }).collect()
}

fn menu(n: usize, default: &str) -> NewTabMenu {
    NewTabMenu::new(&profiles(n), default, &Config::default().keybinds)
}

fn key(code: KeyCode, mods: u8) -> KeyEvent {
    KeyEvent { code, mods: Mods(mods), text: None, action: KeyAction::Press, composing: false }
}

const ANCHOR: Rect = Rect { x: 400, y: 0, w: 16, h: 26 };

#[test]
fn rows_have_bindings_for_first_nine() {
    let m = menu(10, "p1");
    assert_eq!(m.rows.len(), 13, "10 profiles, a separator, Settings, Command palette");
    let MenuRow::Profile { binding, name, default, .. } = &m.rows[0] else { panic!() };
    assert_eq!((name.as_str(), binding.as_deref(), *default), ("p1", Some("Ctrl+Shift+1"), true));
    let MenuRow::Profile { binding, .. } = &m.rows[8] else { panic!() };
    assert_eq!(binding.as_deref(), Some("Ctrl+Shift+9"));
    let MenuRow::Profile { binding, default, .. } = &m.rows[9] else { panic!() };
    assert_eq!((binding.as_deref(), *default), (None, false));
    assert_eq!(m.rows[10], MenuRow::Separator);
    assert_eq!(m.rows[11], MenuRow::Settings(Some("Ctrl+,".into())));
    assert_eq!(m.rows[12], MenuRow::Palette(Some("Ctrl+Shift+P".into())));
}

#[test]
fn opens_on_default() {
    assert_eq!(menu(3, "P2").selected, 1, "the default profile, any case");
    assert_eq!(menu(3, "nope").selected, 0);
}

#[test]
fn down_up_wrap_and_skip_separator() {
    let mut m = menu(2, "p2");
    assert_eq!(m.key(&key(KeyCode::ArrowDown, 0)), None);
    assert_eq!(m.selected, 3, "skips the separator onto Settings");
    m.key(&key(KeyCode::ArrowDown, 0));
    assert_eq!(m.selected, 4);
    m.key(&key(KeyCode::ArrowDown, 0));
    assert_eq!(m.selected, 0, "wraps");
    m.key(&key(KeyCode::ArrowUp, 0));
    assert_eq!(m.selected, 4, "wraps back");
}

#[test]
fn home_end() {
    let mut m = menu(3, "p2");
    m.key(&key(KeyCode::End, 0));
    assert_eq!(m.selected, 5);
    m.key(&key(KeyCode::Home, 0));
    assert_eq!(m.selected, 0);
}

#[test]
fn enter_opens_alt_enter_splits() {
    let mut m = menu(3, "p2");
    assert_eq!(m.key(&key(KeyCode::Enter, 0)), Some(MenuOutcome::Open(1)));
    assert_eq!(m.key(&key(KeyCode::Enter, Mods::ALT)), Some(MenuOutcome::Split(1)));
    m.key(&key(KeyCode::End, 0));
    assert_eq!(m.key(&key(KeyCode::Enter, 0)), Some(MenuOutcome::Palette));
}

#[test]
fn esc_closes() {
    assert_eq!(menu(1, "p1").key(&key(KeyCode::Escape, 0)), Some(MenuOutcome::Close));
}

#[test]
fn click_profile_alt_click_split() {
    let mut m = menu(3, "p1");
    assert_eq!(m.click(&MenuHit::Row(2), false), Some(MenuOutcome::Open(2)));
    assert_eq!(m.click(&MenuHit::Row(1), true), Some(MenuOutcome::Split(1)));
    assert_eq!(m.click(&MenuHit::Row(3), false), None, "the separator does nothing");
    assert_eq!(m.click(&MenuHit::Panel, false), None);
}

#[test]
fn click_settings_and_palette() {
    let mut m = menu(1, "p1");
    assert_eq!(m.click(&MenuHit::Row(2), false), Some(MenuOutcome::Settings));
    assert_eq!(m.click(&MenuHit::Row(3), false), Some(MenuOutcome::Palette));
}

#[test]
fn layout_below_anchor_and_hit() {
    let m = menu(3, "p1");
    let l = m.layout(ANCHOR, (1000, 600), (8, 16), 26);
    assert_eq!((l.panel.x, l.panel.y), (400, 26));
    assert_eq!(l.rows.len(), 6);
    let (i, r) = l.rows[1];
    assert_eq!(NewTabMenu::hit(&l, r.x + 4, r.y + 2), Some(MenuHit::Row(i)));
    assert_eq!(NewTabMenu::hit(&l, 5, 500), None);
}

#[test]
fn layout_moves_left_in_narrow_window() {
    let l = menu(3, "p1").layout(Rect { x: 280, y: 0, w: 16, h: 26 }, (300, 600), (8, 16), 26);
    assert!(l.panel.x + l.panel.w <= 300, "{:?}", l.panel);
    assert!(l.panel.x >= 0);
}

#[test]
fn layout_scrolls_in_short_window() {
    let mut m = menu(30, "p1");
    let l = m.layout(ANCHOR, (1000, 200), (8, 16), 26);
    assert!(l.panel.y + l.panel.h <= 200, "{:?}", l.panel);
    assert!(l.rows.len() < 33);
    assert_eq!(l.rows[0].0, 0);
    for _ in 0..20 {
        m.key(&key(KeyCode::ArrowDown, 0));
    }
    let l = m.layout(ANCHOR, (1000, 200), (8, 16), 26);
    assert!(l.rows.iter().any(|(i, _)| *i == m.selected), "the selection stays visible");
}

#[test]
fn letter_tile_is_stable_and_uppercase() {
    let (c, color) = letter_tile("ubuntu");
    assert_eq!(c, 'U');
    assert_eq!(letter_tile("ubuntu").1, color, "same name, same colour");
    assert_eq!(letter_tile("🦀x").0, 'X', "the first letter or digit");
    assert_eq!(letter_tile("").0, '?');
}

#[test]
fn menu_outcome_maps_to_actions() {
    assert_eq!(menu_action(MenuOutcome::Open(0)), Some(MenuDo::Action(Action::NewTabProfile(1))));
    assert_eq!(menu_action(MenuOutcome::Split(2)), Some(MenuDo::Split(2)));
    assert_eq!(menu_action(MenuOutcome::Settings), Some(MenuDo::Action(Action::OpenSettings)));
    assert_eq!(menu_action(MenuOutcome::Palette), Some(MenuDo::Action(Action::CommandPalette)));
    assert_eq!(menu_action(MenuOutcome::Close), None);
}
