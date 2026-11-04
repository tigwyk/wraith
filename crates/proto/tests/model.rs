use wraith_proto::model::*;

fn leaf(id: u64) -> PaneNode {
    PaneNode::Leaf(id)
}

fn split(axis: Axis, a: PaneNode, b: PaneNode) -> PaneNode {
    PaneNode::Split { axis, ratio: 0.5, a: Box::new(a), b: Box::new(b) }
}

fn area(w: i32, h: i32) -> Rect {
    Rect { x: 0, y: 0, w, h }
}

#[test]
fn split_inserts_new_leaf_by_side() {
    let mut t = leaf(1);
    assert!(t.split(1, 2, Axis::Horizontal, false));
    assert_eq!(t.leaves(), vec![1, 2]);
    assert!(t.split(1, 3, Axis::Vertical, true));
    assert_eq!(t.leaves(), vec![3, 1, 2]);
    assert!(!t.split(99, 4, Axis::Vertical, false));
}

#[test]
fn remove_returns_sibling_as_focus() {
    let mut t = split(Axis::Horizontal, leaf(1), split(Axis::Vertical, leaf(2), leaf(3)));
    assert_eq!(t.remove(1), Removal::Removed { focus: 2 });
    assert_eq!(t.leaves(), vec![2, 3]);
    assert_eq!(t.remove(3), Removal::Removed { focus: 2 });
    assert_eq!(t, leaf(2));
}

#[test]
fn remove_last_leaf_reports_last() {
    assert_eq!(leaf(1).remove(1), Removal::Last);
}

#[test]
fn remove_missing_reports_not_found() {
    let mut t = split(Axis::Horizontal, leaf(1), leaf(2));
    assert_eq!(t.remove(9), Removal::NotFound);
    assert_eq!(t.leaves(), vec![1, 2]);
}

#[test]
fn layout_tiles_area_exactly() {
    let t = split(Axis::Horizontal, leaf(1), split(Axis::Vertical, leaf(2), leaf(3)));
    let rects = t.layout(area(1000, 600), 1, (16, 16));
    assert_eq!(rects.len(), 3);
    let covered: i64 = rects.iter().map(|(_, r)| i64::from(r.w) * i64::from(r.h)).sum();
    // 1000x600 minus the vertical divider column (600) and the horizontal divider row in the right half.
    let right_w = i64::from(rects[1].1.w);
    assert_eq!(covered, 1000 * 600 - 600 - right_w);
    for (i, (_, a)) in rects.iter().enumerate() {
        for (_, b) in &rects[i + 1..] {
            let overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
            assert!(!overlap, "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn layout_never_below_min() {
    let mut t = leaf(1);
    for id in 2..=10 {
        t.split(id - 1, id, if id % 2 == 0 { Axis::Horizontal } else { Axis::Vertical }, false);
    }
    let a = area(40, 20);
    let rects = t.layout(a, 1, (16, 16));
    assert_eq!(rects.len(), 10);
    for (_, r) in &rects {
        assert!(r.w >= 0 && r.h >= 0, "{r:?}");
        assert!(r.x >= 0 && r.y >= 0 && r.x + r.w <= a.w && r.y + r.h <= a.h, "{r:?} outside");
    }
    // Plenty of room: every pane honors the minimum.
    let big = split(Axis::Horizontal, leaf(1), leaf(2)).layout(area(100, 40), 1, (16, 16));
    assert!(big.iter().all(|(_, r)| r.w >= 16 && r.h >= 16));
}

#[test]
fn layout_zero_area_is_empty() {
    let t = split(Axis::Horizontal, leaf(1), leaf(2));
    let rects = t.layout(area(0, 0), 1, (16, 16));
    assert_eq!(rects.len(), 2);
    assert!(rects.iter().all(|(_, r)| r.w == 0 && r.h == 0));
}

#[test]
fn ratio_is_clamped_so_both_sides_keep_min() {
    let mut t = split(Axis::Horizontal, leaf(1), leaf(2));
    assert!(t.set_ratio(&[], 0.0));
    let rects = t.layout(area(200, 50), 1, (30, 10));
    assert!(rects.iter().all(|(_, r)| r.w >= 30));
    assert!(t.set_ratio(&[], 1.0));
    let rects = t.layout(area(200, 50), 1, (30, 10));
    assert!(rects.iter().all(|(_, r)| r.w >= 30));
    assert!(!t.set_ratio(&[true], 0.5), "path into a leaf");
}

fn grid() -> PaneNode {
    split(Axis::Vertical, split(Axis::Horizontal, leaf(1), leaf(2)), split(Axis::Horizontal, leaf(3), leaf(4)))
}

#[test]
fn focus_neighbor_picks_overlapping_rect() {
    let rects = grid().layout(area(400, 400), 1, (10, 10));
    assert_eq!(focus_neighbor(&rects, 1, Dir::Right), Some(2));
    assert_eq!(focus_neighbor(&rects, 1, Dir::Down), Some(3));
    assert_eq!(focus_neighbor(&rects, 4, Dir::Left), Some(3));
    assert_eq!(focus_neighbor(&rects, 4, Dir::Up), Some(2));
}

#[test]
fn focus_neighbor_none_at_edge() {
    let rects = grid().layout(area(400, 400), 1, (10, 10));
    assert_eq!(focus_neighbor(&rects, 1, Dir::Left), None);
    assert_eq!(focus_neighbor(&rects, 1, Dir::Up), None);
    assert_eq!(focus_neighbor(&rects, 9, Dir::Up), None);
}

#[test]
fn nudge_moves_nearest_ancestor_split_on_that_axis() {
    let mut t = grid();
    assert!(t.nudge(1, Dir::Right, 0.1));
    let PaneNode::Split { ratio, a, .. } = &t else { panic!() };
    assert_eq!(*ratio, 0.5, "outer vertical split untouched");
    let PaneNode::Split { ratio, .. } = &**a else { panic!() };
    assert!((ratio - 0.6).abs() < 1e-6);
    assert!(t.nudge(1, Dir::Down, 0.1));
    assert!(!leaf(1).nudge(1, Dir::Right, 0.1));
}

#[test]
fn dividers_report_path_and_parent() {
    let t = split(Axis::Horizontal, leaf(1), split(Axis::Vertical, leaf(2), leaf(3)));
    let divs = t.dividers(area(1000, 600), 1, (16, 16));
    assert_eq!(divs.len(), 2);
    assert_eq!(divs[0].path, Vec::<bool>::new());
    assert_eq!(divs[0].axis, Axis::Horizontal);
    assert_eq!(divs[0].parent, area(1000, 600));
    assert_eq!(divs[0].line.w, 1);
    assert_eq!(divs[0].line.h, 600);
    assert_eq!(divs[1].path, vec![true]);
    assert_eq!(divs[1].axis, Axis::Vertical);
}

fn snapshot() -> ModelSnapshot {
    ModelSnapshot {
        rev: 7,
        workspaces: vec![Workspace {
            id: 1,
            name: "api".into(),
            root_dir: Some(r"C:\src".into()),
            color: 0x3366cc,
            tabs: vec![10],
            active_tab: Some(10),
            profile: "pwsh".into(),
            attached: true,
            last_used: 3,
            quick: false,
        }],
        tabs: vec![Tab { id: 10, workspace: 1, title_override: None, root: grid(), focused: 2, zoomed: None }],
        panes: vec![PaneMeta { id: 2, title: "vim".into(), cwd: None, exit: Some(1), profile: "pwsh".into(), cmdline: "pwsh.exe".into(), agent: None }],
    }
}

#[test]
fn snapshot_json_roundtrip() {
    let s = snapshot();
    let back: ModelSnapshot = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}

#[test]
fn snapshot_postcard_roundtrip() {
    let s = snapshot();
    let back: ModelSnapshot = postcard::from_bytes(&postcard::to_allocvec(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}

#[test]
fn tab_title_prefers_override() {
    let mut s = snapshot();
    assert_eq!(s.tab_title(10), "vim");
    s.tabs[0].title_override = Some("build".into());
    assert_eq!(s.tab_title(10), "build");
    s.tabs[0].title_override = None;
    s.tabs[0].focused = 3;
    assert_eq!(s.tab_title(10), "shell");
}
