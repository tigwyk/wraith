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
    // `--register [<shortcut path>]`: (re)create the Start Menu shortcut with the app id, then exit.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--register") {
        let lnk = args.get(i + 1).map(std::path::PathBuf::from).or_else(wraith_client::notify::default_shortcut);
        let ok = lnk.is_some_and(|l| {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            }
            wraith_client::notify::register_shortcut(&l).inspect_err(|e| { let _ = writeln!(std::io::stderr(), "register: {e}"); }).is_ok()
        });
        std::process::exit(if ok { 0 } else { 1 });
    }
    if std::env::args().any(|a| a == "--server") {
        let _log = init_logging("server");
        let pipe = wraith_ipc::pipe_name();
        tracing::info!("server starting on {pipe}");
        match wraith_server::session::run_server(&pipe) {
            Ok(()) => tracing::info!("server stopped"),
            // Another launch won the race to start the server; that one serves everyone.
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(e) => {
                tracing::error!("server: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let quick = std::env::args().any(|a| a == "--quick-terminal");
    let _log = init_logging(if quick { "quick" } else { "client" });
    if let Err(e) = wraith_client::app::run(quick) {
        tracing::error!("{e}");
        let _ = writeln!(std::io::stderr(), "wraith: {e}");
        std::process::exit(1);
    }
}

/// Daily files `%LOCALAPPDATA%\wraith\logs\<prefix>.YYYY-MM-DD.log`, 7 kept; level from `WRAITH_LOG` (default info).
fn init_logging(prefix: &'static str) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("wraith").join("logs");
    wraith_client::crash::install(dir.clone(), prefix);
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(prefix)
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
    let win = Window::create("wraith smoke", 320, 200)?;
    let gpu = Gpu::new()?;
    let (w, h) = win.client_size();
    let sc = gpu.create_swapchain(win.hwnd(), w, h, false)?;
    sc.wait();
    gpu.clear(sc.rtv().ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?, [0.1, 0.1, 0.1, 1.0]);
    sc.present()?;
    let _ = writeln!(std::io::stdout(), "presented {w}x{h}");
    Ok(())
}
