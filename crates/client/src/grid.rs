//! Grid renderer: caches shaped rows from `FrameDelta`s, draws with one instanced quad pipeline.
//! Only dirty rows are re-shaped (spec §6.3); drawing is O(visible cells).
use std::cell::RefCell;
use std::rc::Rc;

use unicode_width::UnicodeWidthChar;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE,
    D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_INSTANCE_DATA,
    D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT, ID3D11BlendState, ID3D11Buffer,
    ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT};
use windows::core::{PCSTR, Result, s};
use wraith_proto::model::Rect;
use wraith_proto::wire::PaneId;
use wraith_proto::{Color, CursorShape, CursorState, FrameDelta, ImagePlacement, SearchView, SelRange, StyleFlags};

use crate::atlas::{GlyphAtlas, GlyphKey};
use crate::font::{CellMetrics, FaceId, FontStack};
use crate::gpu::Gpu;
use crate::images::{ImageStore, image_quad};

/// One-line overlays drawn over the grid in inverted colors.
#[derive(Clone, Copy)]
pub enum Overlay {
    /// Bottom-left: "[process exited N] ..."
    Exit = 0,
    /// Bottom-right: config diagnostics and similar notices.
    Toast = 1,
    /// Top-right: frame-time overlay.
    Stats = 2,
}

pub struct DrawOpts {
    pub selection: Option<SelRange>,
    pub selection_color: (u8, u8, u8),
    pub cursor_blink_on: bool,
    pub focused: bool,
    pub opacity: f32,
}

type Rgb = (u8, u8, u8);

/// Search match backgrounds: every match, and the one Enter/Shift+Enter selected.
pub const SEARCH_HIT: Rgb = (0x5c, 0x4a, 0x12);
pub const SEARCH_SELECTED: Rgb = (0xb0, 0x7a, 0x00);

/// One instanced rectangle: a solid fill (`kind` 0), a monochrome glyph (1) or a color glyph (2).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Quad {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub uv: [f32; 4],
    /// Straight RGBA.
    pub color: [f32; 4],
    pub kind: u32,
}

/// Quads drawn together, optionally sampling their own texture (inline images).
#[derive(Default)]
pub struct Batch {
    pub quads: Vec<Quad>,
    pub texture: Option<ID3D11ShaderResourceView>,
}

#[derive(Clone, Default)]
struct CachedRow {
    /// (first cell, cell count, color)
    bgs: Vec<(u16, u16, Rgb)>,
    glyphs: Vec<CachedGlyph>,
    /// (first cell, cell count, flags, color) for underline/strike.
    decos: Vec<(u16, u16, u16, Rgb)>,
    /// (first cell, cell count) inside the terminal selection.
    sels: Vec<(u16, u16)>,
    /// Block/box characters drawn as exact rectangles: (cell, char, color, alpha).
    sprites: Vec<(u16, char, Rgb, f32)>,
}

#[derive(Clone)]
struct CachedGlyph {
    cell: u16,
    face: FaceId,
    glyph: u16,
    color: Rgb,
    alpha: f32,
}

pub struct GridRenderer {
    pipe: Rc<Pipeline>,
    rows: Vec<CachedRow>,
    cols: u16,
    fg: Rgb,
    bg: Rgb,
    cursor_color: Option<Rgb>,
    palette: Vec<Rgb>,
    cursor: Option<CursorState>,
    quads: Vec<Quad>,
    /// (cells wide, shaped row) per `Overlay`.
    overlays: [Option<(u16, CachedRow)>; 3],
    /// Image placements of the latest frame (each frame carries all of them).
    images: Vec<ImagePlacement>,
    /// The pane's open search: its count and on-screen matches (latest frame).
    search: Option<SearchView>,
}

/// Shaders and state shared by every grid and the chrome of one window.
pub struct Pipeline {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    sampler: ID3D11SamplerState,
    cbuf: ID3D11Buffer,
    inst: RefCell<Option<(ID3D11Buffer, usize)>>,
}

fn compile(src: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut blob = None;
    let mut errors = None;
    let r = unsafe { D3DCompile(src.as_ptr().cast(), src.len(), None, None, None, entry, target, 0, 0, &mut blob, Some(&mut errors)) };
    if let Err(e) = r {
        let msg = errors.map(|b| unsafe {
            String::from_utf8_lossy(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())).into_owned()
        });
        return Err(windows::core::Error::new(e.code(), msg.unwrap_or_default()));
    }
    Ok(blob.unwrap())
}

fn bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

fn f(c: Rgb, a: f32) -> [f32; 4] {
    [f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0, a]
}

impl Pipeline {
    pub fn new(gpu: &Gpu) -> Result<Pipeline> {
        let src = include_str!("shaders.hlsl");
        let vsb = compile(src, s!("vs"), s!("vs_5_0"))?;
        let psb = compile(src, s!("ps"), s!("ps_5_0"))?;
        let d = &gpu.device;
        let el = |name: PCSTR, format, offset| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: name,
            SemanticIndex: 0,
            Format: format,
            InputSlot: 0,
            AlignedByteOffset: offset,
            InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
            InstanceDataStepRate: 1,
        };
        let elements = [
            el(s!("POS"), DXGI_FORMAT_R32G32_FLOAT, 0),
            el(s!("SIZE"), DXGI_FORMAT_R32G32_FLOAT, 8),
            el(s!("UV"), DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
            el(s!("COLOR"), DXGI_FORMAT_R32G32B32A32_FLOAT, 32),
            el(s!("KIND"), DXGI_FORMAT_R32_UINT, 48),
        ];
        let (mut vs, mut ps, mut layout, mut blend, mut raster, mut sampler, mut cbuf) = (None, None, None, None, None, None, None);
        unsafe {
            d.CreateVertexShader(bytes(&vsb), None, Some(&mut vs))?;
            d.CreatePixelShader(bytes(&psb), None, Some(&mut ps))?;
            d.CreateInputLayout(&elements, bytes(&vsb), Some(&mut layout))?;
            let mut bd = D3D11_BLEND_DESC::default();
            bd.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            d.CreateBlendState(&bd, Some(&mut blend))?;
            let rd = D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, DepthClipEnable: true.into(), ..Default::default() };
            d.CreateRasterizerState(&rd, Some(&mut raster))?;
            let sd = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            d.CreateSamplerState(&sd, Some(&mut sampler))?;
            let cd = D3D11_BUFFER_DESC {
                ByteWidth: 16,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                ..Default::default()
            };
            d.CreateBuffer(&cd, None, Some(&mut cbuf))?;
        }
        Ok(Pipeline {
            vs: vs.unwrap(),
            ps: ps.unwrap(),
            layout: layout.unwrap(),
            blend: blend.unwrap(),
            raster: raster.unwrap(),
            sampler: sampler.unwrap(),
            cbuf: cbuf.unwrap(),
            inst: RefCell::new(None),
        })
    }

    /// Clear `target` to `clear` (premultiplied), then draw every batch in order in one pass.
    pub fn submit(&self, gpu: &Gpu, target: &ID3D11RenderTargetView, size: (u32, u32), atlas: &GlyphAtlas, batches: &[Batch], clear: [f32; 4]) {
        let ctx = &gpu.ctx;
        gpu.clear(target, clear);
        let total: usize = batches.iter().map(|b| b.quads.len()).sum();
        if total == 0 {
            return;
        }
        unsafe {
            let mut inst = self.inst.borrow_mut();
            if inst.as_ref().is_none_or(|(_, cap)| *cap < total) {
                let cap = total.next_power_of_two().max(1024);
                let desc = D3D11_BUFFER_DESC {
                    ByteWidth: (cap * size_of::<Quad>()) as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                };
                let mut b = None;
                if gpu.device.CreateBuffer(&desc, None, Some(&mut b)).is_err() {
                    return;
                }
                *inst = Some((b.unwrap(), cap));
            }
            let (buf, _) = inst.as_ref().unwrap();
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            if ctx.Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m)).is_err() {
                return;
            }
            let mut at = 0;
            for b in batches {
                std::ptr::copy_nonoverlapping(b.quads.as_ptr(), (m.pData as *mut Quad).add(at), b.quads.len());
                at += b.quads.len();
            }
            ctx.Unmap(buf, 0);
            let mut cm = D3D11_MAPPED_SUBRESOURCE::default();
            if ctx.Map(&self.cbuf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut cm)).is_ok() {
                *(cm.pData as *mut [f32; 4]) = [size.0 as f32, size.1 as f32, 0.0, 0.0];
                ctx.Unmap(&self.cbuf, 0);
            }
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: size.0 as f32, Height: size.1 as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            ctx.RSSetState(&self.raster);
            ctx.OMSetBlendState(&self.blend, None, 0xffff_ffff);
            ctx.IASetInputLayout(&self.layout);
            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            let stride = size_of::<Quad>() as u32;
            let offset = 0u32;
            ctx.IASetVertexBuffers(0, 1, Some(&Some(buf.clone())), Some(&stride), Some(&offset));
            ctx.VSSetShader(&self.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let mut first = 0u32;
            for b in batches {
                if b.quads.is_empty() {
                    continue;
                }
                ctx.PSSetShaderResources(0, Some(&[Some(atlas.srv().clone()), b.texture.clone()]));
                ctx.DrawInstanced(4, b.quads.len() as u32, 0, first);
                first += b.quads.len() as u32;
            }
        }
    }
}

impl GridRenderer {
    pub fn new(gpu: &Gpu) -> Result<GridRenderer> {
        Ok(GridRenderer::with_pipeline(Rc::new(Pipeline::new(gpu)?)))
    }

    /// A grid drawing through a pipeline shared with other grids and the window chrome.
    pub fn with_pipeline(pipe: Rc<Pipeline>) -> GridRenderer {
        GridRenderer {
            pipe,
            rows: Vec::new(),
            cols: 0,
            fg: (255, 255, 255),
            bg: (0, 0, 0),
            cursor_color: None,
            palette: Vec::new(),
            cursor: None,
            quads: Vec::new(),
            overlays: [None, None, None],
            images: Vec::new(),
            search: None,
        }
    }

    pub fn pipeline(&self) -> &Rc<Pipeline> {
        &self.pipe
    }

    /// The pane's default background.
    pub fn background(&self) -> (u8, u8, u8) {
        self.bg
    }

    /// The open search's count and selection, for the find bar.
    pub fn search(&self) -> Option<&SearchView> {
        self.search.as_ref()
    }

    /// Some cell on screen is selected (the selection lives in the pane; frames mark its cells).
    pub fn has_selection(&self) -> bool {
        self.rows.iter().any(|r| !r.sels.is_empty())
    }

    pub fn cursor_pos(&self) -> Option<(u16, u16)> {
        self.cursor.filter(|c| c.visible).map(|c| (c.x, c.y))
    }

    pub fn set_overlay(&mut self, kind: Overlay, text: Option<&str>, fonts: &FontStack) {
        self.overlays[kind as usize] = text.map(|t| {
            let width: u16 = t.chars().map(|c| c.width().unwrap_or(0) as u16).sum();
            let glyphs = fonts
                .shape(t, StyleFlags(0))
                .into_iter()
                .map(|g| CachedGlyph { cell: g.cell, face: g.face, glyph: g.glyph, color: (0, 0, 0), alpha: 1.0 })
                .collect();
            (width, CachedRow { glyphs, ..CachedRow::default() })
        });
    }

    pub fn grid_size(&self) -> (u16, u16) {
        (self.cols, self.rows.len() as u16)
    }

    /// Fold a frame into the row cache, re-shaping only the rows it carries.
    pub fn apply(&mut self, frame: &FrameDelta, fonts: &FontStack) {
        if frame.full {
            self.rows = vec![CachedRow::default(); frame.rows as usize];
            self.cols = frame.cols;
            if !frame.colors.palette.is_empty() {
                self.palette = frame.colors.palette.clone();
            }
        }
        self.fg = frame.colors.fg;
        self.bg = frame.colors.bg;
        self.cursor_color = frame.colors.cursor;
        self.cursor = Some(frame.cursor);
        self.images.clone_from(&frame.images);
        self.search.clone_from(&frame.search);
        for row in &frame.dirty_rows {
            let Some(slot) = self.rows.get_mut(row.y as usize) else { continue };
            let mut cached = CachedRow::default();
            let mut cell: u16 = 0;
            for run in &row.runs {
                let width: u16 = run.text.chars().map(|c| c.width().unwrap_or(0) as u16).sum();
                let inverse = run.flags.has(StyleFlags::INVERSE);
                let mut fg = resolve(run.fg, self.fg, &self.palette);
                let mut bg = resolve(run.bg, self.bg, &self.palette);
                if inverse {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if inverse || run.bg != Color::Default {
                    cached.bgs.push((cell, width, bg));
                }
                if run.flags.has(StyleFlags::SELECTED) {
                    cached.sels.push((cell, width));
                }
                if run.flags.0 & (StyleFlags::UNDERLINE | StyleFlags::DOUBLE_UNDERLINE | StyleFlags::CURLY_UNDERLINE | StyleFlags::STRIKE) != 0 {
                    cached.decos.push((cell, width, run.flags.0, fg));
                }
                if !run.flags.has(StyleFlags::INVISIBLE) && run.text.chars().any(|c| c != ' ') {
                    let alpha = if run.flags.has(StyleFlags::FAINT) { 0.5 } else { 1.0 };
                    // Sprite characters are drawn as rectangles; the font sees a space in their place.
                    let mut text = String::with_capacity(run.text.len());
                    let mut at = cell;
                    for c in run.text.chars() {
                        if crate::sprite::sprite_rects(c, 2, 2, 1).is_some() {
                            cached.sprites.push((at, c, fg, alpha));
                            text.push(' ');
                        } else {
                            text.push(c);
                        }
                        at += c.width().unwrap_or(0) as u16;
                    }
                    if text.chars().any(|c| c != ' ') {
                        for g in fonts.shape(&text, run.flags) {
                            cached.glyphs.push(CachedGlyph { cell: cell + g.cell, face: g.face, glyph: g.glyph, color: fg, alpha });
                        }
                    }
                }
                cell += width;
            }
            *slot = cached;
        }
    }

    /// Clear `target` to the grid's background and draw just this grid.
    #[allow(clippy::too_many_arguments, reason = "one draw call's full input; grouping adds a type nobody else uses")]
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        target_size: (u32, u32),
        atlas: &mut GlyphAtlas,
        fonts: &FontStack,
        origin: (f32, f32),
        m: &CellMetrics,
        opts: &DrawOpts,
    ) {
        let mut q = std::mem::take(&mut self.quads);
        q.clear();
        self.push_quads(&mut q, gpu, atlas, fonts, origin, m, opts, None);
        let bg = f(self.bg, opts.opacity);
        let clear = [bg[0] * opts.opacity, bg[1] * opts.opacity, bg[2] * opts.opacity, opts.opacity];
        self.pipe.submit(gpu, target, target_size, atlas, &[Batch { quads: std::mem::take(&mut q), texture: None }], clear);
        self.quads = Vec::new();
    }

    /// Append this grid's quads (background, text, cursor, overlays) to `q`. With `fill`, the pane's
    /// whole rectangle is first painted in its background color.
    #[allow(clippy::too_many_arguments, reason = "one draw call's full input; grouping adds a type nobody else uses")]
    pub fn push_quads(
        &mut self,
        q: &mut Vec<Quad>,
        gpu: &Gpu,
        atlas: &mut GlyphAtlas,
        fonts: &FontStack,
        origin: (f32, f32),
        m: &CellMetrics,
        opts: &DrawOpts,
        fill: Option<Rect>,
    ) {
        self.push_bg(q, origin, m, opts, fill);
        self.push_fg(q, gpu, atlas, fonts, origin, m, opts);
    }

    /// Like `push_quads`, with this pane's images layered in: `z < 0` between the backgrounds and the
    /// text, `z >= 0` over the text. Image quads go into their own batches in `out`; `q` is the open
    /// untextured batch, flushed into `out` before each image batch.
    #[allow(clippy::too_many_arguments, reason = "one draw call's full input; grouping adds a type nobody else uses")]
    pub fn push_layers(
        &mut self,
        out: &mut Vec<Batch>,
        q: &mut Vec<Quad>,
        store: &ImageStore,
        pane: PaneId,
        gpu: &Gpu,
        atlas: &mut GlyphAtlas,
        fonts: &FontStack,
        origin: (f32, f32),
        m: &CellMetrics,
        opts: &DrawOpts,
        rect: Rect,
    ) {
        self.push_bg(q, origin, m, opts, Some(rect));
        self.push_images(out, q, store, pane, origin, m, rect, opts.opacity, true);
        self.push_fg(q, gpu, atlas, fonts, origin, m, opts);
        self.push_images(out, q, store, pane, origin, m, rect, opts.opacity, false);
    }

    #[allow(clippy::too_many_arguments, reason = "internal helper of push_layers")]
    fn push_images(&self, out: &mut Vec<Batch>, q: &mut Vec<Quad>, store: &ImageStore, pane: PaneId, origin: (f32, f32), m: &CellMetrics, clip: Rect, alpha: f32, below: bool) {
        for p in self.images.iter().filter(|p| (p.z < 0) == below) {
            let Some((srv, size)) = store.get(pane, p.image_id) else { continue };
            let Some(quad) = image_quad(p, *size, origin, m, clip, alpha) else { continue };
            out.push(Batch { quads: std::mem::take(q), texture: None });
            out.push(Batch { quads: vec![quad], texture: Some(srv.clone()) });
        }
    }

    fn push_bg(&self, q: &mut Vec<Quad>, origin: (f32, f32), m: &CellMetrics, opts: &DrawOpts, fill: Option<Rect>) {
        let (cw, ch) = (m.w as f32, m.h as f32);
        let cell_xy = |x: u16, y: usize| (origin.0 + f32::from(x) * cw, origin.1 + y as f32 * ch);
        let solid = |q: &mut Vec<Quad>, x: f32, y: f32, w: f32, h: f32, c: Rgb| {
            q.push(Quad { pos: [x, y], size: [w, h], uv: [0.0; 4], color: f(c, 1.0), kind: 0 });
        };
        if let Some(r) = fill {
            solid(q, r.x as f32, r.y as f32, r.w as f32, r.h as f32, self.bg);
        }
        for (y, row) in self.rows.iter().enumerate() {
            for &(x, n, c) in &row.bgs {
                let (px, py) = cell_xy(x, y);
                solid(q, px, py, cw * f32::from(n), ch, c);
            }
        }
        for (y, row) in self.rows.iter().enumerate() {
            for &(x, n) in &row.sels {
                let (px, py) = cell_xy(x, y);
                solid(q, px, py, cw * f32::from(n), ch, opts.selection_color);
            }
        }
        for h in self.search.iter().flat_map(|s| &s.hits) {
            let (px, py) = cell_xy(h.x0, h.y as usize);
            solid(q, px, py, cw * f32::from(h.x1.saturating_sub(h.x0) + 1), ch, if h.selected { SEARCH_SELECTED } else { SEARCH_HIT });
        }
        if let Some(sel) = opts.selection {
            for (y, x0, x1) in selection_spans(sel, self.cols) {
                let (px, py) = cell_xy(x0, y as usize);
                solid(q, px, py, cw * f32::from(x1 - x0 + 1), ch, opts.selection_color);
            }
        }
        if let Some((x, y)) = self.block_cursor(opts) {
            let (px, py) = cell_xy(x, y as usize);
            solid(q, px, py, cw, ch, self.cursor_color.unwrap_or(self.fg));
        }
    }

    fn block_cursor(&self, opts: &DrawOpts) -> Option<(u16, u16)> {
        self.cursor.filter(|c| c.visible && opts.cursor_blink_on && opts.focused && c.style == CursorShape::Block).map(|c| (c.x, c.y))
    }

    /// Text, decorations, the non-block cursor and overlays: everything over the backgrounds.
    #[allow(clippy::too_many_arguments, reason = "one draw call's full input; grouping adds a type nobody else uses")]
    fn push_fg(&self, q: &mut Vec<Quad>, gpu: &Gpu, atlas: &mut GlyphAtlas, fonts: &FontStack, origin: (f32, f32), m: &CellMetrics, opts: &DrawOpts) {
        let (cw, ch) = (m.w as f32, m.h as f32);
        let cell_xy = |x: u16, y: usize| (origin.0 + f32::from(x) * cw, origin.1 + y as f32 * ch);
        let solid = |q: &mut Vec<Quad>, x: f32, y: f32, w: f32, h: f32, c: Rgb| {
            q.push(Quad { pos: [x, y], size: [w, h], uv: [0.0; 4], color: f(c, 1.0), kind: 0 });
        };
        let cursor = self.cursor.filter(|c| c.visible && opts.cursor_blink_on);
        let cursor_rgb = self.cursor_color.unwrap_or(self.fg);
        let block_at = self.block_cursor(opts);
        for (y, row) in self.rows.iter().enumerate() {
            for g in &row.glyphs {
                let e = atlas.get(gpu, fonts, GlyphKey { face: g.face, glyph: g.glyph, subpx: 0 });
                if e.w == 0 {
                    continue;
                }
                let (px, py) = cell_xy(g.cell, y);
                let color = if block_at == Some((g.cell, y as u16)) { self.bg } else { g.color };
                q.push(Quad {
                    pos: [px + f32::from(e.bearing.0), py + m.baseline as f32 + f32::from(e.bearing.1)],
                    size: [f32::from(e.w), f32::from(e.h)],
                    uv: e.uv,
                    color: f(color, g.alpha),
                    kind: if e.color { 2 } else { 1 },
                });
            }
            for &(x, c, color, alpha) in &row.sprites {
                let (px, py) = cell_xy(x, y);
                let color = if block_at == Some((x, y as u16)) { self.bg } else { color };
                for r in crate::sprite::sprite_rects(c, m.w, m.h, m.underline_thickness).unwrap_or_default() {
                    q.push(Quad {
                        pos: [px + r.x as f32, py + r.y as f32],
                        size: [r.w as f32, r.h as f32],
                        uv: [0.0; 4],
                        color: f(color, alpha * r.alpha),
                        kind: 0,
                    });
                }
            }
            for &(x, n, flags, c) in &row.decos {
                let (px, py) = cell_xy(x, y);
                let w = cw * f32::from(n);
                let t = m.underline_thickness as f32;
                if flags & (StyleFlags::UNDERLINE | StyleFlags::CURLY_UNDERLINE) != 0 {
                    // shortcut: curly underline drawn straight; a wave needs a dedicated shader kind.
                    solid(q, px, py + m.underline_pos as f32, w, t, c);
                }
                if flags & StyleFlags::DOUBLE_UNDERLINE != 0 {
                    solid(q, px, py + m.underline_pos as f32 - t, w, t, c);
                    solid(q, px, py + m.underline_pos as f32 + t, w, t, c);
                }
                if flags & StyleFlags::STRIKE != 0 {
                    solid(q, px, py + (m.baseline as f32 * 0.65).round(), w, t, c);
                }
            }
        }
        if let Some(c) = cursor.filter(|_| block_at.is_none()) {
            let (px, py) = cell_xy(c.x, c.y as usize);
            let t = (m.w as f32 / 8.0).round().max(1.0);
            match (opts.focused, c.style) {
                (true, CursorShape::Bar) => solid(q, px, py, t * 2.0, ch, cursor_rgb),
                (true, CursorShape::Underline) => solid(q, px, py + ch - t * 2.0, cw, t * 2.0, cursor_rgb),
                _ => {
                    solid(q, px, py, cw, t, cursor_rgb);
                    solid(q, px, py + ch - t, cw, t, cursor_rgb);
                    solid(q, px, py, t, ch, cursor_rgb);
                    solid(q, px + cw - t, py, t, ch, cursor_rgb);
                }
            }
        }
        let rows = self.rows.len() as u16;
        for (i, o) in self.overlays.iter().enumerate() {
            let Some((width, row)) = o else { continue };
            let x = if i == Overlay::Exit as usize { 0 } else { self.cols.saturating_sub(*width) };
            let y = if i == Overlay::Stats as usize { 0 } else { rows.saturating_sub(1) };
            let (px, py) = cell_xy(x, y as usize);
            solid(q, px, py, cw * f32::from(*width), ch, self.fg);
            for g in &row.glyphs {
                let e = atlas.get(gpu, fonts, GlyphKey { face: g.face, glyph: g.glyph, subpx: 0 });
                if e.w == 0 {
                    continue;
                }
                let gx = px + f32::from(g.cell) * cw;
                q.push(Quad {
                    pos: [gx + f32::from(e.bearing.0), py + m.baseline as f32 + f32::from(e.bearing.1)],
                    size: [f32::from(e.w), f32::from(e.h)],
                    uv: e.uv,
                    color: f(self.bg, 1.0),
                    kind: if e.color { 2 } else { 1 },
                });
            }
        }
    }
}

fn resolve(c: Color, default: Rgb, palette: &[Rgb]) -> Rgb {
    match c {
        Color::Default => default,
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Palette(i) => palette.get(i as usize).copied().unwrap_or(default),
    }
}

/// Inclusive (row, first col, last col) spans covered by `sel`.
pub fn selection_spans(sel: SelRange, cols: u16) -> Vec<(u16, u16, u16)> {
    let last = cols.saturating_sub(1);
    let (a, b) = if (sel.start.1, sel.start.0) <= (sel.end.1, sel.end.0) { (sel.start, sel.end) } else { (sel.end, sel.start) };
    (a.1..=b.1)
        .map(|y| {
            if sel.rectangle {
                (y, a.0.min(b.0), a.0.max(b.0).min(last))
            } else {
                let x0 = if y == a.1 { a.0 } else { 0 };
                let x1 = if y == b.1 { b.0 } else { last };
                (y, x0.min(last), x1.min(last))
            }
        })
        .collect()
}
