# Wraith — Native Windows Terminal on libghostty-vt

Status: approved; M0–M7 built
Date: 2026-11-01
Working name: **Wraith** (rename freely; avoid "Ghostty" in the product name)

## 1. Intent

**Goal.** A native Windows terminal the author uses as a daily driver, replacing Windows Terminal. It competes with Alacritty and Windows Terminal on speed and correctness, and adds Ghostty-grade VT fidelity (via `libghostty-vt`) plus first-class awareness of AI coding agents.

**Stated by the user**
- Built on `libghostty-vt` (https://docs.rs/libghostty-vt).
- Multiple tabs. Multiple workspaces, each grouping its own tabs.
- Split panes inside a tab in v1.
- Inline images (photos, agent screenshots) rendered in the terminal through the Kitty graphics protocol, e.g. `kitten icat`, `chafa`, `yazi`, and AI harnesses that show screenshots inline.
- AI harness integration: a harness icon next to the tab title, and detection of "needs user input" with toasts, tab flash, or similar.
- Detection is hook-first with passive fallback.
- Harnesses at launch: Claude Code, OpenAI Codex CLI, and others (Aider, OpenCode, Copilot CLI, cursor-agent, Gemini CLI, ...) through passive detection and a generic hook.
- Ghostty-style config file.
- Live sessions owned by a background server. Closing the window does not kill shells.
- Architecture: server + thin client. All Rust, raw Win32 + Direct3D 11 + DirectWrite.

**Assumptions**
- Windows 10 1809+ / Windows 11, x64 only (ConPTY minimum). ARM64 later.
- Rust stable, MSVC toolchain.

**Success criteria**
- Keypress-to-pixel latency under 8 ms at 120 Hz on a typical dev machine. **This holds regardless of session length or load:** after 10 million lines of output with scrollback full, while another pane floods output, and after days of uptime (see §6.3).
- Cold start to an interactive prompt under 300 ms when the server is already running.
- Correct rendering of ligatures, color emoji, wide/CJK characters, and IME input.
- Passes `vttest` basics and common TUIs (nvim, htop/btop in WSL, lazygit, Claude Code, Codex).
- Shells survive closing and reopening the window.
- An agent waiting for input in a hidden tab gets the user's attention within 1 s.

**References**
- zen-term (https://github.com/praxis-labs-io/zen-term). Layout and agent-attention model: sidebar + tab bar + pane tree; working/waiting/idle; attention roll-up; "default to idle".
- Rex (https://www.superlogical.com/rex/docs/customize/config). Server/client split, named actions in a palette, key sequences, event stream for the CLI.

## 2. Non-goals for v1

Lua or any scripting language in config. macOS/Linux. Remote/SSH sessions as server-managed hosts. Persisting processes across a *server* restart (scrollback persistence was added in M5: history is saved on `server-stop` and restored above a fresh shell). Tab tear-off into new windows by drag (multiple windows exist; moving tabs between them uses a command). A settings GUI for every key (the built-in panel covers the common ones; see `2026-11-02-settings-gui-design.md`).

## 3. Architecture

### 3.1 Binaries

| Binary | Subsystem | Role |
|---|---|---|
| `wraith.exe` | Windows GUI | Default: client window. `wraith.exe --server`: server. `wraith.exe --activate pane:<id>`: toast activation target. |
| `wr.exe` | Console | `wr hook ...`, `wr ctl ...`, `wr events --json`, `wr config check`, `wr install-hooks`/`uninstall-hooks`. |

If no server is running, the client spawns `wraith.exe --server` detached. The server keeps running after the last client disconnects. It exits only on `wr ctl server-stop`, the palette action `quit_server` (confirms that shells will close), or logoff. There is no tray icon in v1.

### 3.2 Crates (one Cargo workspace)

| Crate | Purpose | Depends on |
|---|---|---|
| `proto` | Wire message types, framing (u32 length prefix + `postcard`), protocol version constant | serde, postcard |
| `server` | Session model, pane threads, pipe server, agent tracker, layout persistence | proto, libghostty-vt, windows |
| `client` | Window, renderer, input mapping, chrome widgets, toasts | proto, windows |
| `config` | Ghostty-syntax parser, typed config, keybind parser, theme loading | (none) |
| `agent` | Pure state engine + harness tables (no I/O) | (none) |
| `cli` | `wr.exe` | proto, config |

`libghostty-vt` is imported **only** by `server` (and by `client` during M1, before the split).

### 3.3 Server threading

- **Main thread:** named-pipe accept loop (overlapped I/O), session model, routing, agent tracker.
- **One thread per pane:** owns the ConPTY handles, `Terminal`, `RenderState`, `key::Encoder`, and `mouse::Encoder`; OSC signals arrive through libghostty callbacks (see the §5.2 M4 note). All of these are `!Send`, so they never leave this thread. Commands arrive on a channel (input, resize, scroll, selection, subscribe/unsubscribe, ready-credit).
- **One reader thread per pane:** blocking `ReadFile` on the ConPTY output pipe, forwarding byte chunks to the pane thread.
- **Bundled ConPTY.** Wraith ships `conpty.dll` + `OpenConsole.exe` from `Microsoft.Windows.Console.ConPTY` (pinned 1.25.260930003, MIT) next to `wraith.exe`. Every pane is created with `ConptyCreatePseudoConsole(..., flags = 8 /* passthrough */, ...)`, and resize/close go through the same DLL. Inbox ConPTY strips the Kitty graphics APC (M0 findings), so inline images need the bundled DLL. If the DLL is missing, panes fall back to inbox `CreatePseudoConsole` without inline images, and a warning is logged. The pane answers the DA1 query (`ESC[c`) that the bundled OpenConsole sends at startup.
- Each pane's shell runs inside its own **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Closing the pane closes the job and kills every descendant. The job also gives the process list for agent detection.

### 3.4 IPC

- Pipe name `\\.\pipe\wraith-<user-SID>`. Its security descriptor grants access to the current user SID only. The server rejects connections whose client token SID differs.
- Handshake: `Hello{proto_version, client_kind: Gui|Cli|Hook}` → `Welcome{server_version}` or `VersionMismatch`.
- Client → server: `Input{pane, KeyEvent|MouseEvent|Text|Paste}`, `Resize{pane, cols, rows, cell_w, cell_h}`, `Subscribe{panes}`, `Ready{pane, frame_seq}`, `Scroll{pane, delta|to}`, `SetSelection{pane, range|none}`, `Copy{pane}`, `Command{Action}`, `AgentEvent{...}` (hooks), `Query{...}` (cli), `Watch` (cli: `wr events` receives every model snapshot).
- Server → client: `ModelSnapshot` then `ModelEvent`s (workspace/tab/pane created, closed, renamed, moved; agent state; attention), `FrameDelta{pane, seq, full: bool, rows: [(y, cells)], cursor, colors, scrollbar}`, `Clipboard{text}`, `Bell{pane}`, `Notify{pane, title, body}`, `ImageUpload{image_id, width, height, rgba}` (sent once per image per client), `ImageDelete{image_id}`. `FrameDelta` also carries the visible image placements `{image_id, cell rect, source rect, z}`.
- **M3 ruling (model sync):** the server pushes a whole `ModelSnapshot` (with a `rev`) after every model change instead of `ModelEvent`s. The model is small (kilobytes), changes at human speed, and a full snapshot cannot drift out of sync. Per-change events stay possible if the model grows. Frame and image events are gated by `Subscribe{panes}`. Image placements in a `FrameDelta` are the full on-screen list, not a delta. Images are uploaded once per view, keyed by id and libghostty generation, and re-sent after a reattach.
- **Encoding happens on the server.** It holds the terminal modes, so the client sends abstract key and mouse events.
- **Backpressure:** at most one unacknowledged `FrameDelta` per pane per client. While waiting for `Ready`, the pane thread keeps dirty state and sends one merged delta when the credit arrives. Floods (`cat` of a large file) cost CPU on the server, never pipe memory.
- **Cell encoding:** per row, a list of style runs `{fg, bg, flags, hyperlink_id, text}`, where `text` is a UTF-8 string of graphemes with per-cell width implied by Unicode width. Wide characters occupy two cells.

### 3.5 Shell integration

When the server spawns a shell it sets `WRAITH_PANE_ID`, `WRAITH_PIPE`, and `TERM_PROGRAM=wraith`. For pwsh, bash, and zsh (including inside WSL through `WSLENV`), it injects an integration script that emits OSC 7 (cwd) and OSC 133 A/B/C/D (prompt and command marks). These enable: new tab/split inherits cwd, jump to previous/next prompt, select command output, and agent "process ended" detection.

## 4. Data model and UI

### 4.1 Model (owned by server, mirrored to clients)

```
Workspace { id, name, root_dir: Option<Path>, color, tabs: Vec<TabId>, active_tab, profile }
Tab       { id, workspace, title_override: Option<String>, root: PaneNode, focused: PaneId, zoomed: Option<PaneId> }
PaneNode  = Leaf(PaneId) | Split { axis: Horizontal|Vertical, ratio: f32, a: Box<PaneNode>, b: Box<PaneNode> }
Pane      { id, cwd, title, exit: Option<i32>, agent: Option<AgentInfo>, attention: Attention }
Window    { id, attached_workspace }   // client-side; several windows can show different workspaces
```

Rules:
- Focus moves by direction (left/right/up/down), resolved geometrically against the pane tree.
- Closing the last pane closes the tab. Closing the last tab closes the workspace. Closing the last workspace leaves an empty default workspace (the window stays open).
- After a close, focus goes to the neighbor in visual order.
- Hidden panes keep their last size. **A pane is never resized below 2×1 cells**, because zero-size resizes crash TUIs.
- One workspace can be attached to only one window at a time. Attaching it in a second window moves it.

### 4.2 Window layout

```
┌──────────┬───────────────────────────────────────────────┐
│ ▸ api    │ [◆ claude ●] [pwsh] [✱ codex] [+]             │
│   web  ● ├───────────────────────┬───────────────────────┤
│   infra  │                       │                       │
│──────────│        pane           │        pane           │
│ AGENTS   │                       ├───────────────────────┤
│ ● claude │                       │        pane           │
│ ◐ codex  │                       │                       │
└──────────┴───────────────────────┴───────────────────────┘
```

- **Title bar:** custom-drawn and integrated with the tab bar (WM_NCCALCSIZE extended frame). Mica backdrop and immersive dark mode through `DwmSetWindowAttribute`. Snap layouts preserved through correct WM_NCHITTEST on the maximize button.
- **Sidebar:** workspace list (name, color chip, attention dot) with each workspace's tabs as a tree under it (a click shows that tab, attaching its workspace if needed), then the Agents section sorted Waiting (oldest first) → Working → Idle. Clicking an entry focuses its pane. Toggle with `ctrl+shift+b`. When collapsed, it reveals on hovering an 8 px strip at the left edge (300 ms grace). *M3 ships the workspace section only. The Agents section, the attention dots and agent-sorted rows arrive with agent integration (M4).*
- **Tab bar:** per tab, harness icon (if any) + title + state badge (spinner = working, accent dot = waiting). Drag to reorder. A command moves a tab to another workspace, and so does dragging it onto a workspace's row in the sidebar (or before a tab listed there); workspaces another window shows and the quick terminal's refuse the drop, and a window whose last tab leaves follows it. Dropping onto another window's tab bar is not supported.
- **Panes:** the focused pane gets a 1 px accent border. Optional `unfocused-split-opacity` dim.
- **Command palette** (`ctrl+shift+p`): fuzzy list of every named action with its current keybinding, grouped by Workspaces / Tabs / Panes / Window / Agents.
- **Workspace picker** (`ctrl+shift+o`): open workspaces + configured workspaces + recent directories.
- **In-app toasts:** for errors and no-op actions, throttled to one every 3 s per kind.
- All chrome is drawn by the client renderer with the active theme's colors.

### 4.3 Workspaces and profiles

- Workspaces come from config (`workspace = name:dir[:layout]`) or are created ad hoc ("Workspace N").
- New tabs in a workspace start in its `root_dir`, else the cwd of the focused pane, else `%USERPROFILE%`.
- Profiles: pwsh, Windows PowerShell, cmd, Git Bash, and each WSL distro are auto-detected. Custom profiles use `profile = name:command`. `default-profile = pwsh`.

### 4.4 Persistence

The server writes `%LOCALAPPDATA%\wraith\layout.json` (workspaces, tabs, pane trees, cwd, titles, profiles) on every model change, debounced 1 s, with atomic replace. On server start it restores this layout with fresh shells. Scrollback is saved on `server-stop` and restored above the fresh shell (M5). Processes do not survive a server restart.

## 5. Agent integration

### 5.1 Hook path (primary)

`wr hook <harness> [event]` behaviour:
1. Read `WRAITH_PANE_ID` and `WRAITH_PIPE`. If either is missing, exit 0 immediately.
2. Read the payload from stdin (Claude Code) or argv (Codex `notify`), capped at 64 KiB.
3. Connect to the pipe as `client_kind: Hook` and send `AgentEvent{pane, harness, kind, message}`.
4. Exit 0 within a 200 ms total deadline, whatever happens. Never print to stdout (some harnesses parse it).

**Claude Code.** `wr install-hooks claude` merges into `~/.claude/settings.json`. It prints the diff, asks for confirmation, writes `settings.json.bak`, and is idempotent (entries tagged so reinstall replaces them). `wr uninstall-hooks claude` removes only the tagged entries.

| Claude hook | Agent event |
|---|---|
| `SessionStart` | `Attached` (icon appears, state Idle) |
| `UserPromptSubmit`, `PreToolUse` | `Working` |
| `Notification` | `Ask{message}` (permission prompt or idle prompt) |
| `Stop` | `TurnEnd` |
| `SessionEnd` | `Detached` |

**Codex CLI.** `wr install-hooks codex` sets `notify = ["wr", "hook", "codex"]` in `~/.codex/config.toml` (same diff/confirm/backup rules). Payload `type: "agent-turn-complete"` → `TurnEnd{message: last-assistant-message}`. The title pattern for Codex's "Action Required" → `Ask`.

**Generic.** `wr hook generic --state working|ask|turn-end|idle|detached [--name <agent>] [--message <text>]`, for any tool or script.

### 5.2 Passive detection (fallback and icon source)

- **Process scan:** about once a second per pane (only panes with a running child), read the pane's Job Object process list (`QueryInformationJobObject` / `JobObjectBasicProcessIdList`) and match executable names (and, for node/python launchers, the command line) against the built-in harness table. Users can extend it with `agent-detect = <name>:<regex>`. A match → `Attached{harness}`. The match disappearing → `Detached`.
- **Terminal signals** (from the sidecar `osc::Parser` and libghostty callbacks):
  - OSC 9 / OSC 777 desktop notification while an agent is attached → `Ask{body}`
  - OSC 9;4 progress active → `Working`
  - BEL while an agent is attached → `Ask`
  - Title matching the harness's `ask_title` regex → `Ask`
  - OSC 133;D after an agent command → `Detached`
- **M4 note:** the vendored Ghostty exposes these signals as callbacks (`on_desktop_notification`, `on_progress_report`, `on_semantic_prompt`), so no sidecar OSC parser is needed. It also provides **OSC 7501 program status** (`state=working|blocked|done|idle|error|clear`), which Wraith maps to Working / Ask / TurnEnd / Idle as a passive signal.
- Built-in harness table (data in `agent` crate): `claude`, `codex`, `gemini`, `aider`, `opencode`, `copilot`, `cursor-agent`, `amp`, `goose`. Each entry: id, display name, icon, process matchers, `ask_title` regex.

### 5.3 State engine

A pure function `fn step(state: AgentState, ev: AgentEvent, ctx: Ctx) -> AgentState`, where `Ctx` = {pane visible, window foreground, user typed since}. Rules live in a priority-ordered table, and the highest-priority matching rule wins.

- States: `Working`, `Waiting(Ask)`, `Waiting(TurnEnd)`, `Idle`.
- **Unknown or ambiguous → Idle.** A false alarm costs more than a quiet row.
- `Waiting(Ask)` latches until the user types in that pane (`Input` with Key/Text/Paste). Focusing or viewing does not clear it.
- `Waiting(TurnEnd)` raises attention only if the pane was not visible when the turn ended. It clears when the pane becomes visible. *M4 ruling:* a pane is visible only when a window in the foreground draws it. A window coming to the foreground counts as viewing its panes.
- Hook events beat passive signals: when a pane has received a hook event in the last 10 minutes, passive state signals for it are ignored (passive icon detection still runs).

### 5.4 Attention and notification

Attention rolls up pane → tab → workspace → window, as a max over children.

| Surface | Signal |
|---|---|
| Tab | Harness icon, state badge, optional pulse (`agent-tab-pulse = true`) |
| Sidebar | Workspace attention dot, Agents list entry |
| Taskbar | `FlashWindowEx` (until foreground) + `ITaskbarList3::SetOverlayIcon` badge, when window not foreground |
| Windows toast | Title = "<Agent> needs input — <workspace>/<tab>", body = message. Button "Open" → activation `wraith.exe --activate pane:<id>` focuses window, workspace, tab, pane. Shown when window not foreground or pane not visible. |

- Config: `agent-notify = toast,flash,badge,sound` (any subset), `agent-notify-when = unfocused|always`, `agent-notify-quiet = 22:00-08:00`.
- *M4 ruling:* a toast click is handled in the window process (the toast's `Activated` event focuses the pane). `--activate pane:<id>` COM activation is not built: clicking a toast after its window closed just opens Wraith, which reattaches.
- Toasts need an AppUserModelID. The installer (or first run, for portable mode) creates a Start Menu shortcut with `System.AppUserModel.ID = Wraith.Terminal` and the toast activator CLSID.

### 5.5 Safety

- Hook/OSC message text is untrusted. Strip control characters and trim to 200 characters before showing it. Never interpret it as markup.
- The pipe is per-user (§3.4). Hook connections may send only `AgentEvent`, and only for an existing pane id.
- Hook installers never write without confirmation and always keep a backup.

## 6. Rendering and input (client)

### 6.1 Renderer

- D3D11 device, `DXGI_SWAP_CHAIN_EFFECT_FLIP_DISCARD`, waitable swap chain for latency (`SetMaximumFrameLatency(1)`). Mica through the DWM system backdrop when `background-opacity < 1`.
- Per pane: instanced quad pass for cell backgrounds and decorations (underline styles, strikethrough, cursor, selection), then instanced glyph pass sampling a glyph atlas. Chrome uses the same pipeline plus a rounded-rect pass.
- Text: DirectWrite `IDWriteTextAnalyzer` for shaping runs (ligatures on by default, `font-feature` configurable), `IDWriteFontFallback` for fallback, `IDWriteFactory4::TranslateColorGlyphRun` for color emoji. Atlas key = (font face, glyph id, size, subpixel x offset quantized to 1/4 px). Grayscale antialiasing with gamma/contrast matched to DirectWrite defaults.
- **Inline images (Kitty graphics):** libghostty-vt (`kitty-graphics` feature, `png` decoding) stores the images and placements on the server. The client keeps one D3D11 texture per `image_id` and draws a textured-quad pass per placement, below or above text by `z`. Image memory is capped by `image-storage-limit` (Ghostty key, default 320 MB per pane), and the oldest images are evicted first. Sixel and iTerm2 OSC 1337 images are not supported, because libghostty-vt does not implement them.
- Only rows marked dirty in `FrameDelta` are re-shaped. The cached per-row geometry is reused otherwise.
- Frame-time overlay (`ctrl+shift+f12`): input-to-present latency, frame time, atlas usage.

### 6.2 Input

- `WM_KEYDOWN`/`WM_SYSKEYDOWN`/`WM_CHAR` with scancode and `ToUnicodeEx`. Compute the physical key + mods + produced text, and send it as an abstract `KeyEvent` that the server maps to `libghostty_vt::key::Event`.
- IME via `ImmGetCompositionString`. The candidate window is positioned at the cursor, and the preedit text is drawn inline (in the palette and find bar it follows their query).
- Mouse: client-side hit testing to the pane cell. When the terminal has mouse tracking on (from a `MouseTracking` pane event), forward it as `MouseEvent`. Otherwise handle selection locally against the pane's cell cache, and send `SetSelection` to the server for copying.
- Clipboard: copy-on-select (`copy-on-select = true`), right-click paste (`right-click-action = paste|menu`). Paste goes through the server's paste-safety check, and a confirmation is shown for multi-line or unsafe paste.

### 6.3 Always fast: performance rules

Typing must never slow down as a session grows. These rules are requirements, and every one has a test:

1. **Input before output.** On every loop turn, the pane thread writes all queued key/text/paste input to the PTY before it parses the next output chunk. Output is parsed in slices of at most 64 KiB, so a keystroke waits behind at most one slice.
2. **Work scales with the screen, not the history.** Building a frame reads only the viewport rows from `RenderState`, and only dirty rows are re-shaped and re-uploaded. Nothing on the per-keystroke or per-frame path walks scrollback.
3. **Memory is bounded.** Scrollback is capped by `scrollback-limit` (lines) and images by `image-storage-limit`. No other structure grows with output: no unbounded logs, transcripts, undo, or per-line caches. The client's row cache holds only visible rows.
4. **Rendering never blocks input.** Floods coalesce through frame backpressure (§3.4). The client processes window messages before presenting, and input is never queued behind a frame.
5. **Measured, not assumed.** The pane reports how long each input waited before being written to the PTY. The frame overlay shows input-to-present latency. CI runs a long-session soak: memory stays flat, and input wait stays under 2 ms p99 while flooding.

Caveat: if a program re-renders its whole transcript on every keystroke, it will be slow in any terminal. Wraith guarantees its own share of the latency stays flat.

## 7. Configuration

- Path: `%APPDATA%\wraith\config`, plus `config-file = path` includes. Ghostty syntax: `key = value`, `#` comments, repeated keys for lists.
- **Ghostty key names and semantics where they overlap**: `font-family`, `font-size`, `font-feature`, `theme`, `background`, `foreground`, `palette`, `background-opacity`, `cursor-style`, `cursor-style-blink`, `window-padding-x/y`, `scrollback-limit`, `image-storage-limit`, `copy-on-select`, `confirm-close-surface`, `unfocused-split-opacity`, `keybind`, `command`, `shell-integration`.
- Wraith-specific: `workspace`, `profile`, `default-profile`, `sidebar-visible`, `sidebar-width`, `agent-*` (§5.4), `agent-detect`, `close-on-exit`.
- Themes: Ghostty theme file format. The Ghostty theme set is bundled.
- Keybinds: Ghostty syntax including sequences (`ctrl+a>c=new_tab`) and `unbind`. Defaults follow Windows Terminal conventions (`ctrl+shift+t`, `ctrl+shift+w`, `ctrl+tab`, `alt+shift+plus/minus` for splits, `alt+arrow` focus).
- Hot reload by file watch (`ReadDirectoryChangesW`), debounced 200 ms. Unknown or invalid keys keep their default, and an in-app toast shows file:line. `wr config check [path]` exits 1 on any error.
- Action names are stable identifiers shared by keybinds, palette, and `wr ctl`.

## 8. Error handling

| Situation | Behaviour |
|---|---|
| Server not running | Client spawns it, retries connection for 2 s, then shows an error dialog with the log folder and Retry / Cancel. |
| Protocol version mismatch | Dialog: "Server is from an older version. Restart server? Running shells will close." Restart / Cancel (cancel exits client). |
| Shell exits | `close-on-exit = clean` (default): exit code 0 closes the pane. If that was the window's last pane, the window moves to a workspace no window shows (not the quick one), or closes when there is none. Any other code shows `[process exited N] Enter: restart · ctrl+shift+w: close`. `always` (or `true`) closes whatever the code; `never` (or `false`) always keeps the pane. |
| ConPTY creation fails | Pane shows the Win32 error text. Other panes unaffected. |
| Server crash | Shells die with their Job Objects. The client shows "Server stopped" and relaunches it on Retry. The layout is restored from `layout.json`. |
| Config error | Per-key fallback + toast (§7). |
| Hook failure | `wr hook` exits 0 silently. Its failure is logged by the server only if it reached the pipe. |
| Logs | `%LOCALAPPDATA%\wraith\logs\{server,client}-YYYYMMDD.log`, 7-day retention, `tracing` with level from `WRAITH_LOG`. |

## 9. Testing

- **Unit:** `agent` state engine (table-driven: event sequences → expected states, including the latch and visibility rules); `config` parser and keybind parser; pane-tree operations (split, close, directional focus, resize clamping); `proto` round-trip for every message.
- **Integration (Windows CI):** a headless server + scripted client over a real pipe, driving `cmd.exe /c echo` and checking `FrameDelta` content; reattach after client disconnect; `wr hook claude Notification` with fixture JSON → pane state `Waiting(Ask)`; `wr install-hooks` against a temp HOME is idempotent and reversible.
- **Performance:** recorded VT streams (large `cat`, `btop` replay, nvim scroll) fed through `vt_write` + frame build, with time budgets tracked in CI. Manual latency check with the frame overlay.
- **Manual acceptance:** vttest, nvim, lazygit, Claude Code, and Codex sessions; IME (Japanese) input; emoji/ligature sample file.
- CI: GitHub Actions `windows-latest`, Zig 0.17 installed, `cargo test --workspace`, `cargo clippy -D warnings`.

## 10. Milestones

Each milestone gets its own implementation plan.

| # | Scope | Exit criteria |
|---|---|---|
| M0 | Spike: build `libghostty-vt` on `x86_64-pc-windows-msvc` with Zig 0.17. Spawn `cmd.exe` in ConPTY, `vt_write` its output, read the grid through `RenderState`, encode a key. Verify that a Kitty graphics APC sequence (`ESC _ G … ESC \`) and OSC 9/777/133 pass through ConPTY intact, both from a native process and from WSL. | Test passes on the dev machine and CI. If the crate fails on MSVC: patch upstream or build libghostty-vt from Ghostty source with Zig directly. Decide before M1. |
| M1 | Single-process, single-pane terminal: Win32 window, D3D11 renderer, DirectWrite, input + IME, selection/clipboard, config + themes + hot reload. | Usable as a daily driver for a single shell. Latency target met. |
| M2 | Server/client split: `proto`, named pipe, pane threads, backpressure, reattach, `wr ctl` basics. | Close window → reopen → same shells and screens. |
| M3 | Workspaces, tabs, splits, sidebar, palette, workspace picker, profiles, shell integration, layout persistence, multiple windows. Inline images (Kitty graphics, §6.1). | Full chrome works. Server restart restores layout. `kitten icat photo.jpg` shows the photo inline, and it scrolls with the text. |
| M4 | Agent integration: `agent` crate, hooks + installers, passive detection, attention roll-up, taskbar + Windows toasts. | Claude Code and Codex in hidden tabs produce a toast within 1 s of asking. Clicking it focuses the pane. |
| M5 | Scrollback search (find bar, `ctrl+shift+f`), scrollback persistence across `server-stop`, local packaging (dist zip, per-user install script, winget manifest validated but not submitted). | Done; see `docs/m5-acceptance.md`. |
| M6 | Hover feedback and polish: every clickable piece of chrome reacts to hover and press, the pointer shape follows the target (resize arrows on dividers), the command palette takes the mouse, tooltips name icon-only buttons and their keys, accents dim in an inactive window, and state changes move briefly (tab underline and workspace bar slide, floating sidebar slides in, palette and find bar fade in, new attention dots flare once). Motion follows Windows' "Show animations" setting and never costs input latency (§6.3). | Done; see `docs/m6-acceptance.md`. |
| M7 | Spec gaps from the 2026-11-02 review: `wr config check`, `wr events --json`, Retry when the server won't start, the right-click menu (`right-click-action = menu`), inline IME composition, `agent-tab-pulse`, and this spec brought up to date. | Done; see `docs/m7-acceptance.md`. |
| Settings | A settings panel drawn in the window (`ctrl+comma`, the gear): the common keys, live, edited in place in the config file. Spec: `2026-11-02-settings-gui-design.md`. | Done; see `docs/settings-acceptance.md`. |
| Quick terminal | A drop-down window on a global hotkey (`keybind = global:...=toggle_quick_terminal`, `quick-terminal-*`), with its own workspace. Spec: `2026-11-03-quick-terminal-design.md`. | Done; see `docs/quick-terminal-acceptance.md`. |
| Backlog | Auto-update, ARM64. | Unscheduled. |

## 11. Known risks

1. **libghostty-vt on Windows is untested** (docs.rs builds only for Linux, and the API is pre-1.0). M0 de-risks this. Pin the crate version and wrap it in the `server` crate so API breaks stay local.
2. **Zig version and libghostty-vt source:** Wraith vendors libghostty-rs (`vendor/libghostty-rs`, see its `VENDORED.md`) with bindings regenerated against a pinned Ghostty `main` commit, which needs Zig 0.17.x. (M0–M3 ran on the published 0.2.2 crate, whose older Ghostty pin needed Zig 0.15.x.) Moving the pin means re-running `gen-bindings` and the full suite.
3. **No OSC 9/777/133 callbacks in libghostty-vt:** resolved in M4. The vendored Ghostty exposes them as callbacks, so no sidecar parser exists.
4. **ConPTY quirks (resolved in M0):** inbox ConPTY passes OSC 9/777/133/7 but strips the Kitty APC. The bundled ConPTY in passthrough mode passes all of them, native and WSL (`docs/m0-findings.md`). Remaining risk: a future ConPTY package changes passthrough behaviour. Mitigation: the version is pinned, and the passthrough tests run in CI.
5. **Unpackaged toasts** require an AUMID shortcut. Without the installer, first run creates it.
