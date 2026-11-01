//! M0 smoke tests: libghostty-vt works on x86_64-pc-windows-msvc.
use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::render::{CellIterator, Dirty, RowIterator};
use libghostty_vt::{RenderState, Terminal, TerminalOptions, key};

fn term(cols: u16, rows: u16) -> Terminal<'static, 'static> {
    Terminal::new(TerminalOptions { cols, rows, max_scrollback: 1000 }).unwrap()
}

/// Text of viewport row `y`, one grapheme per cell, empty cells as spaces.
fn read_row(t: &Terminal, y: usize) -> String {
    let mut rs = RenderState::new().unwrap();
    let snap = rs.update(t).unwrap();
    let mut rows = RowIterator::new().unwrap();
    let mut cells = CellIterator::new().unwrap();
    let mut it = rows.update(&snap).unwrap();
    let mut buf = String::new();
    let mut i = 0;
    while let Some(row) = it.next() {
        if i == y {
            let mut out = String::new();
            let mut ci = cells.update(row).unwrap();
            while let Some(cell) = ci.next() {
                if cell.graphemes_len().unwrap() == 0 {
                    out.push(' ');
                } else {
                    cell.graphemes_utf8(&mut buf).unwrap();
                    out.push_str(&buf);
                }
            }
            return out;
        }
        i += 1;
    }
    panic!("row {y} out of range");
}

fn encode_ctrl_c() -> Vec<u8> {
    let t = term(80, 24);
    let mut enc = key::Encoder::new().unwrap();
    enc.set_options_from_terminal(&t);
    let mut ev = key::Event::new().unwrap();
    ev.set_action(key::Action::Press).set_key(key::Key::C).set_mods(key::Mods::CTRL);
    let mut out = Vec::new();
    enc.encode_to_vec(&ev, &mut out).unwrap();
    out
}

#[test]
fn vt_write_plain_text_appears_in_grid() {
    let mut t = term(20, 5);
    t.vt_write(b"hello\r\nworld");
    assert_eq!(read_row(&t, 0).trim_end(), "hello");
    assert_eq!(read_row(&t, 1).trim_end(), "world");
    assert_eq!((t.cursor_x().unwrap(), t.cursor_y().unwrap()), (5, 1));
}

#[test]
fn render_state_reports_dirty_after_write() {
    let mut t = term(20, 5);
    let mut rs = RenderState::new().unwrap();
    {
        let snap = rs.update(&t).unwrap();
        snap.set_dirty(Dirty::Clean).unwrap();
    }
    t.vt_write(b"x");
    let snap = rs.update(&t).unwrap();
    assert!(!matches!(snap.dirty().unwrap(), Dirty::Clean));
}

#[test]
fn key_encoder_encodes_ctrl_c_as_etx() {
    assert_eq!(encode_ctrl_c(), vec![0x03]);
}

#[test]
fn on_pty_write_receives_dsr_reply() {
    let got = Rc::new(RefCell::new(Vec::<u8>::new()));
    let mut t = term(80, 24);
    let sink = got.clone();
    t.on_pty_write(move |_t, data| sink.borrow_mut().extend_from_slice(data)).unwrap();
    t.vt_write(b"\x1b[6n");
    assert_eq!(&*got.borrow(), b"\x1b[1;1R");
}
