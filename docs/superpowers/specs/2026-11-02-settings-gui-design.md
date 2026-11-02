# Wraith settings GUI — design

Status: implemented (2026-11-02); see `docs/settings-acceptance.md`
Parent spec: `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md` (§7 configuration; §10 backlog "settings GUI")

## 1. Intent

**Goal.** A settings page for the common settings, where each change shows in the window at once. The text config stays the single source of truth; the GUI is a friendlier way to edit it.

**Decided with the user**
- Scope: common settings, live. Rarer settings (keybinds, workspaces, profiles, `agent-detect`, palette, font features) stay in the text file, reached by an "Open config file" button.
- Saving: edit `%APPDATA%\wraith\config` in place. Only the changed key's line is rewritten or appended; every other byte stays as the user wrote it.
- Timing: at once, like Windows Settings. There is no Save button. Text input commits on Enter or when focus leaves the field.
- Build: drawn in the Wraith window by Wraith's own renderer, like the command palette.

**Success criteria**
- Every row's change reaches the window through the existing hot reload, within one debounce (200 ms).
- A GUI change alters the config file by exactly the lines for that key. A diff of the file shows nothing else.
- Opening the panel and moving around in it costs no input latency (parent spec §6.3). It draws nothing when it is closed.
- Every row works with the keyboard alone.

**Non-goals**
- Editing keybinds, workspaces, profiles, `agent-detect`, `palette`, `font-feature`, `command` or the individual colour keys.
- Editing included files (`config-file`).
- A separate window. Screen-reader access (the rest of Wraith's chrome has none either).

## 2. Opening and layout

- **Opening.** There is a new action `open_settings`, bound by default to `ctrl+comma`, which is free. It is listed in the palette under Window. A gear button sits left of the minimize caption button, with the tooltip "Settings  Ctrl+,". `Esc`, the gear or `ctrl+comma` closes the panel.
- **Panel.**
  - Centred over the window, about 640 px wide (capped at the client width minus 2 cells), and up to 80% of the window's height.
  - A page column on the left: Appearance, Behaviour, Agents. Rows on the right, scrolling when they don't fit.
  - It fades in from 40% like the palette and closes instantly. The terminal behind it keeps drawing, so changes are visible as they happen.
- **Rows.** Each row has a label, a control, and a reset dot.
  - The dot shows when the value in effect differs from the default.
  - Clicking the dot, or pressing `Delete` on the focused row, removes the key's lines from the file.
- **Footer.** An "Open config file" button opens `%APPDATA%\wraith\config` in the program registered for `.txt` files (looked up with `AssocQueryStringW`), falling back to `notepad.exe`. The file has no extension, so opening it directly would bring up "How do you want to open this file?". The file is created first if it is missing.

### 2.1 Rows

| Page | Label | Key | Control | Values written |
|---|---|---|---|---|
| Appearance | Theme | `theme` | searchable list of bundled themes | theme file name |
| | Font | `font-family` | searchable list of installed monospace families | family name |
| | Font size | `font-size` | stepper 6–36, step 1 | integer |
| | Background opacity | `background-opacity` | slider 0.30–1.00, step 0.05 | two decimals |
| | Cursor | `cursor-style` | choice: block, bar, underline, block_hollow | as listed |
| | Cursor blink | `cursor-style-blink` | toggle | `true` / `false` |
| | Padding, horizontal | `window-padding-x` | stepper 0–32 | integer |
| | Padding, vertical | `window-padding-y` | stepper 0–32 | integer |
| | Dim unfocused splits | `unfocused-split-opacity` | slider 0.15–1.00, step 0.05 | two decimals |
| Behaviour | Copy on select | `copy-on-select` | toggle | `true` / `false` |
| | Right-click | `right-click-action` | choice: paste, menu | as listed |
| | When a shell exits | `close-on-exit` | choice: clean, always, never | as listed |
| | Default profile | `default-profile` | list of detected and configured profiles | profile name |
| | Scrollback | `scrollback-limit` | choice: 10 MB, 50 MB, 100 MB, 500 MB | bytes (10000000 …) |
| | Show sidebar | `sidebar-visible` | toggle | `true` / `false` |
| | Sidebar width | `sidebar-width` | stepper 10–60 | integer |
| Agents | Notify by | `agent-notify` | four toggles: toast, flash, badge, sound | one line, comma list; all off writes `none` (§4) |
| | Notify | `agent-notify-when` | choice: when unfocused, always | `unfocused` / `always` |
| | Quiet hours | `agent-notify-quiet` | toggle + two HH:MM fields | `HH:MM-HH:MM`; off removes the key |
| | Pulse tabs that need me | `agent-tab-pulse` | toggle | `true` / `false` |

A value outside a control's range that came from the file is shown as it is. The control's first step snaps it into range.

## 3. Architecture

### 3.1 `config::edit` (config crate, pure)

Text edits on one file's contents. They never reorder, reflow or re-encode.

- `set(src: &str, key: &str, value: &str, which: Which) -> String`
  - `Which::Last` (single-valued keys) rewrites the last `key = …` line. `Which::First` (list keys: `font-family`) rewrites the first, keeping later lines as fallbacks.
  - If the key has no line, it appends `key = value` on a new line. A missing final newline is added first.
- `unset(src: &str, key: &str) -> String` removes every `key = …` line.
- The file keeps its line ending (CRLF if any line has one, else LF) and its UTF-8 BOM if it had one. The rest of a rewritten line is replaced; comments on other lines are untouched. (Ghostty syntax has no trailing comments.)
- `origin(path: &Path, key: &str) -> Origin` says where the effective value comes from, following `config-file` includes in load order: `Origin::Here { line }`, `Origin::Included { file, line }` or `Origin::Default`.

### 3.2 `config::settings` (config crate, pure)

The row table of §2.1 as data. Each `Setting` has:
- `key`, `label`, `page`, `which`
- `kind`: `Toggle`, `Choice(&[(&str, &str)])`, `Stepper { min, max, step }`, `Slider { min, max, step }`, `List(ListSource)`, `NotifySet` or `QuietHours`
- `get(&Config) -> String`, the current value as the text it would be written as

Values are checked only by the existing parser. The GUI writes text and the file reload parses it.

### 3.3 `client::settings` (client crate, plain state)

- `SettingsView` holds: `page`, `focus` (row index), `scroll`, an open list `{ row, query, selected, before }`, and a text edit `{ row, text }`.
- `fn key(&mut self, ev, cfg: &Config) -> Option<Edit>` and `fn click(&mut self, hit, cfg) -> Option<Edit>`, where `Edit = Set { key, value, which } | Unset { key } | Preview { key, value } | Revert`.
- Lists reuse `palette::fuzzy` for filtering. Theme names come from the themes folder (`app::themes_dir`). Fonts come from DirectWrite's system collection, keeping families whose regular face reports `IsMonospacedFont`. Profiles come from `SpawnCtx.profiles`.

Drawing lives in `app.rs`, next to the palette. It uses `Painter`, M6's hover washes and focus styling, `Motion` (a new `settings` fade) and tooltips. Hit testing adds `Hit::Settings*` variants in `chrome.rs`.

### 3.4 Write path

1. `Edit::Set` or `Edit::Unset`:
   1. Read the config file fresh (missing → empty plus the header `# Wraith config — edited by hand and by Settings (ctrl+comma)`).
   2. Apply `config::edit`.
   3. Write to `config.tmp` beside it and rename over it (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`).
   4. Mark the next reload as self-made, so the "config reloaded" toast is not shown for it.
2. The existing watcher reloads within the 200 ms debounce, and every window applies the new `Config` as for a hand edit.
3. `Edit::Preview` (moving through the theme or font list) applies `config::edit::set` to the file text in memory and parses the result with the normal loader, so the parser stays the only thing that interprets values. The app then applies that `Config` and re-renders. Nothing is written. `Edit::Revert` (Esc in a list) re-applies the `Config` from before the list opened.

## 4. Edge cases

- **The file is changed by hand while the panel is open.** The reload refreshes the panel's values. Every write reads the file fresh, so it never writes back a stale copy.
- **A write fails** (locked, read-only, or the folder is missing and can't be created): nothing is renamed, the row shows the value in effect again, and a toast says "Couldn't save settings: <error>". It is throttled like other toasts.
- **The key comes from an included file** (`Origin::Included`): the row is read-only, shows "Set in <file name>", and offers Open. The reset dot is hidden.
- **The key appears more than once in this file:** the row edits the line that wins (§3.1 `Which`).
- **A bad hand-written value:** the parser keeps the default and reports a diagnostic. The row shows the value in effect, plus a warning mark whose tooltip is the parser's message. A GUI change replaces that line.
- **Notify by, with all four off:** `agent-notify` has no "none" value today (empty resets to the default). The GUI writes `agent-notify = none`, and the parser gains `none` meaning all off. This is the only parser change.
- **Quiet hours with start equal to end:** rejected in the GUI; the end field shows an error mark and nothing is written.
- **Reduced motion** (Windows "Show animations" off): the panel appears and disappears instantly.

## 5. Keyboard

| Key | In the panel |
|---|---|
| `Tab` / `Shift+Tab` | Next / previous page |
| `↑` / `↓` | Previous / next row |
| `←` / `→` | Change the value: step, slide, or move through a choice |
| `Space` | Toggle |
| `Enter` | Open a list, or commit the selection; commit a text field |
| typing | Filters an open list; edits a focused time field |
| `Delete` | Reset the focused row to its default |
| `Esc` | Close an open list (reverting its preview), else close the panel |

While the panel is open, keys go to it, never to the terminal. The palette's open and close actions still work.

## 6. Testing

- **`config::edit`, unit tests:** replacing the last of several lines; replacing the first line for list keys; appending to a file with and without a final newline; `unset` of one and of several lines; CRLF and BOM preserved; comment lines untouched; an empty file; `origin` through one and through two levels of includes.
- **`config::settings`:** for every row, the control's values written with `set` into an empty file and parsed with `parse_str` give the expected `Config` field. `get` round-trips (writing `get(c)` and parsing yields `c`'s field).
- **Parser:** `agent-notify = none` turns all four ways off.
- **`client::settings`, view tests:** page and row navigation; stepper and slider limits; choice wrapping; list filtering; preview then revert on Esc; reset gives `Unset`; read-only rows give no edits; the quiet-hours check.
- **Write path:** atomic write into a temp folder; a read-only file gives an error and leaves the file unchanged; a missing file is created with the header.
- **Live check:** on a private pipe and config, with the window shown without activation, open the panel with posted keys and change the theme, font size and copy-on-select. The file's diff shows exactly those three lines, and the window redraws with the new theme.
