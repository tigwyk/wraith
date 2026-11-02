# M3 acceptance

Date: 2026-11-01. Branch `m3`. Real windows, with private `WRAITH_PIPE_NAME` and `WRAITH_LAYOUT_PATH` and `NO_COLOR` cleared. Scripted input went only to the test window: `wr ctl send-text`, plus key and mouse scripts that stop if the test window loses the foreground. Screenshots are in `docs/screenshots/2026-11-01-m3-*.png`.

Rows marked **debug** used the debug build during Tasks 8–13. All other rows used the release build (`target/accept`, Zig 0.15.2 from `.tools/`).

## Exit criteria (spec §10, M3)

| Criterion | Result | Evidence |
|---|---|---|
| Full chrome works | **Pass** | Rows below |
| A server restart restores the layout | **Pass** | `-11-restored-after-server-stop` |
| An inline photo shows and scrolls with the text | **Pass** (with `scripts/kitty-show.py`, not `kitten icat`) | `-8-inline-image`, `-9-image-scrolled` |

## Checks

| Check | Result | Evidence / notes |
|---|---|---|
| New tab, vertical and horizontal split | Pass (debug) | `-1-tabs-and-splits` |
| Directional focus, zoom | Pass (unit) | `goto_split_uses_geometry`, `zoomed_tab_shows_only_the_zoomed_pane`. No screenshot. |
| Divider drag | Pass (debug) | `-2-divider-drag` |
| Tab click / close | Pass (debug) | `-1-tabs-and-splits`, view tests |
| Tab drag to reorder | Pass | `-16-tab-drag-reorder` (AAA dragged past BBB) |
| Two workspaces, sidebar toggle, hover reveal | Pass (debug) | `-3-sidebar-workspaces`, `-4-sidebar-hover-reveal` |
| Command palette: open, filter, run | Pass (debug) | `-5-command-palette` |
| Workspace picker: open, recent directory | Pass (debug) | `-6-workspace-picker` ("Recent: C:\Users\Lee" comes from OSC 7) |
| Custom title bar: drag area, min/max/close, hover, maximize/restore | Pass (debug) | `-7-title-bar`. The hit test was probed for each region. Maximize keeps the bar on screen. Restore returns the window to 1000×640. |
| Snap-layout flyout on maximize hover | Pass (debug) | The flyout covered the button on hover; this also blocked a synthetic click there. No screenshot (the flyout belongs to the shell). |
| Second window (`new_window`) | Pass | `-12-second-window` (opens on a new Workspace 2; Workspace 1 dimmed as open elsewhere) |
| Workspace steal | Pass | `-13-workspace-stolen` (window 2 takes Workspace 1), `-14-steal-victim-moves-on` (window 1 moves to Workspace 2) |
| Close window → relaunch reattaches | Pass | `-15-relaunch-reattaches` (same live shell) |
| `wr ctl server-stop` → relaunch restores the layout with fresh shells in the same directories | Pass | `-11-restored-after-server-stop` (two panes, both `C:\Windows`). Found and fixed during Task 11: a WSL pane's POSIX cwd was passed to CreateProcess. |
| pwsh cwd inheritance (`Set-Location` then split) | Pass | `-10-split-inherits-cwd` |
| Profile detection list | Partly | Covered by 16 unit tests (`crates/server/tests/profile.rs`). M3 has no user-visible list of profiles. |
| Inline image scrolls with text | Pass (debug) | `-8`, `-9`. The image is clipped at the pane top and not drawn over the tab bar. |
| zsh / WSL integration | Partly | The WSL pane reported its cwd through OSC 7 (the restore bug above shows this). zsh was not run. |

## Latency (plan step 3)

- `remote_input_round_trip_is_fast` passes in the workspace suite.
- Not run: the four-pane flood with the `ctrl+shift+f12` overlay reading.

## Re-run on Ghostty 7cd6fa0 (after the libghostty-vt upgrade)

Date: 2026-11-01. Release build of `main` at `24509d4` (Zig 0.17, vendored libghostty-rs), then with the caption fix below. Your own config (default profile: WSL). Private pipe `wraith-acc3`. Screenshots: `docs/screenshots/2026-11-01-m3-r*.png`.

| Check | Result | Evidence / notes |
|---|---|---|
| New tab, vertical and horizontal split | Pass | `r01` |
| Directional focus (`alt+left`) | Pass | Focus border moved to the left pane |
| Zoom through the palette | Pass | `r02` (palette filtered to "Toggle split zoom"), `r03` |
| Divider drag | Pass | `r04` |
| Tab drag to reorder | Pass | `r05` (TAB-B dragged before TAB-A) |
| Sidebar toggle | Pass | `ctrl+shift+b` hides and shows it |
| Sidebar hover reveal | Pass, one unexplained miss | `r06`. The first try did not reveal it. Repeating the same sequence did, as did a single move followed by a rest. Another app took the foreground twice during this run, which probably explains the miss. |
| Command palette, workspace picker | Pass | `r02`, `r07` ("Recent › Lee C:\Users\Lee" from OSC 7) |
| Title bar hit test | Pass | Probed: tab = client, empty bar = caption (drag area), min/max/close = 8/9/20, top edge = 12 |
| Close hover | Pass | `r08` |
| Maximize / restore | Pass | Maximized to (-8,-8)-(3448,1400). Restore returned to exactly (156,156)-(1156,796). |
| Split inherits pwsh cwd | Pass | `r09` |
| Inline image (PNG through the new process-global decoder) scrolls with text | Pass | `r09`, `r10` |
| `server-stop` → relaunch restores the layout and directories | Pass | `r11` (3 tabs, 6 panes, `C:\Windows`), and again in `r15` |
| Second window (default WSL profile) | Pass | `r12` |
| Workspace steal | Pass | `r13`; the other window moved to Workspace 2 |
| WSL cwd tracking (OSC 7) | Pass | `cd /tmp` saved `"cwd": "/tmp"` |
| Close window → relaunch reattaches | Pass | `r14` (same live WSL shell, still in `/tmp`) |
| `exit` in the window's last pane closes the window | Pass | The window process ended. Workspace 1's 6 panes were untouched. |

**Bug found and fixed:** on a fresh launch, the tab bar did not leave room for the caption buttons. With 3 tabs, the third tab ran under minimize/maximize/close (`r11`). The reserve was set only on resize, so it was missing until the window was maximized or resized. It is now part of `Geometry`, which is set when the window is created. The test `tabs_leave_room_for_caption_buttons` now builds the view from that geometry. Fixed in `r15`.

**Seen once, not reproduced:** one relaunch after `server-stop` attached a new empty "Workspace 3" instead of the restored Workspace 1. Workspace 1 was restored, just left detached. An exact repeat of the steps restored and attached Workspace 1 normally (`r15`). The layout file from that run had already been overwritten, so the cause is unknown. If it happens again, the restored workspace can be reached from the sidebar.

Not re-run: snap-layout flyout (shell UI); the four-pane flood with the frame overlay; zsh.
