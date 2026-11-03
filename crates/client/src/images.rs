//! Inline images (Kitty graphics, spec §3.4): one GPU texture per uploaded image, keyed by pane, and
//! the quad that draws a placement clipped to its pane.
use std::collections::HashMap;

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE, ID3D11ShaderResourceView,
};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::{Error, Result};
use wraith_proto::ImagePlacement;
use wraith_proto::model::Rect;
use wraith_proto::wire::PaneId;

use crate::font::CellMetrics;
use crate::gpu::Gpu;
use crate::grid::Quad;

/// Shader kind for a quad sampling the image texture (straight alpha).
pub const KIND_IMAGE: u32 = 3;

#[derive(Default)]
pub struct ImageStore {
    tex: HashMap<(PaneId, u32), (ID3D11ShaderResourceView, (u32, u32))>,
}

impl ImageStore {
    /// Replace image `id` of `pane` with `w`×`h` RGBA8 pixels.
    pub fn upload(&mut self, gpu: &Gpu, pane: PaneId, id: u32, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
        self.tex.insert((pane, id), (texture(gpu, w, h, rgba)?, (w, h)));
        Ok(())
    }
}

/// An immutable GPU texture of `w`×`h` RGBA8 pixels.
pub fn texture(gpu: &Gpu, w: u32, h: u32, rgba: &[u8]) -> Result<ID3D11ShaderResourceView> {
    if w == 0 || h == 0 || rgba.len() < w as usize * h as usize * 4 {
        return Err(Error::new(E_INVALIDARG, "image data does not match its size"));
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let init = D3D11_SUBRESOURCE_DATA { pSysMem: rgba.as_ptr().cast(), SysMemPitch: w * 4, SysMemSlicePitch: 0 };
    let (mut tex, mut srv) = (None, None);
    unsafe {
        gpu.device.CreateTexture2D(&desc, Some(&init), Some(&mut tex))?;
        gpu.device.CreateShaderResourceView(tex.as_ref().unwrap(), None, Some(&mut srv))?;
    }
    Ok(srv.unwrap())
}

impl ImageStore {
    pub fn delete(&mut self, pane: PaneId, id: u32) {
        self.tex.remove(&(pane, id));
    }

    pub fn drop_pane(&mut self, pane: PaneId) {
        self.retain_panes(|p| p != pane);
    }

    /// Free the textures of every pane `keep` rejects.
    pub fn retain_panes(&mut self, keep: impl Fn(PaneId) -> bool) {
        self.tex.retain(|(p, _), _| keep(*p));
    }

    /// The texture and its pixel size.
    pub fn get(&self, pane: PaneId, id: u32) -> Option<&(ID3D11ShaderResourceView, (u32, u32))> {
        self.tex.get(&(pane, id))
    }
}

/// The quad drawing `p` (an `img`-sized image) for a grid at `origin`, cut to `clip` with the texture
/// coordinates cut to match. `None` when nothing of it is inside `clip`.
pub fn image_quad(p: &ImagePlacement, img: (u32, u32), origin: (f32, f32), m: &CellMetrics, clip: Rect, alpha: f32) -> Option<Quad> {
    let x = origin.0 + p.col as f32 * m.w as f32 + p.x_off as f32;
    let y = origin.1 + p.row as f32 * m.h as f32 + p.y_off as f32;
    let (w, h) = (p.px.0 as f32, p.px.1 as f32);
    let (iw, ih) = (img.0.max(1) as f32, img.1.max(1) as f32);
    let (sx, sy, sw, sh) = (p.src.0 as f32, p.src.1 as f32, p.src.2 as f32, p.src.3 as f32);
    let (x0, y0) = (x.max(clip.x as f32), y.max(clip.y as f32));
    let (x1, y1) = ((x + w).min((clip.x + clip.w) as f32), (y + h).min((clip.y + clip.h) as f32));
    if x1 <= x0 || y1 <= y0 || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let u = |px: f32| (sx + (px - x) / w * sw) / iw;
    let v = |py: f32| (sy + (py - y) / h * sh) / ih;
    Some(Quad { pos: [x0, y0], size: [x1 - x0, y1 - y0], uv: [u(x0), v(y0), u(x1), v(y1)], color: [1.0, 1.0, 1.0, alpha], kind: KIND_IMAGE })
}
