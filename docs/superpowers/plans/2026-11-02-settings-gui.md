# Wraith Settings GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A settings panel, drawn in the Wraith window, that edits the common settings in `%APPDATA%\wraith\config` in place. Each change shows at once.

**Architecture:**
- The config crate gains two pure modules:
  - `edit`: line edits on the file text, plus an encoding-preserving atomic write;
  - `settings`: the row table as data.
- The client gains `settings.rs`, a pure view state (keys, clicks, layout) like `palette.rs`. It is owned by `WindowView` next to the palette.
- `app.rs` draws the panel. It turns the view's `Edit`s into file writes, or into in-memory previews that are parsed by the normal loader.

**Tech Stack:** Rust (stable, MSVC), `windows` crate (DirectWrite, `AssocQueryStringW`), the existing `notify` watcher.

**Spec:** `docs/superpowers/specs/2026-11-02-settings-gui-design.md`. Its parent spec is `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md` (§6.3 latency, §7 config).

## Global Constraints

- Windows 10 1809+ / 11, x64. Rust stable, MSVC.
- CI must stay green: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
- A GUI change alters the config file by exactly the lines for that key. Every other byte stays as the user wrote it: comments, order, line endings (CRLF if any line has one, else LF), a UTF-8 BOM, UTF-16 encoding.
- Values are interpreted only by the existing parser (`keys::apply`). The GUI writes text. The only parser change is `agent-notify = none`.
- §6.3: the panel draws nothing when it is closed, and adds nothing to the keystroke path while it is closed. Font enumeration runs when the panel opens, never per frame.
- While the panel is open, keys never reach the terminal.
- Live checks:
  - Use only a private pipe (`WRAITH_PIPE_NAME` = a full `\\.\pipe\...` name), `WRAITH_LAYOUT_PATH`, a fake `APPDATA`/`LOCALAPPDATA`, and windows shown without activation.
  - Never steal focus or send global input. Stop only processes you started, by PID.
  - Don't touch `target\release`. Build release binaries with `--target-dir target/settings-release`.
  - Never touch the user's real `%APPDATA%\wraith\config`.
- Evidence: run `rtk proxy cargo test ...` and read the exit code. RTK condenses cargo output, so never grep its summary.

## Notes from reading the code (rulings made while planning)

- **No self-made reload marker.** Spec §3.4 step 4 hides the "config reloaded" toast for GUI writes. That toast does not exist: `reload::reload` only toasts diagnostics. So nothing needs marking.
- **Apply at once after a write.** After a successful write, the app reloads the file at once instead of waiting for the watcher. The watcher's reload 200 ms later applies an equal `Config` and costs one redraw. This meets "within one debounce" with a margin.
- **The atomic rename.** `std::fs::rename` is `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows, so the write uses it directly.
- **`Edit` gains two variants.** The spec's `Edit` has `Set | Unset | Preview | Revert`. The view also has to say "close the panel" and "open the config file", so it gains `Close | OpenFile`. This is one enum instead of a second return type.
- **Sliders draw as `‹ 0.85 ›` with a thin fill bar.** They are clicked on the arrows; there is no drag. Steppers look the same without the bar.

## Review Focus

1. **A UTF-16 config file, or one with a BOM and CRLF.** A GUI change keeps the encoding, the BOM and the line endings. Tests: `write_keeps_utf16`, `set_keeps_bom_and_crlf` (Task 2).
2. **A config file that is not valid UTF-8.** The write refuses, and the file is left byte-for-byte unchanged (a lossy round trip would destroy bytes). Test: `write_refuses_invalid_utf8` (Task 2).
3. **A key set in an included file.** The row is read-only and produces no edits from keys or clicks. Test: `read_only_row_gives_no_edit` (Task 4).
4. **The panel in a tiny window.** The layout stays inside the client area, with no negative sizes, and the rows scroll. Test: `layout_fits_small_window` (Task 4).
5. **Esc in a theme or font list after moving through it.** It reverts the preview, and nothing is written. Test: `list_esc_reverts` (Task 4). Live: the file is unchanged (Task 7).

---

### Task 1: Parser `agent-notify = none` and the `open_settings` action

**Files:**
- Modify: `crates/config/src/keys.rs` (`agent-notify`)
- Modify: `crates/config/src/keybind.rs` (`Action::OpenSettings`, name `open_settings`, parse, `all_actions`, default `ctrl+comma=open_settings`)
- Modify: `crates/client/src/view.rs` (`Ui::Settings`; `WindowView::action` maps `OpenSettings` to `Out { ui: Some(Ui::Settings), .. }`)
- Test: `crates/config/tests/parse.rs`, `crates/config/tests/keybind.rs`, `crates/client/tests/view.rs`

**Interfaces:**
- Produces: `Action::OpenSettings`; `Ui::Settings`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/config/tests/parse.rs
#[test]
fn agent_notify_none_turns_all_off() {
    let (c, d) = parse("agent-notify = none\n");
    assert!(d.is_empty(), "{d:?}");
    let n = c.agent_notify;
    assert!(!n.toast && !n.flash && !n.badge && !n.sound);
}

// crates/config/tests/keybind.rs
#[test]
fn ctrl_comma_opens_settings() {
    let c = Config::default();
    let kb = c.keybinds.iter().find(|k| k.action == Action::OpenSettings).expect("default binding");
    assert_eq!(kb.trigger, parse_keybind("ctrl+comma=open_settings").unwrap().trigger);
    assert!(all_actions().iter().any(|(a, n)| *a == Action::OpenSettings && *n == "open_settings"));
}

// crates/client/tests/view.rs
#[test]
fn open_settings_asks_for_the_panel() {
    let (mut v, ctx) = view_with_one_pane(); // the file's existing fixture helper
    assert_eq!(v.action(Action::OpenSettings, &ctx).ui, Some(Ui::Settings));
}
```

Use the helpers each test file already has for `parse(...)` and the view fixture. Write a small local helper only where none exists.

- [ ] **Step 2: Run them and watch them fail**

Run: `rtk proxy cargo test -p wraith-config -p wraith-client --tests`
Expected: compile errors. `Action::OpenSettings` and `Ui::Settings` don't exist, and `none` is not a known value.

- [ ] **Step 3: Implement**

- `agent-notify`: the word `none` (alone) clears all four. `none` mixed with other words is `Err(bad(value))`.
- In `run_out`, `Ui::Settings` is matched with an empty arm (`Some(Ui::Settings) => {}`). Task 6 fills it.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `rtk proxy cargo test --workspace`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/config crates/client/src/view.rs crates/client/src/app.rs crates/client/tests/view.rs
git commit -m "feat(config): agent-notify = none and the open_settings action"
```

---

### Task 2: `config::edit`

**Files:**
- Create: `crates/config/src/edit.rs` (`pub mod edit;` in `lib.rs`)
- Modify: `crates/config/src/lib.rs`: `collect` and `Entry` become `pub(crate)` for `origins`; `decode` is reused by `read`
- Test: `crates/config/tests/edit.rs`

**Interfaces:**
- Produces:
  - `pub enum Which { First, Last }` (Copy, Debug, PartialEq)
  - `pub fn set(src: &str, key: &str, value: &str, which: Which) -> String`
  - `pub fn unset(src: &str, key: &str) -> String`
  - `pub enum Origin { Here { line: usize }, Included { file: PathBuf, line: usize }, Default }` (Clone, Debug, PartialEq)
  - `pub fn origins(path: &Path, keys: &[(&str, Which)]) -> Vec<Origin>`, one entry per key, in order
  - `pub fn read(path: &Path) -> std::io::Result<String>`. A missing file gives `HEADER` plus a newline. UTF-8 (BOM kept as `\u{feff}`) or UTF-16 decoded. Invalid UTF-8 gives `ErrorKind::InvalidData`.
  - `pub fn write(path: &Path, edit: impl FnOnce(&str) -> String) -> std::io::Result<()>`
  - `pub const HEADER: &str = "# Wraith config — edited by hand and by Settings (ctrl+comma)";`

- [ ] **Step 1: Write the failing tests**

```rust
use wraith_config::edit::{HEADER, Origin, Which, origins, read, set, unset, write};

#[test]
fn set_replaces_last_of_several() {
    assert_eq!(set("font-size = 10\nfont-size = 11\n", "font-size", "14", Which::Last), "font-size = 10\nfont-size = 14\n");
}

#[test]
fn set_replaces_first_for_lists() {
    assert_eq!(set("font-family = A\nfont-family = B\n", "font-family", "C", Which::First), "font-family = C\nfont-family = B\n");
}

#[test]
fn set_appends_when_missing() {
    assert_eq!(set("theme = x\n", "font-size", "14", Which::Last), "theme = x\nfont-size = 14\n");
    assert_eq!(set("theme = x", "font-size", "14", Which::Last), "theme = x\nfont-size = 14\n");
    assert_eq!(set("", "font-size", "14", Which::Last), "font-size = 14\n");
}

#[test]
fn set_keeps_bom_and_crlf() {
    assert_eq!(set("\u{feff}font-size = 10\r\n", "font-size", "14", Which::Last), "\u{feff}font-size = 14\r\n");
    assert_eq!(set("a = 1\r\n", "b", "2", Which::Last), "a = 1\r\nb = 2\r\n");
}

#[test]
fn comments_and_similar_keys_untouched() {
    let src = "# font-size = 9\nfont-size-x = 1\n  font-size=10  \n";
    assert_eq!(set(src, "font-size", "14", Which::Last), "# font-size = 9\nfont-size-x = 1\nfont-size = 14\n");
}

#[test]
fn unset_removes_every_line() {
    assert_eq!(unset("a = 1\nfont-size = 10\n# font-size = 9\nfont-size = 11\n", "font-size"), "a = 1\n# font-size = 9\n");
    assert_eq!(unset("a = 1\n", "font-size"), "a = 1\n");
}

#[test]
fn origins_follow_includes() {
    // dir/config: `font-size = 10` on line 2 and `config-file = inc`; dir/inc: `theme = x` on line 1, plus
    // `config-file = inc2`; dir/inc2: `cursor-style = bar` on line 1.
    let dir = temp("origins");
    // ... write the three files ...
    let o = origins(&dir.join("config"), &[("font-size", Which::Last), ("theme", Which::Last), ("cursor-style", Which::Last), ("sidebar-width", Which::Last)]);
    assert_eq!(o[0], Origin::Here { line: 2 });
    assert_eq!(o[1], Origin::Included { file: dir.join("inc"), line: 1 });
    assert_eq!(o[2], Origin::Included { file: dir.join("inc2"), line: 1 });
    assert_eq!(o[3], Origin::Default);
}

#[test]
fn write_creates_missing_file_with_header() {
    let dir = temp("create");
    let p = dir.join("sub").join("config");
    write(&p, |s| set(s, "font-size", "14", Which::Last)).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), format!("{HEADER}\nfont-size = 14\n"));
}

#[test]
fn write_keeps_utf16() {
    let p = temp("u16").join("config");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("a = 1\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&p, &bytes).unwrap();
    write(&p, |s| set(s, "b", "2", Which::Last)).unwrap();
    let mut want = vec![0xFF, 0xFE];
    want.extend("a = 1\r\nb = 2\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(std::fs::read(&p).unwrap(), want);
}

#[test]
fn write_refuses_invalid_utf8() {
    let p = temp("bad").join("config");
    std::fs::write(&p, b"a = \xff\n").unwrap();
    let e = write(&p, |s| set(s, "b", "2", Which::Last)).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&p).unwrap(), b"a = \xff\n");
}

#[test]
fn write_fails_on_read_only_file_and_leaves_it() {
    let p = temp("ro").join("config");
    std::fs::write(&p, "a = 1\n").unwrap();
    let mut perm = std::fs::metadata(&p).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&p, perm.clone()).unwrap();
    assert!(write(&p, |s| set(s, "b", "2", Which::Last)).is_err());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a = 1\n");
    perm.set_readonly(false);
    std::fs::set_permissions(&p, perm).unwrap();
}
```

`temp(name)` is a local helper. It creates `std::env::temp_dir().join(format!("wraith-edit-{}-{name}", std::process::id()))`, clearing any earlier copy, as `parse.rs` does.

- [ ] **Step 2: Run them and watch them fail**

Run: `rtk proxy cargo test -p wraith-config --test edit`
Expected: compile error, because `wraith_config::edit` doesn't exist.

- [ ] **Step 3: Implement `crates/config/src/edit.rs`**

- **A line matches `key`** when it is not a comment and its text before the first `=`, trimmed, equals `key`. This is the same rule as `parse::lines`.
- **`set`:**
  1. Strip a leading `\u{feff}` and re-add it at the end.
  2. Pick the line ending: `"\r\n"` if `src` contains one, else `"\n"`.
  3. Replace the chosen line's whole text with `{key} = {value}`, keeping that line's own ending.
  4. With no match: add an ending if `src` is non-empty and lacks one, then append the line and an ending.
- **`unset`:** drop every matching line together with its ending.
- **`origins`:** run `collect` on `path` (a missing or unreadable file gives `Default` everywhere). For each key, take the first entry (`Which::First`) or the last entry (`Which::Last`) with that key. Compare `canon(entry.file)` with `canon(path)` to choose `Here` or `Included`.
- **`read`:**
  1. `NotFound` gives `format!("{HEADER}\n")`.
  2. A `FF FE` or `FE FF` prefix decodes as UTF-16.
  3. Otherwise `String::from_utf8`. A UTF-8 BOM stays in the string as `\u{feff}`. Failure gives `InvalidData`.
- **`write`:**
  1. Re-read the file's leading bytes to choose the encoding: UTF-16 LE, UTF-16 BE, or UTF-8.
  2. Call `read`, then apply `edit`, then encode. UTF-16 output keeps its BOM.
  3. `create_dir_all` the parent.
  4. Write `<path>.tmp` beside the file, then `std::fs::rename` it over `path`. On any error, remove the tmp file and return the error.
  5. **Read-only files:** check `metadata(path).permissions().readonly()` first and return `PermissionDenied`, because a rename can replace a read-only target on NTFS.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `rtk proxy cargo test -p wraith-config`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/config/src/edit.rs crates/config/src/lib.rs crates/config/tests/edit.rs
git commit -m "feat(config): in-place config edits with an encoding-preserving atomic write"
```

---

### Task 3: `config::settings`, the row table

**Files:**
- Create: `crates/config/src/settings.rs` (`pub mod settings;` in `lib.rs`)
- Test: `crates/config/tests/settings.rs`

**Interfaces:**
- Consumes: `edit::Which`, `edit::set` (Task 2).
- Produces:
  - `pub enum Page { Appearance, Behaviour, Agents }` (Copy, Debug, PartialEq, Eq, Hash), plus `pub const PAGES: [Page; 3]` and `Page::label(self) -> &'static str`
  - `pub enum ListSource { Themes, Fonts, Profiles }`
  - `pub enum Kind`:
    - `Toggle`
    - `Choice(&'static [(&'static str, &'static str)])`: (value written, label shown)
    - `Stepper { min: i32, max: i32 }`
    - `Slider { min: f32, max: f32, step: f32 }`
    - `List(ListSource)`
    - `NotifySet`
    - `QuietHours`
  - `pub struct Setting { pub key: &'static str, pub label: &'static str, pub page: Page, pub which: Which, pub kind: Kind, pub get: fn(&Config) -> String }`
  - `pub fn all() -> &'static [Setting]`: the 20 rows of spec §2.1, in its order
  - `pub fn on_page(p: Page) -> Vec<usize>`: indexes into `all()`

Exact values, from spec §2.1:

| Key | Kind | `get` |
|---|---|---|
| `theme` | `List(Themes)` | `theme` or `""` |
| `font-family` | `List(Fonts)`, `Which::First` | `font_family[0]` or `""` |
| `font-size` | `Stepper { 6, 36 }` | `format!("{}", font_size)` |
| `background-opacity` | `Slider { 0.30, 1.00, 0.05 }` | `format!("{:.2}", …)` |
| `cursor-style` | `Choice`: block, bar, underline, block_hollow | as written |
| `cursor-style-blink` | `Toggle` | `true`/`false` |
| `window-padding-x` | `Stepper { 0, 32 }` | integer |
| `window-padding-y` | `Stepper { 0, 32 }` | integer |
| `unfocused-split-opacity` | `Slider { 0.15, 1.00, 0.05 }` | `{:.2}` |
| `copy-on-select` | `Toggle` | |
| `right-click-action` | `Choice`: paste, menu | |
| `close-on-exit` | `Choice`: clean, always, never | |
| `default-profile` | `List(Profiles)` | value or `""` |
| `scrollback-limit` | `Choice`: 10000000 "10 MB", 50000000 "50 MB", 100000000 "100 MB", 500000000 "500 MB" | bytes |
| `sidebar-visible` | `Toggle` | |
| `sidebar-width` | `Stepper { 10, 60 }` | integer |
| `agent-notify` | `NotifySet` | the comma list in the order toast,flash,badge,sound; `none` when all are off |
| `agent-notify-when` | `Choice`: unfocused "when unfocused", always | |
| `agent-notify-quiet` | `QuietHours` | `HH:MM-HH:MM` zero-padded, or `""` |
| `agent-tab-pulse` | `Toggle` | |

Every row is `Which::Last` except `font-family`.

- [ ] **Step 1: Write the failing tests**

```rust
use std::path::Path;
use wraith_config::edit::set;
use wraith_config::settings::{Kind, Page, all, on_page};
use wraith_config::{Config, parse_str};

fn themes() -> std::path::PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/themes") }

/// Values each control can write.
fn samples(kind: &Kind) -> Vec<String> {
    match kind {
        Kind::Toggle => vec!["true".into(), "false".into()],
        Kind::Choice(cs) => cs.iter().map(|(v, _)| v.to_string()).collect(),
        Kind::Stepper { min, max } => vec![min.to_string(), max.to_string()],
        Kind::Slider { min, max, .. } => vec![format!("{min:.2}"), format!("{max:.2}")],
        Kind::NotifySet => vec!["toast,sound".into(), "none".into(), "toast,flash,badge,sound".into()],
        Kind::QuietHours => vec!["22:00-07:30".into()],
        Kind::List(_) => vec![],
    }
}

#[test]
fn every_value_round_trips_through_the_parser() {
    for s in all() {
        for v in samples(&s.kind) {
            let (c, d) = parse_str(&set("", s.key, &v, s.which), Path::new("config"), &themes());
            assert!(d.is_empty(), "{} = {v}: {d:?}", s.key);
            assert_eq!((s.get)(&c), v, "{}", s.key);
        }
    }
}

#[test]
fn list_values_round_trip() {
    let theme = std::fs::read_dir(themes()).unwrap().next().unwrap().unwrap().file_name().into_string().unwrap();
    for (key, v) in [("theme", theme.as_str()), ("font-family", "Consolas"), ("default-profile", "pwsh")] {
        let s = all().iter().find(|s| s.key == key).unwrap();
        let (c, _) = parse_str(&set("", key, v, s.which), Path::new("config"), &themes());
        assert_eq!((s.get)(&c), v, "{key}");
    }
}

#[test]
fn defaults_get_what_the_parser_defaults_to() {
    let c = Config::default();
    for s in all() {
        let v = (s.get)(&c);
        let (c2, _) = parse_str(&set("", s.key, &v, s.which), Path::new("config"), &themes());
        assert_eq!((s.get)(&c2), v, "{}", s.key);
    }
}

#[test]
fn pages_hold_the_spec_rows() {
    assert_eq!(all().len(), 20);
    assert_eq!(on_page(Page::Appearance).len(), 9);
    assert_eq!(on_page(Page::Behaviour).len(), 7);
    assert_eq!(on_page(Page::Agents).len(), 4);
}
```

The default `theme` is `""`, and writing `theme = ` resets it, so `defaults_get_what_the_parser_defaults_to` holds for every row.

- [ ] **Step 2: Run them and watch them fail**

Run: `rtk proxy cargo test -p wraith-config --test settings`
Expected: compile error, because `wraith_config::settings` doesn't exist.

- [ ] **Step 3: Implement `settings.rs`** as a `static SETTINGS: [Setting; N]` with the values above.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `rtk proxy cargo test -p wraith-config`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/config/src/settings.rs crates/config/src/lib.rs crates/config/tests/settings.rs
git commit -m "feat(config): the settings row table"
```

---

### Task 4: `client::settings`, the panel's state, keys, clicks and layout

**Files:**
- Create: `crates/client/src/settings.rs` (`pub mod settings;` in `lib.rs`)
- Modify: `crates/client/src/chrome.rs` (`Hit::Settings(SettingsHit)`)
- Modify: `crates/client/src/view.rs`:
  - `settings: Option<SettingsView>`, with `settings()`, `settings_mut()`, `open_settings(SettingsView)`, `close_settings()`;
  - `hit` checks the open panel's layout first (before the palette), returning `Hit::Settings(..)`;
  - a press outside the panel closes it, like `palette_click`.
- Test: `crates/client/tests/settings.rs`

**Interfaces:**
- Consumes: `settings::{all, on_page, Page, Kind, ListSource}`, `edit::{Which, Origin}` (Tasks 2–3); `palette::fuzzy`.
- Produces:
  - `pub enum Edit`:
    - `Set { key: &'static str, value: String, which: Which }`
    - `Unset { key: &'static str }`
    - `Preview { key: &'static str, value: String, which: Which }`
    - `Revert`
    - `Close`
    - `OpenFile`
  - `pub struct Sources { pub themes: Vec<String>, pub fonts: Vec<String>, pub profiles: Vec<String> }`
  - `pub struct RowState { pub origin: Origin, pub warning: Option<String> }`: one per `all()` row
  - `pub enum SettingsHit`:
    - `Page(Page)`
    - `Row(usize)`
    - `Dec(usize)`
    - `Inc(usize)`
    - `Way(usize, u8)`: notify toggles 0–3
    - `Time(usize, u8)`: quiet fields 0–1
    - `Reset(usize)`
    - `ListItem(usize)`
    - `OpenFile`
    - `Panel`

    Every `usize` is an index into `all()`, except `ListItem`'s, which indexes the filtered list.
  - `pub struct SettingsLayout { pub panel: Rect, pub pages: Vec<(Page, Rect)>, pub rows: Vec<(usize, Rect)>, pub list: Option<(Rect, Vec<(usize, Rect)>)>, pub open_file: Rect, pub targets: Vec<(SettingsHit, Rect)> }`. `targets` holds every clickable part, and `hit` walks it.
  - `impl SettingsView`:
    - `pub fn new(sources: Sources, rows: Vec<RowState>) -> SettingsView`
    - `pub fn set_rows(&mut self, rows: Vec<RowState>)`: after a reload
    - `pub fn key(&mut self, ev: &KeyEvent, cfg: &Config) -> Option<Edit>`
    - `pub fn text(&mut self, t: &str) -> Option<Edit>`: typed text or IME text into a list query or a time field
    - `pub fn click(&mut self, hit: &SettingsHit, cfg: &Config) -> Option<Edit>`
    - `pub fn scroll(&mut self, rows: i32)`
    - `pub fn layout(&self, client: (i32, i32), cell: (i32, i32), bar_h: i32) -> SettingsLayout`
    - `pub fn hit(l: &SettingsLayout, x: i32, y: i32) -> Option<SettingsHit>`
    - pub read access to `page`, `focus` (an index into `all()`), the open list (`query`, `items()` filtered and best first, `selected`) and the time edit (`field`, `start`, `end`), for drawing

Behaviour (spec §2, §4, §5):
- **Keys:**
  - `Tab` and `Shift+Tab` move between pages, wrapping. `↑`/`↓` move between rows and stop at the ends.
  - `←`/`→`: steppers ±1 and sliders ±step, clamped; choices wrap. Each gives `Set`. A value from the file that is out of range snaps into range on the first step.
  - `Space` flips a toggle (`Set`). On `NotifySet` it flips the first way. On `QuietHours` it switches the row off (`Unset`) or on (opens the time edit, prefilled `22:00` and `07:00`).
  - `Delete` gives `Unset`. `Esc` gives `Close`.
- **Lists:**
  - `Enter` on a list row opens it, with the current value selected. Typing filters with `fuzzy`. `↑`/`↓` give `Preview` for the newly selected item (themes and fonts only; profiles don't preview).
  - `Enter` gives `Set`. `Esc` gives `Revert` when a preview was sent, and otherwise closes the list with no edit.
- **Quiet hours:** `Enter` on the row opens the time edit. Typing accepts digits and `:` only, up to 5 characters. `Enter` on the start field moves to the end field; `Enter` on the end field validates (`HH:MM`, start ≠ end) and gives `Set` with `start-end`. When invalid, the edit stays open with `invalid = true`.
- **Read-only rows** (`Origin::Included`) give `None` for every key and click except navigation.
- **Layout:**
  - Panel width `min(640, client_w − 2·cell_w)`. Height `min(80% of client_h, what the rows need)`, centred horizontally, top at `bar_h + 12`.
  - Pages column 14 cells wide. Rows `cell_h + 10` tall. Footer row for "Open config file".
  - `scroll` keeps the focused row visible. Nothing has a negative width or height. Below a 40×10-cell client, rows still lay out at least one row.

- [ ] **Step 1: Write the failing tests**

```rust
use wraith_client::chrome::Hit;
use wraith_client::settings::{Edit, RowState, SettingsHit, SettingsView, Sources};
use wraith_config::Config;
use wraith_config::edit::{Origin, Which};
use wraith_config::settings::{Page, all};
use wraith_proto::{KeyAction, KeyCode, KeyEvent, Mods};

fn view() -> SettingsView {
    let rows = all().iter().map(|_| RowState { origin: Origin::Default, warning: None }).collect();
    let src = Sources { themes: vec!["Dracula".into(), "Nord".into()], fonts: vec!["Cascadia Mono".into(), "Consolas".into()], profiles: vec!["pwsh".into()] };
    SettingsView::new(src, rows)
}
fn key(code: KeyCode) -> KeyEvent { /* Press, no mods, no text: build it the way tests/view.rs builds key events */ }
fn idx(k: &str) -> usize { all().iter().position(|s| s.key == k).unwrap() }
fn focus(v: &mut SettingsView, k: &str) { /* press Tab or ↓ until v.focus == idx(k) */ }

#[test] fn tab_changes_page_and_wraps() { /* Appearance → Behaviour → Agents → Appearance; Shift+Tab goes back */ }
#[test] fn arrows_move_rows_and_stop_at_ends() { /* ↑ on the first row stays; ↓ past the last stays */ }

#[test]
fn stepper_clamps() {
    let mut v = view();
    focus(&mut v, "font-size");
    let mut c = Config::default();
    c.font_size = 36.0;
    assert_eq!(v.key(&key(KeyCode::ArrowRight), &c), Some(Edit::Set { key: "font-size", value: "36".into(), which: Which::Last }));
    c.font_size = 50.0; // from the file, out of range
    assert_eq!(v.key(&key(KeyCode::ArrowLeft), &c), Some(Edit::Set { key: "font-size", value: "36".into(), which: Which::Last }));
}

#[test]
fn slider_steps_by_005() { /* background-opacity 1.00, ← gives "0.95"; at 0.30, ← gives "0.30" */ }
#[test] fn choice_wraps() { /* cursor-style block_hollow, → gives "block" */ }
#[test] fn space_toggles() { /* copy-on-select false gives Set "true" */ }
#[test] fn delete_resets() { assert_eq!(/* Delete on font-size */, Some(Edit::Unset { key: "font-size" })); }

#[test]
fn list_filters_previews_and_commits() {
    // theme row: Enter opens; typing "no" leaves ["Nord"]; ↓/↑ give Preview; Enter gives Set "Nord".
}

#[test]
fn list_esc_reverts() {
    // Enter, ↓ (Preview), Esc gives Edit::Revert; a second Esc gives Edit::Close.
    // Enter then Esc with no movement gives None, and the list is closed.
}

#[test]
fn notify_all_off_writes_none() {
    // agent-notify with only toast on: clicking Way(idx, 0) gives Set "none".
}

#[test]
fn quiet_hours_rejects_equal_times() {
    // Enter; type "22:00"; Enter; type "22:00"; Enter gives None, and the edit stays open with invalid = true.
    // Retyping the end as "07:00" and pressing Enter gives Set "22:00-07:00".
}

#[test]
fn read_only_row_gives_no_edit() {
    let mut rows: Vec<RowState> = all().iter().map(|_| RowState { origin: Origin::Default, warning: None }).collect();
    rows[idx("font-size")].origin = Origin::Included { file: "inc".into(), line: 1 };
    let mut v = SettingsView::new(Sources { themes: vec![], fonts: vec![], profiles: vec![] }, rows);
    focus(&mut v, "font-size");
    let c = Config::default();
    for k in [KeyCode::ArrowLeft, KeyCode::ArrowRight, KeyCode::Space, KeyCode::Delete, KeyCode::Enter] {
        assert_eq!(v.key(&key(k), &c), None, "{k:?}");
    }
    assert_eq!(v.click(&SettingsHit::Inc(idx("font-size")), &c), None);
    assert_eq!(v.click(&SettingsHit::Reset(idx("font-size")), &c), None);
}

#[test]
fn layout_fits_small_window() {
    let v = view();
    for client in [(1600, 900), (500, 300), (200, 120)] {
        let l = v.layout(client, (8, 16), 26);
        for (_, r) in l.targets.iter() {
            assert!(r.w >= 0 && r.h >= 0 && r.x >= 0 && r.y >= 0 && r.x + r.w <= client.0 && r.y + r.h <= client.1, "{client:?} {r:?}");
        }
        assert!(!l.rows.is_empty(), "{client:?}");
    }
}

#[test]
fn hit_finds_controls() {
    let v = view();
    let l = v.layout((1600, 900), (8, 16), 26);
    let (h, r) = &l.targets[0];
    assert_eq!(SettingsView::hit(&l, r.x + 1, r.y + 1).as_ref(), Some(h));
    assert_eq!(SettingsView::hit(&l, 0, 0), None);
}
```

Fill in the commented bodies as the comments state, one assertion per stated outcome. Add a view test in `crates/client/tests/view.rs`: with the panel open, `v.hit` over the panel gives `Hit::Settings(_)`, and a press outside it closes the panel.

- [ ] **Step 2: Run them and watch them fail**

Run: `rtk proxy cargo test -p wraith-client --test settings --test view`
Expected: compile error, because `wraith_client::settings` doesn't exist.

- [ ] **Step 3: Implement `settings.rs`** and the view hooks, as the Behaviour list above says.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `rtk proxy cargo test -p wraith-client`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/client/src/settings.rs crates/client/src/lib.rs crates/client/src/chrome.rs crates/client/src/view.rs crates/client/tests/settings.rs crates/client/tests/view.rs
git commit -m "feat(client): settings panel state, keys, clicks and layout"
```

---

### Task 5: The gear button and font enumeration

**Files:**
- Modify: `crates/client/src/chrome.rs`:
  - `Hit::Gear`;
  - `pub fn gear_rect(bar: Rect, caption_w: i32, cell_w: i32) -> Rect`: 3 cells wide, flush against the caption buttons' left edge;
  - `HitCtx.gear: Rect`, checked in the tab-bar branch before tabs;
  - `tooltip_for(Hit::Gear)` gives `Settings  Ctrl+,` from the binding of `Action::OpenSettings`.
- Modify: `crates/client/src/view.rs`: `tab_rects` gets `reserved_right = caption_w + gear.w`; `hit` passes `gear`; `pub fn gear_rect(&self) -> Rect`.
- Modify: `crates/client/src/font.rs`: `pub fn monospace_families() -> Vec<String>`.
- Test: `crates/client/tests/chrome.rs`, `crates/client/tests/font.rs`

**Interfaces:**
- Produces: `Hit::Gear`, `chrome::gear_rect`, `WindowView::gear_rect`, `font::monospace_families`.

- [ ] **Step 1: Write the failing tests**

```rust
// crates/client/tests/chrome.rs
#[test]
fn gear_sits_left_of_the_captions_and_is_hit() {
    let bar = Rect { x: 200, y: 0, w: 1000, h: 26 };
    let g = gear_rect(bar, 138, 8);
    assert_eq!(g, Rect { x: 200 + 1000 - 138 - 24, y: 0, w: 24, h: 26 });
    // With HitCtx built as the file's existing tests build it, plus `gear: g`, a point inside g gives Hit::Gear.
}

#[test]
fn gear_tooltip_names_the_binding() {
    let binds = wraith_config::Config::default().keybinds;
    assert_eq!(tooltip_for(&Hit::Gear, &binds).as_deref(), Some("Settings  Ctrl+,"));
}

// crates/client/tests/font.rs
#[test]
fn monospace_families_lists_installed_monospace_only() {
    let f = monospace_families();
    assert!(f.iter().any(|n| n == "Consolas"), "{f:?}");
    assert!(!f.iter().any(|n| n == "Arial"), "{f:?}");
    assert!(f.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `rtk proxy cargo test -p wraith-client --test chrome --test font`
Expected: compile error, because `gear_rect`, `Hit::Gear` and `monospace_families` don't exist.

- [ ] **Step 3: Implement**

`monospace_families` steps:
1. Get the system collection.
2. For each family, take `GetFirstMatchingFont(NORMAL, NORMAL, NORMAL)`, cast it to `IDWriteFont1`, and keep the family when `IsMonospacedFont()` is true.
3. Name each kept family by its `en-us` name, falling back to index 0.
4. Sort and dedup.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `rtk proxy cargo test -p wraith-client`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/client/src/chrome.rs crates/client/src/view.rs crates/client/src/font.rs crates/client/tests/chrome.rs crates/client/tests/font.rs
git commit -m "feat(client): settings gear button and monospace font list"
```

---

### Task 6: Wire the panel into the app

**Files:**
- Modify: `crates/client/src/app.rs`
- Modify: `crates/client/src/reload.rs`:
  - `State.diags: Vec<Diagnostic>`, kept from the last load;
  - `pub(crate) fn reload` and `pub(crate) fn apply_config`;
  - `pub(crate) fn path(app) -> &Path`.
- Modify: `crates/client/src/motion.rs` (`pub settings: Fade`)
- Test: `crates/client/tests/reload.rs` (`row_states_match_diagnostics_by_line`)

**Interfaces:**
- Consumes: everything above.
- Produces: `pub fn row_states(path: &Path, diags: &[Diagnostic]) -> Vec<RowState>` in `reload.rs`. It runs `edit::origins` over `all()`, and gives a row the warning of a diagnostic with the same file and the row's `Here { line }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn row_states_match_diagnostics_by_line() {
    // config: line 1 `font-size = abc`, line 2 `theme = `
    let p = /* temp file with that text */;
    let d = vec![Diagnostic { file: p.clone(), line: 1, message: "invalid value `abc`".into() }];
    let rows = row_states(&p, &d);
    let i = all().iter().position(|s| s.key == "font-size").unwrap();
    assert_eq!(rows[i].origin, Origin::Here { line: 1 });
    assert_eq!(rows[i].warning.as_deref(), Some("invalid value `abc`"));
    assert!(rows.iter().enumerate().all(|(j, r)| j == i || r.warning.is_none()));
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `rtk proxy cargo test -p wraith-client --test reload`
Expected: compile error, because `row_states` doesn't exist.

- [ ] **Step 3: Implement the wiring in `app.rs`**

- **Open and close:**
  - `Ui::Settings` toggles the panel: it closes an open panel, and otherwise opens `SettingsView::new(Sources { themes, fonts, profiles }, row_states(...))`.
    - `themes`: the file names in `themes_dir()`, sorted.
    - `fonts`: `monospace_families()`.
    - `profiles`: the names in `spawn_ctx.profiles`.
  - A left press on `Hit::Gear` does the same.
  - `Edit::Close` closes the panel. Before closing, it reverts any open preview.
- **Keys** (`on_key`, before the palette and find routing):
  - While the panel is open, a key with Ctrl goes to `matcher`. Only `OpenSettings` and `CommandPalette` run; other matched actions are dropped.
  - Every other key goes to `settings.key`.
  - The panel consumes every key, including releases.
  - When the palette is open on top of the panel, the palette keeps its current priority.
- **Text:** a `WM_IME_COMPOSITION` commit and typed text go to `settings.text` when the panel is open. The palette still comes first when both are open.
- **Mouse:**
  - A left press on `Hit::Settings(h)` calls `settings.click(&h, &config)`.
  - The wheel over the panel calls `settings.scroll(-n)`.
  - The panel's targets join hover and press like palette rows: add them to the `press` exclusion list where `PaletteRow` is.
- **`apply_edit(edit)`:**
  - `Set` and `Unset`: `edit::write(path, …)`.
    - On `Ok`: drop any preview base, then `reload::reload(self)`. Refresh `settings.set_rows(row_states(..))`.
    - On `Err(e)`: `toast_text(self, "settings", format!("Couldn't save settings: {e}"))`. If a preview is active, revert it.
  - `Preview`:
    1. Keep `preview_base = Some(config.clone())` if it is not set yet.
    2. `edit::read(path)`, then `edit::set`, then `wraith_config::parse_str(&text, path, &themes_dir())`, then `reload::apply_config`.
    3. A read error gives the same toast as a failed write.
  - `Revert`: `reload::apply_config(preview_base.take())`.
  - `OpenFile`:
    1. Create the file with `edit::write(path, |s| s.to_string())` if it is missing.
    2. Ask `AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_EXECUTABLE, ".txt", "open")` for the program.
    3. Spawn it with the path. Fall back to `notepad.exe`.
  - The read-only "Open" on a row opens that included file the same way.
- **Reload while the panel is open:** after `reload::reload` (from the watcher or the hotkey), refresh `set_rows`.
- **Drawing** (after the palette block, using the same `Painter`, colours and `motion.overlay(|m| &mut m.settings, open, now)` fade from 40% plus the 8 px rise):
  - the panel border in `accent` and the panel in `col.bar_bg`;
  - the page column, with the current page in `tab_active_bg` plus a 2 px accent edge;
  - rows: label in `bar_fg`; the control at the right;
  - a reset dot `●` in `accent` when `get(config) != get(Config::default())` and the row is not read-only;
  - a read-only row shows `Set in <file name>` and `Open` in `bar_fg_dim`;
  - a warning row shows `⚠` with its message as the tooltip;
  - the focused row gets the hover wash;
  - an open list draws as a dropdown under its row, at most 10 items, with the query line;
  - the footer shows "Open config file".
- **The gear:** draw `⚙` centred in `view.gear_rect()`, with the hover and press washes of the other tab-bar buttons, and add `Hit::Gear` to the tooltip branch (`r = view.gear_rect()`).
- **The closed panel** costs nothing: every settings branch is behind `if let Some(s) = self.view.settings()`.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `rtk proxy cargo test --workspace` and `rtk proxy cargo clippy --workspace --all-targets -- -D warnings`
Expected: both exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/client
git commit -m "feat(client): settings panel in the window, with live preview and in-place saves"
```

---

### Task 7: Live check and docs

**Files:**
- Create: `docs/settings-acceptance.md`
- Modify: `README.md`. Add a "Settings" line under Documentation: Ctrl+, opens the panel, and the rarer keys stay in the file.
- Modify: `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md`. Move "settings GUI" out of the §10 backlog, and mark it done with a link to the settings spec.
- Modify: `docs/superpowers/specs/2026-11-02-settings-gui-design.md`. Set its status to implemented.

- [ ] **Step 1: Build a release client and server in a separate target dir**

Run: `rtk proxy cargo build --release --target-dir target/settings-release`
Expected: exit 0.

- [ ] **Step 2: Live check** with the Global Constraints' private environment:

1. Create a fake `APPDATA` holding `wraith\config` with this text, CRLF line endings:

   ```
   # mine
   font-size = 12
   copy-on-select = false
   ```

2. Start the window shown without activation. Post `WM_KEYDOWN` for Ctrl+, to its HWND, as earlier acceptance runs did.
3. **Theme:** move to Theme, press Enter, press ↓ twice, and press Enter.
4. **Font size:** move to Font size and press → twice.
5. **Copy on select:** move to Copy on select and press Space.
6. Diff the file. The result must be the original lines, with `font-size = 14` and `copy-on-select = true` rewritten in place and `theme = <name>` appended. It must still have CRLF line endings, and there must be no other change.
7. Take a screenshot showing the new theme, the panel and the gear (the PrintWindow method from earlier acceptance runs).
8. **Revert:** open the font list, press ↓, and press Esc. The file is unchanged, and the font is back.
9. **Read-only:** set the file read-only and press → on Font size. A "Couldn't save settings" toast shows, and the file is unchanged.
10. Stop your processes by PID.

- [ ] **Step 3: Write `docs/settings-acceptance.md`** with each check, its result and the screenshot path, in the format of `docs/m7-acceptance.md`.

- [ ] **Step 4: Commit**

```bash
git add docs README.md
git commit -m "docs: settings GUI acceptance and spec status"
```
