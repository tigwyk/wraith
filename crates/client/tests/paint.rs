#![cfg(windows)]
//! Several grids and chrome drawn through one pipeline in one submit.
use std::rc::Rc;

use wraith_client::atlas::GlyphAtlas;
use wraith_client::font::FontStack;
use wraith_client::gpu::Gpu;
use wraith_client::grid::{Batch, DrawOpts, GridRenderer, Pipeline};
use wraith_client::paint::{Painter, text_width};
use wraith_proto::model::Rect;
use wraith_proto::{Colors, CursorShape, CursorState, FrameDelta, Scrollbar};

fn frame(bg: (u8, u8, u8)) -> FrameDelta {
    FrameDelta {
        seq: 1,
        full: true,
        cols: 4,
        rows: 2,
        dirty_rows: vec![],
        cursor: CursorState { x: 0, y: 0, visible: false, style: CursorShape::Block },
        colors: Colors { fg: (255, 255, 255), bg, cursor: None, palette: vec![(0, 0, 0); 256] },
        scrollbar: Scrollbar { total: 2, offset: 0, len: 2 },
        title: None,
        images: vec![],
        search: None,
    }
}

fn opts() -> DrawOpts {
    DrawOpts { font: 0, selection: None, selection_color: (1, 1, 1), cursor_blink_on: true, focused: true, opacity: 1.0 }
}

const CLEAR: [f32; 4] = [50.0 / 255.0, 60.0 / 255.0, 70.0 / 255.0, 1.0];

#[test]
fn two_grids_render_into_disjoint_rects() {
    let gpu = Gpu::new().unwrap();
    let fonts = FontStack::new(&["Cascadia Mono".into()], 12.0, 96, &[]).unwrap();
    let mut atlas = GlyphAtlas::new(&gpu, 1024).unwrap();
    let pipe = Rc::new(Pipeline::new(&gpu).unwrap());
    let (mut a, mut b) = (GridRenderer::with_pipeline(pipe.clone()), GridRenderer::with_pipeline(pipe.clone()));
    a.apply(&frame((1, 2, 3)), &fonts);
    b.apply(&frame((9, 8, 7)), &fonts);
    let off = gpu.offscreen(200, 100).unwrap();
    let m = fonts.metrics();
    let (mut qa, mut qb) = (Vec::new(), Vec::new());
    a.push_quads(&mut qa, &gpu, &mut atlas, &fonts, (0.0, 0.0), &m, &opts(), Some(Rect { x: 0, y: 0, w: 90, h: 100 }));
    b.push_quads(&mut qb, &gpu, &mut atlas, &fonts, (110.0, 0.0), &m, &opts(), Some(Rect { x: 110, y: 0, w: 90, h: 100 }));
    pipe.submit(&gpu, off.rtv(), (200, 100), &atlas, &[Batch { quads: qa, texture: None }, Batch { quads: qb, texture: None }], CLEAR);
    assert_eq!(off.read_pixel(&gpu, 45, 50), [1, 2, 3, 255]);
    assert_eq!(off.read_pixel(&gpu, 150, 50), [9, 8, 7, 255]);
    assert_eq!(off.read_pixel(&gpu, 100, 50), [50, 60, 70, 255], "the gap shows the chrome color");
}

#[test]
fn painter_text_draws_glyph_pixels() {
    let gpu = Gpu::new().unwrap();
    let fonts = FontStack::new(&["Cascadia Mono".into()], 12.0, 96, &[]).unwrap();
    let mut atlas = GlyphAtlas::new(&gpu, 1024).unwrap();
    let pipe = Pipeline::new(&gpu).unwrap();
    let off = gpu.offscreen(200, 100).unwrap();
    let mut q = Vec::new();
    let advance = Painter { q: &mut q }.text(&gpu, &mut atlas, &fonts, 10, 20, "WW", (255, 255, 255), 1.0);
    assert_eq!(advance, text_width(&fonts, "WW"));
    assert_eq!(advance, 2 * fonts.metrics().w as i32);
    pipe.submit(&gpu, off.rtv(), (200, 100), &atlas, &[Batch { quads: q, texture: None }], [0.0, 0.0, 0.0, 1.0]);
    let m = fonts.metrics();
    let mut lit = 0;
    for y in 20..20 + m.h {
        for x in 10..10 + advance as u32 {
            if off.read_pixel(&gpu, x, y) != [0, 0, 0, 255] {
                lit += 1;
            }
        }
    }
    assert!(lit > 20, "only {lit} lit pixels for two W glyphs");
    assert_eq!(off.read_pixel(&gpu, 150, 90), [0, 0, 0, 255], "nothing outside the text");
}

#[test]
fn painter_rect_alpha_blends() {
    let gpu = Gpu::new().unwrap();
    let atlas = GlyphAtlas::new(&gpu, 256).unwrap();
    let pipe = Pipeline::new(&gpu).unwrap();
    let off = gpu.offscreen(40, 40).unwrap();
    let mut q = Vec::new();
    Painter { q: &mut q }.rect(Rect { x: 0, y: 0, w: 20, h: 40 }, (255, 0, 0), 0.5);
    pipe.submit(&gpu, off.rtv(), (40, 40), &atlas, &[Batch { quads: q, texture: None }], [0.0, 0.0, 1.0, 1.0]);
    let [r, g, b, a] = off.read_pixel(&gpu, 10, 10);
    assert!((i32::from(r) - 128).abs() <= 2 && g == 0 && (i32::from(b) - 128).abs() <= 2 && a == 255, "{r} {g} {b} {a}");
    assert_eq!(off.read_pixel(&gpu, 30, 10), [0, 0, 255, 255]);
}
