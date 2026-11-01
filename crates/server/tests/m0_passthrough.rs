//! M0: which sequences survive ConPTY. Results feed docs/m0-findings.md.
#![cfg(windows)]
use std::io::Read;
use std::time::{Duration, Instant};

use wraith_server::pty::Pty;
use wraith_server::testseq::SEQS;

/// Pty output for `cmdline` until `want` appears, EOF, or 20 s (WSL cold start is slow).
fn capture(cmdline: &str, want: &[u8]) -> Vec<u8> {
    let mut p = Pty::spawn(cmdline, None, &[], 120, 30).unwrap();
    let mut r = p.take_reader();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut acc = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(c) => acc.extend(c),
            Err(_) => break,
        }
        if acc.windows(want.len()).any(|w| w == want) {
            break;
        }
    }
    acc
}

fn check(name: &str, cmdline: &str) {
    let want = SEQS.iter().find(|s| s.0 == name).unwrap().2;
    let got = capture(cmdline, want);
    let found = got.windows(want.len()).any(|w| w == want);
    eprintln!("[{name}] {cmdline}\n  found={found}\n  raw={}", got.escape_ascii());
    assert!(found, "{name}: payload did not survive ConPTY");
}

fn native(name: &str) {
    check(name, &format!("\"{}\" {name}", env!("CARGO_BIN_EXE_emit-seq")));
}

fn wsl(name: &str) {
    let bytes = SEQS.iter().find(|s| s.0 == name).unwrap().1;
    let octal: String = bytes.iter().map(|b| format!("\\{b:03o}")).collect();
    check(name, &format!("wsl.exe -e printf \"{octal}\""));
}

#[test] fn passthrough_kitty() { native("kitty") }
#[test] fn passthrough_osc9() { native("osc9") }
#[test] fn passthrough_osc777() { native("osc777") }
#[test] fn passthrough_osc133() { native("osc133") }
#[test] fn passthrough_osc7() { native("osc7") }
#[test] #[ignore = "needs WSL"] fn passthrough_kitty_wsl() { wsl("kitty") }
#[test] #[ignore = "needs WSL"] fn passthrough_osc9_wsl() { wsl("osc9") }
#[test] #[ignore = "needs WSL"] fn passthrough_osc777_wsl() { wsl("osc777") }
#[test] #[ignore = "needs WSL"] fn passthrough_osc133_wsl() { wsl("osc133") }
#[test] #[ignore = "needs WSL"] fn passthrough_osc7_wsl() { wsl("osc7") }
