# Quick terminal — acceptance

Spec: `docs/superpowers/specs/2026-11-03-quick-terminal-design.md`
Plan: `docs/superpowers/plans/2026-11-03-quick-terminal.md`
Date: 2026-11-03, Windows 11, debug build.

## Automated

- `cargo test --workspace`: green (67 test binaries).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- New tests:
  - config: the `quick-terminal-*` keys, their diagnostics, and `global:` with `unbind` and `clear`;
  - the server: the quick workspace is made once, skipped by reattach, and survives the layout and old layouts;
  - `AttachQuick` takes the quick workspace back from a normal window;
  - the client's pure maths: the hotkey's Win32 mapping, `quick_rect` (percent, px at 144 DPI, an offset work area, clamping), the toggle rules, the slide, and the mutex name.

## Live check (private pipe, fake APPDATA/LOCALAPPDATA, test hotkey `ctrl+alt+shift+f11`)

| Check | Result |
| --- | --- |
| `wraith --quick-terminal` starts and stays running | ✅ |
| Its window exists and is hidden | ✅ `IsWindowVisible` false |
| Window styles | ✅ ex-style `0x200088` (no redirection bitmap, tool window, topmost); style `WS_POPUP` |
| The hotkey registers | ✅ log: `quick terminal: hotkey Ctrl+Alt+Shift+F11` |
| A second `--quick-terminal` on the same pipe exits | ✅ within 2 s, exit 0; log: `quick terminal already running` |
| `wr ctl server-stop` ends it quietly (no dialog) | ✅ exited within 10 s; no processes left |
| Toggle 1 (`WM_HOTKEY` posted, id 1): shows | ✅ visible, foreground; slides in (top −29 px at 120 ms); rect `0,0 3440×557`, exactly 40% of the 3440×1392 primary work area |
| First frame | ✅ tab bar, sidebar with the "Quick" workspace, the shell's output and a focused block cursor |
| Toggle 2: hides | ✅ not visible after the slide |
| Toggle 3: shows the same shell again | ✅ same output and prompt; `wr ctl list` shows one pane, still running |
| Toggle 4: hides | ✅ |
| A normal launch starts the quick terminal | ✅ seen by accident: `server_outlives_killed_client_process` found the quick terminal's extra shell until tests turned the hotkey off |

## Not checked live

- The cursor after the second summon: its screenshot shows none, most likely the blink's off phase; not confirmed.
- The real Win+` press, and the toast when Windows Terminal holds it.
- Autohide by clicking another app, and the IME candidate window not hiding it.
- Multiple monitors, `quick-terminal-screen = mouse`, a top or left taskbar, and a 150% DPI monitor (covered only by `quick_rect`'s unit tests).
