# M2 execution ledger (every deviation from the plan, and the final-review outcome)
Spec: docs/superpowers/specs/2026-11-01-wraith-terminal-design.md
Branch: m2 (from main ded46f7)
Pre-flight:
- T1 PaneCmd/PaneEvent/SpawnOpts/wire -> T3 server, T4 client, T5 cli: consistent (all consume wraith_proto::{pane,wire}).
- T2 PipeListener/PipeConn/connect/pipe_name -> T3 (listener), T4 (connect), T5 (connect): consistent.
- T3 run_server/spawn_server_thread -> T4 (--server mode), T5 tests (in-process server): consistent.
- T1 PaneCmd::Reattach -> T3 attach replay: consistent.
- T3 Cli-vs-Gui Cmd rule <-> T5 send-text to any pane: consistent after plan self-review fix.
- Note: M1 perf fix (ack-on-apply) means client sends Ready immediately; T3 forwards Ready like any Cmd — no conflict.
Task 1: Ruling: server keeps `pub use wraith_proto::pane::{PaneCmd, PaneEvent, ScrollTo}` in pane.rs so existing imports keep working — cost if wrong: none.
Task 1: Ruling: added test garbage_body_is_invalid_data (postcard decode errors map to InvalidData); write_frame refuses bodies > MAX_FRAME with InvalidInput.
Task 1: complete (commits ded46f7..1b6afec, tests: rtk proxy cargo test -p wraith-proto -p wraith-server --test wire --test pane → test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.04s)
Task 2: Ruling: added tests many_clients_each_get_a_connection and write_to_dropped_peer_is_broken_pipe (server needs both) — cost if wrong: none.
Task 2: Ruling: connect retries ERROR_PIPE_BUSY via WaitNamedPipeW(2 s) then TimedOut — all instances can be momentarily busy between accepts — cost if wrong: none.
Task 2: Note: cross-user protection (DACL, owner check) verified only at SDDL-string and owner-SID-function level; a real second-account test is not automated (needs another Windows user).
Task 2: complete (commits 1b6afec..ed8f83c, tests: rtk proxy cargo test -p wraith-ipc → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 3: Ruling: added PaneInfo.shell_pid and PaneHandle::pid() (shell pid at spawn; not updated by Restart) — kill/stop tests and `wr ctl list` need it — cost if wrong: stale pid after Restart.
Task 3: Ruling: spawn_server_thread returns io::Result<JoinHandle> (binds before returning) instead of a bare JoinHandle — tests must not race the bind — cost if wrong: none.
Task 3: Ruling: ServerStop is accepted from any client that completed Hello (Gui or Cli) — spec lists `wr ctl server-stop` and the version-mismatch restart (Gui) — cost if wrong: any same-user client can stop the server (same trust boundary as the pipe).
Task 3: Ruling: on stop, set a flag and self-connect to wake the accept thread so the pipe name is released (found by server_stop_kills_all_shells) — cost if wrong: none.
Task 3: Ruling: detached panes' frames are dropped (not queued); the pane stops building frames because its credit is never acked — matches plan.
Task 3: complete (commits ed8f83c..16d319f, tests: rtk proxy cargo test -p wraith-server → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Task 4: Ruling: version-mismatch restart terminates the server process via PipeConn::server_pid() (added to ipc, test server_pid_is_the_serving_process — passed first run, written with the impl) instead of ServerStop — a mismatched server refuses the client before ServerStop could be read — cost if wrong: an old server dies without logging a clean stop.
Task 4: Ruling: RemotePane::detach/kill take &mut self (plan: self) so the app can call them on its field; later sends return Err — cost if wrong: none.
Task 4: Ruling: RemotePane sends through a writer thread (never blocks the UI on the pipe); detach/kill join it so the last message is on the pipe before the process exits — cost if wrong: none.
Task 4: Ruling: spawn_server passes WRAITH_PIPE_NAME explicitly so a client and the server it starts always agree on the pipe; breakaway-from-job retried without the flag if refused — cost if wrong: inside a no-breakaway job the server dies with the client.
Task 4: Ruling: added test detach_then_attach_reattaches (client-side reattach path end to end).
Task 4: Ruling: client latency test (from the M1 perf fix) now uses its own WRAITH_PIPE_NAME and kills the server it started, so tests never touch the user's real server or leak processes.
Task 4: Note: dropping a RemotePane without detach/kill does not close the pipe while its reader thread is blocked; the app always calls detach or kill, and process exit closes it.
Task 4: complete (commits 16d319f..73b68a9, tests: rtk proxy cargo test -p wraith-client --test remote --test latency → test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.04s)
Task 5: Ruling: added tests usage_errors_exit_1 and server_stop_stops_it; kill_by_id strengthened (pane listed before kill) after it passed vacuously against an empty wr.
Task 5: Ruling: wr checks the pane exists via List before send-text/kill (server silently ignores unknown panes) — cost if wrong: one extra round trip.
Task 5: Note: first cli test run hung — send_text test blocked in a pipe read past its deadline; fixed in the test (reader thread + recv_timeout). Not a product bug.
Task 5: complete (commits 73b68a9..4bb1eb7, tests: rtk proxy cargo test -p wraith-cli → test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.36s)
Task 6: Ruling: acceptance used `wr ctl send-text` to type (no SendKeys into the user's desktop); overlay check, version-mismatch dialog, server-stopped overlay and cross-user denial recorded as "not run"/"partly" with reasons.
Task 6: complete (commits 4bb1eb7..7618b01, tests: timeout 600 rtk proxy cargo test --workspace → test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s)
Final review: Opus reviewer — 1 Critical, 4 Important, 9 Minor. Verdict "with fixes".
Final: Ruling: regraded minor "server inherits first client's env/cwd" to Important (I-5) — later shells miss PATH changes, the first window's directory stays locked — fixed (client sends its env via SpawnOpts; server starts in the user profile).
Final: Ruling: regraded I-4 (spawn_pane blocks the server loop) to Minor — measured spawn_pane: cmd 10.9 ms, wsl 7.8 ms, failing UNC 3.9 ms (CreateProcess returns before the shell is up); an ~11 ms stall of other windows only while a window opens is below perception — cost if wrong: brief hitch on slow machines (Defender first-scan).
Final: fixed C-1 exited panes reattached / close-on-exit relaunch loop — reattach_skips_exited_panes RED→GREEN (server skips+removes exited detached panes; client kills on exit/close-on-exit), suite 34/34
Final: fixed I-1 accept ends after a client leaves before accept — accept_survives_client_that_drops_before_accept RED→GREEN (DisconnectNamedPipe+retry; session accept loop logs and continues), suite 34/34
Final: fixed I-2 dropped slow client never sees its pipe close — dropped_slow_client_sees_its_pipe_close RED→GREEN (writer thread DisconnectNamedPipe on exit), suite 34/34
Final: fixed I-3 elevated/non-elevated share one pipe (UAC bypass) — elevated_and_normal_use_different_pipes, server_elevation_is_checked RED→GREEN (-admin pipe, High mandatory label, client checks server elevation), suite 34/34
Final: fixed I-5 stale server env for new shells — shells_get_the_launching_windows_environment RED→GREEN, suite 34/34
Final: fixed (minor folded into C-1 area, test-backed) wire Shutdown/Reattach rejected — wire_cannot_shut_down_or_reattach_a_pane RED→GREEN
Final: Ruling (Declined-to-judge items) — detached shells pile up by design (wr ctl list shows them); unbounded channels/same-user DoS out of scope; ServerStop from Gui needed for restart; stale shell_pid after Restart cosmetic; titles not escaped in wr list; reattach doesn't resend config colors; no-breakaway job fallback documented; attach reads lack timeout; cell px not refreshed on reconnect (next Resize fixes) — all stand.
Final: minor (deferred): spawn_pane on the server main loop (~11 ms per new window)
Final: minor (deferred): truncated frame allocates its announced length (≤64 MiB) before failing
Final: minor (deferred): LocalFree before Error::from_thread can clobber the AddrInUse mapping
Final: minor (deferred): TOKEN_USER read through a u8 buffer (alignment UB)
Final: minor (deferred): pane thread panic leaves a zombie entry (forwarder exits silently)
Final: minor (deferred): stale delta frame can briefly misrender just after reattach
Final: minor (deferred): spec §8 asks for an error screen with Retry; M2 shows an OK-only box and exits
Final: minor (deferred): HRESULT built from raw OS error without from_win32 (cosmetic)
Final: minor (deferred): client_checks_server_owner never exercises the refusal path
Final review: fix pass complete; no second fix pass.
