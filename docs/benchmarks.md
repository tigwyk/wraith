# Benchmarks

`scripts\bench.ps1` measures a Wraith build and prints a markdown table. Each part of the run uses
its own pipe, `APPDATA` and `LOCALAPPDATA`, so your own Wraith, config and layout are not touched.
Wraith windows open on screen while it runs, so leave the machine alone for about a minute.

```powershell
cargo build --release -p wraith-client -p wraith-cli
scripts\bench.ps1                     # benches target\release\wraith.exe
scripts\bench.ps1 -Exe other\wraith.exe -MB 64 -Keys 400 -Keep
```

`-Keep` keeps the logs in `%TEMP%\wraith-bench-<pid>` so you can check the numbers.

## Results

Wraith 0.5.0 (release build) · 13th Gen Intel Core i9-13900K · NVIDIA GeForce RTX 5070 Ti ·
Windows 11 · 2026-11-04

| Measure | Result | Notes |
| --- | --- | --- |
| Startup, cold | 338 ms | starts the server, the window and the shell |
| Startup, warm | 21 ms | a second window; the server is already running |
| Throughput | 188 MB/s | 32 MB of colored log lines in 0.17 s, through ConPTY |
| Key to echo, median | 0.4 ms | 200 of 200 keys echoed by cmd.exe; timed to the Present call |
| Key to echo, p99 | 19.3 ms | |
| Memory, window | 79 MB | private bytes, one tab, idle |
| Memory, server | 9 MB | private bytes, one pane, idle |
| Frame build, median | 0.1 ms | 625 presents; CPU time to draw and submit one frame |
| Frame build, p99 | 0.2 ms | |
| Frame apply, median | 0.1 ms | the window applying a frame from the server |
| Output held, median | 0.2 ms | how long the pane waited for the window to take its next frame |

## What each number means

- **Startup**: from the process start to the first frame presented, read from the
  `presented` log line.
- **Throughput**: a PowerShell script in the pane writes the file to its console in 64 KB writes and
  times the writes, as `time cat` does on Unix. ConPTY, the Windows console layer every Windows
  terminal sits behind, is part of this number.
- **Key to echo**: the script posts key messages straight to the window, so there is no global input
  and no focus change. It alternates `a` and Backspace. The time runs from the window taking the key
  to the `Present` call for the first frame from the server after it, which is the echo
  (`key echoed echo_us=` in the client log). The screen shows that frame at its next refresh, so add
  up to one refresh interval (16.7 ms at 60 Hz) for photons. The same measure is in the
  `Ctrl+Shift+F12` overlay.
- **Frame build**: `presented total_us=`, CPU time from the start of drawing to `Present` while a
  full-screen program redraws 40 rows every 15 ms. GPU time is not included.
- **Frame apply**: `frame arrived apply_us=`, the window applying a frame from the server to its grid.
- **Output held**: `frame sent held_us=`, how long the pane's next frame waited for the window to
  accept the last one. This is high when the UI thread falls behind.
- **Memory**: private bytes of the window and server processes after the echo test. The window's
  share is mostly the GPU driver.

## Not measured

- Windows Terminal or other terminals on the same machine; run their equivalents for a comparison.
- Time to photons. That needs a camera or a hardware latency tester.
- Profiles inside a frame. Use Windows Performance Analyzer or Superluminal on a release build for
  that.
