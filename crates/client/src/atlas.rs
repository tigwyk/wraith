//! Glyph atlas: DirectWrite rasterization into one RGBA texture, shelf-packed.
//! Mono glyphs store coverage as (255,255,255,a); color glyphs store premultiplied RGBA.
use std::collections::HashMap;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11ShaderResourceView,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_GLYPH_RUN, DWRITE_GRID_FIT_MODE_DEFAULT, DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE, DWRITE_TEXTURE_ALIASED_1x1, IDWriteFontFace,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::Result;

use crate::font::{FaceId, FontStack};
use crate::font_table::FontId;
use crate::gpu::Gpu;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: FontId,
    pub face: FaceId,
    pub glyph: u16,
    /// Subpixel x offset in quarter pixels (0..4).
    pub subpx: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasEntry {
    /// u0, v0, u1, v1
    pub uv: [f32; 4],
    pub w: u16,
    pub h: u16,
    /// Offset of the bitmap's top-left from the pen position on the baseline.
    pub bearing: (i16, i16),
    pub color: bool,
}

pub struct GlyphAtlas {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    size: u32,
    map: HashMap<GlyphKey, AtlasEntry>,
    x: u32,
    y: u32,
    row_h: u32,
    /// Bumped whenever the atlas is cleared; cached quads from an older generation are stale.
    pub generation: u64,
}

const PAD: u32 = 1;

impl GlyphAtlas {
    pub fn new(gpu: &Gpu, size: u32) -> Result<GlyphAtlas> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size,
            Height: size,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let mut tex = None;
        let mut srv = None;
        unsafe {
            gpu.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
            gpu.device.CreateShaderResourceView(tex.as_ref().unwrap(), None, Some(&mut srv))?;
        }
        Ok(GlyphAtlas { tex: tex.unwrap(), srv: srv.unwrap(), size, map: HashMap::new(), x: PAD, y: PAD, row_h: 0, generation: 0 })
    }

    pub fn srv(&self) -> &ID3D11ShaderResourceView {
        &self.srv
    }

    pub fn fill_ratio(&self) -> f32 {
        (self.y + self.row_h) as f32 / self.size as f32
    }

    pub fn get(&mut self, gpu: &Gpu, fonts: &FontStack, key: GlyphKey) -> AtlasEntry {
        if let Some(e) = self.map.get(&key) {
            return *e;
        }
        // Not cached: the face may be added later, and this key must then rasterize.
        let Some((face, color)) = fonts.face(key.face) else { return AtlasEntry { uv: [0.0; 4], w: 0, h: 0, bearing: (0, 0), color: false } };
        let bmp = rasterize(fonts, &face, color, key).unwrap_or_default();
        let e = self.insert(gpu, &bmp);
        self.map.insert(key, e);
        e
    }

    fn insert(&mut self, gpu: &Gpu, b: &Bitmap) -> AtlasEntry {
        let (w, h) = (b.w.min(self.size - 2 * PAD), b.h.min(self.size - 2 * PAD));
        if w == 0 || h == 0 {
            return AtlasEntry { uv: [0.0; 4], w: 0, h: 0, bearing: (0, 0), color: b.color };
        }
        if self.x + w + PAD > self.size {
            self.x = PAD;
            self.y += self.row_h + PAD;
            self.row_h = 0;
        }
        if self.y + h + PAD > self.size {
            // shortcut: clear instead of growing; a 2048² atlas holds ~10k glyphs at 12pt. Grow if users hit resets.
            self.map.clear();
            self.x = PAD;
            self.y = PAD;
            self.row_h = 0;
            self.generation += 1;
        }
        let (x, y) = (self.x, self.y);
        let dst = D3D11_BOX { left: x, top: y, front: 0, right: x + w, bottom: y + h, back: 1 };
        unsafe {
            gpu.ctx.UpdateSubresource(&self.tex, 0, Some(&dst), b.rgba.as_ptr().cast(), b.w * 4, 0);
        }
        self.x += w + PAD;
        self.row_h = self.row_h.max(h);
        let s = self.size as f32;
        AtlasEntry {
            uv: [x as f32 / s, y as f32 / s, (x + w) as f32 / s, (y + h) as f32 / s],
            w: w as u16,
            h: h as u16,
            bearing: (b.left as i16, b.top as i16),
            color: b.color,
        }
    }
}

#[derive(Default)]
struct Bitmap {
    w: u32,
    h: u32,
    left: i32,
    top: i32,
    rgba: Vec<u8>,
    color: bool,
}

fn run<'a>(face: &'a IDWriteFontFace, em: f32, glyph: &'a u16, advance: &'a f32) -> DWRITE_GLYPH_RUN {
    DWRITE_GLYPH_RUN {
        fontFace: std::mem::ManuallyDrop::new(Some(face.clone())),
        fontEmSize: em,
        glyphCount: 1,
        glyphIndices: glyph,
        glyphAdvances: advance,
        glyphOffsets: std::ptr::null(),
        isSideways: false.into(),
        bidiLevel: 0,
    }
}

/// Coverage of one glyph run, with its bounds relative to the pen position.
fn coverage(fonts: &FontStack, r: &DWRITE_GLYPH_RUN, x: f32) -> Option<(RECT, Vec<u8>)> {
    unsafe {
        let a = fonts
            .factory()
            .CreateGlyphRunAnalysis(
                r,
                None,
                DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                x,
                0.0,
            )
            .ok()?;
        let rect = a.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1).ok()?;
        let (w, h) = ((rect.right - rect.left).max(0) as usize, (rect.bottom - rect.top).max(0) as usize);
        let mut buf = vec![0u8; w * h];
        if w > 0 && h > 0 {
            a.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &rect, &mut buf).ok()?;
        }
        Some((rect, buf))
    }
}

fn rasterize(fonts: &FontStack, face: &IDWriteFontFace, color: bool, key: GlyphKey) -> Option<Bitmap> {
    let em = fonts.em_px();
    let advance = 0.0f32;
    let x = f32::from(key.subpx) / 4.0;
    let base = run(face, em, &key.glyph, &advance);
    if color && let Some(b) = rasterize_color(fonts, &base, x) {
        drop_run(base);
        return Some(b);
    }
    let (rect, cov) = coverage(fonts, &base, x)?;
    drop_run(base);
    let rgba = cov.iter().flat_map(|&a| [255, 255, 255, a]).collect();
    Some(Bitmap {
        w: (rect.right - rect.left) as u32,
        h: (rect.bottom - rect.top) as u32,
        left: rect.left,
        top: rect.top,
        rgba,
        color: false,
    })
}

/// COLR layers composited (premultiplied) into one RGBA bitmap. None if the glyph has no color layers.
fn rasterize_color(fonts: &FontStack, base: &DWRITE_GLYPH_RUN, x: f32) -> Option<Bitmap> {
    let layers = unsafe {
        fonts.factory().TranslateColorGlyphRun(x, 0.0, base, None, DWRITE_MEASURING_MODE_NATURAL, None, 0).ok()?
    };
    let mut parts = Vec::new();
    unsafe {
        while layers.MoveNext().ok()?.as_bool() {
            let l = &*layers.GetCurrentRun().ok()?;
            // 0xFFFF: draw in the text color; white keeps it neutral inside an emoji.
            let c = if l.paletteIndex == 0xFFFF { [1.0, 1.0, 1.0, 1.0] } else { [l.runColor.r, l.runColor.g, l.runColor.b, l.runColor.a] };
            let (rect, cov) = coverage(fonts, &l.glyphRun, l.baselineOriginX)?;
            parts.push((rect, cov, c));
        }
    }
    let left = parts.iter().map(|p| p.0.left).min()?;
    let top = parts.iter().map(|p| p.0.top).min()?;
    let right = parts.iter().map(|p| p.0.right).max()?;
    let bottom = parts.iter().map(|p| p.0.bottom).max()?;
    let (w, h) = ((right - left).max(0) as usize, (bottom - top).max(0) as usize);
    let mut rgba = vec![0f32; w * h * 4];
    for (rect, cov, c) in &parts {
        let lw = (rect.right - rect.left) as usize;
        for (i, &a) in cov.iter().enumerate() {
            let (px, py) = ((rect.left - left) as usize + i % lw, (rect.top - top) as usize + i / lw);
            let a = f32::from(a) / 255.0 * c[3];
            let d = &mut rgba[(py * w + px) * 4..][..4];
            // Source-over with premultiplied alpha.
            for k in 0..3 {
                d[k] = c[k] * a + d[k] * (1.0 - a);
            }
            d[3] = a + d[3] * (1.0 - a);
        }
    }
    Some(Bitmap {
        w: w as u32,
        h: h as u32,
        left,
        top,
        rgba: rgba.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect(),
        color: true,
    })
}

/// DWRITE_GLYPH_RUN holds a ManuallyDrop face reference; release it.
fn drop_run(mut r: DWRITE_GLYPH_RUN) {
    unsafe { std::mem::ManuallyDrop::drop(&mut r.fontFace) };
}
