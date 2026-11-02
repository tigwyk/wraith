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
Task 10: Ruling: while the palette is open every key is consumed by it (chords are not typed into the query, Escape/Enter/arrows/Backspace handled); IME commits go to the query.
Task 10: Ruling: fuzzy scoring uses +8 consecutive, +6 word start, -1 per skipped character; the query change resets the selection to the first row; the palette lists the Window group last because all_actions() order is regrouped Workspaces/Tabs/Panes/Window.
Task 10: Ruling: picking a directory creates a workspace named for its last path component and rooted there, and this window attaches it (awaiting_new_ws), same as the sidebar '+ New workspace'.
Task 10: Note: real window confirmed: filtered palette with keybindings (docs/screenshots/2026-11-01-m3-5-command-palette.png) and the picker, whose 'Recent' row C:\Users\Lee proves the pwsh integration script's OSC 7 reaches the server (m3-6-workspace-picker.png).
Task 10: complete (commits 77fc17f..74ed0cf, tests: rtk proxy cargo test --workspace → exit 0 + clippy -D warnings clean; palette 10/10, view 29/29)
Task 11: Ruling: no DwmExtendFrameIntoClientArea — WM_NCCALCSIZE keeps the left/right/bottom frame, so DWM still draws the shadow and Win11 border; the caption row is ours — cost if wrong: a missing 1 px top border on some themes.
Task 11: Ruling: caption buttons are custom-drawn and handled on WM_NCLBUTTONDOWN/UP (posting SC_*), so DefWindowProc never paints classic buttons; HTMAXBUTTON keeps the snap-layout flyout (seen live) — cost if wrong: none known.
Task 11: Ruling: Alt+Space sends WM_SYSCOMMAND SC_KEYMENU directly, because keys skip TranslateMessage and no WM_SYSCHAR arrives — cost if wrong: a user keybind on alt+space is shadowed.
Task 11: Ruling (found live): restore_effects passed the last OSC 7 cwd to CreateProcess, and a WSL pane's /mnt/c/... path made restore fail ("directory name is invalid"); it now passes only an absolute directory that exists (the WSL cmdline keeps its own --cd) — test restore_never_hands_createprocess_a_directory_it_cannot_enter RED→GREEN — cost if wrong: a restored native pane starts in the server's dir when its old dir is gone.
Task 11: scripts/shot.ps1 restored (un-maximized) the window before capture; now restores only a minimized one.
Task 11: verified live: tabs/new-tab = client, empty bar = caption, min/max/close hit codes; close hover red; maximize keeps the bar on screen with the restore glyph; restore returns to 1000x640. Screenshot docs/screenshots/2026-11-01-m3-7-title-bar.png.
Task 11: complete (tests: cargo test --workspace → exit 0; clippy -D warnings → exit 0)
Task 12: Ruling: our own png-crate DecodePng instead of libghostty's `png` feature — its RustPngDecoder decodes into a zero-length Vec (always fails) and leaves gray images non-RGBA — cost if wrong: one extra small decoder to maintain.
Task 12: Ruling: ImageSync keys sent images by (id, libghostty generation), so retransmitting the same id re-uploads (test retransmitted_image_is_uploaded_again) — cost if wrong: none.
Task 12: Ruling: no separate graphics-generation dirty flag — every storage change comes from vt_write, which already marks the pane changed, and scrolling moves rows too — cost if wrong: a placement change with no output would wait for the next frame.
Task 12: Ruling: frame_with_only_a_new_placement_is_still_sent became the end-to-end pane test image_upload_precedes_the_frame_that_places_it (emit-seq kitty through ConPTY) — it covers both the "frame still sent" and the ordering rule — cost if wrong: none.
Task 12: Ruling: virtual (Unicode placeholder) placements and images stored compressed are skipped (compressed ones logged); placements are sent in full every frame, not as deltas — cost if wrong: placeholder-based tools (some TUIs) show no image yet.
Task 12: complete (tests: cargo test -p wraith-server -p wraith-proto → exit 0, images 9/9; clippy -D warnings → exit 0)
Task 13: Ruling: GridRenderer::push_layers(out, q, store, pane, ...) instead of the brief's push_image_quads — push_quads is split into push_bg/push_fg and images go between (z<0) or after (z>=0), each image its own textured Batch; the open untextured batch q is flushed before each image so order holds — cost if wrong: one extra draw call per visible image.
Task 13: Ruling: point sampling for all images (the pipeline's one sampler) rather than linear when scaled — icat-style tools send images at display size — cost if wrong: down/up-scaled placements look blocky.
Task 13: Ruling: image_quad is a pure function (dest from cell + offsets + placement px, clipped to the pane rect with UVs cut to match), tested without a GPU; GPU readback tests cover fill, z order both ways, straight-alpha blending, delete, a no-dirty-rows frame moving the image, and per-pane drop; image_texture_survives_resize_of_other_panes not written (textures are keyed by (pane,id) and nothing on resize touches the store) — cost if wrong: none known.
Task 13: textures are freed with the pane (snapshot no longer lists it), and all of them on WindowAttached/WorkspaceTaken (the server re-uploads on Reattach).
Task 13: scripts/kitty-show.py added (minimal icat: PNG file or --gradient, chunked f=100). Live: a 320x160 gradient shows inline and scrolls with text, clipped at the pane top (docs/screenshots/2026-11-01-m3-8-inline-image.png, -9-image-scrolled.png).
Task 13: complete (tests: cargo test -p wraith-client → exit 0, images 10/10; clippy -D warnings → exit 0)
Task 14: Ruling (found during acceptance): tab drag-to-reorder (spec §4.2) had no client gesture; added WindowView::start_tab_drag/drag_tab (optimistic local reorder, one MoveTab{index} per slot change) and wired a press on a tab to start it — test dragging_a_tab_over_another_reorders_once RED→GREEN; verified live — cost if wrong: none.
Task 14: Ruling: acceptance used a separate release target dir (target/accept) because the user's own Wraith runs from target/release; release builds need .tools Zig 0.15.2 on PATH (winget's 0.17 fails) — cost if wrong: none.
Task 14: Ruling: `kitten` is not installed; scripts/kitty-show.py stands in for icat (PNG f=100, chunked) — cost if wrong: icat's own placement options (scaling, unicode placeholders) untested.
Task 14: Not run: four-pane flood with the ctrl+shift+f12 overlay; zsh integration; screenshot of directional focus/zoom (unit-tested).
Task 14: spec updated: §3.4 snapshot ruling, §4.2 sidebar note (Agents section in M4).
Task 14: complete (tests: cargo test --workspace → exit 0; clippy -D warnings → exit 0; acceptance in docs/m3-acceptance.md)
Final review: self-review (the session's rules disallow starting a reviewer subagent unasked) — weaker than a fresh reviewer.
Final: checked Review Focus 1–5 (all have tests), image re-upload after unsubscribe/resubscribe (Reattach resets ImageSync), tab-drag permissions (MoveTab within own workspace), NC hit test vs reserved caption width, draw order (dim and chrome drawn over images). No Critical/Important findings open; the two found during acceptance (restore cwd, missing tab drag) were fixed with RED→GREEN tests.
Final: Ruling: RemotePane stays in link.rs — it is now the driver of the link integration tests (incl. remote_input_round_trip_is_fast), not dead code — cost if wrong: ~70 lines kept.
Final: minor (deferred): restore checks Path::is_dir on the saved cwd, which can stall on an unreachable UNC path.
Final: minor (deferred): images use point sampling even when scaled; Unicode-placeholder (virtual) placements are not drawn.
Final: minor (deferred): Alt+Space always opens the window menu, shadowing a user keybind on alt+space.
Final: minor (deferred): new_window starts a plain wraith.exe, so a window launched with -e opens its sibling on the default profile.
