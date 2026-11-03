# Wraith Windows Terminal profiles and new-tab menu — design

Status: implemented (2026-11-02); see `docs/wt-profiles-acceptance.md`
Parent spec: `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md` (§4.3 profiles, §7 configuration)

## 1. Intent

**Goal.** Someone coming from Windows Terminal (WT) opens Wraith and finds the shells they already use, in the same order, with the same colours, one click away from the `+` button.

**Decided with the user**
- Import works both ways: Wraith reads WT's settings live, and a one-time import writes the profiles into Wraith's config for people who want to stop depending on WT.
- Per-profile appearance is wanted. It is split in two:
  - **This spec** carries the colour scheme per profile.
  - **Spec 2 (later)** carries font face, size, padding and opacity per profile. Wraith has one glyph atlas and one cell size per window, so per-pane fonts are a renderer change of their own.
- The `+` button becomes a split button with a profile menu, as in WT.

**Success criteria**
- With WT installed and no Wraith `profile` lines, the `⌄` menu lists exactly WT's visible, launchable profiles, in WT's order, and each one starts the same command WT would start.
- A tab opened from a WT profile with a `colorScheme` shows that scheme. Other panes keep the global theme.
- `wr import-wt` followed by uninstalling WT gives the same menu and colours. Running it twice changes the config once.
- Opening, moving through and closing the menu costs no input latency (parent spec §6.3). It draws nothing when closed.
- The menu works with the keyboard alone.

**Non-goals**
- Font face, font size, padding, opacity, cursor shape, background images, acrylic, `antialiasingMode` per profile (spec 2 or never).
- Running Azure Cloud Shell or any profile whose command only WT can produce. These are skipped and logged.
- WT `actions`/keybindings, `newTabMenu` folders and match rules, and writing back to WT's files.
- Shift+click to open a profile in a new window.

## 2. Reading Windows Terminal's settings

A new module, `wraith_config::wt`. Its parser is pure: it takes file contents, not paths, so it is tested with fixtures. A thin loader finds and reads the files.

### 2.1 Files

- **Settings.** The first file that exists, in this order:
  1. `%LOCALAPPDATA%\Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json` (Store, stable)
  2. `%LOCALAPPDATA%\Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json` (Store, Preview)
  3. `%LOCALAPPDATA%\Microsoft\Windows Terminal\settings.json` (unpackaged)
- **Fragments.** Every `*.json` one level below each app folder in `%LOCALAPPDATA%\Microsoft\Windows Terminal\Fragments\` and `%ProgramData%\Microsoft\Windows Terminal\Fragments\`. The app folder name is the profile's `source` (for example `Microsoft.WSL`).
- **Format.** WT's files are JSON with `//` and `/* */` comments and trailing commas. A small pre-pass removes these, without touching text inside strings, then `serde_json` (already in the lockfile) parses the result. A file that does not parse is skipped with a warning, and Wraith carries on with its own profiles.

### 2.2 Resolving a profile

For each entry of `profiles.list` (or `profiles` when it is a plain array), in order:
1. **Merge.** Start from `profiles.defaults`. A fragment profile with the same `guid` (or a fragment `updates` entry naming it) fills keys the list entry does not set. The list entry's own keys win. Fragment profiles not in the list are appended after the list, in file order.
2. **Hidden.** `hidden: true` skips the profile.
3. **Command.** The first that applies:
   - `commandline`, with `%VAR%` expanded from the environment (an unknown variable is left as written);
   - a built-in guid: `{61c54bbd-c2c6-5271-96e7-009a87ff44bf}` → `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`, `{0caa0dad-35be-5f56-a8ff-afceeeaa6101}` → `%SystemRoot%\System32\cmd.exe`;
   - by `source`: `Windows.Terminal.Wsl` → `wsl.exe -d <name>`; `Windows.Terminal.PowershellCore` → `pwsh.exe`.
   - Otherwise the profile is skipped and logged (`Windows.Terminal.Azure`, `Windows.Terminal.VisualStudio` without a command line, and unknown sources).
4. **Kept fields.**
   - `name` (required; an entry without one is skipped).
   - `startingDirectory`, with `%VAR%` expanded. `%USERPROFILE%` and `~` are accepted.
   - `icon`: see §5.3.
   - `colorScheme`: a name, or `{ "light": …, "dark": … }`, from which the dark name is taken.
   - `tabTitle`, used as the initial tab title.
- **Default.** The top-level `defaultProfile` is a guid or a name. It resolves to the profile's name.

### 2.3 Colour schemes

A scheme name is looked up first in WT's `schemes` array, then by name in Wraith's themes folder (which already ships Campbell, One Half Dark and the other WT built-ins). A WT scheme gives `foreground`, `background`, `cursorColor` and the 16 colours `black` … `brightWhite`. A name found in neither place is ignored with a warning, and the profile uses the global theme.

## 3. The live profile list

- **Order.** Wraith's profile list (`build_spawn_ctx`) becomes:
  1. WT's resolved profiles, in WT's order, when WT's settings were read. Wraith's own detection is then not used: WT already lists its WSL distros and shells, under names that differ from detected ones (`Ubuntu 22.04` versus `Ubuntu2204_New`), so adding both would show each distro twice. Without WT settings, Wraith's detected profiles are used, as today.
  2. the `profile =` lines. A line with a name (any case) already in the list replaces that profile in place; a new name is appended.
- **`Profile` gains** `dir: Option<String>`, `icon: Option<String>`, `scheme: Option<Scheme>` and `title: Option<String>`. `Scheme` holds `fg`, `bg`, `cursor` and `palette: [Rgb; 16]`.
- **Default profile.** `default-profile` wins. If it is unset, WT's `defaultProfile` is used, then the existing fallback (pwsh, then the first profile).
- **New config key.** `windows-terminal-profiles = true|false`, default `true`. With `false`, nothing is read from WT. A config reload rereads the WT files.
- **Starting directory.** A new tab uses the profile's `dir` when it has one; otherwise the existing rule (workspace root, focused pane's cwd, `USERPROFILE`) applies.
- **Colours per pane.** Each pane already records its profile name (`PaneMeta.profile`). When a pane is created, and whenever `resend_colors` runs, the client sends that profile's scheme through the existing `PaneCmd::SetColors`. A pane whose profile has no scheme, or whose profile no longer exists, gets the global theme, exactly as today.

## 4. One-time import

- **Entry points.** `wr import-wt`, and the palette command "Import Windows Terminal profiles".
- **What it writes** to `%APPDATA%\wraith\config`, through `wraith_config::edit` (comments, order, line endings and encoding are kept):
  - one `profile = <name>:<command>` per resolved WT profile;
  - `profile-dir = <name>:<path>` when the profile has a starting directory;
  - `profile-theme = <name>:<scheme>` when it has a scheme;
  - `profile-icon = <name>:<icon>` when it has an icon that is a file path or an emoji;
  - `default-profile = <name>` when the config has none;
  - `windows-terminal-profiles = false`, last.
- **Schemes.** A scheme that exists only in WT is written to `%APPDATA%\wraith\themes\<name>` as a Ghostty theme file (`palette = N=#rrggbb`, `foreground`, `background`, `cursor-color`). An existing theme file of that name is left alone.
- **Running it twice.** A profile whose name already has a `profile` line is skipped, with all its other lines. The second run writes nothing and reports "0 profiles imported".
- **Output.** The CLI prints one line per profile (imported, skipped because it exists, or skipped because WT alone can run it) and a total. The palette command shows the total as a toast.
- **New config keys** `profile-dir`, `profile-theme` and `profile-icon` take `name:value`. They are also usable by hand. A key naming an unknown profile is a diagnostic, like other bad lines.
- **Failure.** If WT's settings are not found or do not parse, nothing is written, and the error is printed or shown as a toast.

## 5. The `+` menu

### 5.1 Button

- The `+` button becomes a split button: `+` and `⌄`, each a cell wide, sharing one hover wash per half.
- `+` opens a new tab with the default profile, as today. Its tooltip is unchanged.
- `⌄` opens the menu, with the tooltip "Open a profile  Ctrl+Shift+Space". Clicking it again closes the menu.

### 5.2 Menu

- **Placement.** Below the `⌄` button, its left edge aligned with the `+` button. It is moved left to stay inside the window, and scrolls when it is taller than the space below the tab bar.
- **Rows, top to bottom:**
  1. one row per profile, in list order: icon, name, and the binding of `new_tab_profile:N` for that row (by default `Ctrl+Shift+1` … `Ctrl+Shift+9` for the first nine). The default profile's name is bold.
  2. a separator;
  3. Settings, with its binding (`Ctrl+,`);
  4. Command palette, with its binding (`Ctrl+Shift+P`).
- **Mouse.** A click on a profile opens it in a new tab. Alt+click splits the focused pane with that profile instead (the existing split action, along the longer side). Settings and Command palette open those. A click outside, or on `⌄`, closes the menu.
- **Keys.** Up and Down move the selection and wrap. Home and End jump. Enter opens the selection; Alt+Enter splits. Esc closes. While the menu is open, other keys go to the menu, not the terminal.
- **Look.** Drawn by Wraith's renderer, like the palette, with the same panel colours and fade-in. Hovered and selected rows get the palette's selection wash.

### 5.3 Icons

- An icon that is one or two characters long (an emoji or a glyph) is drawn as text, in the UI font with fallback.
- An icon that is a path to a `.png` file is decoded with the existing PNG decoder and drawn as a cell-high image. Decoded icons are cached by path.
- An `ms-appx:///…` icon is resolved against WT's install folder when that folder is readable. The folder is found with `GetPackagesByPackageFamily` and `GetPackagePathByFullName` for the family of the settings file that was read (`Microsoft.WindowsTerminal_8wekyb3d8bbwe` or the Preview family). Unpackaged WT has no `ms-appx` icons to resolve.
- Anything else, including a failed load, is drawn as a tile with the profile name's first letter, coloured by a hash of the name.

### 5.4 Actions and bindings

- `open_new_tab_menu`, default `ctrl+shift+space`. Opens the menu with the default profile selected.
- `new_tab_profile:N` (N = 1…9), defaults `ctrl+shift+1` … `ctrl+shift+9`. It opens the Nth profile of the list. An N beyond the list does nothing.
- The palette lists "New tab: <profile>" for every profile, under Tabs.

### 5.5 Structure

`NewTabMenu` is pure view state owned by `WindowView`, like the palette and `SettingsView`. It has `key`, `click`, `layout` and `hit`, and it returns an outcome (`Open(profile)`, `Split(profile)`, `Settings`, `Palette`, `Close`). `app.rs` draws it and acts on the outcome. Only one overlay is open at a time: opening the menu closes the palette or the settings panel, and the reverse.

## 6. Testing

- **`wt` parser** (`config/tests/wt.rs`), with fixtures `tests/fixtures/wt/settings.json` and a fragment. The fixture is shaped like a real WT file, with comments and trailing commas, and has no personal data. Cases:
  - comments and trailing commas are removed, but not inside strings;
  - defaults merge, and list entries win;
  - a fragment profile is merged by guid, and an unlisted fragment profile is appended;
  - hidden profiles are skipped;
  - each command rule: `commandline` with `%VAR%`, both built-in guids, WSL and PowershellCore sources, and Azure skipped;
  - a light/dark scheme takes the dark name;
  - a scheme is found in WT's `schemes`, then in Wraith's themes, and otherwise ignored;
  - `defaultProfile` resolves from a guid and from a name;
  - a file that does not parse gives an error, not a panic.
- **Profile list** (client tests): the order and name clash rules of §3, no detected profiles when WT was read, `windows-terminal-profiles = false`, and the default-profile fallback.
- **Import** (`config/tests/import.rs` and a CLI test): the lines written, a second run writes nothing, comments are kept, a WT-only scheme is written to the themes folder and an existing one is not overwritten.
- **New config keys:** `profile-dir`, `profile-theme` and `profile-icon` round-trip through the parser, and one naming an unknown profile is a diagnostic.
- **Menu view** (`client/tests/new_tab_menu.rs`): keys (wrap, Home/End, Enter, Alt+Enter, Esc), clicks (a profile, Alt+click, Settings, outside), the bindings shown on the first nine rows, placement moved inside a narrow window, and scrolling in a short one.
- **Colours per pane:** a pane of a profile with a scheme is sent that scheme, and a pane without one is sent the global colours, both on creation and on `resend_colors`.
- **Live check** (`docs/wt-profiles-acceptance.md`), on a private pipe with a fake `APPDATA`, the window shown without activation. It reads the real WT settings file and changes nothing in it. Checks: the menu matches WT's visible launchable profiles, Ubuntu opens with its scheme, and `wr import-wt` into the fake config gives the same menu with `windows-terminal-profiles = false`.

## 7. Review focus

The inputs the tests above are least likely to catch:
1. A WT file being written by WT while Wraith reads it: a partial file must give a parse warning, not an empty or half profile list that sticks.
2. A profile name containing `:`, which `profile = name:command` cannot hold. The import skips it with a message; the live list keeps it.
3. Two WT profiles with the same name: the first wins, and the second is logged.
4. A huge or deeply nested fragment from a third-party installer: parsing must stay bounded and must not stall start-up.
5. Removing a profile from WT while its panes are open: those panes keep running and fall back to the global theme on the next `resend_colors`.
