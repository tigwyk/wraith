#![cfg(windows)]
use std::time::{Duration, Instant};

use wraith_client::app::{KeybindMatcher, Match, SelAction, Selection, grid_size, needs_transparency, pixel_to_cell, ScChange, swapchain_change};
use wraith_client::font::CellMetrics;
use wraith_config::{Action, parse_keybind};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};

fn m8x16() -> CellMetrics {
    CellMetrics { w: 8, h: 16, baseline: 12, underline_pos: 14, underline_thickness: 1 }
}

#[test]
fn zero_client_rect_does_not_resize() {
    assert_eq!(grid_size(0, 0, &m8x16(), 4, 4), None);
    assert_eq!(grid_size(0, 500, &m8x16(), 4, 4), None);
}

#[test]
fn grid_size_respects_padding_and_floor() {
    assert_eq!(grid_size(10 * 8 + 8, 5 * 16 + 8, &m8x16(), 4, 4), Some((10, 5)));
    assert_eq!(grid_size(10 * 8 + 8 + 7, 5 * 16 + 8 + 15, &m8x16(), 4, 4), Some((10, 5)));
}

#[test]
fn tiny_window_clamps_to_two_by_one() {
    assert_eq!(grid_size(9, 9, &m8x16(), 4, 4), Some((2, 1)));
}

#[test]
fn pixel_to_cell_edges() {
    assert_eq!(pixel_to_cell(7.9, 15.9, (0.0, 0.0), &m8x16()), (0, 0));
    assert_eq!(pixel_to_cell(8.0, 16.0, (0.0, 0.0), &m8x16()), (1, 1));
    assert_eq!(pixel_to_cell(-3.0, -3.0, (4.0, 4.0), &m8x16()), (0, 0));
}

fn key(code: KeyCode, mods: u8) -> KeyEvent {
    KeyEvent { code, mods: Mods(mods), text: None, action: KeyAction::Press, composing: false }
}

#[test]
fn keybind_single_matches_with_mods() {
    let mut k = KeybindMatcher::new(&[parse_keybind("ctrl+shift+c=copy_to_clipboard").unwrap()]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::KeyC, Mods::CTRL | Mods::SHIFT), t), Match::Action(Action::Copy));
    assert_eq!(k.feed(&key(KeyCode::KeyC, Mods::CTRL), t), Match::None(vec![key(KeyCode::KeyC, Mods::CTRL)]));
}

#[test]
fn keybind_named_keys() {
    let mut k = KeybindMatcher::new(&[
        parse_keybind("shift+page_up=scroll_page_up").unwrap(),
        parse_keybind("ctrl+equal=increase_font_size:1").unwrap(),
        parse_keybind("ctrl+zero=reset_font_size").unwrap(),
    ]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::PageUp, Mods::SHIFT), t), Match::Action(Action::ScrollPageUp));
    assert_eq!(k.feed(&key(KeyCode::Equal, Mods::CTRL), t), Match::Action(Action::IncreaseFontSize(1.0)));
    assert_eq!(k.feed(&key(KeyCode::Digit0, Mods::CTRL), t), Match::Action(Action::ResetFontSize));
}

#[test]
fn keybind_sequence_pending_then_action() {
    let mut k = KeybindMatcher::new(&[parse_keybind("ctrl+a>c=reset_font_size").unwrap()]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::KeyA, Mods::CTRL), t), Match::Pending);
    assert_eq!(k.feed(&key(KeyCode::KeyC, 0), t), Match::Action(Action::ResetFontSize));
}

#[test]
fn keybind_sequence_mismatch_flushes_buffer() {
    let mut k = KeybindMatcher::new(&[parse_keybind("ctrl+a>c=reset_font_size").unwrap()]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::KeyA, Mods::CTRL), t), Match::Pending);
    assert_eq!(k.feed(&key(KeyCode::KeyX, 0), t), Match::None(vec![key(KeyCode::KeyA, Mods::CTRL), key(KeyCode::KeyX, 0)]));
}

#[test]
fn keybind_sequence_timeout_flushes() {
    let mut k = KeybindMatcher::new(&[parse_keybind("ctrl+a>c=reset_font_size").unwrap()]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::KeyA, Mods::CTRL), t), Match::Pending);
    assert!(k.tick(t + Duration::from_millis(900)).is_empty());
    assert_eq!(k.tick(t + Duration::from_millis(1100)), vec![key(KeyCode::KeyA, Mods::CTRL)]);
}

#[test]
fn modifier_keys_alone_never_match_or_buffer() {
    let mut k = KeybindMatcher::new(&[parse_keybind("ctrl+a>c=reset_font_size").unwrap()]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::KeyA, Mods::CTRL), t), Match::Pending);
    // Releasing/pressing Shift between the two keys must not break the sequence.
    assert_eq!(k.feed(&key(KeyCode::ShiftLeft, Mods::SHIFT), t), Match::Pending);
    assert_eq!(k.feed(&key(KeyCode::KeyC, 0), t), Match::Action(Action::ResetFontSize));
}

#[test]
fn double_click_requests_word_triple_requests_line() {
    let mut s = Selection::new(Duration::from_millis(500));
    let t = Instant::now();
    assert_eq!(s.press((3, 1), t, false), SelAction::Start);
    s.release();
    assert_eq!(s.press((3, 1), t + Duration::from_millis(100), false), SelAction::Word { x: 3, y: 1 });
    s.release();
    assert_eq!(s.press((3, 1), t + Duration::from_millis(200), false), SelAction::Line { y: 1 });
    s.release();
    assert_eq!(s.press((3, 1), t + Duration::from_millis(2000), false), SelAction::Start);
}

#[test]
fn drag_produces_range_and_alt_makes_rectangle() {
    let mut s = Selection::new(Duration::from_millis(500));
    let t = Instant::now();
    s.press((2, 0), t, false);
    let r = s.drag((5, 1)).unwrap();
    assert_eq!((r.start, r.end, r.rectangle), ((2, 0), (5, 1), false));
    assert_eq!(s.release().unwrap().end, (5, 1));
    s.press((1, 1), t + Duration::from_secs(5), true);
    assert!(s.drag((4, 3)).unwrap().rectangle);
}

#[test]
fn click_without_drag_clears_selection() {
    let mut s = Selection::new(Duration::from_millis(500));
    s.press((2, 0), Instant::now(), false);
    assert_eq!(s.release(), None);
}

#[test]
fn wheel_accumulates_partial_touchpad_deltas() {
    use wraith_client::app::wheel_notches;
    let mut acc = 0;
    assert_eq!(wheel_notches(&mut acc, 120), 1);
    assert_eq!(wheel_notches(&mut acc, -240), -2);
    // Precision touchpads send small deltas; 4 x 30 = one notch.
    assert_eq!((0..3).map(|_| wheel_notches(&mut acc, 30)).sum::<i32>(), 0);
    assert_eq!(wheel_notches(&mut acc, 30), 1);
    assert_eq!(acc, 0);
}

#[test]
fn dash_e_takes_the_rest_of_the_command_line() {
    use wraith_client::app::command_from_args;
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(command_from_args(&a(&["wraith.exe"])), None);
    assert_eq!(command_from_args(&a(&["wraith.exe", "-e", "wsl.exe"])), Some("wsl.exe".into()));
    assert_eq!(
        command_from_args(&a(&["wraith.exe", "-e", r"C:\Program Files\x.exe", "-NoExit", "a b"])),
        Some(r#""C:\Program Files\x.exe" -NoExit "a b""#.into())
    );
    assert_eq!(command_from_args(&a(&["wraith.exe", "-e"])), None);
}

#[test]
fn reset_drops_a_pending_sequence() {
    let mut k = KeybindMatcher::new(&[parse_keybind("ctrl+a>c=new_tab").unwrap()]);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::KeyA, Mods::CTRL), t), Match::Pending);
    k.reset();
    assert!(k.tick(t + Duration::from_secs(5)).is_empty(), "nothing is flushed to the terminal later");
}

#[test]
fn ctrl_shift_digit_matches_new_tab_profile() {
    let mut k = KeybindMatcher::new(&wraith_config::Config::default().keybinds);
    let t = Instant::now();
    assert_eq!(k.feed(&key(KeyCode::Digit1, Mods::CTRL | Mods::SHIFT), t), Match::Action(Action::NewTabProfile(1)));
    assert_eq!(k.feed(&key(KeyCode::Digit9, Mods::CTRL | Mods::SHIFT), t), Match::Action(Action::NewTabProfile(9)));
}

#[test]
fn ctrl_shift_space_matches_menu() {
    let mut k = KeybindMatcher::new(&wraith_config::Config::default().keybinds);
    assert_eq!(k.feed(&key(KeyCode::Space, Mods::CTRL | Mods::SHIFT), Instant::now()), Match::Action(Action::OpenNewTabMenu));
}

#[test]
fn transparency_choice() {
    use wraith_server::profile::{Profile, from_config};
    let c = wraith_config::Config::default();
    let opaque: Vec<Profile> = vec![from_config("a", "cmd.exe"), Profile { opacity: Some(1.0), ..from_config("b", "pwsh.exe") }];
    assert!(!needs_transparency(&c, &opaque));
    let mut see_through = c.clone();
    see_through.background_opacity = 0.9;
    assert!(needs_transparency(&see_through, &opaque));
    let half = vec![from_config("a", "cmd.exe"), Profile { opacity: Some(0.5), ..from_config("b", "pwsh.exe") }];
    assert!(needs_transparency(&c, &half));
}

#[test]
fn swapchain_change_on_reload() {
    // (is_composition, was_wanted, wanted)
    assert_eq!(swapchain_change(false, false, false), ScChange::Keep);
    assert_eq!(swapchain_change(true, true, true), ScChange::Keep);
    assert_eq!(swapchain_change(true, true, false), ScChange::Switch, "composition to HWND switches live");
    assert_eq!(swapchain_change(false, false, true), ScChange::KeepAndNotify, "HWND to composition waits for a new window");
    assert_eq!(swapchain_change(false, true, true), ScChange::Keep, "told once per crossing");
}
