//! Frames say which rows soft-wrap onto the next, so the client can rejoin a wrapped URL.
use libghostty_vt::{RenderState, Terminal};
use wraith_server::frame_build::build_frame;

#[test]
fn soft_wrapped_rows_are_flagged() {
    let mut t = Terminal::new(10, 4).unwrap();
    t.vt_write(b"0123456789abc\r\nshort\r\n");
    let mut rs = RenderState::new().unwrap();
    let snap = rs.update(&t).unwrap();
    let f = build_frame(&snap, 0, true).unwrap();
    let wrapped: Vec<bool> = f.dirty_rows.iter().map(|r| r.wrapped).collect();
    assert_eq!(wrapped, [true, false, false, false]);
}
