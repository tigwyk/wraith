use wraith_client::titlebar::{
    Caption, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, NcGeom, caption_rects, nc_hit,
};

fn geom(maximized: bool) -> NcGeom {
    NcGeom { client: (1000, 600), border: 8, caption_h: 30, maximized, caps: caption_rects(1000, 30, 96) }
}

#[test]
fn caption_rects_are_flush_right_and_scale_with_dpi() {
    let c = caption_rects(1000, 30, 96);
    assert_eq!(c.map(|(k, _)| k), [Caption::Minimize, Caption::Maximize, Caption::Close]);
    assert_eq!((c[2].1.x, c[2].1.w, c[2].1.h), (1000 - 46, 46, 30));
    assert_eq!(c[1].1.x, c[2].1.x - 46);
    assert_eq!(c[0].1.x, c[1].1.x - 46);
    let big = caption_rects(1000, 45, 144);
    assert_eq!(big[2].1.w, 69, "46 px at 150 %");
    assert_eq!(big[2].1.x + big[2].1.w, 1000);
}

#[test]
fn nc_hit_maximize_button_is_htmaxbutton() {
    let g = geom(false);
    let max = g.caps[1].1;
    assert_eq!(nc_hit(&g, max.x + 5, 15, false), HTMAXBUTTON, "Windows 11 shows its snap layouts over this code");
    assert_eq!(nc_hit(&g, g.caps[0].1.x + 5, 15, false), HTMINBUTTON);
    assert_eq!(nc_hit(&g, g.caps[2].1.x + 5, 15, false), HTCLOSE);
    assert_eq!(nc_hit(&g, g.caps[2].1.x + 5, 1, false), HTCLOSE, "buttons win over the top resize edge");
}

#[test]
fn nc_hit_borders_resize_unless_maximized() {
    let g = geom(false);
    assert_eq!(nc_hit(&g, -3, 300, false), HTLEFT);
    assert_eq!(nc_hit(&g, 1003, 300, false), HTRIGHT);
    assert_eq!(nc_hit(&g, 500, 603, false), HTBOTTOM);
    assert_eq!(nc_hit(&g, 500, 1, true), HTTOP, "a thin resize edge at the very top");
    assert_eq!(nc_hit(&g, 500, 10, true), HTCLIENT);
    let m = geom(true);
    assert_eq!(nc_hit(&m, 500, 1, true), HTCLIENT, "a maximized window has no edges");
    assert_eq!(nc_hit(&m, -3, 300, false), HTCLIENT);
}

#[test]
fn nc_hit_title_drag_area_excludes_tabs_and_caption_buttons() {
    let g = geom(false);
    assert_eq!(nc_hit(&g, 600, 15, false), HTCAPTION, "empty tab-bar space drags the window");
    assert_eq!(nc_hit(&g, 600, 15, true), HTCLIENT, "tabs and the new-tab button are clicks");
    assert_eq!(nc_hit(&g, 600, 100, false), HTCLIENT, "below the bar is the content");
}

#[test]
fn nc_hit_corners_beat_edges() {
    let g = geom(false);
    assert_eq!(nc_hit(&g, -2, -2, false), HTTOPLEFT);
    assert_eq!(nc_hit(&g, 1002, -2, false), HTTOPRIGHT);
    assert_eq!(nc_hit(&g, 2, 1, false), HTTOPLEFT, "top-left corner inside the client area");
    assert_eq!(nc_hit(&g, -2, 602, false), HTBOTTOMLEFT);
    assert_eq!(nc_hit(&g, 1002, 602, false), HTBOTTOMRIGHT);
}
