//! Time-based chrome motion: pure values of time, so every rule is checked without a window.
use std::time::{Duration, Instant};

use wraith_client::motion::{Fade, Pulse, Slide, duration, ease_out};
use wraith_proto::model::Rect;

const D: Duration = Duration::from_millis(120);

fn r(x: i32) -> Rect {
    Rect { x, y: 0, w: 100, h: 20 }
}

#[test]
fn ease_out_is_monotonic_and_hits_ends() {
    assert_eq!(ease_out(0.0), 0.0);
    assert_eq!(ease_out(1.0), 1.0);
    assert_eq!(ease_out(-1.0), 0.0);
    assert_eq!(ease_out(2.0), 1.0);
    let mut last = 0.0;
    for i in 1..=100 {
        let v = ease_out(i as f32 / 100.0);
        assert!(v >= last, "monotonic at {i}");
        last = v;
    }
    assert!(ease_out(0.5) > 0.5, "ease-out: fast start");
}

#[test]
fn retarget_mid_flight_starts_from_current_value() {
    let t0 = Instant::now();
    let mut s = Slide::new(r(0));
    s.retarget(r(200), t0, D);
    let mid = t0 + D / 2;
    let before = s.value(mid);
    assert!(before.x > 0 && before.x < 200, "{before:?}");
    s.retarget(r(400), mid, D);
    let after = s.value(mid);
    assert!((after.x - before.x).abs() <= 1, "no jump: {before:?} → {after:?}");
    assert_eq!(s.value(mid + D), r(400));
}

#[test]
fn slide_ends_exactly_on_target_and_goes_inactive() {
    let t0 = Instant::now();
    let mut s = Slide::new(r(0));
    assert!(!s.active(t0));
    s.retarget(r(0), t0, D);
    assert!(!s.active(t0), "same target: nothing moves");
    s.retarget(r(150), t0, D);
    assert!(s.active(t0 + D / 2));
    assert_eq!(s.value(t0 + D), r(150));
    assert!(!s.active(t0 + D));
}

#[test]
fn snap_jumps_without_motion() {
    let t0 = Instant::now();
    let mut s = Slide::new(r(0));
    s.retarget(r(300), t0, D);
    s.snap(r(50));
    assert_eq!(s.value(t0), r(50));
    assert!(!s.active(t0));
}

#[test]
fn fade_reverses_from_current_value() {
    let t0 = Instant::now();
    let mut f = Fade::default();
    assert_eq!(f.value(t0), 0.0);
    f.set(true, t0, D);
    let mid = t0 + D / 2;
    let v = f.value(mid);
    assert!(v > 0.0 && v < 1.0);
    f.set(false, mid, D);
    assert!((f.value(mid) - v).abs() < 0.01, "reverses where it was");
    assert_eq!(f.value(mid + D), 0.0);
    assert!(!f.active(mid + D));
    f.set(true, t0, D);
    f.set(true, t0 + D / 2, D);
    assert_eq!(f.value(t0 + D), 1.0, "setting the same state again does not restart it");
}

#[test]
fn pulse_plays_once() {
    let t0 = Instant::now();
    let p = Pulse::new(t0, Duration::from_millis(600));
    assert_eq!(p.value(t0), 0.0);
    assert!(p.value(t0 + Duration::from_millis(300)) > 0.9, "peaks halfway");
    assert_eq!(p.value(t0 + Duration::from_millis(600)), 0.0);
    assert_eq!(p.value(t0 + Duration::from_secs(5)), 0.0);
    assert!(p.active(t0 + Duration::from_millis(100)) && !p.active(t0 + Duration::from_millis(600)));
}

#[test]
fn reduced_motion_completes_at_once() {
    assert_eq!(duration(D, false), Duration::ZERO);
    assert_eq!(duration(D, true), D);
    let t0 = Instant::now();
    let mut s = Slide::new(r(0));
    s.retarget(r(200), t0, duration(D, false));
    assert!(!s.active(t0));
    assert_eq!(s.value(t0), r(200));
    let mut f = Fade::default();
    f.set(true, t0, Duration::ZERO);
    assert_eq!(f.value(t0), 1.0);
    assert!(!Pulse::new(t0, Duration::ZERO).active(t0));
}

#[test]
fn motion_snaps_on_layout_change_and_pulses_new_dots_once() {
    use wraith_client::motion::{Dot, Layout, Motion, SlideSlot};
    let t0 = Instant::now();
    let win = Layout { client: (1000, 600), sidebar_w: 0, cell: (8, 16), workspace: Some(1) };
    let mut m = Motion::new(true);
    m.begin(win);
    assert_eq!(m.slide(SlideSlot::Underline, Some(r(0)), None, t0), Some(r(0)), "first appearance: in place");
    m.end(t0);
    m.begin(win);
    m.slide(SlideSlot::Underline, Some(r(200)), None, t0);
    assert!(m.active(t0 + D / 2), "moving to the new tab");
    m.end(t0);
    m.begin(Layout { client: (800, 600), ..win });
    assert_eq!(m.slide(SlideSlot::Underline, Some(r(160)), None, t0), Some(r(160)), "resized: jumps");
    m.end(t0);
    m.set_lit(std::collections::HashSet::new(), t0);
    m.set_lit(std::collections::HashSet::from([Dot::Tab(1)]), t0);
    assert!(m.dot(Dot::Tab(1), t0 + Duration::from_millis(300)) > 0.9, "pulsing");
    m.end(t0 + Duration::from_millis(700));
    assert!(!m.active(t0 + Duration::from_millis(700)), "the pulse played once and stays lit without motion");
    let mut off = Motion::new(false);
    off.begin(win);
    off.slide(SlideSlot::Underline, Some(r(0)), None, t0);
    off.slide(SlideSlot::Underline, Some(r(300)), None, t0);
    off.set_lit(std::collections::HashSet::new(), t0);
    off.set_lit(std::collections::HashSet::from([Dot::Agent(3)]), t0);
    off.hover(Some(&wraith_client::chrome::Hit::NewTab), t0);
    assert!(!off.active(t0), "reduced motion: nothing to animate");
}

#[test]
fn overlay_shows_on_its_first_frame() {
    use wraith_client::motion::Motion;
    let t0 = Instant::now();
    let mut m = Motion::new(true);
    assert!(m.overlay(|m| &mut m.palette, true, t0) >= 0.4, "the key press shows something at once");
    assert_eq!(m.overlay(|m| &mut m.palette, true, t0 + D), 1.0);
    assert_eq!(m.overlay(|m| &mut m.palette, false, t0 + D), 0.0, "closing is instant");
}

#[test]
fn layout_change_snaps_existing_slides_without_replaying_entrance() {
    use wraith_client::motion::{Layout, Motion, SlideSlot};
    let t0 = Instant::now();
    let a = Layout { client: (1000, 600), sidebar_w: 0, cell: (8, 16), workspace: Some(1) };
    let mut m = Motion::new(true);
    let s = Rect { x: 0, y: 0, w: 192, h: 600 };
    let off = Rect { x: -192, ..s };
    m.begin(a);
    assert_eq!(m.slide(SlideSlot::Sidebar, Some(s), Some(off), t0), Some(off), "enters from the left");
    m.end(t0);
    let later = t0 + Duration::from_secs(1);
    m.begin(Layout { client: (900, 600), ..a });
    let s2 = Rect { h: 600, ..s };
    assert_eq!(m.slide(SlideSlot::Sidebar, Some(s2), Some(off), later), Some(s2), "resized: stays in place, no second entrance");
    m.end(later);
    m.begin(Layout { client: (900, 600), workspace: Some(2), ..a });
    assert_eq!(m.slide(SlideSlot::Sidebar, Some(s2), Some(off), later), Some(s2), "workspace switch: the sidebar stays put");
}

#[test]
fn workspace_switch_lets_the_workspace_bar_slide() {
    use wraith_client::motion::{Layout, Motion, SlideSlot};
    let t0 = Instant::now();
    let a = Layout { client: (1000, 600), sidebar_w: 192, cell: (8, 16), workspace: Some(1) };
    let mut m = Motion::new(true);
    m.begin(a);
    m.slide(SlideSlot::WsBar, Some(r(0)), None, t0);
    m.slide(SlideSlot::Underline, Some(r(0)), None, t0);
    m.end(t0);
    m.begin(Layout { workspace: Some(2), ..a });
    m.slide(SlideSlot::WsBar, Some(r(40)), None, t0);
    assert_eq!(m.slide(SlideSlot::Underline, Some(r(300)), None, t0), Some(r(300)), "another workspace's tabs: the underline jumps");
    assert!(m.active(t0 + D / 2), "the workspace bar slides to the new row");
}

#[test]
fn pulse_only_when_attention_starts() {
    use std::collections::HashSet;
    use wraith_client::motion::{Dot, Motion};
    let t0 = Instant::now();
    let mid = t0 + Duration::from_millis(300);
    let mut m = Motion::new(true);
    m.set_lit(HashSet::from([Dot::Tab(1)]), t0);
    assert_eq!(m.dot(Dot::Tab(1), mid), 0.0, "already lit when the window started: no pulse");
    m.set_lit(HashSet::from([Dot::Tab(1), Dot::Agent(5)]), t0);
    assert!(m.dot(Dot::Agent(5), mid) > 0.9, "newly asking: pulses");
    m.set_lit(HashSet::from([Dot::Tab(1), Dot::Agent(5)]), mid);
    assert_eq!(m.dot(Dot::Tab(1), mid), 0.0, "drawn again (sidebar revealed, workspace switched): no pulse");
}

#[test]
fn breathe_rises_and_falls_over_period() {
    use wraith_client::motion::{BREATHE, Motion};
    let mut m = Motion::new(true);
    let t0 = Instant::now();
    m.set_breathing(true, t0);
    assert!(m.breathe(t0) < 0.01);
    assert!(m.breathe(t0 + BREATHE / 2) > 0.99);
    assert!(m.breathe(t0 + BREATHE) < 0.01);
}

#[test]
fn breathe_is_steady_when_animations_off() {
    use wraith_client::motion::Motion;
    let mut m = Motion::new(false);
    let t0 = Instant::now();
    m.set_breathing(true, t0);
    assert_eq!(m.breathe(t0), 0.5);
    assert_eq!(m.breathe(t0 + Duration::from_millis(777)), 0.5);
    assert_eq!(m.frame_interval(t0 + Duration::from_millis(777)), None);
}

#[test]
fn breathing_stops_when_attention_clears() {
    use wraith_client::motion::Motion;
    let mut m = Motion::new(true);
    let t0 = Instant::now();
    m.set_breathing(true, t0);
    m.set_breathing(false, t0 + Duration::from_millis(400));
    assert_eq!(m.breathe(t0 + Duration::from_millis(800)), 0.0);
}

#[test]
fn frame_interval_slows_for_breathing_only() {
    use wraith_client::motion::{Motion, SlideSlot};
    let mut m = Motion::new(true);
    let t0 = Instant::now();
    m.set_breathing(true, t0);
    assert_eq!(m.frame_interval(t0), Some(Duration::from_millis(50)));
    m.slide(SlideSlot::Underline, Some(r(0)), None, t0);
    m.slide(SlideSlot::Underline, Some(r(200)), None, t0);
    assert_eq!(m.frame_interval(t0 + Duration::from_millis(10)), Some(Duration::from_millis(16)));
}

#[test]
fn frame_interval_none_when_nothing_moves() {
    use wraith_client::motion::Motion;
    let m = Motion::new(true);
    assert_eq!(m.frame_interval(Instant::now()), None);
}

#[test]
fn running_motion_draws_every_frame_whatever_the_breath_deadline() {
    use wraith_client::motion::{Motion, SlideSlot};
    let mut m = Motion::new(true);
    let t0 = Instant::now();
    m.set_breathing(true, t0);
    m.slide(SlideSlot::Underline, Some(r(0)), None, t0);
    m.slide(SlideSlot::Underline, Some(r(200)), None, t0);
    let now = t0 + Duration::from_millis(10);
    let far = now + Duration::from_millis(50);
    assert!(m.due(now, far), "a slide must not wait for the breathing deadline");
    assert_eq!(m.after_frame(now, far), (true, Some(Duration::from_millis(16))), "redraw at vsync");
}

#[test]
fn breathing_alone_waits_for_its_deadline() {
    use wraith_client::motion::Motion;
    let mut m = Motion::new(true);
    let t0 = Instant::now();
    m.set_breathing(true, t0);
    let next = t0 + Duration::from_millis(50);
    assert!(!m.due(t0 + Duration::from_millis(20), next));
    assert!(m.due(next, next));
    assert_eq!(m.after_frame(t0 + Duration::from_millis(20), next), (false, Some(Duration::from_millis(31))));
    assert_eq!(Motion::new(true).after_frame(t0, next), (false, None), "nothing moving: no wake-up");
}
