//! D3D11 device, flip-model swap chain, and an offscreen target for pixel tests.
use windows::Win32::Foundation::{HANDLE, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIDevice, IDXGIFactory2, IDXGISwapChain2,
};
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::core::{Interface, Result};

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
}

impl Gpu {
    /// Hardware device, falling back to WARP (e.g. RDP sessions without a GPU).
    pub fn new() -> Result<Gpu> {
        Self::create(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| Self::create(D3D_DRIVER_TYPE_WARP))
    }

    fn create(driver: D3D_DRIVER_TYPE) -> Result<Gpu> {
        let mut device = None;
        let mut ctx = None;
        unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )?;
        }
        Ok(Gpu { device: device.unwrap(), ctx: ctx.unwrap() })
    }

    pub fn clear(&self, rtv: &ID3D11RenderTargetView, rgba: [f32; 4]) {
        unsafe { self.ctx.ClearRenderTargetView(rtv, &rgba) };
    }

    pub fn offscreen(&self, w: u32, h: u32) -> Result<Offscreen> {
        let tex = self.texture(w, h, DXGI_FORMAT_R8G8B8A8_UNORM, true)?;
        let mut rtv = None;
        unsafe { self.device.CreateRenderTargetView(&tex, None, Some(&mut rtv))? };
        Ok(Offscreen { tex, rtv: rtv.unwrap(), w, h })
    }

    fn texture(&self, w: u32, h: u32, format: DXGI_FORMAT, render_target: bool) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | if render_target { D3D11_BIND_RENDER_TARGET.0 } else { 0 }) as u32,
            ..Default::default()
        };
        let mut tex = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
        Ok(tex.unwrap())
    }

    pub fn create_swapchain(&self, hwnd: HWND, w: u32, h: u32) -> Result<SwapChain> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w.max(1),
            Height: h.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            // A stale frame during resize shows at 1:1 instead of being stretched to the new size.
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            ..Default::default()
        };
        unsafe {
            let dxgi: IDXGIDevice = self.device.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let sc: IDXGISwapChain2 = factory.CreateSwapChainForHwnd(&self.device, hwnd, &desc, None, None)?.cast()?;
            // One queued frame: input-to-photon latency over throughput.
            sc.SetMaximumFrameLatency(1)?;
            let waitable = sc.GetFrameLatencyWaitableObject();
            let mut s = SwapChain { sc, waitable, rtv: None, device: self.device.clone() };
            s.make_rtv()?;
            Ok(s)
        }
    }
}

pub struct Offscreen {
    tex: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    w: u32,
    h: u32,
}

impl Offscreen {
    pub fn rtv(&self) -> &ID3D11RenderTargetView {
        &self.rtv
    }

    /// RGBA of pixel (x, y). Test-only path: copies the whole texture to a staging buffer.
    pub fn read_pixel(&self, gpu: &Gpu, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.w && y < self.h);
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            self.tex.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut staging)).unwrap();
            let staging = staging.unwrap();
            gpu.ctx.CopyResource(&staging, &self.tex);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).unwrap();
            let p = (m.pData as *const u8).add((y * m.RowPitch + x * 4) as usize);
            let px = [*p, *p.add(1), *p.add(2), *p.add(3)];
            gpu.ctx.Unmap(&staging, 0);
            px
        }
    }
}

pub struct SwapChain {
    sc: IDXGISwapChain2,
    waitable: HANDLE,
    rtv: Option<ID3D11RenderTargetView>,
    device: ID3D11Device,
}

impl SwapChain {
    fn make_rtv(&mut self) -> Result<()> {
        unsafe {
            let back: ID3D11Texture2D = self.sc.GetBuffer(0)?;
            let mut rtv = None;
            self.device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
            self.rtv = rtv;
        }
        Ok(())
    }

    pub fn rtv(&self) -> &ID3D11RenderTargetView {
        self.rtv.as_ref().unwrap()
    }

    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        self.rtv = None;
        unsafe {
            self.sc.ResizeBuffers(
                0,
                w.max(1),
                h.max(1),
                DXGI_FORMAT::default(),
                DXGI_SWAP_CHAIN_FLAG(DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0),
            )?;
        }
        self.make_rtv()
    }

    /// Block until the compositor can take another frame (at most ~1 refresh).
    pub fn wait(&self) {
        unsafe { WaitForSingleObjectEx(self.waitable, 100, true) };
    }

    pub fn present(&self) -> Result<()> {
        unsafe { self.sc.Present(1, DXGI_PRESENT(0)).ok() }
    }
}
