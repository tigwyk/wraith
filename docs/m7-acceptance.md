# M7 acceptance — spec gaps

Date: 2026-11-02. Branch `m7`.

**Setup:**
- Debug build.
- A private pipe, layout file, `APPDATA` and `LOCALAPPDATA`.
- Test config: `cursor-style-blink = false`, `keybind = f7=new_tab`, `agent-notify = badge` (no toast, no flash), `agent-tab-pulse = true`.
- The window was shown without activation. Keys were posted to its queue only, and it was captured with `PrintWindow`. There was no focus stealing and no global input.

| Check | Result | Evidence / notes |
|---|---|---|
| `wr config check`: clean config | Pass | `wr: config ok (<path>)`, exit 0 |
| `wr config check`: bad line, includes, missing file, default path | Pass | `<path>:1: invalid value \`huge\``, exit 1. `crates/cli/tests/check.rs`: 5 tests |
| `wr events --json` streams model changes | Pass | Live run: a new tab and a `wr hook generic --state ask` produced `workspace_opened`, two `pane_opened` and `agent … "state":"asking","attention":true`. 6 diff tests and 3 end-to-end tests |
| `wr events` ends when the server stops | Pass | It exited with code 0 after `wr ctl server-stop` (live, and `events_exit_when_server_stops`) |
| Retry when the server won't start | Pass (logic) | `retry_attach_*` (3 tests). The dialog was not shown live: making the server fail to start needs a broken install |
| Right-click menu | Pass (logic) | `context_menu_*` (3 tests) and `has_selection_follows_the_frame`. Live: needs the real pointer |
| Inline IME composition | Pass (logic) | `preedit_*` and `ime_end_clears_preedit` (5 tests). Live: needs a Japanese IME |
| `agent-tab-pulse` breathes the asking tab | Pass | The background tab with the asking agent read (29,33,38) and (44,66,84) in captures 800 ms apart. The tab with no agent stayed (40,44,52) |
| `agent-tab-pulse` cost | Pass | 31 ms of client CPU over 3 s while breathing (20 Hz frames). 0 ms with the pulse turned off by hot reload while the agent still asked |
| `agent-tab-pulse` with "Show animations" off | Pass (logic) | `breathe_is_steady_when_animations_off`: a steady tint and no frames. Live: needs you to flip the Windows setting |
| Spec brought up to date | Pass | Status line, §2, §3.3, §3.4, §4.4, §6.1, §6.2, §8, §11 risk 3, and the M7 row in §10 |

Not run live: the right-click menu, IME composition, the Retry dialog and reduced motion. These need the real pointer, an IME, a broken server install or a Windows setting changed, so they are left for you.
