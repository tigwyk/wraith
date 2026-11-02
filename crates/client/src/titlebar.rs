//! The custom title bar (spec §4.2): caption button geometry and the non-client hit test that keeps
//! Windows' resize edges, window dragging and Windows 11 snap layouts working. Pure, so it is tested
//! without a window.
use wraith_proto::model::Rect;

pub const HTCLIENT: isize = 1;
pub const HTCAPTION: isize = 2;
pub const HTMINBUTTON: isize = 8;
pub const HTMAXBUTTON: isize = 9;
pub const HTLEFT: isize = 10;
pub const HTRIGHT: isize = 11;
pub const HTTOP: isize = 12;
pub const HTTOPLEFT: isize = 13;
pub const HTTOPRIGHT: isize = 14;
pub const HTBOTTOM: isize = 15;
pub const HTBOTTOMLEFT: isize = 16;
pub const HTBOTTOMRIGHT: isize = 17;
pub const HTCLOSE: isize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Width of one caption button at 96 dpi.
const BUTTON_W: i32 = 46;

/// Width of the three caption buttons together.
pub fn caption_width(dpi: u32) -> i32 {
    3 * (BUTTON_W * dpi as i32 / 96)
}

/// Minimize, maximize and close, flush right along a bar `h` px tall.
pub fn caption_rects(client_w: i32, h: i32, dpi: u32) -> [(Caption, Rect); 3] {
    let w = caption_width(dpi) / 3;
    let at = |i: i32| Rect { x: client_w - (3 - i) * w, y: 0, w, h };
    [(Caption::Minimize, at(0)), (Caption::Maximize, at(1)), (Caption::Close, at(2))]
}

/// Everything the hit test needs, in client pixels.
pub struct NcGeom {
    pub client: (i32, i32),
    /// Resize border thickness.
    pub border: i32,
    pub caption_h: i32,
    pub maximized: bool,
    pub caps: [(Caption, Rect); 3],
}

/// `WM_NCHITTEST` result for the point (`x`, `y`) in client coordinates (outside the client rectangle
/// is the resize border Windows keeps on the left, right and bottom). `interactive`: the point is
/// over a tab, the new-tab button or a workspace row, which take clicks instead of dragging.
pub fn nc_hit(g: &NcGeom, x: i32, y: i32, interactive: bool) -> isize {
    for (cap, r) in &g.caps {
        if r.contains(x, y) {
            return match cap {
                Caption::Minimize => HTMINBUTTON,
                Caption::Maximize => HTMAXBUTTON,
                Caption::Close => HTCLOSE,
            };
        }
    }
    let (w, h) = g.client;
    if !g.maximized {
        // Windows draws no top border of its own now, so the top resize edge is a thin strip of the bar.
        let top = y < g.border.min(4);
        let (left, right, bottom) = (x < 0, x >= w, y >= h);
        let near_left = left || x < g.border;
        let near_right = right || x >= w - g.border;
        if top && near_left {
            return HTTOPLEFT;
        }
        if top && near_right {
            return HTTOPRIGHT;
        }
        if bottom && left {
            return HTBOTTOMLEFT;
        }
        if bottom && right {
            return HTBOTTOMRIGHT;
        }
        if top {
            return HTTOP;
        }
        if left {
            return HTLEFT;
        }
        if right {
            return HTRIGHT;
        }
        if bottom {
            return HTBOTTOM;
        }
    }
    if (0..g.caption_h).contains(&y) && !interactive && (0..w).contains(&x) { HTCAPTION } else { HTCLIENT }
}
