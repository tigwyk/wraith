#![cfg(windows)]
//! A TUI repainting the screen on every token must not have its output held back by the UI thread
//! waiting on vsync (spec §6.3). Measured from the real binary's debug log.
use std::time::Duration;

#[test]
fn ui_thread_never_holds_pane_output() {
    let dir = std::env::temp_dir().join(format!("wraith-latency-{}", std::process::id()));
    let pipe = format!(r"\\.\pipe\wraith-latency-test-{}", std::process::id());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 300 frames of 40 rows, one write every ~15 ms, like a TUI redrawing per streamed token.
    let script = "$e=[char]27; $o=[Console]::Out; for($i=0;$i -lt 300;$i++){                   $f=-join (0..39 | % { \"$e[3$($_%7+1)m row $_ frame $i the quick brown fox$e[0m$e[K`r`n\" });                   $o.Write(\"$e[H$f\"); $o.Flush(); Start-Sleep -Milliseconds 15 }";
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_wraith"))
        .args(["-e", "powershell.exe", "-NoProfile", "-Command", script])
        .env("LOCALAPPDATA", &dir)
        .env("APPDATA", &dir) // no user config
        .env("WRAITH_LOG", "wraith_server::pane=debug")
        .env("WRAITH_PIPE_NAME", &pipe) // never touch the user's real server
        .env_remove("NO_COLOR")
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(10));
    let _ = child.kill();
    let _ = child.wait();
    let _ = wraith_client::remote::kill_server(&pipe); // the client started it; don't leave it running
    std::thread::sleep(Duration::from_millis(300));

    let logs = dir.join("wraith").join("logs");
    let text: String = std::fs::read_dir(&logs)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap_or_default())
        .collect();
    let mut held: Vec<u64> = text
        .lines()
        .filter(|l| l.contains("frame sent"))
        .filter_map(|l| l.split("held_us=").nth(1)?.split_whitespace().next()?.parse().ok())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(held.len() >= 5, "only {} frames logged", held.len());
    held.sort_unstable();
    let p50 = held[held.len() / 2];
    eprintln!("frames {}, median held {p50} us", held.len());
    assert!(p50 < 1_000, "pane output waited a median {p50} us for the UI to accept a frame");
}
