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

## ConPTY (bundled: Microsoft.Windows.Console.ConPTY 1.25.260930003)

`conpty.dll` + `OpenConsole.exe` from NuGet (MIT, Authenticode-valid, signed by Microsoft Corporation). Called through `ConptyCreatePseudoConsole`.

| Sequence | flags=0 native | flags=0 WSL | **flags=8 (passthrough) native** | **flags=8 WSL** |
|---|---|---|---|---|
| Kitty graphics APC | Passed | Stripped | **Passed** | **Passed** |
| OSC 9 / 777 / 133 / 7 | Passed | Passed | **Passed** | **Passed** |

The bundled OpenConsole sends `ESC[1t` and `ESC[c` (a DA1 query) at startup. The pane's `on_device_attributes` handler must answer DA1.

## Decision

- **Passive agent detection (OSC 9/777/133/7): GO.** ConPTY passes these through, both native and WSL.
- **Inline images (Kitty graphics): GO with bundled ConPTY in passthrough mode (flags = 8).** NO-GO on inbox ConPTY, which strips the APC.
  Proposed: ship `conpty.dll` + `OpenConsole.exe` next to `wraith.exe` and always create panes with `ConptyCreatePseudoConsole(..., 8, ...)`. Inbox ConPTY is the fallback, without images, if the DLL is missing. Pending user approval of the spec change.

M1 (the single-pane terminal) does not depend on this decision.
