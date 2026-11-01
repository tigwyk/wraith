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
