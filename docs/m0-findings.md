# M0 findings — libghostty-vt + ConPTY on Windows

Date: 2026-11-01 · Machine: Windows 11 build 26300.9711, x86_64-pc-windows-msvc, Rust 1.99.1

## libghostty-vt

| Check | Result |
|---|---|
| `libghostty-vt =0.2.2` builds on MSVC | **Pass**, with **Zig 0.15.2** (the pinned Ghostty commit requires 0.15.x; 0.16/0.17 fail). Static link. |
| `vt_write` → grid via `RenderState` row/cell iterators | Pass |
| Dirty tracking after write | Pass |
| `key::Encoder` Ctrl+C → `0x03` | Pass |
| `on_pty_write` DSR reply `ESC[1;1R` | Pass |

API notes: `TerminalOptions.max_scrollback` counts **lines**. `key::Key` variants are `A..Z`, not `KeyA`. The upstream repo's `master` API is newer than 0.2.2 (e.g. `Terminal::new(cols, rows)`), so we stay pinned to 0.2.2 and follow docs.rs, not the README.

## ConPTY (inbox `kernel32!CreatePseudoConsole`)

| Sequence | Native process | WSL (Ubuntu 22.04) |
|---|---|---|
| Kitty graphics APC `ESC _ G … ESC \` | **Stripped** | **Stripped** |
| OSC 9 notification | Passed | Passed |
| OSC 777 notify | Passed | Passed |
| OSC 133 prompt mark | Passed | Passed |
| OSC 7 cwd | Passed | Passed |

ConPTY drops the APC entirely. The reader only sees ConPTY's own startup modes (`ESC[?9001h ESC[?1004h`).

Job Object: closing a pane kills descendants that have their own console (`start "" /min`). Verified with a mutation check (the test fails with the kill disabled).

## Decision

- **Passive agent detection (OSC 9/777/133/7): GO.** ConPTY passes these through, both native and WSL.
- **Inline images (Kitty graphics): NO-GO on inbox ConPTY.** Pending the user's decision on the next step:
  1. **Bundle Microsoft's ConPTY** (`Microsoft.Windows.Console.ConPTY` NuGet 1.25.260930003, MIT, Authenticode-signed by Microsoft: `conpty.dll` + `OpenConsole.exe`), the same approach WezTerm and Windows Terminal use. It is not yet tested whether its OpenConsole passes APC. The test run was blocked by the session's permission policy, because it executes a downloaded binary.
  2. If the bundled ConPTY also strips APC: a ConPTY bypass for WSL/SSH children. This is a large redesign (spec §11 risk 4).
  3. Ship without inline images.

M1 (the single-pane terminal) does not depend on this decision.
