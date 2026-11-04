# Wraith quick terminal (drop-down window) — design

Status: implemented (2026-11-03); see `docs/quick-terminal-acceptance.md`
Parent spec: `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md` (§10 Backlog: "Quake/drop-down window")

## 1. Intent

**Goal.** A global hotkey slides a terminal down from the top of the screen from anywhere in Windows, and the same key hides it again. This is Windows Terminal's "quake mode" and Ghostty's "quick terminal".

**Decided with the user (2026-11-03)**
- The default hotkey is Win+` and can be changed or turned off.
- The quick terminal can be summoned after Wraith has been launched once in the session. Nothing registers at sign-in.
- It shows its own workspace. The shells in it keep running while it is hidden.
- It hides when it loses focus. A config key turns that off.
- Approach: a separate, hidden client process holds the hotkey (§3). The rejected options were a server-held hotkey that starts a client on demand (each summon would pay GPU and font start-up) and every window registering the hotkey (only one process can hold it).

**Success criteria**
- After any Wraith launch, Win+` shows the quick terminal within one frame of the slide starting, from any app, without a visible start-up delay.
- Pressing it again while the quick terminal is in front hides it. Pressing it while it is shown but in the background brings it to the front.
- A shell started in the quick terminal is still running, with its scrollback, after a hide and the next summon.
- Configs written for Ghostty's quick terminal (`quick-terminal-*`, `keybind = global:...=toggle_quick_terminal`) work unchanged, within the v1 limits in §2.
- If another app already holds the hotkey, the user is told once, and fixing the config works without restarting.
- A hidden quick terminal costs no CPU: no timers and no frames while hidden.

**Non-goals (v1)**
- `left`, `right` and `center` positions.
- Starting at sign-in.
- More than one quick terminal, or one per monitor.
- Global hotkeys for any action other than `toggle_quick_terminal`.

## 2. Configuration

These follow Ghostty's names and value syntax.

| Key | Default | Values |
| --- | --- | --- |
| `keybind = global:<trigger>=toggle_quick_terminal` | `global:super+backquote=toggle_quick_terminal` | Any trigger of the existing keybind syntax, prefixed with `global:`. `global:super+backquote=unbind` removes the default. |
| `quick-terminal-position` | `top` | `top`, `bottom`. `left`, `right` and `center` are diagnostics ("not supported yet; using top"). |
| `quick-terminal-size` | `40%` | One or two sizes separated by `,`, each `N%` or `Npx`. The first is the primary axis (height for top and bottom); the second is the width, defaulting to the full work area. A bare number is a diagnostic, as in Ghostty. |
| `quick-terminal-screen` | `main` | `main` (the primary monitor) or `mouse` (the monitor under the pointer when the hotkey fires). Ghostty's `macos-menu-bar` is read as `main`. |
| `quick-terminal-autohide` | `true` | `true`, `false`. |
| `quick-terminal-animation-duration` | `0.2` | Seconds, `0` to `1`. `0` means no slide. Windows' "Animation effects" being off also means no slide. |

- **Parsing.** `global:` is a keybind prefix. Of the global bindings, only `toggle_quick_terminal` is accepted in v1; any other global action is a diagnostic. When several global toggles are given, the last one wins, as with any keybind.
- **The hotkey.** It is turned into a Win32 hotkey: `super`, `ctrl`, `alt` and `shift` become `MOD_WIN`, `MOD_CONTROL`, `MOD_ALT` and `MOD_SHIFT`, plus `MOD_NOREPEAT`.
  - A named key (`backquote`, `grave_accent`, `f1`…`f24`, `space`, `enter`, `tab`, `escape`) maps to its virtual-key code. `backquote` and `grave_accent` mean `VK_OEM_3`.
  - A single character goes through `VkKeyScanW` for the current layout.
  - A physical `[Code]` uses the scan-code table the keymap already has.
  - A key with no mapping is a diagnostic.
- **The settings panel.** It gains two rows on the Behaviour page: the position (a choice of top or bottom) and autohide (a toggle). The hotkey and the size stay text in the config file; the panel has no text-field kind.
- **Reload.** Every key above applies on reload. The process re-registers the hotkey when it changes, moves and resizes the window if it is shown, and exits when no global toggle is left (§3.1).

## 3. Process and window

### 3.1 The quick-terminal process

- **Start.** `wraith.exe --quick-terminal` is a client process like any other. Every normal client launch starts it after attaching to the server, when the config has a global toggle and no quick-terminal process holds the mutex. It is started detached, outside the client's job, the same way `link::spawn_server` starts the server, with the same environment rules.
- **One per user and pipe.** It is single-instance through a named mutex, `Local\wraith-quick-<hash of the pipe name>`. A second `--quick-terminal` that cannot take the mutex exits at once. Private test pipes therefore get their own quick terminal.
- **Start-up.** It creates the window, the GPU device, the fonts, the swap chain and the server link as `app::run` does, then leaves the window hidden and registers the hotkey with `RegisterHotKey(hwnd, 1, ...)`.
- **Hotkey taken.** If `RegisterHotKey` fails, it shows one Windows notification through the existing notifier: "Wraith quick terminal: <key> is in use by another app. Set another key with `keybind = global:...=toggle_quick_terminal`." It keeps running and tries again on each config reload.
- **Exit.** It exits when:
  - the server stops or is lost, the same as a normal window today;
  - a reload leaves no global toggle;
  - `wr ctl server-stop` stops the server.

### 3.2 The window

- **Created hidden.** It is created with `WS_EX_TOOLWINDOW` (no taskbar button, not in Alt+Tab) and `WS_EX_TOPMOST`. The tab bar, tabs, sidebar key, palette and settings work as in any window. The caption buttons stay: minimize and close hide it, and maximize does nothing.
- **Rectangle.** A pure function computes it: `quick_rect(work_area, position, size, dpi) -> RECT`. It uses the monitor's work area, so the taskbar is not covered. `top` anchors to the top edge and `bottom` to the bottom edge. The secondary axis is centred when it is narrower than the work area.
- **The monitor.** `main` is the primary monitor. `mouse` is `MonitorFromPoint(GetCursorPos())`. The rectangle is recomputed on every summon, so a monitor or DPI change applies on the next one.
- **Showing.**
  - It resizes the swap chain to the target size while still hidden, renders a frame, and only then shows the window, so the first visible frame is complete. This reuses the composition first-frame logic from 0.4.0.
  - The slide moves the window only (`SetWindowPos` with `SWP_NOSIZE`), from just off the edge to its place over the animation duration, eased with the existing `motion::ease_out`. The swap chain is never resized during a slide.
  - It then calls `SetForegroundWindow`. Windows allows this inside `WM_HOTKEY` handling.
- **Hiding.**
  - The same slide runs in reverse, then `SW_HIDE`. Focus returns to the window that had it, which Windows does by itself on hide.
  - While hidden, the blink, spinner and frame timers do not run.

### 3.3 Toggle rules

On `WM_HOTKEY`:
- **Hidden:** show.
- **Shown and the foreground window:** hide.
- **Shown but not the foreground window:** bring to the front.

Also:
- **Losing focus** (`WM_ACTIVATE` with `WA_INACTIVE`) hides it when `quick-terminal-autohide` is on. Exceptions: focus moving to its own popups (the IME candidate window, message boxes) and the moment during a slide.
- **Closing the window** (the close key, Alt+F4) hides it. It does not close the process.
- **Closing the last tab** hides it. The window's existing rule gives its empty workspace a new shell, so the next summon shows a fresh tab with the default profile.

## 4. The quick workspace

- **The flag.** `Workspace` gains `quick: bool`, with `#[serde(default)]` so older saved layouts still load. The default name is "Quick".
- **The protocol.** A new message, `ClientMsg::AttachQuick`, asks for the quick workspace. `AttachWindow` is unchanged. The wire protocol version goes up by one (4 to 5), and the existing version-mismatch prompt handles an old server.
- **Server rules:**
  - `AttachQuick` takes the workspace flagged `quick`, or creates one with the flag set.
  - `most_recent_detached()` skips quick workspaces, so a normal window never picks it up by itself.
  - Asking for the quick workspace by id from a normal window works under the existing rules: it is refused while the quick terminal holds it, and allowed when the quick terminal is not running.
  - The flag is saved with the layout, so the quick workspace and its tabs survive a server restart like any workspace.
- **In normal windows.** It is listed in the sidebar like any workspace, held by the quick terminal.

## 5. Errors and edge cases

| Case | Behaviour |
| --- | --- |
| The hotkey is held by another app | One notification (§3.1); the process waits for a reload. |
| The hotkey has no virtual-key mapping | A config diagnostic; no registration. |
| The monitor is removed while shown | `WM_DISPLAYCHANGE` recomputes the rectangle on the primary monitor. |
| A DPI change while shown | Handled as in a normal window, then the rectangle is recomputed. |
| The server restarts | The process exits like a window. The next normal launch starts it again. |
| The user starts `wraith --quick-terminal` by hand | Same as an automatic start; the mutex keeps it single. |
| Full-screen or exclusive-mode apps | Windows delivers the hotkey anyway. The quick terminal is topmost but may draw under exclusive full-screen games; not handled in v1. |

## 6. Testing

- **Unit tests:**
  - **config:** the `global:` prefix, `toggle_quick_terminal`, the default binding and `unbind`, every `quick-terminal-*` value and its diagnostics, and the size syntax (`40%`, `300px`, `50%,800px`, a bare `40` rejected).
  - **client, the hotkey:** a trigger turned into `(modifiers, vk)` for `super+backquote`, `ctrl+shift+f12`, a character, and an unmappable key.
  - **client, `quick_rect`:** top and bottom, percent and pixels, a second-axis width, a work area offset by a left or top taskbar, and 150% DPI for pixel sizes.
  - **client, the toggle:** the decision as a pure function of `(visible, foreground)`, and the slide positions at t = 0, the middle and the end, and with zero duration.
  - **server:** the quick attach creates the workspace, then reuses it; `most_recent_detached` skips it; the flag survives `to_json` / `from_json`; an old layout without the field loads.
- **Live check, without stealing focus.** On a private pipe with a fake config:
  - `wraith --quick-terminal` runs on that pipe.
  - Toggling is done by posting `WM_HOTKEY` to its HWND. That can bring it to the foreground, so the check runs once, with the user told first, or verifies visibility and the rectangle with `IsWindowVisible` and `GetWindowRect` only.
  - No real global keypresses are sent. Results are recorded in `docs/quick-terminal-acceptance.md`.
