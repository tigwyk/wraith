//! Per-frame work must scale with the viewport, not the history (spec §6.3).
use std::time::{Duration, Instant};

use libghostty_vt::{RenderState, Terminal};
use wraith_server::frame_build::build_frame;

fn time_full_frames(scrollback_lines: usize) -> Duration {
    let mut t = Terminal::new(200, 60).unwrap();
    t.set_scrollback_max_lines(Some(20_000)).unwrap();
    for i in 0..(60 + scrollback_lines) {
        t.vt_write(format!("\x1b[3{}mline {i} {}\r\n", i % 8, "x".repeat(150)).as_bytes());
    }
    let mut rs = RenderState::new().unwrap();
    let start = Instant::now();
    for seq in 0..50 {
        let snap = rs.update(&t).unwrap();
        let f = build_frame(&snap, seq, true).unwrap();
        assert_eq!(f.dirty_rows.len(), 60);
    }
    start.elapsed()
}

#[test]
fn frame_cost_independent_of_scrollback() {
    let empty = time_full_frames(0);
    let full = time_full_frames(10_000);
    let ratio = full.as_secs_f64() / empty.as_secs_f64();
    assert!(ratio < 1.5, "empty {empty:?} vs 10k scrollback {full:?} (ratio {ratio:.2})");
}
