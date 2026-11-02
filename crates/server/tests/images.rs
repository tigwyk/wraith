//! Kitty graphics: placements from libghostty, image uploads to the client (spec §3.4, M3).
use libghostty_vt::{Terminal, TerminalOptions};
use wraith_proto::pane::PaneEvent;
use wraith_server::images::{ImageSync, collect, install_png_decoder};

const BIG: usize = 64 << 20;

fn term() -> Terminal<'static, 'static> {
    let mut t = Terminal::new(TerminalOptions { cols: 20, rows: 5, max_scrollback: 100 }).unwrap();
    t.resize(20, 5, 8, 16).unwrap();
    t.set_kitty_image_storage_limit(1 << 20).unwrap();
    t
}

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

fn rgba(id: u32, w: u32, h: u32, px: &[u8]) -> Vec<u8> {
    format!("\x1b_Ga=T,f=32,s={w},v={h},i={id},q=2;{}\x1b\\", b64(px)).into_bytes()
}

fn uploads(ev: &[PaneEvent]) -> Vec<u32> {
    ev.iter().filter_map(|e| if let PaneEvent::ImageUpload { image_id, .. } = e { Some(*image_id) } else { None }).collect()
}

#[test]
fn rgba_transmit_and_display_yields_placement() {
    let mut t = term();
    let px: Vec<u8> = (0..16).collect();
    t.vt_write(&rgba(7, 2, 2, &px));
    let (ev, placements) = ImageSync::default().sync(&t, BIG);
    assert_eq!(ev, vec![PaneEvent::ImageUpload { image_id: 7, width: 2, height: 2, rgba: px }]);
    assert_eq!(placements.len(), 1, "{placements:?}");
    let p = &placements[0];
    assert_eq!((p.image_id, p.col, p.row, p.cols, p.rows), (7, 0, 0, 1, 1));
    assert_eq!((p.src, p.px), ((0, 0, 2, 2), (2, 2)));
}

#[test]
fn png_transmit_decodes() {
    install_png_decoder();
    let mut t = term();
    t.vt_write(b"\x1b_Ga=T,f=100,i=3,q=2;iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==\x1b\\");
    let (ev, placements) = ImageSync::default().sync(&t, BIG);
    assert_eq!(ev.len(), 1, "{ev:?}");
    let PaneEvent::ImageUpload { image_id, width, height, rgba } = &ev[0] else { panic!("{ev:?}") };
    assert_eq!((*image_id, *width, *height), (3, 1, 1));
    assert_eq!(rgba.len(), 4);
    assert_eq!(rgba[3], 255, "opaque");
    assert_eq!(placements.len(), 1);
}

#[test]
fn gray_and_rgb_expand_to_rgba() {
    use libghostty_vt::kitty::graphics::ImageFormat;
    use wraith_server::images::to_rgba;
    assert_eq!(to_rgba(ImageFormat::Rgb, 1, 2, &[1, 2, 3, 4, 5, 6]), Some(vec![1, 2, 3, 255, 4, 5, 6, 255]));
    assert_eq!(to_rgba(ImageFormat::Gray, 2, 1, &[9, 10]), Some(vec![9, 9, 9, 255, 10, 10, 10, 255]));
    assert_eq!(to_rgba(ImageFormat::GrayAlpha, 1, 1, &[9, 50]), Some(vec![9, 9, 9, 50]));
    assert_eq!(to_rgba(ImageFormat::Rgba, 1, 1, &[1, 2, 3, 4]), Some(vec![1, 2, 3, 4]));
    assert_eq!(to_rgba(ImageFormat::Rgb, 2, 2, &[1, 2, 3]), None, "short data is rejected, not read past");
}

#[test]
fn upload_happens_once_until_reattach() {
    let mut t = term();
    t.vt_write(&rgba(7, 2, 2, &[0; 16]));
    let mut sync = ImageSync::default();
    assert_eq!(uploads(&sync.sync(&t, BIG).0), vec![7]);
    let (ev, placements) = sync.sync(&t, BIG);
    assert!(ev.is_empty(), "{ev:?}");
    assert_eq!(placements.len(), 1, "placements are sent every frame");
    sync.reset();
    assert_eq!(uploads(&sync.sync(&t, BIG).0), vec![7], "a new view gets the image again");
}

#[test]
fn retransmitted_image_is_uploaded_again() {
    let mut t = term();
    let mut sync = ImageSync::default();
    t.vt_write(&rgba(7, 2, 2, &[0; 16]));
    sync.sync(&t, BIG);
    t.vt_write(&rgba(7, 2, 2, &[255; 16]));
    let (ev, _) = sync.sync(&t, BIG);
    assert!(matches!(&ev[..], [PaneEvent::ImageUpload { image_id: 7, rgba, .. }] if rgba[0] == 255), "{ev:?}");
}

#[test]
fn deleted_image_sends_delete() {
    let mut t = term();
    let mut sync = ImageSync::default();
    t.vt_write(&rgba(7, 2, 2, &[0; 16]));
    sync.sync(&t, BIG);
    t.vt_write(b"\x1b_Ga=d,d=I,i=7,q=2\x1b\\");
    let (ev, placements) = sync.sync(&t, BIG);
    assert_eq!(ev, vec![PaneEvent::ImageDelete { image_id: 7 }]);
    assert!(placements.is_empty());
}

#[test]
fn oversize_image_is_skipped() {
    let mut t = term();
    t.vt_write(&rgba(7, 2, 2, &[0; 16]));
    let (ev, placements) = ImageSync::default().sync(&t, 8);
    assert!(ev.is_empty() && placements.is_empty(), "{ev:?} {placements:?}");
}

#[test]
fn placement_scrolls_with_text() {
    let mut t = term();
    // 16x32 px = 2 cols x 2 rows at 8x16 cells.
    t.vt_write(&rgba(7, 16, 32, &[0; 16 * 32 * 4]));
    let mut rows = vec![collect(&t)[0].row];
    for _ in 0..10 {
        t.vt_write(b"\n");
        match collect(&t).first() {
            Some(p) => rows.push(p.row),
            None => break,
        }
    }
    assert_eq!(rows[0], 0);
    assert!(rows.windows(2).all(|w| w[1] <= w[0]), "{rows:?}");
    assert_eq!(*rows.last().unwrap(), -1, "partly scrolled off, then gone: {rows:?}");
    assert!(collect(&t).is_empty());
}

#[test]
fn storage_limit_evicts_oldest() {
    let mut t = term();
    t.set_kitty_image_storage_limit(64).unwrap();
    for id in 1..=3 {
        t.vt_write(&rgba(id, 4, 4, &[0; 64]));
    }
    let ids: Vec<u32> = collect(&t).iter().map(|p| p.image_id).collect();
    assert!(!ids.contains(&1) && ids.contains(&3), "{ids:?}");
}
