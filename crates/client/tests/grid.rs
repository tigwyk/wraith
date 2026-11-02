#![cfg(windows)]
use wraith_client::atlas::GlyphAtlas;
use wraith_client::font::FontStack;
use wraith_client::gpu::{Gpu, Offscreen};
use wraith_client::grid::{DrawOpts, GridRenderer};
use wraith_proto::{Color, Colors, CursorShape, CursorState, FrameDelta, Row, Scrollbar, SelRange, StyleFlags, StyleRun};

struct Rig {
    gpu: Gpu,
    fonts: FontStack,
    atlas: GlyphAtlas,
    grid: GridRenderer,
    off: Offscreen,
}

fn rig() -> Rig {
    let gpu = Gpu::new().unwrap();
    let fonts = FontStack::new(&["Cascadia Mono".into()], 12.0, 96, &[]).unwrap();
    let atlas = GlyphAtlas::new(&gpu, 1024).unwrap();
    let grid = GridRenderer::new(&gpu).unwrap();
    let off = gpu.offscreen(200, 100).unwrap();
    Rig { gpu, fonts, atlas, grid, off }
}

fn run(text: &str, fg: Color, bg: Color, flags: u16) -> StyleRun {
    StyleRun { fg, bg, flags: StyleFlags(flags), hyperlink_id: 0, text: text.into() }
}

fn frame(full: bool, rows: Vec<Row>, cursor: Option<(u16, u16)>) -> FrameDelta {
    FrameDelta {
        seq: 1,
        full,
        cols: 20,
        rows: 5,
        dirty_rows: rows,
        cursor: CursorState { x: cursor.map_or(0, |c| c.0), y: cursor.map_or(0, |c| c.1), visible: cursor.is_some(), style: CursorShape::Block },
        colors: Colors { fg: (255, 255, 255), bg: (1, 2, 3), cursor: Some((0, 255, 0)), palette: vec![(0, 0, 0); 256] },
        scrollbar: Scrollbar { total: 5, offset: 0, len: 5 },
        title: None,
        images: vec![],
    }
}

fn row(y: u16, runs: Vec<StyleRun>) -> Row {
    Row { y, runs, wrapped: false }
}

impl Rig {
    fn draw(&mut self, opts: &DrawOpts) {
        let m = self.fonts.metrics();
        self.grid.draw(&self.gpu, self.off.rtv(), (200, 100), &mut self.atlas, &self.fonts, (0.0, 0.0), &m, opts);
    }
    /// Pixel at the center of cell (x, y).
    fn cell(&self, x: u32, y: u32) -> [u8; 4] {
        let m = self.fonts.metrics();
        self.off.read_pixel(&self.gpu, x * m.w + m.w / 2, y * m.h + m.h / 2)
    }
}

fn opts() -> DrawOpts {
    DrawOpts { selection: None, selection_color: (100, 100, 200), cursor_blink_on: true, focused: true, opacity: 1.0 }
}

#[test]
fn background_fills_with_frame_bg() {
    let mut r = rig();
    r.grid.apply(&frame(true, vec![], None), &r.fonts);
    r.draw(&opts());
    assert_eq!(r.off.read_pixel(&r.gpu, 150, 90), [1, 2, 3, 255]);
}

#[test]
fn full_block_fills_cell_with_fg() {
    let mut r = rig();
    r.grid.apply(&frame(true, vec![row(0, vec![run("█", Color::Rgb(200, 0, 0), Color::Default, 0)])], None), &r.fonts);
    r.draw(&opts());
    assert_eq!(r.cell(0, 0), [200, 0, 0, 255]);
    assert_eq!(r.cell(1, 0), [1, 2, 3, 255]);
}

#[test]
fn inverse_swaps_colors() {
    let mut r = rig();
    let inv = run("█", Color::Rgb(255, 0, 0), Color::Rgb(0, 0, 255), StyleFlags::INVERSE);
    r.grid.apply(&frame(true, vec![row(0, vec![inv])], None), &r.fonts);
    r.draw(&opts());
    assert_eq!(r.cell(0, 0), [0, 0, 255, 255]);
}

#[test]
fn cell_background_color_is_drawn() {
    let mut r = rig();
    r.grid.apply(&frame(true, vec![row(1, vec![run("  ", Color::Default, Color::Rgb(9, 99, 199), 0)])], None), &r.fonts);
    r.draw(&opts());
    assert_eq!(r.cell(1, 1), [9, 99, 199, 255]);
}

#[test]
fn cursor_block_drawn_at_position() {
    let mut r = rig();
    r.grid.apply(&frame(true, vec![], Some((2, 0))), &r.fonts);
    r.draw(&opts());
    assert_eq!(r.cell(2, 0), [0, 255, 0, 255]);
    assert_eq!(r.cell(1, 0), [1, 2, 3, 255]);
}

#[test]
fn partial_frame_keeps_other_rows() {
    let mut r = rig();
    let block = |y| row(y, vec![run("█", Color::Rgb(200, 0, 0), Color::Default, 0)]);
    r.grid.apply(&frame(true, vec![block(0), block(1)], None), &r.fonts);
    r.grid.apply(&frame(false, vec![row(1, vec![run(" ", Color::Default, Color::Default, 0)])], None), &r.fonts);
    r.draw(&opts());
    assert_eq!(r.cell(0, 0), [200, 0, 0, 255]);
    assert_eq!(r.cell(0, 1), [1, 2, 3, 255]);
}

#[test]
fn selection_highlights_cells() {
    let mut r = rig();
    r.grid.apply(&frame(true, vec![], None), &r.fonts);
    r.draw(&DrawOpts { selection: Some(SelRange { start: (0, 0), end: (3, 0), rectangle: false }), ..opts() });
    assert_eq!(r.cell(1, 0), [100, 100, 200, 255]);
    assert_eq!(r.cell(5, 0), [1, 2, 3, 255]);
}

#[test]
fn selection_spans_linear_reversed_and_rectangle() {
    use wraith_client::grid::selection_spans;
    let lin = SelRange { start: (5, 2), end: (3, 0), rectangle: false }; // dragged upwards
    assert_eq!(selection_spans(lin, 20), vec![(0, 3, 19), (1, 0, 19), (2, 0, 5)]);
    let rect = SelRange { start: (8, 1), end: (2, 2), rectangle: true };
    assert_eq!(selection_spans(rect, 20), vec![(1, 2, 8), (2, 2, 8)]);
    let clipped = SelRange { start: (0, 0), end: (50, 0), rectangle: false };
    assert_eq!(selection_spans(clipped, 20), vec![(0, 0, 19)]);
}

#[test]
fn selected_flag_draws_selection_color() {
    let mut r = rig();
    let sel = run("ab", Color::Default, Color::Default, StyleFlags::SELECTED);
    r.grid.apply(&frame(true, vec![row(2, vec![sel])], None), &r.fonts);
    r.draw(&opts());
    let px = r.off.read_pixel(&r.gpu, r.fonts.metrics().w + 1, 2 * r.fonts.metrics().h + 1);
    assert_eq!(px, [100, 100, 200, 255]);
}

/// Every pixel of the cell block [x0, x1) x [y0, y1) (in cells) has color `c`.
fn filled(r: &Rig, cells: (u32, u32, u32, u32), c: [u8; 4]) -> Vec<(u32, u32)> {
    let m = r.fonts.metrics();
    let (x0, y0, x1, y1) = cells;
    let mut bad = Vec::new();
    for y in y0 * m.h..y1 * m.h {
        for x in x0 * m.w..x1 * m.w {
            if r.off.read_pixel(&r.gpu, x, y) != c {
                bad.push((x, y));
            }
        }
    }
    bad
}

const RED: [u8; 4] = [200, 0, 0, 255];
const BG: [u8; 4] = [1, 2, 3, 255];

#[test]
fn full_blocks_tile_without_seams() {
    let mut r = rig();
    let blocks = |y| row(y, vec![run("███", Color::Rgb(200, 0, 0), Color::Default, 0)]);
    r.grid.apply(&frame(true, vec![blocks(0), blocks(1)], None), &r.fonts);
    r.draw(&opts());
    let gaps = filled(&r, (0, 0, 3, 2), RED);
    assert!(gaps.is_empty(), "{} unfilled pixels, first {:?}", gaps.len(), &gaps[..gaps.len().min(8)]);
}

#[test]
fn half_and_quadrant_blocks_are_exact() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.grid.apply(&frame(true, vec![row(0, vec![run("▀▐▛", Color::Rgb(200, 0, 0), Color::Default, 0)])], None), &r.fonts);
    r.draw(&opts());
    let (hw, hh) = (m.w / 2, m.h / 2);
    let px = |x, y| r.off.read_pixel(&r.gpu, x, y);
    // ▀ upper half
    assert_eq!((px(0, 0), px(m.w - 1, hh - 1), px(0, hh), px(m.w - 1, m.h - 1)), (RED, RED, BG, BG));
    // ▐ right half
    let x = m.w;
    assert_eq!((px(x, 0), px(x + hw - 1, m.h - 1), px(x + hw, 0), px(x + m.w - 1, m.h - 1)), (BG, BG, RED, RED));
    // ▛ all but lower-right quadrant
    let x = 2 * m.w;
    assert_eq!((px(x, 0), px(x + m.w - 1, 0), px(x, m.h - 1), px(x + m.w - 1, m.h - 1)), (RED, RED, RED, BG));
}

#[test]
fn box_lines_join_across_cells() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.grid.apply(&frame(true, vec![
        row(0, vec![run("┌──┐", Color::Rgb(200, 0, 0), Color::Default, 0)]),
        row(1, vec![run("│  │", Color::Rgb(200, 0, 0), Color::Default, 0)]),
    ], None), &r.fonts);
    r.draw(&opts());
    let mid_y = (0..m.h).find(|&y| r.off.read_pixel(&r.gpu, m.w + m.w / 2, y) == RED).expect("horizontal line row");
    // The horizontal line is unbroken from the corner's centre to the other corner's centre.
    for x in m.w / 2 + 1..3 * m.w + m.w / 2 {
        assert_eq!(r.off.read_pixel(&r.gpu, x, mid_y), RED, "gap at x={x}");
    }
    let mid_x = (0..m.w).find(|&x| r.off.read_pixel(&r.gpu, x, m.h + m.h / 2) == RED).expect("vertical line column");
    // The vertical line runs from the corner down through the next row without a break.
    for y in mid_y + 1..2 * m.h {
        assert_eq!(r.off.read_pixel(&r.gpu, mid_x, y), RED, "gap at y={y}");
    }
}

#[test]
fn sprite_shapes() {
    use wraith_client::sprite::sprite_rects;
    let cover = |c, w: u32, h: u32| {
        let mut g = vec![0u8; (w * h) as usize];
        for r in sprite_rects(c, w, h, 1).unwrap() {
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    g[(y * w + x) as usize] = (r.alpha * 255.0) as u8;
                }
            }
        }
        g
    };
    assert!(cover('█', 9, 19).iter().all(|&a| a == 255));
    assert_eq!(cover('▄', 4, 4), [0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255]);
    assert_eq!(cover('▚', 2, 2), [255, 0, 0, 255]);
    assert!(cover('▒', 4, 4).iter().all(|&a| a == 127));
    assert!(sprite_rects('a', 9, 19, 1).is_none());
    assert!(sprite_rects('╱', 9, 19, 1).is_none(), "diagonals stay with the font");
    // Every box-drawing char we claim draws something.
    for c in ('\u{2500}'..='\u{2570}').chain('\u{2574}'..='\u{257F}') {
        assert!(sprite_rects(c, 9, 19, 1).is_some_and(|r| !r.is_empty()), "{c:?}");
    }
}
