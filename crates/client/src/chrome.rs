//! Window chrome geometry and hit testing (spec §4.2): sidebar, tab bar and content regions. Pure
//! rectangle math, so every rule is tested without a window.
use wraith_proto::model::{Divider, Rect};
use wraith_proto::wire::PaneId;
use wraith_proto::model::{TabId, WorkspaceId};

/// How far from a divider line a press still grabs it.
const DIVIDER_SLOP: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    pub sidebar: Option<Rect>,
    pub tabbar: Rect,
    pub content: Rect,
}

/// Sidebar on the left (full height), then the tab bar over the content.
pub fn regions(client: (i32, i32), cell: (i32, i32), sidebar_cols: Option<u16>, bar_h: i32) -> Regions {
    let (w, h) = (client.0.max(0), client.1.max(0));
    let sw = sidebar_cols.map_or(0, |c| (i32::from(c) * cell.0).clamp(0, w / 2));
    let bar = bar_h.clamp(0, h);
    Regions {
        sidebar: sidebar_cols.map(|_| Rect { x: 0, y: 0, w: sw, h }),
        tabbar: Rect { x: sw, y: 0, w: w - sw, h: bar },
        content: Rect { x: sw, y: bar, w: w - sw, h: h - bar },
    }
}

/// `count` equal tabs along `bar`, then the new-tab button. Tabs are at most 24 cells wide and shrink
/// (rather than overflow) when crowded. `reserved_right` keeps room for the caption buttons.
pub fn tab_rects(count: usize, bar: Rect, cell_w: i32, reserved_right: i32) -> (Vec<Rect>, Rect) {
    let button = 3 * cell_w;
    let avail = (bar.w - reserved_right - button).max(0);
    let w = if count == 0 { 0 } else { (avail / count as i32).min(24 * cell_w) };
    let tabs: Vec<Rect> = (0..count as i32).map(|i| Rect { x: bar.x + i * w, y: bar.y, w, h: bar.h }).collect();
    (tabs, Rect { x: bar.x + count as i32 * w, y: bar.y, w: button.min(bar.w), h: bar.h })
}

/// The close button at the right end of a tab.
pub fn close_rect(tab: Rect, cell_w: i32) -> Rect {
    let w = (2 * cell_w).min(tab.w);
    Rect { x: tab.x + tab.w - w, y: tab.y, w, h: tab.h }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Tab(TabId),
    TabClose(TabId),
    NewTab,
    Workspace(WorkspaceId),
    NewWorkspace,
    /// An index into `HitCtx::dividers`.
    Divider(usize),
    Pane(PaneId),
    /// Empty sidebar area.
    Sidebar,
    /// Empty tab bar area (drags the window).
    Bar,
    None,
}

pub struct HitCtx<'a> {
    pub regions: &'a Regions,
    pub tabs: &'a [(TabId, Rect)],
    pub new_tab: Rect,
    pub workspaces: &'a [(WorkspaceId, Rect)],
    pub new_workspace: Rect,
    pub panes: &'a [(PaneId, Rect)],
    pub dividers: &'a [Divider],
    pub cell_w: i32,
}

pub fn hit(c: &HitCtx, x: i32, y: i32) -> Hit {
    if c.regions.tabbar.contains(x, y) {
        if let Some((id, r)) = c.tabs.iter().find(|(_, r)| r.contains(x, y)) {
            return if close_rect(*r, c.cell_w).contains(x, y) { Hit::TabClose(*id) } else { Hit::Tab(*id) };
        }
        return if c.new_tab.contains(x, y) { Hit::NewTab } else { Hit::Bar };
    }
    if c.regions.sidebar.is_some_and(|s| s.contains(x, y)) {
        if let Some((id, _)) = c.workspaces.iter().find(|(_, r)| r.contains(x, y)) {
            return Hit::Workspace(*id);
        }
        return if c.new_workspace.contains(x, y) { Hit::NewWorkspace } else { Hit::Sidebar };
    }
    if c.regions.content.contains(x, y) {
        if let Some(i) = c.dividers.iter().position(|d| {
            let l = d.line;
            Rect { x: l.x - DIVIDER_SLOP, y: l.y - DIVIDER_SLOP, w: l.w + 2 * DIVIDER_SLOP, h: l.h + 2 * DIVIDER_SLOP }.contains(x, y)
        }) {
            return Hit::Divider(i);
        }
        if let Some((id, _)) = c.panes.iter().find(|(_, r)| r.contains(x, y)) {
            return Hit::Pane(*id);
        }
    }
    Hit::None
}

type Rgb = (u8, u8, u8);

/// `a` blended `t` of the way toward `b`.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromeColors {
    pub bar_bg: Rgb,
    pub bar_fg: Rgb,
    pub bar_fg_dim: Rgb,
    pub tab_active_bg: Rgb,
    pub divider: Rgb,
}

/// Chrome colors derived from the active theme (spec §4.2: all chrome uses the theme's colors).
pub fn chrome_colors(bg: Rgb, fg: Rgb, _accent: Rgb) -> ChromeColors {
    let luma = (u32::from(bg.0) * 299 + u32::from(bg.1) * 587 + u32::from(bg.2) * 114) / 1000;
    let bar_bg = mix(bg, (0, 0, 0), if luma < 128 { 0.3 } else { 0.06 });
    ChromeColors { bar_bg, bar_fg: fg, bar_fg_dim: mix(fg, bar_bg, 0.45), tab_active_bg: bg, divider: mix(bg, fg, 0.25) }
}
