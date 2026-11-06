#![cfg(windows)]
use wraith_client::gpu::Gpu;

#[test]
fn offscreen_clear_reads_back() {
    let gpu = Gpu::new().unwrap();
    let off = gpu.offscreen(4, 4).unwrap();
    gpu.clear(off.rtv(), [10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0, 1.0]);
    assert_eq!(off.read_pixel(&gpu, 0, 0), [10, 20, 30, 255]);
    assert_eq!(off.read_pixel(&gpu, 3, 3), [10, 20, 30, 255]);
}

/// A failed ResizeBuffers (Explorer restarting re-lays out maximized windows) must leave a usable
/// render target: the window drew from an empty one and aborted.
#[test]
fn failed_resize_keeps_a_render_target() {
    let win = wraith_client::window::Window::create("wraith resize test", 320, 200).unwrap();
    let gpu = wraith_client::gpu::Gpu::new().unwrap();
    let mut sc = gpu.create_swapchain(win.hwnd(), 320, 200, false).unwrap();
    assert!(sc.resize(100_000, 100_000).is_err(), "over the texture size limit");
    let rtv = sc.rtv().expect("still a render target");
    gpu.clear(rtv, [0.0; 4]);
    assert!(sc.resize(400, 300).is_ok() && sc.rtv().is_some());
}
