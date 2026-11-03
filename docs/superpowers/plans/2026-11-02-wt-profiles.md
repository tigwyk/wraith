# Windows Terminal Profiles and New-Tab Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wraith lists Windows Terminal's profiles (live, or imported once), gives each its colour scheme, and opens them from a `+ ⌄` split button and menu.

**Architecture:** A pure parser `wraith_config::wt` turns WT's JSONC settings and fragments into resolved profiles and schemes; the client merges them into `SpawnCtx.profiles` and picks colours per profile. `wraith_config::wt::import` writes them into the Wraith config through `edit`. `NewTabMenu` is pure view state owned by `WindowView`, like `SettingsView`; `app.rs` draws it.

**Tech Stack:** Rust 2024 workspace, `serde_json` (already locked), the existing D3D renderer, `windows` crate for the package lookup.

**Spec:** `docs/superpowers/specs/2026-11-02-wt-profiles-design.md`

## Global Constraints

- Never touch the user's real `%APPDATA%\wraith\config`, `target\release`, or WT's own files (read only). Live checks use a fake `APPDATA`/`LOCALAPPDATA` copy, a private `WRAITH_PIPE_NAME`, `WRAITH_LAYOUT_PATH`, and a window shown without activation; never `scripts/keys.ps1` or `scripts/shot.ps1`.
- Verify with `rtk proxy cargo …` and exit codes; RTK reformats cargo output.
- `cargo clippy --workspace --all-targets -- -D warnings` stays clean; `cargo test --workspace` stays green.
- `wraith-server` does not gain a dependency on `wraith-config`; profile extras cross that boundary as plain strings.
- Every config write goes through `wraith_config::edit` (keeps comments, order, line endings, BOM/UTF-16; atomic).
- New default keybinds: `ctrl+shift+space=open_new_tab_menu`, `ctrl+shift+1` … `ctrl+shift+9` = `new_tab_profile:1` … `:9`.
- New config keys: `windows-terminal-profiles` (bool, default `true`), `profile-dir`, `profile-theme`, `profile-icon` (each `name:value`).
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. WT rewriting `settings.json` while Wraith reads it: a truncated file gives `Err`, the previous/own profiles stay; never an empty list that sticks. (Test in Task 1: `truncated_file_is_an_error`; Task 3: `wt_error_falls_back_to_detected`.)
2. A WT profile name containing `:` cannot be written as `profile = name:command`: import skips it with a reason; the live list keeps it. (Task 4: `colon_in_name_is_skipped`.)
3. Two WT profiles with the same name: the first wins, the second is dropped. (Task 1: `duplicate_names_keep_first`.)
4. A huge or deeply nested fragment: files over 1 MiB are skipped unread, so start-up never stalls. (Task 1: `oversized_fragment_is_skipped`.)
5. A profile removed from WT while its panes are open: those panes get the global colours on the next `resend_colors`. (Task 3: `unknown_profile_gets_global_colors`.)

## Planning notes (rulings against the spec)

- **`tabTitle` is dropped.** `ModelCmd::NewTab` has no title, and the tab id is unknown until the snapshot; adding a title to the protocol is out of proportion. Cost if wrong: WT users see the shell's own title instead of their fixed one.
- **`profile-dir`/`-theme`/`-icon` naming an unknown profile** is a warning in the log when the profile list is built, not a parse diagnostic: the parser cannot know WT or detected profiles. Cost if wrong: a typo is less visible.
- **Split button halves are 2 cells each**, not 1, so each is an easy click target. Cost if wrong: 1 cell more of tab bar.

---

### Task 1: WT settings parser

**Files:**
- Create: `crates/config/src/wt.rs`, `crates/config/tests/wt.rs`, `crates/config/tests/fixtures/wt/settings.json`, `crates/config/tests/fixtures/wt/fragment.json`
- Modify: `crates/config/src/lib.rs` (`pub mod wt;`), `crates/config/src/theme.rs`, `crates/config/Cargo.toml` (`serde_json = "1"`)

**Interfaces:**
- Produces:
  - `pub struct Scheme { pub fg: Rgb, pub bg: Rgb, pub cursor: Option<Rgb>, pub palette: [Rgb; 16] }` (in `wt.rs`, re-exported as `wraith_config::Scheme`)
  - `pub struct WtProfile { pub name: String, pub command: String, pub dir: Option<String>, pub icon: Option<String>, pub scheme: Option<String> }`
  - `pub struct WtSettings { pub profiles: Vec<WtProfile>, pub default: Option<String>, pub schemes: Vec<(String, Scheme)>, pub skipped: Vec<(String, String)> /* name, reason */, pub family: Option<String> /* package family of the file read */ }`
  - `pub fn strip_jsonc(src: &str) -> String`
  - `pub fn parse(settings: &str, fragments: &[(String, String)] /* (source, json) */, env: &dyn Fn(&str) -> Option<String>) -> Result<WtSettings, String>`
  - `pub fn load() -> Option<Result<WtSettings, String>>` — `None` when no WT settings file exists; uses `load_from(local_appdata, program_data)`.
  - `pub fn load_from(local: &Path, program_data: &Path) -> Option<Result<WtSettings, String>>`
  - `pub fn scheme_from_theme(themes_dir: &Path, name: &str) -> Option<Scheme>` (in `theme.rs`, re-exported from `wt`), built by applying the theme to `Config::default()` and taking `foreground`, `background`, `cursor_color`, `palette[..16]`.
  - `impl WtSettings { pub fn scheme(&self, name: &str, themes_dir: &Path) -> Option<Scheme> }` — WT `schemes` first, then `scheme_from_theme`.

- [ ] **Step 1: Write the fixture.** `settings.json` shaped like a real WT file, with `//` and `/* */` comments, trailing commas, a `"//"` inside a string (`"commandline": "cmd.exe /k echo http://x"`), `defaultProfile` as a guid, `profiles.defaults` with `colorScheme: "Campbell"`, and list entries: Windows PowerShell built-in guid without `commandline`; cmd built-in guid without `commandline`; `{"name":"Azure Cloud Shell","source":"Windows.Terminal.Azure"}`; `{"name":"Ubuntu-22.04","source":"Windows.Terminal.Wsl"}`; a hidden profile; a custom profile with `commandline: "%SystemRoot%\\System32\\cmd.exe /k"`, `startingDirectory: "%USERPROFILE%\\src"`, `icon: "🦀"`, `colorScheme: {"light":"One Half Light","dark":"Mine"}`; a duplicate name; `PowershellCore` source. `schemes`: one named `Mine` with all 16 colours. `fragment.json`: `{"profiles":[{"guid":"{11111111-…}","name":"From Fragment","commandline":"bash.exe"}, {"updates":"<guid of the custom profile>","icon":"x.png"}]}`. No personal data.

- [ ] **Step 2: Write the failing tests** in `crates/config/tests/wt.rs`, calling `parse(&fixture, &[("Vendor.App".into(), fragment)], &env)` with `env` mapping `SystemRoot→C:\Windows`, `USERPROFILE→C:\Users\u`:
  - `strip_jsonc_keeps_strings`: `strip_jsonc(r#"{"a":"//x", /* c */ "b":[1,],}// end"#)` parses with `serde_json` to `{"a":"//x","b":[1]}`.
  - `builtin_guids_get_commands`: profile `Windows PowerShell` → `C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe`; `cmd` → `C:\Windows\System32\cmd.exe`.
  - `sources_get_commands`: `Ubuntu-22.04` → `wsl.exe -d Ubuntu-22.04`; the PowershellCore profile → `pwsh.exe`.
  - `azure_is_skipped_with_reason`: not in `profiles`; `skipped` contains `("Azure Cloud Shell", _)`.
  - `hidden_is_skipped`.
  - `env_vars_expand_and_unknown_stay`: custom profile command `C:\Windows\System32\cmd.exe /k`, dir `Some("C:\Users\u\src")`; a `%NOPE%` in a command stays literal.
  - `defaults_merge_and_entry_wins`: a profile without `colorScheme` has `Some("Campbell")`; the custom one has `Some("Mine")` (dark half).
  - `fragment_merges_by_guid_and_appends`: custom profile icon stays `🦀` (list entry wins over `updates`); `From Fragment` is last.
  - `duplicate_names_keep_first`.
  - `default_resolves_from_guid_and_name`: `default == Some("Windows PowerShell")`; with `"defaultProfile":"cmd"` → `Some("cmd")`.
  - `scheme_lookup_order`: `settings.scheme("Mine", themes)` is the WT colours; `"Campbell"` comes from the fixture themes folder (add a `Campbell` theme file to `tests/fixtures/themes` if missing); `"Nope"` → `None`.
  - `truncated_file_is_an_error`: `parse(&fixture[..fixture.len()/2], …)` is `Err`.
  - `load_from_reads_first_file_and_fragments` (temp dirs): Store-stable path wins over unpackaged; a fragment under `Microsoft\Windows Terminal\Fragments\Vendor.App\f.json` is merged; `family == Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe")`.
  - `oversized_fragment_is_skipped`: a 2 MiB fragment file is ignored.
  - `load_from_without_wt_is_none`.

- [ ] **Step 3: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-config --test wt`. Expected: compile error, `wt` module missing.

- [ ] **Step 4: Implement `wt.rs` and `theme::scheme_from_theme`.** `strip_jsonc` is a char scanner tracking "in string" and "escaped"; it drops comments and a `,` followed only by whitespace/comments before `]`/`}`. `parse` deserialises into `serde_json::Value` and walks it (no derive structs: WT has too many optional shapes). Built-in guids and the `source` rules are exactly spec §2.2. `%VAR%` expansion: replace each `%NAME%` for which `env` returns `Some`. `load_from` checks the three paths of spec §2.1 in order, reads fragments one level deep under both `Fragments` roots, skips files over 1 MiB, and records the package family from the path.

- [ ] **Step 5: Run to see them pass.** Run: `rtk proxy cargo test -p wraith-config --test wt`. Expected: all pass.

- [ ] **Step 6: Commit.** `git commit -m "feat(config): parse Windows Terminal settings and fragments"`

### Task 2: Config keys, `edit::append`, and actions

**Files:**
- Modify: `crates/config/src/lib.rs`, `crates/config/src/keys.rs`, `crates/config/src/edit.rs`, `crates/config/src/keybind.rs`, `crates/client/src/palette.rs` (group for new actions)
- Test: `crates/config/tests/parse.rs`, `crates/config/tests/edit.rs`, `crates/config/tests/keybind.rs`, `crates/client/tests/app.rs`

**Interfaces:**
- Produces:
  - `Config.windows_terminal_profiles: bool` (default `true`)
  - `Config.profile_extras: Vec<(String, ProfileExtra, String)>` with `pub enum ProfileExtra { Dir, Theme, Icon }`; a later line for the same (name, kind) replaces the earlier; an empty value for `profile-*` clears that kind for all names (same rule as `profile =`).
  - `pub fn edit::append(src: &str, key: &str, value: &str) -> String` — always appends `key = value` (the `None` branch of `set`, factored out and shared).
  - `Action::NewTabProfile(u8)` (1-based, name `new_tab_profile`), `Action::OpenNewTabMenu` (`open_new_tab_menu`), `Action::ImportWtProfiles` (`import_windows_terminal_profiles`).

- [ ] **Step 1: Write the failing tests.**
  - parse: `windows_terminal_profiles_key` (`false` parses, `maybe` is a diagnostic); `profile_extras_parse` (`profile-dir = Ubuntu:~/src` gives `("Ubuntu", Dir, "~/src")`; a value without `:` is a diagnostic; a name with spaces `My Shell:x` works).
  - edit: `append_adds_line_even_when_key_exists` (two `profile` lines stay, a third appended, CRLF kept).
  - keybind: `new_tab_profile_parses` (`new_tab_profile:3` → `NewTabProfile(3)`, `:0` and missing are errors); `new_defaults_bound` (`ctrl+shift+space` → `OpenNewTabMenu`, `ctrl+shift+1`…`9` → `NewTabProfile(1..=9)`); `all_actions` contains `open_new_tab_menu` and `import_windows_terminal_profiles`.
  - client app (matcher): `ctrl_shift_digit_matches_new_tab_profile` — feed `KeybindMatcher` the `KeyEvent` the translator produces for Ctrl+Shift+1 (code `Digit1`, mods CTRL|SHIFT) and expect `Match::Action(NewTabProfile(1))`; `ctrl_shift_space_matches_menu`.

- [ ] **Step 2: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-config` and `rtk proxy cargo test -p wraith-client --test app`. Expected: compile errors for the new names.

- [ ] **Step 3: Implement.** Keys in `keys::apply` next to `profile`. `append` shares the append branch with `set`. Actions in `Action`, `name()`, `parse_action`, `all_actions` (`(NewTabProfile(1), "new_tab_profile:1")`, `(OpenNewTabMenu, …)`, `(ImportWtProfiles, …)`), `defaults()`. Palette groups: `NewTabProfile`/`OpenNewTabMenu` under Tabs, `ImportWtProfiles` under Window. If the digit test fails because shifted digits arrive as `!`, fix the matcher's key normalisation (match on the physical digit), not the test. Handle the new actions as no-ops in `WindowView::action` for now.

- [ ] **Step 4: Run to see them pass.** Same commands. Expected: pass.

- [ ] **Step 5: Commit.** `git commit -m "feat(config): WT profile keys, edit::append and new-tab menu actions"`

### Task 3: Merged profile list and colours per profile

**Files:**
- Modify: `crates/server/src/profile.rs` (fields + `Default`), `crates/client/src/view.rs` (`SpawnCtx`), `crates/client/src/app.rs` (`build_spawn_ctx`, `resend_colors`)
- Create: `crates/client/src/profiles.rs` (merge), `crates/client/tests/profiles.rs`
- Test: also update struct literals in `crates/client/tests/view.rs`, `crates/server/tests/profile.rs` (`..Default::default()`)

**Interfaces:**
- Consumes: Task 1 `WtSettings`, `Scheme`, `scheme`; Task 2 `Config.windows_terminal_profiles`, `profile_extras`, `NewTabProfile`.
- Produces:
  - `Profile` gains `pub dir: Option<String>, pub icon: Option<String>, pub scheme: Option<String>`; `ShellKind` gets `#[default] Other`; `Profile: Default`.
  - `pub fn profiles::merge(wt: Option<&Result<WtSettings, String>>, detected: impl FnOnce() -> Vec<Profile>, config: &Config) -> Merged` with `pub struct Merged { pub profiles: Vec<Profile>, pub default: Option<String>, pub warnings: Vec<String> }`. Rules = spec §3: WT `Ok` → WT profiles (via `from_config(name, command)` + extras), `detected` not called; WT `None`/`Err` → `detected()` (plus a warning for `Err`); then `profile =` lines replace by name (any case) in place or append; then `profile_extras` apply by name (unknown name → warning); `windows_terminal_profiles = false` → WT ignored. `default` = `config.default_profile`, else WT default.
  - `SpawnCtx` gains `pub schemes: HashMap<String, Scheme>` (only the schemes the profiles name, resolved once) and `pub fn colors(&self, profile: &str) -> ((u8,u8,u8), (u8,u8,u8), Vec<(u8,u8,u8)>)` — the profile's scheme over the global 256 palette (first 16 replaced), else the global colours. `spawn` uses `colors(&p.name)` and `p.dir` (a profile dir, with `~` expanded to `USERPROFILE`, beats the passed cwd for new tabs only — pass it from `new_tab_msg`, not from splits).
  - `WindowView::action(NewTabProfile(n))` opens `ctx.profiles[n-1]` like `NewTab` (out of range: nothing). `new_tab_msg` takes `Option<&Profile>`.

- [ ] **Step 1: Write the failing tests** (`crates/client/tests/profiles.rs`, building `WtSettings` by hand):
  - `wt_profiles_in_order_and_no_detection` (detected closure panics if called).
  - `config_profile_replaces_in_place_and_appends`.
  - `extras_apply_and_unknown_warns`.
  - `wt_off_uses_detected`.
  - `wt_error_falls_back_to_detected` (warning present).
  - `default_from_config_then_wt`.
  - `colors_use_profile_scheme` and `unknown_profile_gets_global_colors` on `SpawnCtx::colors`.
  - view test `new_tab_profile_spawns_nth_with_its_dir`: `Action::NewTabProfile(2)` yields `ModelCmd::NewTab { profile: <2nd name>, spawn.cwd: <its dir> }`; `NewTabProfile(9)` with 2 profiles yields no messages.
  - view test `split_keeps_focused_cwd_not_profile_dir`.

- [ ] **Step 2: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-client --test profiles --test view`. Expected: compile errors.

- [ ] **Step 3: Implement.** `build_spawn_ctx` calls `wt::load()` only when `config.windows_terminal_profiles`, then `profiles::merge`, logs `warnings` with `tracing::warn!`, fills `schemes` via `WtSettings::scheme` (and `theme::scheme_from_theme` for `profile-theme` names when WT is off), and keeps the existing `command` override after the merge. `resend_colors` sends `ctx.colors(&meta.profile)` per visible pane (profile from `snap.pane(p)`).

- [ ] **Step 4: Run to see them pass,** then the whole workspace: `rtk proxy cargo test --workspace`. Expected: green.

- [ ] **Step 5: Commit.** `git commit -m "feat: live Windows Terminal profiles with per-profile colours"`

### Task 4: One-time import

**Files:**
- Modify: `crates/config/src/wt.rs` (import), `crates/cli/src/main.rs` (`import-wt`), `crates/client/src/app.rs` (palette action → toast)
- Test: `crates/config/tests/import.rs`, `crates/cli/tests/cli.rs`

**Interfaces:**
- Consumes: Task 1 `WtSettings`, Task 2 `edit::append`, `ImportWtProfiles`.
- Produces:
  - `pub enum ImportOutcome { Imported, Exists, Unsupported(String) }`
  - `pub struct ImportReport { pub rows: Vec<(String, ImportOutcome)>, pub themes_written: Vec<String> }` with `pub fn imported(&self) -> usize`
  - `pub fn plan_import(src: &str, wt: &WtSettings, themes_dir: &Path) -> (String, ImportReport, Vec<(String, String)> /* theme name, file text */)` — pure.
  - `pub fn import(config: &Path, themes_dir: &Path, wt: &WtSettings) -> std::io::Result<ImportReport>` — `edit::write` for the config, then writes each new theme file only if it does not exist.

- [ ] **Step 1: Write the failing tests** (`crates/config/tests/import.rs`):
  - `writes_profile_lines_and_extras` — for a hand-built `WtSettings`, the new source contains `profile = Ubuntu:wsl.exe -d Ubuntu`, `profile-dir = …`, `profile-theme = Mine`, `profile-icon = 🦀`, `default-profile = …` (only when absent), and `windows-terminal-profiles = false` as the last line.
  - `second_run_writes_nothing` — `plan_import` on its own output returns the same string and `imported() == 0`.
  - `comments_and_crlf_kept`.
  - `colon_in_name_is_skipped` — `Unsupported("name contains ':'")`.
  - `wt_only_scheme_becomes_theme_file` — text has `palette = 0=#rrggbb` … `15=`, `foreground = `, `background = `, `cursor-color = `; a scheme that exists in `themes_dir` is not in the list.
  - `import_does_not_overwrite_theme` (temp dir, `import`).
  - CLI test `import_wt_with_fake_appdata` — run `wr import-wt` with `LOCALAPPDATA` pointing at a temp dir holding the Task 1 fixture and `APPDATA` at another; exit 0, stdout ends `N profiles imported`, config written.
  - CLI test `import_wt_without_wt_fails` — exit 1, stderr names the missing file, no config created.

- [ ] **Step 2: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-config --test import` and `rtk proxy cargo test -p wraith-cli --test cli`. Expected: compile errors / unknown command.

- [ ] **Step 3: Implement.** `plan_import` uses `edit::append` for `profile*` lines and `edit::set` for `default-profile` and `windows-terminal-profiles`; existing names come from parsing `src` with `parse_str`. CLI: `["import-wt"] => import_wt()` prints `imported|exists|skipped (<reason>)\t<name>` rows and `N profiles imported`; add to `USAGE` and the doc comment. Client: `Action::ImportWtProfiles` runs `wt::load()` + `import` and shows the toast `Imported N Windows Terminal profiles` (or `Couldn't import: <e>`); the config watcher then reloads.

- [ ] **Step 4: Run to see them pass.** Same commands, then `rtk proxy cargo test --workspace`. Expected: green.

- [ ] **Step 5: Commit.** `git commit -m "feat: wr import-wt and palette import of Windows Terminal profiles"`

### Task 5: Split button and `NewTabMenu` view state

**Files:**
- Create: `crates/client/src/new_tab_menu.rs`, `crates/client/tests/new_tab_menu.rs`
- Modify: `crates/client/src/chrome.rs`, `crates/client/src/view.rs`, `crates/client/src/lib.rs`, `crates/client/tests/chrome.rs`

**Interfaces:**
- Consumes: Task 3 `SpawnCtx.profiles`, Task 2 actions.
- Produces:
  - `chrome::tab_rects(...) -> (Vec<Rect>, Rect /* plus */, Rect /* chevron */)`: the button area grows from 3 to 4 cells; plus = first 2, chevron = last 2. `HitCtx.new_tab_menu: Rect`; `Hit::NewTabMenu` (the chevron) and `Hit::Menu(MenuHit)`; tooltip `Open a profile  Ctrl+Shift+Space`.
  - `pub enum MenuRow { Profile { index: usize, name: String, icon: Option<String>, binding: Option<String>, default: bool }, Separator, Settings(Option<String>), Palette(Option<String>) }`
  - `pub enum MenuOutcome { Open(usize), Split(usize), Settings, Palette, Close }`
  - `pub enum MenuHit { Row(usize), Panel }`
  - `pub struct NewTabMenu { pub rows: Vec<MenuRow>, pub selected: usize, pub scroll: usize }` with `new(profiles: &[Profile], default: &str, keybinds: &[Keybind]) -> NewTabMenu` (selection on the default profile), `key(&mut self, ev: &KeyEvent) -> Option<MenuOutcome>`, `click(&mut self, hit: &MenuHit, alt: bool) -> Option<MenuOutcome>`, `layout(&self, anchor: Rect, client: (i32,i32), cell: (i32,i32), bar_h: i32) -> MenuLayout`, `hit(l: &MenuLayout, x, y) -> Option<MenuHit>`. `MenuLayout { panel: Rect, rows: Vec<(usize, Rect)> }`. Separators are never selected.
  - `WindowView`: `menu: Option<NewTabMenu>`, `open_menu`, `close_menu`, `menu()`, `menu_mut()`, `menu_layout()`; `hit` checks palette, then menu, then settings. `Action::OpenNewTabMenu` returns `Out { ui: Some(Ui::NewTabMenu) }`; opening one overlay closes the others (palette/settings/menu).

- [ ] **Step 1: Write the failing tests.**
  - chrome: `tab_rects_split_button` (plus and chevron each 2 cells, adjacent); `hit_chevron_is_new_tab_menu`; `tooltip_for_chevron`.
  - menu (`new_tab_menu.rs`): `rows_have_bindings_for_first_nine` (10 profiles → rows 1–9 show `Ctrl+Shift+N`, row 10 none, then separator, Settings `Ctrl+,`, Palette `Ctrl+Shift+P`); `opens_on_default`; `down_up_wrap_and_skip_separator`; `home_end`; `enter_opens_alt_enter_splits`; `esc_closes`; `click_profile_alt_click_split`; `click_settings_and_palette`; `layout_moves_left_in_narrow_window`; `layout_scrolls_in_short_window` (selection kept visible).
  - view: `open_menu_closes_palette_and_settings`; `hit_prefers_menu_over_tabs`.

- [ ] **Step 2: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-client --test new_tab_menu --test chrome --test view`. Expected: compile errors.

- [ ] **Step 3: Implement.** Row height = one cell height + padding, as the palette's rows. Panel width = the widest row's text in cells + icon + binding column, capped at client width − 2 cells. Anchor = the plus rect's left edge, top = bar bottom.

- [ ] **Step 4: Run to see them pass.** Same command. Expected: pass.

- [ ] **Step 5: Commit.** `git commit -m "feat(client): split new-tab button and profile menu view state"`

### Task 6: Draw and wire the menu; palette entries; docs

**Files:**
- Modify: `crates/client/src/app.rs`, `crates/client/src/palette.rs`, `crates/client/src/motion.rs` (`pub menu: Fade`), `README.md`, `docs/superpowers/specs/2026-11-02-wt-profiles-design.md` (status)
- Create: `docs/wt-profiles-acceptance.md`
- Test: `crates/client/tests/palette.rs`, `crates/client/tests/app.rs`

**Interfaces:**
- Consumes: Task 5 `NewTabMenu`, `MenuOutcome`; Task 3 `NewTabProfile`.
- Produces: `palette_items(binds: &[Keybind], profiles: &[String]) -> Vec<Item>` adding `New tab: <name>` rows (group Tabs, `Choice::Action(NewTabProfile(i+1))` for every profile; the palette entry carries the binding text for the first nine); `pub fn letter_tile(name: &str) -> (char, (u8,u8,u8))` (first alphanumeric char uppercased, colour from a stable hash of the name over a fixed 8-colour list).

- [ ] **Step 1: Write the failing tests.** palette: `new_tab_rows_per_profile`. app: `letter_tile_is_stable_and_uppercase` (`letter_tile("ubuntu").0 == 'U'`, same colour twice, `"🦀x"` → `'X'`). app routing (pure part): `menu_outcome_open_maps_to_new_tab_profile` — a helper `fn menu_action(o: MenuOutcome) -> Option<(Action, bool /* split */)>` maps `Open(i)` → `NewTabProfile(i+1)`, `Split(i)` → split with profile `i`, `Settings` → `OpenSettings`, `Palette` → `CommandPalette`.

- [ ] **Step 2: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-client --test palette --test app`. Expected: compile errors.

- [ ] **Step 3: Implement.** In `app.rs`: chevron press toggles the menu; keys go to the menu while open (before the matcher, as the palette does); a press outside closes it; a split outcome sends `ModelCmd::Split` on the focused pane with the chosen profile, axis along the pane's longer side (reuse the `NewSplit` arm by factoring its body into `split_msg(dir, profile)`). Draw: panel and fade as the palette; per row an icon cell (a 1–2 char `icon` as text, else the letter tile), the name (bold for the default), the binding right-aligned dim; selection wash as the palette's; the chevron `⌄` drawn like `+` with its own hover wash.

- [ ] **Step 4: Run to see them pass,** then `rtk proxy cargo test --workspace` and `rtk proxy cargo clippy --workspace --all-targets -- -D warnings`. Expected: green, clean.

- [ ] **Step 5: Live check** per spec §6 on a debug build (never `target\release`), fake `APPDATA`/`LOCALAPPDATA` holding a *copy* of the real WT `settings.json` and fragments, private pipe, window shown without activation, messages posted to the HWND only. Record in `docs/wt-profiles-acceptance.md`: the menu rows vs WT's visible launchable profiles, Ubuntu's scheme, `wr import-wt` into the fake config then `windows-terminal-profiles = false` giving the same menu.

- [ ] **Step 6: Docs.** README: a "Windows Terminal profiles" paragraph, key rows `Ctrl+Shift+Space` and `Ctrl+Shift+1…9`, `wr import-wt` in the CLI list. Spec status → implemented.

- [ ] **Step 7: Commit.** `git commit -m "feat(client): draw the new-tab profile menu; docs and acceptance"`

### Task 7: Image icons

**Files:**
- Modify: `crates/config/src/wt.rs` (`ms-appx` resolution helper), `crates/client/src/app.rs` (icon cache and draw), `crates/client/Cargo.toml` (`png` and the `Win32_Storage_Packaging_Appx` feature)
- Test: `crates/config/tests/wt.rs`, `crates/client/tests/app.rs`

**Interfaces:**
- Consumes: Task 1 `WtSettings.family`, Task 6 menu drawing.
- Produces: `pub fn wt::resolve_icon(icon: &str, install_dir: Option<&Path>) -> Option<PathBuf>` (a `.png` path as is; `ms-appx:///X` → `install_dir/X`; anything else `None`); `fn package_dir(family: &str) -> Option<PathBuf>` in the client via `GetPackagesByPackageFamily` + `GetPackagePathByFullName`; an icon cache `HashMap<PathBuf, Option<(texture, (u32,u32))>>` decoded once with `png`, drawn cell-high; any failure falls back to the letter tile.

- [ ] **Step 1: Write the failing tests.** `resolve_icon_rules` (png path kept; `ms-appx:///ProfileIcons/x.png` with `Some(dir)` → `dir/ProfileIcons/x.png`, with `None` → `None`; `🦀` → `None`). app: `bad_png_falls_back_to_tile` on the pure decode helper `decode_icon(bytes) -> Option<(u32,u32,Vec<u8>)>` (garbage → `None`, a 2×2 PNG fixture → `Some((2,2,_))`).

- [ ] **Step 2: Run to see them fail.** Run: `rtk proxy cargo test -p wraith-config --test wt` and `rtk proxy cargo test -p wraith-client --test app`. Expected: compile errors.

- [ ] **Step 3: Implement.** Upload decoded icons through the same D3D texture path `ImageStore` uses, drawn with the existing image quad shader in the menu's icon cell.

- [ ] **Step 4: Run to see them pass,** then the workspace tests and clippy. Expected: green, clean. Re-run the live check's menu screenshot and note the icons in `docs/wt-profiles-acceptance.md`.

- [ ] **Step 5: Commit.** `git commit -m "feat(client): profile icons from PNG files and Windows Terminal's package"`
