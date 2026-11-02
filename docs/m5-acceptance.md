# M5 acceptance — scrollback search, scrollback persistence, local packaging

Date: 2026-11-02 (local evening of 11-01). Branch `m5`, release build. Private pipe and layout file, and a temporary `APPDATA` with a test config (`keybind = f9=find`, `keybind = f8=scroll_to_top`). With these, keys could be posted straight to the test window's message queue, with no focus stealing and no global input.

| Check | Result | Evidence / notes |
|---|---|---|
| Find bar opens, searches as you type, shows the count | Pass | `m5-1-find-typed`: "Find: line-150" with count 1 over a 300-line flood |
| Enter selects the older match, scrolls to it, highlights it | Pass | `m5-2-find-next`: line-150 at the top, highlighted, 1/1 |
| Esc closes the bar and clears the highlights | Pass | Seen live; the view stays where the search left it |
| Searching keeps input fast while a pane floods | Pass | `search_keeps_input_fast_while_flooding`: p99 under 2 ms, max under 5 ms |
| `server-stop` saves each pane's history | Pass | `scrollback\1.snap` (17 KB for 300 lines) next to the layout |
| Restart restores the history, with a marker | Pass | `m5-3-restored-history-top`: history starts at line-1. `m5-4-restored-marker`: "── restored 2026-11-01 20:41 ──", found by search (1/1). The file is removed after loading. |
| Corrupt or other-version snapshot | Pass | `corrupt_snapshot_gives_fresh_pane` |
| Stale snapshot files | Pass | `stale_snapshots_are_removed` |
| Clients cannot write snapshots | Pass | `clients_cannot_send_save_scrollback` |
| Dist zip | Pass | `scripts\dist.ps1` → `dist\wraith-0.1.0-x64.zip` (3.8 MB); its SHA256 is written into the winget manifest |
| winget manifest | Pass | `winget validate` succeeded (zip + portable `wraith`/`wr`). Not submitted; the URL is a placeholder. |
| Install (dry run) into a temp prefix | Pass | All files and 892 themes in place; your PATH and registry unchanged |
| Installed copy runs | Pass | It runs, and its pane uses the bundled `OpenConsole.exe` from the install folder |
| Install refuses while that copy runs | Pass | Refusals exit 1 |
| `wraith.exe --register` | Pass | The shortcut's AUMID reads back as `Wraith.Terminal` |
| Real install/uninstall (PATH, Start Menu, Apps & features) | Not run | Changes your machine; the dry runs cover the logic up to those steps |

Bugs found during acceptance:
1. **Restore marker:** the marker overwrote an old line and then vanished. The soft reset (and `ESC[?1049l` outside the alternate screen) moved the cursor home. Fixed: explicit mode resets, and the alternate screen is left only when active. The test now checks all 200 lines survive.
2. **Scrollback cap (regression from the Ghostty upgrade):** libghostty's new default ~10 KB scrollback *byte* cap left every pane about one page of history (under 1,000 lines), whatever `scrollback-limit` said. Fixed: the byte cap is cleared, and the line limit governs.

Not run: the quake window, settings GUI, auto-update and ARM64 (out of scope); a real install.
