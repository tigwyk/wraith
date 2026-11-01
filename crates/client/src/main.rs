#![windows_subsystem = "windows"]
use std::io::Write;

use wraith_client::gpu::Gpu;
use wraith_client::window::Window;

fn main() {
    if std::env::args().any(|a| a == "--smoke-test") {
        std::process::exit(match smoke_test() {
            Ok(()) => 0,
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "smoke test failed: {e}");
                1
            }
        });
    }
    if let Err(e) = wraith_client::app::run() {
        let _ = writeln!(std::io::stderr(), "wraith: {e}");
        std::process::exit(1);
    }
}

/// Open a window, present one cleared frame, exit. Used by CI to prove the GPU path works.
fn smoke_test() -> windows::core::Result<()> {
    let win = Window::create("wraith smoke", 320, 200, false)?;
    let gpu = Gpu::new()?;
    let (w, h) = win.client_size();
    let sc = gpu.create_swapchain(win.hwnd(), w, h)?;
    sc.wait();
    gpu.clear(sc.rtv(), [0.1, 0.1, 0.1, 1.0]);
    sc.present()?;
    let _ = writeln!(std::io::stdout(), "presented {w}x{h}");
    Ok(())
}
