#![cfg(windows)]
use std::rc::Rc;

use wraith_client::atlas::GlyphAtlas;
use wraith_client::font::FontStack;
use wraith_client::gpu::Gpu;
use wraith_client::grid::{Batch, DrawOpts, GridRenderer, Pipeline};
use wraith_proto::model::Rect;
use wraith_proto::{Colors, CursorShape, CursorState, FrameDelta, Scrollbar};

#[test]
fn pane_opacity_is_premultiplied() {
    let gpu = Gpu::new().unwrap();
    let fonts = FontStack::new(&["Cascadia Mono".into()], 12.0, 96, &[]).unwrap();
    let mut atlas = GlyphAtlas::new(&gpu, 1024).unwrap();
    let pipe = Rc::new(Pipeline::new(&gpu).unwrap());
    let mut grid = GridRenderer::with_pipeline(pipe.clone());
    let off = gpu.offscreen(200, 100).unwrap();
    grid.apply(
        &FrameDelta {
            seq: 1,
            full: true,
            cols: 4,
            rows: 2,
            dirty_rows: vec![],
            cursor: CursorState { x: 0, y: 0, visible: false, style: CursorShape::Block },
            colors: Colors { fg: (0, 0, 0), bg: (255, 255, 255), cursor: None, palette: vec![(0, 0, 0); 256] },
            scrollbar: Scrollbar { total: 2, offset: 0, len: 2 },
            title: None,
            images: vec![],
            search: None,
        },
        &fonts,
    );
    let opts = DrawOpts { font: 0, selection: None, selection_color: (0, 0, 0), cursor_blink_on: true, focused: true, opacity: 0.5 };
    let mut q = Vec::new();
    // The grid starts at (20, 20): the 20 px around it are the pane's padding.
    grid.push_quads(&mut q, &gpu, &mut atlas, &fonts, (20.0, 20.0), &fonts.metrics(), &opts, Some(Rect { x: 0, y: 0, w: 200, h: 100 }));
    pipe.submit(&gpu, off.rtv(), (200, 100), &atlas, &[Batch { quads: q, texture: None }], [0.0; 4]);
    let px = off.read_pixel(&gpu, 5, 5);
    assert!(px.iter().all(|&c| c.abs_diff(128) <= 1), "padding pixel {px:?}");
}
