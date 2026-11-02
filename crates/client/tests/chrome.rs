use wraith_client::chrome::{Hit, HitCtx, close_rect, hit, regions, tab_rects};
use wraith_proto::model::{Axis, Divider, Rect};

const CELL: (i32, i32) = (8, 16);

fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn regions_reserve_titlebar_tabbar_sidebar() {
    let g = regions((1000, 600), CELL, Some(24), 26);
    assert_eq!(g.sidebar, Some(r(0, 0, 192, 600)));
    assert_eq!(g.tabbar, r(192, 0, 808, 26));
    assert_eq!(g.content, r(192, 26, 808, 574));
}

#[test]
fn no_sidebar_when_hidden() {
    let g = regions((1000, 600), CELL, None, 26);
    assert_eq!(g.sidebar, None);
    assert_eq!(g.tabbar, r(0, 0, 1000, 26));
    assert_eq!(g.content, r(0, 26, 1000, 574));
}

#[test]
fn content_never_negative_in_tiny_window() {
    for (w, h) in [(0, 0), (10, 10), (50, 20), (300, 5)] {
        let g = regions((w, h), CELL, Some(24), 26);
        for rect in [g.tabbar, g.content].into_iter().chain(g.sidebar) {
            assert!(rect.w >= 0 && rect.h >= 0 && rect.x >= 0 && rect.y >= 0, "{w}x{h}: {rect:?}");
            assert!(rect.x + rect.w <= w.max(0) && rect.y + rect.h <= h.max(0), "{w}x{h}: {rect:?} outside");
        }
    }
}

#[test]
fn tab_rects_fit_bar_with_new_tab_button() {
    let bar = r(0, 0, 800, 26);
    let (tabs, plus) = tab_rects(3, bar, 8, 0);
    assert_eq!(tabs.len(), 3);
    assert!(tabs.iter().all(|t| t.w == 24 * 8), "wide bars cap tabs at 24 cells");
    assert_eq!(plus.x, 3 * 24 * 8);
    let (tabs, plus) = tab_rects(40, bar, 8, 0);
    assert!(tabs.iter().all(|t| t.w > 0 && t.w < 8 * 8), "crowded tabs shrink below the minimum rather than overflow");
    assert!(plus.x + plus.w <= 800 && tabs.last().unwrap().x + tabs.last().unwrap().w <= plus.x);
    let (tabs, plus) = tab_rects(2, bar, 8, 140);
    assert!(plus.x + plus.w <= 800 - 140, "caption buttons keep their space: {tabs:?} {plus:?}");
    let (none, plus) = tab_rects(0, bar, 8, 0);
    assert!(none.is_empty() && plus.x == 0);
}

static WS: [(u64, Rect); 1] = [(7, Rect { x: 0, y: 40, w: 192, h: 20 })];

fn ctx<'a>(g: &'a wraith_client::chrome::Regions, tabs: &'a [(u64, Rect)], panes: &'a [(u64, Rect)], divs: &'a [Divider]) -> HitCtx<'a> {
    HitCtx { regions: g, tabs, new_tab: r(392, 0, 24, 26), workspaces: &WS, new_workspace: r(0, 560, 192, 20), panes, dividers: divs, cell_w: 8 }
}

#[test]
fn hit_prefers_tab_close_over_tab() {
    let g = regions((1000, 600), CELL, Some(24), 26);
    let tabs = [(1, r(192, 0, 100, 26)), (2, r(292, 0, 100, 26))];
    let c = ctx(&g, &tabs, &[], &[]);
    assert_eq!(hit(&c, 200, 10), Hit::Tab(1));
    let close = close_rect(tabs[1].1, 8);
    assert_eq!(hit(&c, close.x + 2, 10), Hit::TabClose(2));
    assert_eq!(hit(&c, 310, 10), Hit::Tab(2));
    assert_eq!(hit(&c, 395, 10), Hit::NewTab, "new-tab button");
    assert_eq!(hit(&c, 900, 10), Hit::Bar);
}

#[test]
fn hit_divider_has_slop_of_three_px() {
    let g = regions((1000, 600), CELL, None, 26);
    let panes = [(1, r(0, 26, 499, 574)), (2, r(500, 26, 500, 574))];
    let divs = [Divider { path: vec![], axis: Axis::Horizontal, parent: r(0, 26, 1000, 574), line: r(499, 26, 1, 574) }];
    let c = ctx(&g, &[], &panes, &divs);
    assert_eq!(hit(&c, 499, 300), Hit::Divider(0));
    assert_eq!(hit(&c, 497, 300), Hit::Divider(0));
    assert_eq!(hit(&c, 502, 300), Hit::Divider(0));
    assert_eq!(hit(&c, 480, 300), Hit::Pane(1));
    assert_eq!(hit(&c, 520, 300), Hit::Pane(2));
}

#[test]
fn hit_pane_inside_content_and_sidebar_rows() {
    let g = regions((1000, 600), CELL, Some(24), 26);
    let panes = [(1, g.content)];
    let c = ctx(&g, &[], &panes, &[]);
    assert_eq!(hit(&c, 500, 300), Hit::Pane(1));
    assert_eq!(hit(&c, 10, 45), Hit::Workspace(7));
    assert_eq!(hit(&c, 10, 565), Hit::NewWorkspace);
    assert_eq!(hit(&c, 10, 300), Hit::Sidebar);
    assert_eq!(hit(&c, -5, 300), Hit::None);
}

#[test]
fn chrome_colors_follow_the_theme() {
    use wraith_client::chrome::{chrome_colors, mix};
    assert_eq!(mix((0, 0, 0), (200, 100, 50), 0.0), (0, 0, 0));
    assert_eq!(mix((0, 0, 0), (200, 100, 50), 1.0), (200, 100, 50));
    assert_eq!(mix((0, 0, 0), (200, 100, 50), 0.5), (100, 50, 25));
    let dark = chrome_colors((40, 44, 52), (255, 255, 255), (97, 175, 239));
    assert_eq!(dark.tab_active_bg, (40, 44, 52), "the active tab joins the content");
    assert!(dark.bar_bg.0 < 40 && dark.bar_bg.1 < 44, "dark themes get a darker bar: {:?}", dark.bar_bg);
    assert_eq!(dark.bar_fg, (255, 255, 255));
    assert!(dark.bar_fg_dim.0 < 255 && dark.bar_fg_dim.0 > dark.bar_bg.0, "dim text sits between text and bar");
    let light = chrome_colors((250, 250, 250), (20, 20, 20), (0, 90, 200));
    assert!(light.bar_bg.0 < 250, "{:?}", light.bar_bg);
}
