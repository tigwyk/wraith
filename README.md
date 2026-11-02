<!-- LOGO -->
<h1>
<p align="center">
  <img src="assets/icon/wraith.svg" alt="Wraith, a four-pane ghost" width="128">
  <br>Wraith
</h1>
  <p align="center">
    A fast, native Windows terminal built on <code>libghostty-vt</code>.
    <br />
    Tabs, splits and workspaces, with shells that outlive the window and agents that ask for you.
    <br />
    <a href="https://tigwyk.github.io/wraith/">Website</a>
    ·
    <a href="#about">About</a>
    ·
    <a href="https://github.com/tigwyk/wraith/releases/latest">Download</a>
    ·
    <a href="#documentation">Documentation</a>
    ·
    <a href="#building">Building</a>
  </p>
</p>

## About

Wraith is a terminal emulator for Windows 10 1809+ and Windows 11 (x64).
It pairs Ghostty's terminal core, [`libghostty-vt`](https://docs.rs/libghostty-vt),
with a window written in Rust on raw Win32, Direct3D 11 and DirectWrite.
Wraith aims to replace Windows Terminal as a daily driver: as fast as
Alacritty, as faithful as Ghostty, and native to Windows.

- **Sessions that survive the window.** A background server owns every shell.
  Close the window, open it again, and your shells and screens are still there.
  Scrollback is kept across a server restart too.
- **Workspaces, tabs and splits.** Each workspace groups its own tabs. Panes
  split in any direction. The layout comes back after a restart.
- **Coding agents that get your attention.** Wraith recognises Claude Code,
  Codex, Gemini CLI, Aider, OpenCode, Copilot CLI, cursor-agent, Amp and Goose.
  Each one gets an icon on its tab. When an agent in a hidden tab needs you,
  Wraith shows a toast, flashes the taskbar and badges the tab within a second.
- **Inline images.** Photos and screenshots render in the terminal through the
  Kitty graphics protocol (`kitten icat`, `chafa`, `yazi`, and agents that show
  screenshots).
- **Always fast.** Keystrokes are written before output is parsed, frames only
  re-shape what changed, and memory stays bounded. Typing stays quick after
  millions of lines of output, even while another pane floods.
- **Ghostty-style config.** Same file syntax and key names, the whole Ghostty
  theme set, keybind sequences, and hot reload.

## Download

Get the zip from the [latest release](https://github.com/tigwyk/wraith/releases/latest),
unpack it, and install for your user (no admin needed):

```powershell
.\install.ps1
```

This copies Wraith to `%LOCALAPPDATA%\Programs\Wraith`, puts `wr` on your
`PATH`, and adds a Start Menu shortcut (which Windows needs for toasts).
`.\install.ps1 -DryRun` shows what it would do first, and `uninstall.ps1`
removes it again.

## Documentation

### Configuration

Wraith reads `%APPDATA%\wraith\config`, written in Ghostty's syntax:

```ini
font-family = Cascadia Mono
font-size = 13
theme = Catppuccin Mocha
background-opacity = 0.95
keybind = ctrl+a>c=new_tab
workspace = api:C:\src\api
agent-tab-pulse = true
```

Saving the file applies it at once. A bad line keeps its default and shows a
toast naming the file and line. `wr config check` reports every error and
exits 1 if there are any.

`Ctrl+,` (or the gear in the tab bar) opens Settings: theme, font, opacity,
cursor, padding, behaviour and agent notifications. Each change shows at once
and rewrites only that key's line in the file. Keybinds, workspaces, profiles
and the other rarer keys stay in the text file, one click away.

### Keys

| Keys | Action |
| --- | --- |
| `Ctrl+Shift+T` / `Ctrl+Shift+W` | New tab / close pane |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Alt+Shift+Plus` / `Alt+Shift+Minus` | Split right / split down |
| `Alt+Arrow` | Move focus between panes |
| `Ctrl+Shift+P` | Command palette (every action, with its key) |
| `Ctrl+,` | Settings |
| `Ctrl+Shift+O` | Workspace picker |
| `Ctrl+Shift+F` | Find in scrollback |
| `Ctrl+Shift+B` | Show or hide the sidebar |
| `Ctrl+Shift+N` | New window |
| `Ctrl+Shift+F12` | Frame-time and latency overlay |

### Coding agents

Wraith spots agents on its own by their process and terminal signals. Hooks
make detection exact. These commands show the change, ask before writing, and
keep a backup:

```powershell
wr install-hooks claude    # Claude Code (~/.claude/settings.json)
wr install-hooks codex     # Codex CLI (~/.codex/config.toml)
```

Any other tool or script can report its state with
`wr hook generic --state working|ask|turn-end|idle|detached`.

### The `wr` command

| Command | Does |
| --- | --- |
| `wr ctl list` | List panes with their workspace, state and title |
| `wr ctl send-text <pane> <text>` | Type into a pane |
| `wr ctl kill <pane>` / `wr ctl server-stop` | Close a pane / stop the server and its shells |
| `wr events --json` | Stream workspace, pane and agent changes as JSON lines |
| `wr config check [path]` | Check a config file |

The full design lives in the
[spec](docs/superpowers/specs/2026-11-01-wraith-terminal-design.md).

## Building

Wraith needs Rust stable with the MSVC toolchain, and Zig 0.17.x (which builds
the vendored `libghostty-vt`; `scripts\check-zig.ps1` checks the version).

```powershell
cargo build --release      # target\release\wraith.exe and wr.exe
cargo test --workspace
scripts\dist.ps1           # dist\wraith-<version>-x64.zip
```

## Roadmap and Status

Wraith is the author's daily driver and is still young.

|  #  | Milestone                                                              | Status |
| :-: | ---------------------------------------------------------------------- | :----: |
|  0  | `libghostty-vt` on Windows; ConPTY passes Kitty graphics and OSC intact |   ✅   |
|  1  | One-pane terminal: renderer, input and IME, selection, config, themes  |   ✅   |
|  2  | Server and client: shells survive the window                           |   ✅   |
|  3  | Workspaces, tabs, splits, sidebar, palette, layouts, inline images     |   ✅   |
|  4  | Coding-agent awareness: hooks, detection, toasts, taskbar              |   ✅   |
|  5  | Scrollback search and persistence, packaging                           |   ✅   |
|  6  | Hover feedback and motion polish                                       |   ✅   |
|  7  | `wr events`, `wr config check`, right-click menu, inline IME           |   ✅   |
|  –  | Settings GUI                                                           |   ✅   |
|  –  | Drop-down (quake) window, auto-update, ARM64                           |   ❌   |
