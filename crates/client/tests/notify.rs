//! When an agent notification shows, and how (spec §5.4).
use wraith_client::notify::{Ways, notify_ways};
use wraith_config::AgentNotify;

fn cfg() -> AgentNotify {
    AgentNotify::default()
}

#[test]
fn defaults_toast_flash_badge_when_unfocused() {
    let w = notify_ways(&cfg(), false, true, 12 * 60);
    assert_eq!(w, Ways { toast: true, flash: true, badge: true, sound: false });
}

#[test]
fn foreground_and_visible_shows_nothing_unless_always() {
    assert_eq!(notify_ways(&cfg(), true, true, 600), Ways::default(), "the user is looking at it");
    assert!(notify_ways(&cfg(), true, false, 600).toast, "foreground but the pane is hidden");
    assert!(!notify_ways(&cfg(), true, false, 600).flash, "no taskbar flash for a foreground window");
    let always = AgentNotify { when_always: true, ..cfg() };
    assert!(notify_ways(&always, true, true, 600).toast);
}

#[test]
fn quiet_hours_silence_everything_but_the_badge() {
    let quiet = AgentNotify { quiet: Some((22 * 60, 8 * 60)), sound: true, ..cfg() };
    let at = |h: u32| notify_ways(&quiet, false, false, h * 60);
    assert_eq!(at(23), Ways { toast: false, flash: false, badge: true, sound: false }, "across midnight");
    assert_eq!(at(3), Ways { toast: false, flash: false, badge: true, sound: false });
    assert!(at(12).toast && at(12).sound);
}

#[test]
fn config_subset_is_respected() {
    let only_flash = AgentNotify { toast: false, badge: false, ..cfg() };
    assert_eq!(notify_ways(&only_flash, false, false, 600), Ways { toast: false, flash: true, badge: false, sound: false });
}
