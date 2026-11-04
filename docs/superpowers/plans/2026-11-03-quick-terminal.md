# Quick Terminal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A global hotkey (Win+` by default) slides a Wraith window with its own workspace down from the top of the screen, and hides it again.

**Architecture:**
- Config gains Ghostty's `quick-terminal-*` keys and a `keybind = global:...=toggle_quick_terminal` hotkey.
- The server gains a `quick` workspace flag and an `AttachQuick` message.
- The client gains a pure `quick` module: hotkey mapping, window rectangle, toggle and slide maths.
- A `--quick-terminal` mode of the client process holds the hotkey and keeps the hidden window ready. Every normal launch starts that process when it is not running.

**Tech Stack:** Rust; the `windows` crate (Win32: `RegisterHotKey`, `MonitorFromPoint`, `GetMonitorInfoW`, `MapVirtualKeyW`, `CreateMutexW`).

**Spec:** `docs/superpowers/specs/2026-11-03-quick-terminal-design.md`

## Global Constraints

- **Config names and syntax follow Ghostty:**
  - `keybind = global:<trigger>=toggle_quick_terminal`
  - `quick-terminal-position`, `quick-terminal-size`, `quick-terminal-screen`, `quick-terminal-autohide`, `quick-terminal-animation-duration`
- **Defaults:**
  - the hotkey `super+backquote`
  - position `top`
  - size `40%`
  - screen `main`
  - autohide `true`
  - animation duration `0.2` seconds
- **Supported values:**
  - Position: only `top` and `bottom` are supported. `left`, `right` and `center` give the diagnostic "`<v>` is not supported yet; using top".
  - Size: a bare number with no `%` or `px` is a diagnostic.
  - Screen: `macos-menu-bar` is read as `main`.
- **The protocol:** `PROTO_VERSION` goes from 4 to 5.
- **The mutex:** `Local\wraith-quick-<16 hex digits of FNV-1a 64 of the pipe name>`.
- **Hidden means idle:** while hidden, the quick terminal renders nothing and runs no blink, spinner or motion timers.
- **Live checks:**
  - Never send global input.
  - Never touch `target\release`, the real `%APPDATA%\wraith\config` or the user's real pipe.
  - Use a private `WRAITH_PIPE_NAME` and a fake APPDATA and LOCALAPPDATA.
  - Stop your own processes by PID only.
- **Verify by exit code:** RTK reformats cargo output. Run `rtk proxy cargo ...` (or plain cargo, with zig on PATH) and check the exit code.

## Review Focus

1. **A hotkey held by another app.** One toast, the process keeps running, and a config reload with a new key registers it.
2. **Autohide.** Focus moving to the window's own popups (IME candidates, message boxes) must not hide it; focus moving to another app must.
3. **A server restart while the quick terminal is hidden.** The process must exit quietly, with no message box, because nobody sees it.
4. **A normal window and the quick workspace.** A normal window opened while the quick terminal is not running must never pick up the quick workspace by itself. When the quick terminal starts, it takes the workspace back from a window that chose it by id.
5. **Monitors.** A summon on a monitor with a top or left taskbar, or at 150% DPI, must fill the work area correctly and not cover the taskbar.

---

### Task 1: Config — quick-terminal keys and the global hotkey

**Files:**
- Modify: `crates/config/src/lib.rs` (new types and `Config` fields, defaults, re-exports)
- Modify: `crates/config/src/keys.rs` (the `keybind` key and new keys)
- Modify: `crates/config/src/keybind.rs` (`parse_global`)
- Modify: `crates/config/src/settings.rs` (two rows)
- Test: `crates/config/tests/parse.rs`, `crates/config/tests/keybind.rs`, `crates/config/tests/settings.rs`

**Interfaces:**
- **Produces, in `wraith_config`:**
  - `pub enum QuickPosition { Top, Bottom }`
  - `pub enum QuickScreen { Main, Mouse }`
  - `pub enum Dim { Percent(f32), Px(u32) }`
  - `pub struct QuickSize { pub primary: Dim, pub secondary: Option<Dim> }`
  - These `Config` fields:
    - `quick_terminal_hotkey: Option<Trigger>`
    - `quick_terminal_position: QuickPosition`
    - `quick_terminal_size: QuickSize`
    - `quick_terminal_screen: QuickScreen`
    - `quick_terminal_autohide: bool`
    - `quick_terminal_animation_duration: f32`
  - `pub fn parse_global(value: &str) -> Result<Option<Trigger>, String>` in `keybind`, re-exported. `value` is the part after `global:`. It returns `Ok(Some(trigger))` for `toggle_quick_terminal` and `Ok(None)` for `unbind`.
  - All types derive `Clone, Debug, PartialEq`. `QuickPosition`, `QuickScreen` and `Dim` also derive `Copy`.

- [ ] **Step 1: Write the failing tests**

In `crates/config/tests/parse.rs`, use the file's existing helper for loading a config from text (whatever its current tests use):

```rust
#[test]
fn quick_terminal_defaults() {
    let c = Config::default();
    assert_eq!(c.quick_terminal_hotkey, Some(Trigger { mods: Mods(Mods::SUPER), key: TriggerKey::Named("backquote".into()) }));
    assert_eq!(c.quick_terminal_position, QuickPosition::Top);
    assert_eq!(c.quick_terminal_size, QuickSize { primary: Dim::Percent(40.0), secondary: None });
    assert_eq!(c.quick_terminal_screen, QuickScreen::Main);
    assert!(c.quick_terminal_autohide);
    assert_eq!(c.quick_terminal_animation_duration, 0.2);
}

#[test]
fn quick_terminal_keys_parse() {
    let c = load_str("quick-terminal-position = bottom\nquick-terminal-size = 50%,800px\nquick-terminal-screen = mouse\nquick-terminal-autohide = false\nquick-terminal-animation-duration = 0\n");
    assert_eq!(c.quick_terminal_position, QuickPosition::Bottom);
    assert_eq!(c.quick_terminal_size, QuickSize { primary: Dim::Percent(50.0), secondary: Some(Dim::Px(800)) });
    assert_eq!(c.quick_terminal_screen, QuickScreen::Mouse);
    assert!(!c.quick_terminal_autohide);
    assert_eq!(c.quick_terminal_animation_duration, 0.0);
    assert_eq!(load_str("quick-terminal-size = 300px\n").quick_terminal_size.primary, Dim::Px(300));
    assert_eq!(load_str("quick-terminal-screen = macos-menu-bar\n").quick_terminal_screen, QuickScreen::Main);
}

#[test]
fn quick_terminal_bad_values_are_diagnostics() {
    for (line, want) in [
        ("quick-terminal-position = left", "not supported yet; using top"),
        ("quick-terminal-size = 40", "quick-terminal-size"),
        ("quick-terminal-size = 0%", "quick-terminal-size"),
        ("quick-terminal-animation-duration = 2", "quick-terminal-animation-duration"),
    ] {
        let (c, d) = load_with_diags(line);
        assert!(d.iter().any(|d| d.message.contains(want)), "{line}: {d:?}");
        assert_eq!(c.quick_terminal_position, QuickPosition::Top);
    }
}

#[test]
fn global_keybind_sets_the_hotkey() {
    let c = load_str("keybind = global:ctrl+shift+f12=toggle_quick_terminal\n");
    assert_eq!(c.quick_terminal_hotkey, Some(Trigger { mods: Mods(Mods::CTRL | Mods::SHIFT), key: TriggerKey::Named("f12".into()) }));
    assert_eq!(load_str("keybind = global:super+backquote=unbind\n").quick_terminal_hotkey, None);
    assert_eq!(load_str("keybind = clear\n").quick_terminal_hotkey, None);
    assert!(load_str("keybind = clear\nkeybind =\n").quick_terminal_hotkey.is_some(), "an empty keybind restores the defaults");
    let (_, d) = load_with_diags("keybind = global:ctrl+a=new_tab");
    assert!(d.iter().any(|d| d.message.contains("toggle_quick_terminal")), "{d:?}");
    let (_, d) = load_with_diags("keybind = ctrl+a=toggle_quick_terminal");
    assert!(d.iter().any(|d| d.message.contains("global:")), "{d:?}");
}
```

In `crates/config/tests/keybind.rs`:

```rust
#[test]
fn parse_global_rejects_sequences() {
    assert!(parse_global("ctrl+a>ctrl+b=toggle_quick_terminal").is_err());
}
```

In `crates/config/tests/settings.rs`:
- Bump the counts: `all()` from 20 to 22, and `Behaviour` from 7 to 9.
- Assert that `quick-terminal-position` is a `Kind::Choice` with `[("top","Top"),("bottom","Bottom")]`.
- Assert that `quick-terminal-autohide` is a `Kind::Toggle`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p wraith-config`
Expected: compile errors for the missing `QuickPosition`, `parse_global` and the new fields.

- [ ] **Step 3: Implement**

- **Parsing a size:** `quick_size(v)` splits on `,` into one or two parts. Each part is `N%`, with N in 1–100, or `Npx`, with N ≥ 1. Anything else is the error "invalid quick-terminal-size `<v>`: use 40% or 300px".
- **The animation duration:** in 0–1. Errors name the key.
- **`parse_global(v)`:**
  - Reuse the private `parse_trigger`, and split on the last `=` as `parse_keybind` does.
  - More than one trigger (`>`) is the error "a global keybind takes one key".
  - The action `toggle_quick_terminal` gives `Some`, and `unbind` gives `None`.
  - Any other action is the error "`global:` only supports toggle_quick_terminal".
- **The `keybind` key in `keys.rs`:**
  - A `global:` prefix calls `parse_global` and stores the result in `quick_terminal_hotkey`.
  - An empty value also resets `quick_terminal_hotkey` to its default.
  - `clear` also sets it to `None`.
  - A non-global binding whose action text is `toggle_quick_terminal` is the error "toggle_quick_terminal needs `global:`". Check the raw action text before calling `keybind::apply`. That way no `Action` variant is added.
- **The settings rows** go on the Behaviour page, after `sidebar-visible`:
  - "Quick terminal position", `Kind::Choice(&[("top","Top"),("bottom","Bottom")])`
  - "Quick terminal hides on focus loss", `Kind::Toggle`

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p wraith-config`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/config
git commit -m "feat(config): quick-terminal keys and the global toggle_quick_terminal hotkey"
```

### Task 2: Server — the quick workspace and AttachQuick

**Files:**
- Modify: `crates/proto/src/model.rs` (`Workspace.quick`)
- Modify: `crates/proto/src/wire.rs` (`ClientMsg::AttachQuick`, `PROTO_VERSION = 5`)
- Modify: `crates/server/src/model.rs` (`quick_workspace`, `most_recent_detached`)
- Modify: `crates/server/src/session.rs` (dispatch)
- Modify every `Workspace { .. }` struct literal (add `quick: false`):
  - `crates/cli/tests/events.rs`
  - `crates/client/tests/palette.rs`
  - `crates/client/tests/view.rs:352`
  - `crates/proto/tests/model.rs`
  - `crates/server/src/model.rs` (`add_workspace`)
- Test: `crates/server/tests/model.rs`, `crates/server/tests/session.rs`

**Interfaces:**
- **Produces:**
  - `Workspace { .., #[serde(default)] pub quick: bool }`
  - `ClientMsg::AttachQuick`
  - `Model::quick_workspace(&mut self) -> WorkspaceId`, which finds the flagged workspace or creates one named "Quick" with `quick: true`.

- [ ] **Step 1: Write the failing tests**

In `crates/server/tests/model.rs`:

```rust
#[test]
fn quick_workspace_is_created_once_and_skipped_by_reattach() {
    let mut m = Model::default();
    let q = m.quick_workspace();
    assert_eq!(m.quick_workspace(), q);
    let ws = m.snapshot().workspaces.into_iter().find(|w| w.id == q).unwrap();
    assert!(ws.quick);
    assert_eq!(ws.name, "Quick");
    assert_eq!(m.most_recent_detached(), None, "a normal window never picks the quick workspace");
    let normal = m.new_empty_workspace();
    assert_eq!(m.most_recent_detached(), Some(normal));
}

#[test]
fn quick_flag_survives_the_layout_and_old_layouts_load() {
    let mut m = Model::default();
    let q = m.quick_workspace();
    let back = Model::from_json(&m.to_json()).unwrap();
    assert!(back.snapshot().workspaces.iter().any(|w| w.id == q && w.quick));
    let old = m.to_json().replace(",\"quick\":true", "");
    assert!(Model::from_json(&old).is_ok());
}
```

(Use the file's existing constructor and snapshot accessor if they're named differently from `Model::default()` and `snapshot()`.)

In `crates/server/tests/session.rs`:
- Generalize `Client::attach_window` into `fn attach(&mut self, m: ClientMsg) -> WorkspaceId`, and keep `attach_window(ws)` as `self.attach(ClientMsg::AttachWindow { workspace: ws })`.
- Add:

```rust
#[test]
fn attach_quick_gets_the_quick_workspace_and_takes_it_back() {
    let pipe = server();
    let mut q = Client::gui(&pipe);
    let wq = q.attach(ClientMsg::AttachQuick);
    assert!(q.snap.workspace(wq).unwrap().quick);
    q.send(ClientMsg::DetachWindow);
    thread::sleep(Duration::from_millis(100));
    let mut a = Client::gui(&pipe);
    assert_ne!(a.attach_window(None), wq, "a normal window skips the quick workspace");
    assert_eq!(a.attach_window(Some(wq)), wq, "but may open it by id while the quick terminal is gone");
    let mut q2 = Client::gui(&pipe);
    assert_eq!(q2.attach(ClientMsg::AttachQuick), wq);
    assert!(std::iter::from_fn(|| a.next(secs(2))).any(|m| matches!(m, ServerMsg::WorkspaceTaken { workspace } if workspace == wq)));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p wraith-server --test model --test session`
Expected: compile errors: no `quick_workspace`, no `AttachQuick`.

- [ ] **Step 3: Implement**

- Add the field (`#[serde(default)]`) and the message, and bump `PROTO_VERSION`.
- `most_recent_detached` filters `!w.quick`.
- `quick_workspace` uses `add_workspace(Some("Quick".into()), None, "")`, sets `quick = true`, and calls `touch()`.
- In `session.rs`: `ClientMsg::AttachQuick => { let ws = self.model.quick_workspace(); self.take_workspace(id, ws) }`. Use the same `Gui`-only gate that `AttachWindow` has.
- Add `quick: false` to every struct literal listed above.

- [ ] **Step 4: Run the whole workspace's tests**

Run: `cargo test --workspace`
Expected: exit 0. The protocol bump and the new field touch all the crates.

- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "feat(server): a quick workspace normal windows skip, and AttachQuick"
```

### Task 3: Client — the pure quick-terminal maths

**Files:**
- Create: `crates/client/src/quick.rs` (pure functions first; Task 4 adds the Win32 side)
- Modify: `crates/client/src/lib.rs` (`pub mod quick;`)
- Modify: `crates/client/src/app.rs` (`named_key` and `key_char` become `pub(crate)`)
- Test: `crates/client/tests/quick.rs`

**Interfaces:**
- **Consumes:**
  - `wraith_config::{Trigger, TriggerKey, QuickPosition, QuickSize, Dim}` (Task 1)
  - `crate::input::scancode_to_code`
  - `wraith_proto::model::Rect`
- **Produces, in `wraith_client::quick`:**
  - `pub const MOD_ALT: u32 = 1, MOD_CONTROL: u32 = 2, MOD_SHIFT: u32 = 4, MOD_WIN: u32 = 8, MOD_NOREPEAT: u32 = 0x4000;`
  - `pub fn hotkey_code(t: &Trigger) -> Option<(u32, KeyCode)>`: the Win32 modifiers, including `MOD_NOREPEAT`, and the physical key.
  - `pub fn scancode_of(code: KeyCode) -> Option<(u16, bool)>`: (the set-1 scancode, whether it's an extended key), found by searching `scancode_to_code` over 0x01–0x7F, non-extended first.
  - `pub fn quick_rect(work: Rect, pos: QuickPosition, size: &QuickSize, dpi: u32) -> Rect`
  - `pub enum Toggle { Show, Hide, Front }` and `pub fn toggle(visible: bool, foreground: bool) -> Toggle`
  - `pub fn slide_y(target: Rect, pos: QuickPosition, progress: f32, showing: bool) -> i32`
  - `pub fn progress(elapsed: Duration, duration_s: f32) -> f32`: 1.0 when the duration is ≤ 0, otherwise clamped to 0–1.
  - `pub fn mutex_name(pipe: &str) -> String`

- [ ] **Step 1: Write the failing tests** in `crates/client/tests/quick.rs`

```rust
use std::time::Duration;
use wraith_client::quick::*;
use wraith_config::{Dim, QuickPosition, QuickSize, Trigger, TriggerKey};
use wraith_proto::{KeyCode, Mods, model::Rect};

fn trig(mods: u8, key: TriggerKey) -> Trigger { Trigger { mods: Mods(mods), key } }
const WORK: Rect = Rect { x: 0, y: 0, w: 1920, h: 1040 };
fn pct(p: f32) -> QuickSize { QuickSize { primary: Dim::Percent(p), secondary: None } }

#[test]
fn hotkeys_map_to_win32() {
    assert_eq!(hotkey_code(&trig(Mods::SUPER, TriggerKey::Named("backquote".into()))), Some((MOD_WIN | MOD_NOREPEAT, KeyCode::Backquote)));
    assert_eq!(hotkey_code(&trig(Mods::CTRL | Mods::SHIFT, TriggerKey::Named("f12".into()))), Some((MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, KeyCode::F12)));
    assert_eq!(hotkey_code(&trig(Mods::ALT, TriggerKey::Char('`'))).map(|h| h.1), Some(KeyCode::Backquote));
    assert_eq!(hotkey_code(&trig(0, TriggerKey::Physical("KeyQ".into()))).map(|h| h.1), Some(KeyCode::KeyQ));
    assert_eq!(hotkey_code(&trig(0, TriggerKey::Named("nope".into()))), None);
    assert_eq!(scancode_of(KeyCode::Backquote), Some((0x29, false)));
    assert_eq!(scancode_of(KeyCode::F12), Some((0x58, false)));
}

#[test]
fn rect_fills_the_work_area() {
    assert_eq!(quick_rect(WORK, QuickPosition::Top, &pct(40.0), 96), Rect { x: 0, y: 0, w: 1920, h: 416 });
    assert_eq!(quick_rect(WORK, QuickPosition::Bottom, &pct(40.0), 96), Rect { x: 0, y: 624, w: 1920, h: 416 });
    let px = QuickSize { primary: Dim::Px(300), secondary: Some(Dim::Px(800)) };
    assert_eq!(quick_rect(WORK, QuickPosition::Top, &px, 144), Rect { x: 360, y: 0, w: 1200, h: 450 });
    let offset = Rect { x: 100, y: 48, w: 1820, h: 1032 };
    assert_eq!(quick_rect(offset, QuickPosition::Top, &pct(50.0), 96), Rect { x: 100, y: 48, w: 1820, h: 516 });
    assert_eq!(quick_rect(WORK, QuickPosition::Top, &QuickSize { primary: Dim::Px(5000), secondary: None }, 96).h, 1040, "clamped to the work area");
}

#[test]
fn toggle_rules() {
    assert_eq!(toggle(false, false), Toggle::Show);
    assert_eq!(toggle(true, true), Toggle::Hide);
    assert_eq!(toggle(true, false), Toggle::Front);
}

#[test]
fn slide_runs_from_off_screen_to_place() {
    let r = Rect { x: 0, y: 48, w: 1920, h: 400 };
    assert_eq!(slide_y(r, QuickPosition::Top, 0.0, true), 48 - 400);
    assert_eq!(slide_y(r, QuickPosition::Top, 1.0, true), 48);
    assert_eq!(slide_y(r, QuickPosition::Top, 1.0, false), 48 - 400);
    assert_eq!(slide_y(r, QuickPosition::Bottom, 0.0, true), 48 + 400);
    let mid = slide_y(r, QuickPosition::Top, 0.5, true);
    assert!(mid > 48 - 200 && mid < 48, "eased out: past halfway at half time, got {mid}");
    assert_eq!(progress(Duration::from_millis(100), 0.2), 0.5);
    assert_eq!(progress(Duration::ZERO, 0.0), 1.0);
    assert_eq!(progress(Duration::from_secs(9), 0.2), 1.0);
}

#[test]
fn mutex_name_is_stable_and_per_pipe() {
    let a = mutex_name(r"\\.\pipe\wraith-S-1-5-21-1");
    assert!(a.starts_with(r"Local\wraith-quick-") && a.len() == r"Local\wraith-quick-".len() + 16, "{a}");
    assert_eq!(a, mutex_name(r"\\.\pipe\wraith-S-1-5-21-1"));
    assert_ne!(a, mutex_name(r"\\.\pipe\wraith-test"));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p wraith-client --test quick`
Expected: compile error: no module `quick`.

- [ ] **Step 3: Implement `quick.rs` (pure functions only)**

- **`hotkey_code`:**
  - `Named` goes through `named_key`.
  - `Char(c)` goes through the first code from `scancode_of`'s search whose `key_char` is `c`.
  - `Physical(n)` goes through the first searched code whose `format!("{code:?}") == n`.
  - The modifiers map SUPER→WIN, CTRL→CONTROL, ALT→ALT and SHIFT→SHIFT, plus NOREPEAT.
- **`quick_rect`:**
  - `Px` is scaled by `dpi / 96` and rounded. `Percent` is a fraction of that axis of the work area.
  - Both are clamped to 1 to that axis of the work area.
  - The secondary axis defaults to the full width and is centred.
- **`slide_y`:**
  - The off-screen position is `target.y - h` for top and `target.y + h` for bottom.
  - Showing gives `off + (target - off) * ease_out(p)`.
  - Hiding gives `target + (off - target) * ease_out(p)`.
  - Round to an `i32`.
- **`mutex_name`:** FNV-1a 64 (offset `0xcbf29ce484222325`, prime `0x100000001b3`) over the UTF-8 bytes, formatted `{:016x}`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p wraith-client --test quick`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/client
git commit -m "feat(client): quick-terminal hotkey mapping, placement and slide maths"
```

### Task 4: Client — the `--quick-terminal` process

**Files:**
- Modify: `crates/client/src/main.rs` (`--quick-terminal` calls `app::run(true)`; the default calls `app::run(false)`)
- Modify: `crates/client/src/window.rs` (`Window::create_ex(title, w, h, quick: bool)`; `create` calls it with `false`; quick adds `WS_EX_TOOLWINDOW | WS_EX_TOPMOST`)
- Modify: `crates/client/src/link.rs` (`spawn_detached(exe, arg, pipe, env)`; `spawn_server` calls it with `"--server"`)
- Modify: `crates/client/src/quick.rs` (the Win32 side, as `reload.rs` does it: free functions taking `&mut App`)
- Modify: `crates/client/src/app.rs` (the `quick: Option<quick::State>` field and the hooks listed below)
- Modify: `crates/client/src/reload.rs` (`apply_config` calls `quick::on_reload(app, &old)` before `request_redraw`)
- Test: `crates/client/tests/app.rs` or `quick.rs`, for any pure helper added here

**Interfaces:**
- **Consumes:** Tasks 1–3.
- **Produces:**
  - `pub fn run(quick: bool) -> Result<()>`
  - In `quick.rs`:
    - `pub(crate) struct State { shown: bool, slide: Option<(Instant, bool)>, target: Rect, registered: Option<Trigger>, _mutex: OwnedHandle }`
    - `pub(crate) fn take_instance(pipe: &str) -> Option<OwnedHandle>`: `CreateMutexW`. Returns `None` when `ERROR_ALREADY_EXISTS`.
    - `pub(crate) fn running(pipe: &str) -> bool`: `OpenMutexW(SYNCHRONIZE)` succeeds.
    - `pub(crate) fn on_hotkey(app: &mut App)`
    - `pub(crate) fn hide(app: &mut App)`
    - `pub(crate) fn on_deactivate(app: &mut App, other: HWND)`
    - `pub(crate) fn tick(app: &mut App, now: Instant) -> Option<u32>`: the sleep while sliding.
    - `pub(crate) fn on_reload(app: &mut App, old: &Config)`
    - `pub(crate) fn register(app: &mut App)`

- [ ] **Step 1: Startup in `run(quick)`**
  - **Quick mode:**
    - Take the instance first, and return `Ok(())` if another process holds it.
    - Create the window with `create_ex(.., true)` and never call `win.show()`.
    - Send `ClientMsg::AttachQuick` instead of `AttachWindow { workspace: None }`.
    - Call `quick::register(app)` once the `App` exists.
  - **Normal mode:** after `link.send(AttachWindow ..)`, if `config.quick_terminal_hotkey.is_some() && !quick::running(&pipe_name)`, call `link::spawn_detached(&exe, "--quick-terminal", &pipe_name, &spawn_env())`. Log any error. Never fail the window because of it.

- [ ] **Step 2: Registration and toasts**
  - **`register`:**
    - `UnregisterHotKey(hwnd, 1)` first.
    - Then `hotkey_code`, then `scancode_of`, then `MapVirtualKeyW(sc | if ext {0xE000} else {0}, MAPVK_VSC_TO_VK_EX)`, then `RegisterHotKey(hwnd, 1, mods, vk)`.
  - **On failure:** `app.notifier.show(Ways { toast: true, ..Default::default() }, 0, "Wraith quick terminal", &format!("{key} is in use by another app. Set another with keybind = global:...=toggle_quick_terminal"))`. Store `registered = None`.
  - **`on_reload`:**
    - No hotkey left: set `app.quit = true` (the loop exits, `DetachWindow` is sent by the existing close path or the pipe drop).
    - Hotkey changed, or not registered: `register`.
    - Shown, and the position, size or screen changed: re-place the window at the new `quick_rect`.
    - Normal windows ignore all of this.

- [ ] **Step 3: Show, hide and slide**
  - **The work area:**
    - `main` is `MonitorFromPoint((0,0), MONITOR_DEFAULTTOPRIMARY)`.
    - `mouse` is `MonitorFromPoint(GetCursorPos(), MONITOR_DEFAULTTONEAREST)`.
    - Then `GetMonitorInfoW(..).rcWork` is turned into a `Rect`, with the DPI from `GetDpiForMonitor` (MDT_EFFECTIVE_DPI).
  - **Show:**
    - `target = quick_rect(..)`.
    - `SetWindowPos(HWND_TOPMOST, target.x, slide_y(target,pos,0,true), target.w, target.h, SWP_NOACTIVATE)`. The `WM_SIZE` this sends resizes the swap chain while the window is hidden.
    - `render_now(true)`, `ShowWindow(SW_SHOW)`, `SetForegroundWindow`.
    - If the duration is greater than 0 and `window::animations_enabled()`, start the slide. Otherwise go straight to `target.y`.
    - Set `shown = true` and `dirty = true`.
  - **Hide:**
    - With an animation, start the reverse slide.
    - Without one, `ShowWindow(SW_HIDE)` straight away.
    - Set `shown = false` when the hide begins.
  - **`tick`:**
    - While sliding, `SetWindowPos(.., slide_y(..), SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)`.
    - When progress reaches 1, clear the slide, and call `SW_HIDE` if the slide was a hide.
    - Return `Some(8)` while sliding.
  - **`on_hotkey`:** match `toggle(shown, GetForegroundWindow() == hwnd)`: `Show` shows, `Hide` hides, and `Front` calls `SetForegroundWindow`.

- [ ] **Step 4: Hooks in `app.rs`**
  - **`on_msg` `Raw`:**
    - `WM_HOTKEY` calls `quick::on_hotkey`.
    - `WM_ACTIVATE` with `LOWORD(wparam) == WA_INACTIVE` calls `quick::on_deactivate(app, HWND(lparam))`. That hides the window only when:
      - autohide is on;
      - the window is shown;
      - no slide is running;
      - `GetAncestor(other, GA_ROOTOWNER) != hwnd`.
    - Return `None` so the default handling still runs.
  - **`WM_DISPLAYCHANGE`:** in quick mode while shown, re-place the window at a freshly computed `quick_rect` (spec §5, a monitor removed).
  - **`WinMsg::Close`:** in quick mode, `quick::hide` and return `Some(LRESULT(0))`.
  - **The caption buttons:** the `WM_NCLBUTTONUP` caption branch, in quick mode, makes Minimize and Close call `quick::hide`, and Maximize does nothing.
  - **`ExitOut::CloseWindow`:** in quick mode, send the messages and call `quick::hide` instead of `detach_and_close`.
  - **`reconnect`:** quick mode sends `AttachQuick`.
  - **`ServerMsg::WorkspaceTaken`:** quick mode replaces the view's messages with `[ClientMsg::AttachQuick]`.
  - **`drain_server_msgs`:** quick mode with `server_lost` sets `self.quit = true`, with no dialog and no "press Enter".
  - **`idle`:**
    - In quick mode while hidden and not sliding: drain the server messages, run `reload::poll`, then return `Idle { sleep_ms: next reload deadline or INFINITE (u32::MAX), wait_on: None }`. Skip blink, spinner, motion and rendering.
    - Otherwise call `quick::tick` and `min` its sleep into `sleep`.

- [ ] **Step 5: Build, lint and test**

Run: `cargo build -p wraith-client && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: every command exits 0.

- [ ] **Step 6: Commit**

```bash
git add crates/client
git commit -m "feat(client): --quick-terminal process with a global hotkey, slide and autohide"
```

### Task 5: Live check, docs, roadmap

**Files:**
- Create: `docs/quick-terminal-acceptance.md`
- Modify:
  - `README.md` (a quick-terminal section with the keys table; the roadmap row for the drop-down window goes to ✅, with auto-update and ARM64 left ❌)
  - `site/index.html` (the drop-down item moves from "Next" to done; a short feature line)
  - `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md` §10 (a quick-terminal row; the Backlog keeps auto-update and ARM64)
  - the spec status line ("implemented")

- [ ] **Step 1: The live check**
  - **Build.** `cargo build` (debug).
  - **Fake environment.** A fake APPDATA config holds `keybind = global:ctrl+alt+shift+f11=toggle_quick_terminal`, an unlikely key, so registration succeeds without clashing with the user's real Win+`.
  - **Start.** Start `target\debug\wraith.exe --quick-terminal` with that APPDATA, a fake LOCALAPPDATA and a private `WRAITH_PIPE_NAME`.
  - **Check:**
    - The process is alive and its window is hidden (`IsWindowVisible` false).
    - A second `--quick-terminal` on the same pipe exits within 2 s.
  - **Toggle.** Tell the user first, because this brings a window to the front. Post `WM_HOTKEY` (wparam 1) to the HWND, then check:
    - `IsWindowVisible` is true;
    - `GetWindowRect` matches 40% of the primary work area;
    - a screenshot shows the tab bar and a prompt.

    Post it again and check the window hides.
  - **Clean up.** `wr ctl server-stop` on the private pipe, then stop any leftover processes of your own by PID.
  - **Record.** Write the results in the acceptance doc, including what was not checked live (the real Win+` press, autohide by clicking another app, multiple monitors).

- [ ] **Step 2: The docs.** Follow the README's existing key-table style. The site gets one feature line and the roadmap move.

- [ ] **Step 3: Commit**

```bash
git add docs README.md site/index.html
git commit -m "docs: quick terminal acceptance, README, site and spec status"
```
