# M1 acceptance

Date: 2026-11-01 · Build: `cargo build -p wraith-client --release` on Windows 11 26300

Automated coverage: 25 test suites green (`cargo test --workspace`), clippy clean. The items below need a person at the keyboard. Claude ran the app but could not see or type into it, so those items are marked **not run** rather than passed.

| Check | Result | Notes |
|---|---|---|
| App starts, spawns pwsh under bundled OpenConsole | **Pass** (scripted) | 52–57 MB working set at idle |
| Log file created | **Pass** (scripted) | `%LOCALAPPDATA%\wraith\logs\client.2026-11-01.log` |
| Killing the app leaves no orphaned console processes | **Pass** (scripted) | Job Object + bundled ConPTY |
| Input stays fast under output flood | **Pass** (automated) | `input_not_starved_by_flood`: max < 5 ms, p99 < 2 ms (pane side) |
| Memory flat once scrollback is full | **Pass** (automated) | `memory_flat_after_scrollback_full`: < 16 MiB growth 50k→200k lines |
| Frame cost independent of scrollback | **Pass** (automated) | `frame_cost_independent_of_scrollback`: ratio < 1.5 |
| nvim, lazygit, WSL htop, claude render and respond | not run | |
| vttest menus 1 and 2 | not run | |
| Ligatures (`=> != ===`), emoji (`😀👍🏽`), CJK (`日本語`) aligned | not run | Shaping and fallback are unit-tested (`font.rs`) |
| Japanese IME commits inline | not run | IME window is positioned at the cursor; preedit is not drawn inline |
| Overlay (`ctrl+shift+f12`) shows input→present < 8 ms while holding a key at 120 Hz | not run | |
| Long session: `emit-seq flood 10000000`, then hold a key: still < 8 ms, memory flat | not run | `target\release\emit-seq.exe` |
| 100 MB `type` keeps the window draggable | not run | |
| Moving between 100% and 150% monitors keeps text crisp | not run | DPI scaling is unit-tested (`metrics_scale_with_dpi`) |
| Saving a bad config shows the toast; terminal keeps working | not run | Reload + toast logic unit-tested (`reload.rs`) |

Known M1 limits (shortcut comments in code): fixed font-fallback list; atlas clears instead of growing; curly underline drawn straight; `command` changes apply on next start; only the main config file is watched (not includes); no `TranslateMessage`, so Alt+Space does not open the system menu.
