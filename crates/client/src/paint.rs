//! Chrome drawing: solid rectangles and text appended to the same quad list the pane grids use,
//! so a whole window is one submit (spec §4.2, §6.1). Text uses the terminal font and cell metrics.
use unicode_width::UnicodeWidthStr;
use wraith_proto::StyleFlags;
use wraith_proto::model::Rect;

use crate::atlas::{GlyphAtlas, GlyphKey};
use crate::font::FontStack;
use crate::gpu::Gpu;
use crate::grid::Quad;

type Rgb = (u8, u8, u8);

fn color(c: Rgb, a: f32) -> [f32; 4] {
    [f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0, a]
}

/// Width of `s` in pixels: its cells times the cell width.
pub fn text_width(fonts: &FontStack, s: &str) -> i32 {
    (s.width() as u32 * fonts.metrics().w) as i32
}

pub struct Painter<'a> {
    pub q: &'a mut Vec<Quad>,
}

impl Painter<'_> {
    pub fn rect(&mut self, r: Rect, rgb: Rgb, alpha: f32) {
        if r.w <= 0 || r.h <= 0 {
            return;
        }
        self.q.push(Quad { pos: [r.x as f32, r.y as f32], size: [r.w as f32, r.h as f32], uv: [0.0; 4], color: color(rgb, alpha), kind: 0 });
    }

    /// Draw `s` with its top-left at (`x`, `y`); returns the advance in pixels.
    #[allow(clippy::too_many_arguments, reason = "one text run's full input")]
    pub fn text(&mut self, gpu: &Gpu, atlas: &mut GlyphAtlas, fonts: &FontStack, x: i32, y: i32, s: &str, rgb: Rgb, alpha: f32) -> i32 {
        let m = fonts.metrics();
        if s.chars().any(|c| c != ' ') {
            for g in fonts.shape(s, StyleFlags(0)) {
                let e = atlas.get(gpu, fonts, GlyphKey { face: g.face, glyph: g.glyph, subpx: 0 });
                if e.w == 0 {
                    continue;
                }
                let gx = x + (u32::from(g.cell) * m.w) as i32;
                self.q.push(Quad {
                    pos: [(gx + i32::from(e.bearing.0)) as f32, (y + m.baseline as i32 + i32::from(e.bearing.1)) as f32],
                    size: [f32::from(e.w), f32::from(e.h)],
                    uv: e.uv,
                    color: color(rgb, alpha),
                    kind: if e.color { 2 } else { 1 },
                });
            }
        }
        text_width(fonts, s)
    }
}

/// Move quads already pushed by (`dx`, `dy`) and scale their opacity: how an overlay slides or fades
/// as one piece without each drawing call knowing about it.
pub fn shift(q: &mut [Quad], dx: f32, dy: f32, alpha: f32) {
    for quad in q {
        quad.pos[0] += dx;
        quad.pos[1] += dy;
        quad.color[3] *= alpha;
    }
}
