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
    HitCtx { regions: g, tabs, new_tab: r(392, 0, 24, 26), workspaces: &WS, new_workspace: r(0, 560, 192, 20), agents: &[], sidebar_tabs: &[], panes, dividers: divs, cell_w: 8 }
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

#[test]
fn cursor_for_divider_is_resize() {
    use wraith_client::chrome::{Cursor, cursor_for};
    let vline = Divider { path: vec![], axis: Axis::Horizontal, parent: r(0, 26, 1000, 574), line: r(499, 26, 1, 574) };
    let hline = Divider { path: vec![], axis: Axis::Vertical, parent: r(0, 26, 1000, 574), line: r(0, 300, 1000, 1) };
    let divs = [vline, hline];
    assert_eq!(cursor_for(&Hit::Divider(0), &divs, false), Cursor::SizeWE, "side-by-side panes: drag left/right");
    assert_eq!(cursor_for(&Hit::Divider(1), &divs, false), Cursor::SizeNS, "stacked panes: drag up/down");
}

#[test]
fn cursor_over_tracking_pane_is_arrow() {
    use wraith_client::chrome::{Cursor, cursor_for};
    assert_eq!(cursor_for(&Hit::Pane(1), &[], false), Cursor::IBeam);
    assert_eq!(cursor_for(&Hit::Pane(1), &[], true), Cursor::Arrow, "the program owns the mouse");
    assert_eq!(cursor_for(&Hit::Tab(10), &[], false), Cursor::Arrow);
    assert_eq!(cursor_for(&Hit::None, &[], false), Cursor::Arrow);
}

#[test]
fn tooltip_names_action_and_binding() {
    use wraith_client::chrome::tooltip_for;
    let binds = wraith_config::Config::default().keybinds;
    assert_eq!(tooltip_for(&Hit::NewTab, &binds).as_deref(), Some("New tab  Ctrl+Shift+T"));
    assert!(tooltip_for(&Hit::TabClose(3), &binds).unwrap().starts_with("Close tab"));
    assert!(tooltip_for(&Hit::NewWorkspace, &binds).unwrap().starts_with("New workspace"));
    assert_eq!(tooltip_for(&Hit::NewTab, &[]).as_deref(), Some("New tab"), "unbound: the name alone");
}

#[test]
fn tooltip_follows_rebinding() {
    use wraith_client::chrome::tooltip_for;
    let mut binds = wraith_config::Config::default().keybinds;
    binds.retain(|k| k.action != wraith_config::Action::NewTab);
    binds.push(wraith_config::parse_keybind("ctrl+t=new_tab").unwrap());
    assert_eq!(tooltip_for(&Hit::NewTab, &binds).as_deref(), Some("New tab  Ctrl+T"));
}

#[test]
fn no_tooltip_for_panes_or_rows() {
    use wraith_client::chrome::tooltip_for;
    let binds = wraith_config::Config::default().keybinds;
    for h in [Hit::Pane(1), Hit::Tab(1), Hit::Workspace(1), Hit::Agent(1), Hit::SidebarTab(1), Hit::Divider(0), Hit::Bar, Hit::None] {
        assert_eq!(tooltip_for(&h, &binds), None, "{h:?}");
    }
}

#[test]
fn context_menu_copy_needs_selection() {
    use wraith_client::chrome::context_menu;
    use wraith_config::Action;
    let binds = wraith_config::Config::default().keybinds;
    let find = |m: &[wraith_client::chrome::MenuItem], a: Action| m.iter().find(|i| i.action == Some(a)).unwrap().enabled;
    assert!(!find(&context_menu(false, &binds), Action::Copy));
    assert!(find(&context_menu(true, &binds), Action::Copy));
    assert!(find(&context_menu(false, &binds), Action::Paste));
}

#[test]
fn context_menu_labels_carry_keys() {
    use wraith_client::chrome::context_menu;
    let binds = wraith_config::Config::default().keybinds;
    let m = context_menu(true, &binds);
    assert!(m[0].label.ends_with("\tCtrl+Shift+C"), "{:?}", m[0].label);
    let unbound = context_menu(true, &[]);
    assert_eq!(unbound[0].label, "Copy", "no binding: no tab");
}

#[test]
fn context_menu_order_and_separators() {
    use wraith_client::chrome::context_menu;
    use wraith_config::Action::*;
    use wraith_proto::model::Dir;
    let actions: Vec<_> = context_menu(false, &[]).iter().map(|i| i.action).collect();
    assert_eq!(
        actions,
        vec![
            Some(Copy),
            Some(Paste),
            None,
            Some(NewSplit(Dir::Right)),
            Some(NewSplit(Dir::Down)),
            None,
            Some(NewTab),
            Some(CommandPalette),
            None,
            Some(CloseSurface),
        ]
    );
}
