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
