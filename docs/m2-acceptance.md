# M2 acceptance — server/client split

Date: 2026-11-01 · Build: `target/preview` release, private test pipe (the user's real setup untouched).
Automated: 34 test suites green (`cargo test --workspace`), raw `cargo clippy -D warnings` exit 0.

| Check | Result | Evidence |
|---|---|---|
| Window X detaches; shell keeps running in the server | **Pass** (scripted) | `wr ctl list` → `1 detached running` after WM_CLOSE |
| Relaunch reattaches the same shell and screen | **Pass** (screenshot) | `2026-11-01-m2-2-reattached.png`: earlier commands still on screen, `%WRAITH_X%` = `kept`, title replayed |
| Force-killed client (no Detach sent) → pane detached, next launch reattaches | **Pass** (screenshot) | `2026-11-01-m2-3-after-client-kill.png` |
| `wr ctl server-stop` ends the server and its shells; `wr` then exits 2 | **Pass** (scripted) | |
| Typing latency through the pipe under an output flood | **Pass** (automated) | `remote_input_round_trip_is_fast`: p99 < 3 ms, max < 10 ms (doubled on CI runners, which stall on their own) |
| UI never holds pane output (M1 perf fix) through the server | **Pass** (automated) | `ui_thread_never_holds_pane_output` |
| Eight simultaneous launches share one server | **Pass** (automated) | `concurrent_launches_share_one_server` |
| Server death shows `[server stopped] Enter: reconnect` | partly | events disconnect within 1 s is tested (`server_death_disconnects_events`); the overlay and Enter→reconnect were not exercised in a window |
| Version-mismatch restart dialog | not run | needs two builds with different `PROTO_VERSION` |
| Latency overlay (Ctrl+Shift+F12) in a reattached window | not run | needs a keyboard |
| Pipe refused to another Windows user | not run | needs a second account; DACL string and owner check are unit-tested |
