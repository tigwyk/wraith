# M3 execution ledger (every deviation from the plan, and the final-review outcome)
Spec: docs/superpowers/specs/2026-11-01-wraith-terminal-design.md
Plan: docs/superpowers/plans/2026-11-01-wraith-m3.md
Branch: m3 (from main 10b5be4)
Pre-flight:
- T1 model types -> T3 Model, T4 wire, T8 view: consistent (PaneId stays in wire; model.rs imports it).
- T3 ModelCmd/SpawnOpts.image_storage_limit -> T4 session, T12 pane: consistent.
- T4 v2 messages -> T6 ServerLink, T8 view, cli: consistent.
- T7 Pipeline/Batch -> T8 draw, T13 images: consistent.
- T12 ImagePlacement/PaneEvent -> T13 store/draw: consistent.
Task 1: Ruling: layout_never_below_min asserts bounds/count when the area cannot fit the minimums (plan text said 'min or exactly min'); found+fixed a b-rect offset past the area when the usable span is 0 — cost if wrong: none.
Task 1: complete (commits 90c5735..79d55d5, tests: rtk proxy cargo test -p wraith-proto → exit 0, 15 model tests pass)
Task 2: Ruling: added shell-integration key (detect|none|pwsh|bash|zsh → bool) and Config.shell_integration here, since Task 5 reads it — cost if wrong: none.
Task 2: Ruling: client app.rs gets a temporary wildcard arm for the new Actions so the workspace builds; Task 8 replaces it.
Task 2: Note: test source with Windows paths uses forward slashes (tool heredocs halve backslashes); parser still tested for drive-colon handling.
Task 2: complete (commits f2a69e3..997ee9b, tests: rtk proxy cargo test --workspace → exit 0; config 22+15 tests)
Task 3: Ruling: ModelCmd lives in wraith_proto::model; SpawnOpts gained image_storage_limit now (all constructors updated) — needed by NewWorkspace restore defaults.
Task 3: Ruling: a window whose workspace is removed (last tab closed) needs a re-attach; Model drops the workspace and creates a fresh default (test pins this). Session (Task 4) must handle 'my workspace vanished' by attaching the most recent detached one.
Task 3: Ruling: from_json also drops panes not referenced by any tab and resets attached/exit; any dangling reference rejects the whole file.
Task 3: complete (commits 4195838..f1584d1, tests: rtk proxy cargo test --workspace → exit 0; model 17/17)
Task 4: Ruling: run_server uses default_layout_path() (WRAITH_LAYOUT_PATH or %LOCALAPPDATA%); spawn_server_thread has NO persistence and spawn_server_thread_at(pipe, Option<PathBuf>) takes an explicit path — tests run in parallel threads, a process-global env var would race — cost if wrong: none.
Task 4: Ruling: PaneEvent::ImageUpload/ImageDelete variants added now (Task 12 fills them) so routing compiles; Reattach now also re-sends MouseTracking so a window that subscribes later learns the mode.
Task 4: Ruling: a window whose workspace disappears (last tab closed) is moved server-side to the most recent detached or a new empty workspace (WindowAttached + Snapshot) — pinned by closing_the_last_tab_moves_the_window_to_a_workspace.
Task 4: Ruling: model commands from a GUI window are limited to its own workspace (+ NewWorkspace anywhere; Close/Rename/MoveTab-to only on workspaces nobody else holds); CLI unrestricted.
Task 4: Ruling: tests wait for the effect of a model command with a List barrier (snapshots arrive in order before the Panes reply) instead of 'first newer snapshot'.
Task 4: Note: wraith-client and wraith-cli do not compile against protocol v2 until Task 6; Tasks 4-5 run -p wraith-proto -p wraith-server -p wraith-config -p wraith-ipc.
Task 4: Note: pane tests for cwd added to server/tests/pane.rs (cwd_from_uri_converts_drive_and_posix_paths, osc7_sends_cwd_event).
Task 4: complete (commits 7f485c3..29e9db4, tests: rtk proxy cargo test -p wraith-proto -p wraith-server -p wraith-config -p wraith-ipc → exit 0; session 22/22, pane 19/19, model 17/17)
Task 5: Ruling: WSL integration runs the user's own login shell (bash or zsh) via 'sh -c' on $SHELL instead of forcing bash — forcing bash would silently replace a zsh/fish user's shell; any other shell starts plain with -l (no integration) — cost if wrong: fish/nushell users get cwd inheritance only if their shell emits OSC 7.
Task 5: Ruling: Git Bash starts as 'bash.exe -i --rcfile integration.bash' (no --login, which would ignore --rcfile); the script sources /etc/profile and ~/.bash_profile itself on MSYS — cost if wrong: login-only customizations missing.
Task 5: Ruling: integration is skipped for any command that already carries -Command/-File/--login/--rcfile/-e style arguments; detect() runs 'wsl -l -q' (hidden window), so the client calls it once at startup — cost if wrong: ~50 ms at launch with WSL installed.
Task 5: Ruling: added wslenv_with() and wired it into session.start_pane so TERM_PROGRAM and WRAITH_PANE_ID cross into WSL (spec §3.5 WSLENV); WRAITH_PIPE stays Windows-only.
Task 5: Note: zsh/bash scripts are checked for OSC strings only; a real zsh/bash run is part of Task 14 acceptance (manual); the PowerShell script is parse-checked by powershell.exe in tests.
Task 5: complete (commits 33f14fb..3d5ab66, tests: rtk proxy cargo test -p wraith-proto -p wraith-server -p wraith-config -p wraith-ipc → exit 0; profile 16/16)
Task 6: Ruling: remote.rs became link.rs with ServerLink (all messages) PLUS a thin single-pane RemotePane adapter (AttachWindow, use/open the first tab's focused pane, Subscribe) so app.rs compiles and runs until Task 8 drives ServerLink directly; RemotePane is deleted in Task 8 — cost if wrong: throwaway ~100 lines.
Task 6: Ruling: spawn_server/connect gained an env slice (attach_with_env) so tests start servers with their own WRAITH_LAYOUT_PATH instead of touching the user's layout.json; latency.rs already isolates through LOCALAPPDATA.
Task 6: Ruling: fixed my Task 4 test dropped_slow_client_sees_its_pipe_close — its helper thread kept draining the pipe, so the client was not slow and it only passed by timing (found by the first full-workspace run); now a raw connection that stops reading; 3/3 green reruns.
Task 6: complete (commits 4d9f520..b30bae4, tests: rtk proxy cargo test --workspace → exit 0, 37 suites ok; link 8/8, cli all)
Task 7: Ruling: Painter::text/text_width take the FontStack instead of a CellMetrics argument (metrics come from fonts.metrics()); Batch.texture is bound at slot t1 but only exercised by Task 13.
Task 7: Ruling: GridRenderer::push_quads gained a fill: Option<Rect> that paints the pane's background rectangle (a multi-pane frame has no single clear color); draw() passes None and keeps its clear-to-background behavior, so the M1 grid tests are the unchanged regression suite.
Task 7: complete (commits dde695c..83058ec, tests: rtk proxy cargo test --workspace → exit 0; paint 3/3, grid 13/13)
Task 8: Ruling: view logic (snapshot mirror, rects, subscriptions, actions) is a pure WindowView; the GridRenderers live in App keyed by pane (created on a pane's first Frame) — tests need no GPU, and App is only plumbing.
Task 8: Ruling: tab bar sits in its own row below the standard caption (the integrated title bar is Task 11); chrome colors derive from the theme (chrome_colors) instead of a separate palette.
Task 8: Ruling: new panes take their profile from the focused pane (Split) or workspace/default profile (NewTab); '-e <cmd>' and 'command =' become a synthetic profile 'command' and the default, so existing launch flags keep working.
Task 8: Ruling: Selection state lives per pane in PaneView and is dropped when a pane leaves the active tab; exit overlay is drawn from the snapshot (PaneMeta.exit), not from the Exited event, so it survives reattach.
Task 8: Ruling: acceptance driving uses scripts/keys.ps1 and scripts/mouse.ps1 (SendInput, abort unless Wraith is foreground and, for clicks, the pointer is over it); two stray clicks in my first mouse run landed before I added the pointer guard.
Task 8: Note: verified in the real window with screenshots (docs/screenshots/2026-11-01-m3-1-tabs-and-splits.png, m3-2-divider-drag.png): new tab, split right/down, tab click, divider drag, focused border, dim of unfocused panes.
Task 8: complete (commits 70c13a4..bf9b4b4, tests: rtk proxy cargo test --workspace → exit 0, clippy -D warnings clean for wraith-client; chrome 8/8, view 15/15)
Task 9: Ruling: the hover-revealed sidebar floats over the panes (no relayout) and is checked before the tab bar in hit(); it hides 300 ms after the pointer leaves it or the window (WM_MOUSELEAVE via TrackMouseEvent; the constant is defined locally because the windows crate does not export it).
Task 9: Ruling: a workspace this window creates (sidebar '+ New workspace' or new_workspace) is attached when the first unknown workspace id appears in a snapshot; configured workspaces missing by name are created once per window start and are not attached.
Task 9: Ruling: closing a workspace with running shells asks first (Out.confirm_close_workspace); the server moves the window to another workspace.
Task 9: Note: clippy -D warnings on the whole workspace also flagged Task 5 profile.rs (chunks_exact → as_chunks) and two test idioms; fixed here. Real-window check: docs/screenshots/2026-11-01-m3-3-sidebar-workspaces.png, m3-4-sidebar-hover-reveal.png (switching workspaces via the sidebar, floating reveal over the panes).
Task 9: complete (commits 4648eb3..8daecb1, tests: rtk proxy cargo test --workspace → exit 0 + cargo clippy --workspace --all-targets -D warnings clean; view 24/24)
Task 9: Correction: the T9 ledger line above was written after a full-workspace run in which version_mismatch_is_reported failed once; it is NOT 'exit 0' for that commit. Cause (pre-existing since M2, found by my own stress test version_mismatch_is_reported_every_time, red 1/1 before the fix): the server dropped a client right after sending VersionMismatch/Error; closing a pipe discards a reply the client has not read yet, so a mismatched client could see EOF instead of the restart dialog.
Task 9: Ruling: a client refused at Hello is flagged refused, told why and then ignored until it closes its own end (server never closes first for refusals) — cost if wrong: one idle pipe connection per refused client until it disconnects.
Task 9: complete (fix commit b2bd8a1, tests: rtk proxy cargo test --workspace → exit 0 (gated on the exit code this time) + clippy -D warnings clean; session 23/23 x3 reruns)
