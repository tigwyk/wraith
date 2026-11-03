#![cfg(windows)]
//! Inline images (Kitty graphics): placement geometry and layered drawing against an offscreen target.
use wraith_client::atlas::GlyphAtlas;
use wraith_client::font::{CellMetrics, FontStack};
use wraith_client::gpu::{Gpu, Offscreen};
use wraith_client::grid::{Batch, DrawOpts, GridRenderer, Quad};
use wraith_client::images::{ImageStore, image_quad};
use wraith_proto::model::Rect;
use wraith_proto::{Color, Colors, CursorShape, CursorState, FrameDelta, ImagePlacement, Row, Scrollbar, StyleFlags, StyleRun};

fn metrics() -> CellMetrics {
    CellMetrics { w: 8, h: 16, baseline: 12, underline_pos: 14, underline_thickness: 1 }
}

fn placement(id: u32, col: i32, row: i32, px: (u32, u32), z: i32) -> ImagePlacement {
    ImagePlacement { image_id: id, col, row, cols: 0, rows: 0, x_off: 0, y_off: 0, src: (0, 0, px.0, px.1), px, z }
}

#[test]
fn quad_covers_its_cells_with_offsets() {
    let mut p = placement(1, 2, 1, (16, 32), 0);
    p.x_off = 3;
    p.y_off = 4;
    let q = image_quad(&p, (16, 32), (10.0, 20.0), &metrics(), Rect { x: 0, y: 0, w: 1000, h: 1000 }, 1.0).unwrap();
    assert_eq!(q.pos, [10.0 + 16.0 + 3.0, 20.0 + 16.0 + 4.0]);
    assert_eq!(q.size, [16.0, 32.0]);
    assert_eq!(q.uv, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(q.kind, 3);
}

#[test]
fn partly_scrolled_image_is_clipped() {
    // Row -2 of a 4-row image: only the bottom half shows, and it samples the bottom half.
    let p = placement(1, 0, -2, (8, 64), 0);
    let q = image_quad(&p, (8, 64), (0.0, 0.0), &metrics(), Rect { x: 0, y: 0, w: 100, h: 100 }, 1.0).unwrap();
    assert_eq!((q.pos[1], q.size[1]), (0.0, 32.0));
    assert_eq!((q.uv[1], q.uv[3]), (0.5, 1.0));
}

#[test]
fn source_rect_maps_to_uv_and_offscreen_is_none() {
    let mut p = placement(1, 0, 0, (10, 10), 0);
    p.src = (5, 0, 5, 10);
    let q = image_quad(&p, (10, 10), (0.0, 0.0), &metrics(), Rect { x: 0, y: 0, w: 100, h: 100 }, 1.0).unwrap();
    assert_eq!(q.uv, [0.5, 0.0, 1.0, 1.0]);
    let gone = placement(1, 0, 20, (10, 10), 0);
    assert!(image_quad(&gone, (10, 10), (0.0, 0.0), &metrics(), Rect { x: 0, y: 0, w: 100, h: 100 }, 1.0).is_none());
}

struct Rig {
    gpu: Gpu,
    fonts: FontStack,
    atlas: GlyphAtlas,
    grid: GridRenderer,
    off: Offscreen,
    store: ImageStore,
}

fn rig() -> Rig {
    let gpu = Gpu::new().unwrap();
    let fonts = FontStack::new(&["Cascadia Mono".into()], 12.0, 96, &[]).unwrap();
    let atlas = GlyphAtlas::new(&gpu, 1024).unwrap();
    let grid = GridRenderer::new(&gpu).unwrap();
    let off = gpu.offscreen(200, 100).unwrap();
    Rig { gpu, fonts, atlas, grid, off, store: ImageStore::default() }
}

fn frame(rows: Vec<Row>, images: Vec<ImagePlacement>) -> FrameDelta {
    FrameDelta {
        seq: 1,
        full: true,
        cols: 20,
        rows: 5,
        dirty_rows: rows,
        cursor: CursorState { x: 0, y: 0, visible: false, style: CursorShape::Block },
        colors: Colors { fg: (255, 255, 255), bg: (1, 2, 3), cursor: None, palette: vec![(0, 0, 0); 256] },
        scrollbar: Scrollbar { total: 5, offset: 0, len: 5 },
        title: None,
        images,
        search: None,
    }
}

fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    c.repeat((w * h) as usize)
}

impl Rig {
    fn draw(&mut self) {
        let m = self.fonts.metrics();
        let opts = DrawOpts { font: 0, selection: None, selection_color: (0, 0, 0), cursor_blink_on: true, focused: true, opacity: 1.0 };
        let (mut out, mut q): (Vec<Batch>, Vec<Quad>) = (Vec::new(), Vec::new());
        self.grid.push_layers(&mut out, &mut q, &self.store, 1, &self.gpu, &mut self.atlas, &self.fonts, (0.0, 0.0), &m, &opts, Rect { x: 0, y: 0, w: 200, h: 100 });
        out.push(Batch { quads: q, texture: None });
        self.grid.pipeline().submit(&self.gpu, self.off.rtv(), (200, 100), &self.atlas, &out, [0.0, 0.0, 0.0, 1.0]);
    }
    fn cell(&self, x: u32, y: u32) -> [u8; 4] {
        let m = self.fonts.metrics();
        self.off.read_pixel(&self.gpu, x * m.w + m.w / 2, y * m.h + m.h / 2)
    }
}

fn block(fg: Color) -> Row {
    Row { y: 0, wrapped: false, runs: vec![StyleRun { fg, bg: Color::Default, flags: StyleFlags(0), hyperlink_id: 0, text: "█".into() }] }
}

#[test]
fn solid_red_image_fills_its_cell_rect() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.store.upload(&r.gpu, 1, 7, 2 * m.w, m.h, &solid(2 * m.w, m.h, [255, 0, 0, 255])).unwrap();
    r.grid.apply(&frame(vec![], vec![placement(7, 1, 1, (2 * m.w, m.h), 0)]), &r.fonts);
    r.draw();
    assert_eq!(r.cell(1, 1), [255, 0, 0, 255]);
    assert_eq!(r.cell(2, 1), [255, 0, 0, 255]);
    assert_eq!(r.cell(0, 1), [1, 2, 3, 255], "outside the placement");
    assert_eq!(r.cell(3, 1), [1, 2, 3, 255]);
}

#[test]
fn image_below_text_is_covered_by_glyph_pixels() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.store.upload(&r.gpu, 1, 7, m.w, m.h, &solid(m.w, m.h, [255, 0, 0, 255])).unwrap();
    r.grid.apply(&frame(vec![block(Color::Rgb(0, 200, 0))], vec![placement(7, 0, 0, (m.w, m.h), -1)]), &r.fonts);
    r.draw();
    assert_eq!(r.cell(0, 0), [0, 200, 0, 255]);
}

#[test]
fn image_above_text_covers_glyph() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.store.upload(&r.gpu, 1, 7, m.w, m.h, &solid(m.w, m.h, [255, 0, 0, 255])).unwrap();
    r.grid.apply(&frame(vec![block(Color::Rgb(0, 200, 0))], vec![placement(7, 0, 0, (m.w, m.h), 0)]), &r.fonts);
    r.draw();
    assert_eq!(r.cell(0, 0), [255, 0, 0, 255]);
}

#[test]
fn half_transparent_image_blends_over_background() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.store.upload(&r.gpu, 1, 7, m.w, m.h, &solid(m.w, m.h, [255, 255, 255, 128])).unwrap();
    r.grid.apply(&frame(vec![], vec![placement(7, 0, 0, (m.w, m.h), 0)]), &r.fonts);
    r.draw();
    let px = r.cell(0, 0);
    assert!((126..=130).contains(&px[0]), "straight alpha is premultiplied once: {px:?}");
}

#[test]
fn deleted_image_is_not_drawn() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.store.upload(&r.gpu, 1, 7, m.w, m.h, &solid(m.w, m.h, [255, 0, 0, 255])).unwrap();
    r.store.delete(1, 7);
    r.grid.apply(&frame(vec![], vec![placement(7, 0, 0, (m.w, m.h), 0)]), &r.fonts);
    r.draw();
    assert_eq!(r.cell(0, 0), [1, 2, 3, 255]);
}

#[test]
fn placements_follow_the_latest_frame() {
    let mut r = rig();
    let m = r.fonts.metrics();
    r.store.upload(&r.gpu, 1, 7, m.w, m.h, &solid(m.w, m.h, [255, 0, 0, 255])).unwrap();
    r.grid.apply(&frame(vec![], vec![placement(7, 0, 0, (m.w, m.h), 0)]), &r.fonts);
    let mut moved = frame(vec![], vec![placement(7, 0, 2, (m.w, m.h), 0)]);
    moved.full = false;
    r.grid.apply(&moved, &r.fonts);
    r.draw();
    assert_eq!(r.cell(0, 2), [255, 0, 0, 255], "a frame with no dirty rows still moves the image");
    assert_eq!(r.cell(0, 0), [1, 2, 3, 255]);
}

#[test]
fn drop_pane_frees_textures() {
    let r = rig();
    let mut store = ImageStore::default();
    store.upload(&r.gpu, 1, 7, 1, 1, &[0; 4]).unwrap();
    store.upload(&r.gpu, 2, 7, 1, 1, &[0; 4]).unwrap();
    store.drop_pane(1);
    assert!(store.get(1, 7).is_none());
    assert!(store.get(2, 7).is_some(), "the same image id in another pane is separate");
    assert!(store.upload(&r.gpu, 1, 8, 2, 2, &[0; 4]).is_err(), "short pixel data is refused");
}
