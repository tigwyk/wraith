# M4 acceptance — agent integration

Date: 2026-11-02 (local evening of 11-01). Branch `m4`, release build, private pipe `wraith-m4acc`. The user was using the machine during the run, so checks avoided stealing focus or sending keystrokes:
- **Input:** went through `wr ctl send-text` (over the pipe) and direct `wr hook` runs.
- **Window captures:** used `PrintWindow` on a window restored without activation.
- **Toast evidence:** Windows' notification history for the app id `Wraith.Terminal`, plus Wraith's own log, which records each notify decision and each toast shown.
- **Do Not Disturb:** it was on, so toast banners did not pop up. The toasts went to the notification center (history), which is how Windows handles DND, not something Wraith controls.

Spec exit criterion (§10 M4): *Claude Code and Codex in hidden tabs produce a toast within 1 s of asking. Clicking it focuses the pane.*

| Check | Result | Evidence / notes |
|---|---|---|
| Hook → toast latency (Claude `Notification` payload, background window) | **Pass** | 7–12 ms from `wr hook claude` to the toast shown, 3 runs (log timestamps). `wr hook` itself exits in 6–8 ms. |
| Toast content | Pass | History: "Claude Code needs input — Workspace 1/shell" / "Claude needs your permission to use Bash"; "Codex finished — Workspace 1/shell" / its last message. Text nodes only, silent by default. |
| Codex `notify` payload (`agent-turn-complete`) | Pass | Toast 12 ms after the hook, with the window minimized. Codex itself is not installed; the documented payload was used. |
| Real Claude Code detected passively (process scan) | Pass | `m4-1-claude-detected`: `claude.exe` in the pane → ✻ icon on the tab and "Claude Code · idle" in the AGENTS list. |
| Claude exits → agent cleared | Pass | `m4-2-claude-exit-detaches`: tab icon and AGENTS section gone. |
| Real Claude permission prompt → toast | **Not run** | Claude stopped at its folder-trust prompt for `C:\Users\Lee`. Accepting it grants Claude rights there and saves that in the user's Claude config, so the run declined (Esc). The hook path it would take is covered by the payload checks above and `crates/cli/tests/hook.rs`. |
| Clicking the toast focuses the pane | **Not run live** | The click handler is wired: `Activated` → `WM_TOAST_CLICKED` → `focus_pane`, then restore and foreground. Clicking a notification-center toast while the user was working was not done. `focus_pane` is unit-tested (`agent_row_click_focuses_pane_across_workspace`). |
| Tab badge, workspace dot, AGENTS row | Pass | `m4-3-ask-badge` |
| Taskbar flash and overlay badge | Partly | The policy chose flash+badge (log). The taskbar itself was not captured. |
| Start Menu shortcut with the AUMID created on first run | Pass | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Wraith.lnk` appeared; toasts under `Wraith.Terminal` are accepted (`notifier setting: Enabled`). |
| Window in front with its pane visible → no toast | Pass | Log: `foreground=true visible=true → ways none` (spec: shown only when the window is in the background or the pane is hidden). |
| Hook installer into a separate settings file (`--file`), used by `claude --settings` | Pass | 6 events installed. The user's own `~/.claude/settings.json` was not touched. |

Bugs found and fixed during acceptance:
1. **Turn end in a background window:** a turn ending in a pane drawn by a background window raised no attention, because "visible" meant subscribed. It now means drawn by a foreground window (`turn_end_in_a_background_window_counts_as_unseen`).
2. **Minimized start:** a window started minimized believed it was in front and suppressed every toast. The client now starts unfocused, and `WM_SETFOCUS` sets focus.

Not run: OSC 9/777/7501 from a real program (covered by `crates/server/tests/pane.rs` through ConPTY); Claude Code inside WSL (needs `wr install-hooks claude --wsl <distro>` and a trusted folder); the latency regression with four panes and the frame overlay (`remote_input_round_trip_is_fast` passes).
