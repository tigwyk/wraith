//! Panics go to `crash.log` synchronously. A panic in the window procedure can't unwind, so the process
//! aborts right after the hook: the non-blocking log writer would lose it, and a GUI app has no stderr.
use std::io::Write;
use std::path::PathBuf;

/// Append every panic (process kind, pid, local time, message with location, backtrace) to
/// `dir\crash.log`, then run the default hook.
pub fn install(dir: PathBuf, kind: &'static str) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let when = format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
        let entry = format!(
            "{when} {kind} pid {} v{}: {info}\n{}\n",
            std::process::id(),
            env!("CARGO_PKG_VERSION"),
            std::backtrace::Backtrace::force_capture()
        );
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
            let _ = f.write_all(entry.as_bytes());
        }
        tracing::error!("{info}");
        default(info);
    }));
}
