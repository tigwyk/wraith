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
