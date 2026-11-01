//! M0: ConPTY + Job Object behaviour.
#![cfg(windows)]
use std::io::Read;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use wraith_server::pty::{Pty, clamp_size};

/// Read from `r` on a helper thread until `needle` appears or `timeout` passes.
fn read_until(mut r: std::fs::File, needle: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let mut acc = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(chunk) => {
                acc.extend_from_slice(&chunk);
                if acc.windows(needle.len()).any(|w| w == needle) {
                    return Some(acc);
                }
            }
            Err(_) => break,
        }
    }
    None
}

fn wait_exit(p: &Pty, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(code) = p.try_wait() {
            return Some(code);
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn echo_output_is_readable() {
    let mut p = Pty::spawn("cmd.exe /c echo wraith-hello", None, &[], 80, 24).unwrap();
    assert!(read_until(p.take_reader(), b"wraith-hello", Duration::from_secs(5)).is_some());
}

#[test]
fn exit_code_is_reported() {
    let mut p = Pty::spawn("cmd.exe /c exit 7", None, &[], 80, 24).unwrap();
    // ConPTY blocks the child on output until someone drains it.
    let _r = read_until(p.take_reader(), b"\0never", Duration::from_millis(1));
    assert_eq!(wait_exit(&p, Duration::from_secs(5)), Some(7));
}

#[test]
fn env_is_passed_to_child() {
    let env = [("WRAITH_TEST".to_string(), "env-ok".to_string())];
    let mut p = Pty::spawn("cmd.exe /c echo %WRAITH_TEST%", None, &env, 80, 24).unwrap();
    assert!(read_until(p.take_reader(), b"env-ok", Duration::from_secs(5)).is_some());
}

#[test]
fn cwd_is_applied() {
    let dir = std::env::temp_dir();
    let mut p = Pty::spawn("cmd.exe /c cd", Some(&dir), &[], 200, 24).unwrap();
    let want = dir.to_string_lossy().trim_end_matches('\\').to_string();
    assert!(read_until(p.take_reader(), want.as_bytes(), Duration::from_secs(5)).is_some());
}

#[test]
fn drop_kills_grandchildren() {
    let mut p = Pty::spawn(
        // `start /min` gives the grandchild its own console, so only the job can kill it.
        "cmd.exe /c start \"\" /min ping -n 60 127.0.0.1 & ping -n 60 127.0.0.1",
        None,
        &[],
        80,
        24,
    )
    .unwrap();
    let reader = p.take_reader();
    thread::spawn(move || {
        let _ = read_until(reader, b"\0never", Duration::from_secs(30));
    });
    // Wait for both pings to be in the job.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut pids = p.job_pids();
    while pids.len() < 3 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
        pids = p.job_pids();
    }
    assert!(pids.len() >= 3, "expected cmd + 2 pings in job, got {pids:?}");
    drop(p);
    let deadline = Instant::now() + Duration::from_secs(2);
    while pids.iter().any(|&pid| wraith_server::pty::is_alive(pid)) {
        assert!(Instant::now() < deadline, "descendants survived drop: {pids:?}");
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn resize_clamps_to_two_by_one() {
    assert_eq!(clamp_size(0, 0), (2, 1));
    assert_eq!(clamp_size(1, 5), (2, 5));
    assert_eq!(clamp_size(120, 40), (120, 40));
    let p = Pty::spawn("cmd.exe", None, &[], 80, 24).unwrap();
    p.resize(0, 0).unwrap();
}

#[test]
fn default_backend_is_bundled() {
    use wraith_server::pty::{Backend, default_backend};
    assert!(matches!(default_backend(), Backend::Bundled(_)), "got {:?}", default_backend());
}

#[test]
fn missing_dll_falls_back_to_inbox() {
    use wraith_server::pty::{Backend, load_backend};
    assert!(matches!(load_backend(Some(std::path::Path::new("C:/nope/conpty.dll"))), Backend::Inbox));
}

#[test]
fn inbox_pty_still_works() {
    use wraith_server::pty::Backend;
    let mut p = Pty::spawn_with(&Backend::Inbox, "cmd.exe /c echo inbox-ok", None, &[], 80, 24).unwrap();
    assert!(matches!(p.backend(), Backend::Inbox));
    assert!(read_until(p.take_reader(), b"inbox-ok", Duration::from_secs(5)).is_some());
}
