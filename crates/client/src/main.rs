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
    let _log = init_logging();
    if let Err(e) = wraith_client::app::run() {
        tracing::error!("{e}");
        let _ = writeln!(std::io::stderr(), "wraith: {e}");
        std::process::exit(1);
    }
}

/// Daily files `%LOCALAPPDATA%\wraith\logs\client.YYYY-MM-DD.log`, 7 kept; level from `WRAITH_LOG` (default info).
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("wraith").join("logs");
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("client")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_env("WRAITH_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_writer(writer).with_ansi(false).with_env_filter(filter).init();
    Some(guard)
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
