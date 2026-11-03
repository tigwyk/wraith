# Per-profile appearance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each pane draws in its profile's font face, font size, padding and background opacity. Zoom works per pane, and window transparency works for the first time.

**Architecture:**
- The client gets a font table (`FontId` → `FontStack`), and `GlyphKey` gains `font`, so one atlas serves every font.
- `WindowView` gives each pane a `Look` (font, cell, pad, opacity), used for layout, spawn, mouse and drawing. The server is unchanged: it already takes the cell size per pane.
- When any opacity is below 1, the window presents through a DirectComposition swap chain with premultiplied alpha. Otherwise it keeps today's HWND swap chain.

**Tech Stack:** Rust; `windows` 0.62 (Direct3D 11, DXGI, DirectWrite, DirectComposition); serde_json for WT settings.

**Spec:** `docs/superpowers/specs/2026-11-03-per-profile-appearance-design.md`. Read it with this plan; § references below point into it.

**Base branch:** `feat/wt-profiles` (tigwyk/wraith#11), or `main` once that is merged. This plan names spec 1's code (`profiles.rs`, `wt.rs`, `ProfileExtra`), which does not exist on older `main`.

## Global Constraints

- Never touch the real `%APPDATA%\wraith\config` or `target\release`. Never push to `main`. Commit per task; push only when the user asks.
- Live runs use a private `WRAITH_PIPE_NAME` and `WRAITH_LAYOUT_PATH`, a fake `APPDATA`/`LOCALAPPDATA`, the window shown without activation, and posted input only. Never use `scripts/keys.ps1` or `scripts/shot.ps1`.
- Verify builds and tests by exit code: `rtk proxy cargo test -p <crate> ...; echo EXIT=$?`. Never grep RTK's output for "error".
- With every opacity at 1, presentation stays the HWND flip swap chain exactly as today (parent spec §6.3).
- A profile that sets none of the four values gives a pane identical to today's.
- Ranges: font size 4–72 (`clamp_font_size`); padding 0–32 per axis; opacity 0.0–1.0 in Wraith config, 0–100 in WT.
- Chrome (tab bar, sidebar, palette, menus, settings, toasts, tooltips) uses font 0 (the window font) and is always opaque.

## Review Focus

1. **A pruned `FontId` reused for a different font** would make the atlas return another font's glyphs. Ids never repeat until the table and atlas are rebuilt together. Test: `ids_are_not_reused_after_prune` (Task 4).
2. **Zooming a pane until 2×1 cells no longer fit.** The pane must clamp, never send 0 rows. Test: `zoom_past_min_size_keeps_at_least_2x1` (Task 5).
3. **A profile font that is not installed** (WT's bundled Cascadia Mono) must fall back to the window families at the profile's size, not to DirectWrite's default font. Test: `uninstalled_family_falls_back_to_window_families` (Task 6).
4. **A WT value of the wrong type** (`"opacity": "high"`, `"padding": true`) must drop that value with a warning and keep the profile. Test: `wrong_typed_appearance_values_are_ignored` (Task 2).
5. **A split mixing a 12pt and a 20pt pane in one tab:** the tree layout's minimum must fit the larger cell, so neither pane gets under 2×1. Test: `tab_min_size_uses_largest_pane_look` (Task 5).

---

### Task 1: Transparency probe (throwaway)

**Files:**
- Create: `crates/client/examples/composition_probe.rs` (deleted again at the end of this task)
- Create: `docs/per-profile-appearance-acceptance.md` (probe section only)
- Modify: `crates/client/Cargo.toml`: add the `windows` feature `"Win32_Graphics_DirectComposition"`, which Task 7 also needs

**Interfaces:** Produces only findings. No code survives this task except the Cargo feature.

- [ ] **Step 1:** Write the example.
  - It opens a `WS_EX_NOREDIRECTIONBITMAP` window, shown without activation (`SW_SHOWNOACTIVATE`), over a solid #FF0000 helper window that is also shown without activation.
  - It makes a `CreateSwapChainForComposition` swap chain: `DXGI_ALPHA_MODE_PREMULTIPLIED`, `B8G8R8A8`, flip-discard, `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT`, and `SetMaximumFrameLatency(1)`.
  - It binds the swap chain through `DCompositionCreateDevice` → `CreateTargetForHwnd(hwnd, true)` → `CreateVisual` → `SetContent` → `SetRoot` → `Commit`.
  - Each frame clears to premultiplied black at alpha 0.5 (`[0,0,0,0.5]`).
  - A `--hwnd` flag uses `CreateSwapChainForHwnd` with `ALPHA_MODE_IGNORE` instead, as `gpu.rs` does today.
- [ ] **Step 2:** Run it and capture the screen region with `Graphics.CopyFromScreen`, without activating either window.
  - Run: `cargo run -p wraith-client --example composition_probe`
  - Expected: the pixel at the window centre is about #800000 (red behind, blended 50%). With `--hwnd` it is #000000.
- [ ] **Step 3:** Measure present-to-present latency with the waitable object: time from `WaitForSingleObjectEx` returning to `Present` returning, over 600 frames in each mode. Record the median and p99 for each.
- [ ] **Step 4:** Write both results into `docs/per-profile-appearance-acceptance.md` under "Probe", then delete the example.
  - If the composition pixel is not blended, stop and report to the user: §4 of the spec needs rework.
  - If composition p99 is more than 1 ms above HWND p99, note it. The opt-in rule (Task 7) stands either way.
- [ ] **Step 5: Commit** — `git add crates/client/Cargo.toml docs/per-profile-appearance-acceptance.md && git commit -m "docs: composition swap chain probe results"`

### Task 2: Appearance values in config and WT parsing

**Files:**
- Modify: `crates/config/src/lib.rs` (`ProfileExtra`, new pub value parsers)
- Modify: `crates/config/src/keys.rs:144-157` (the `profile-*` arm)
- Modify: `crates/config/src/wt.rs` (`WtProfile`, `WtSettings`, `parse`)
- Modify: `crates/config/tests/fixtures/wt/settings.json`
- Test: `crates/config/tests/parse.rs`, `crates/config/tests/wt.rs`

**Interfaces:**
- Produces, in `wraith_config`:
  - `ProfileExtra::{Dir, Theme, Icon, Font, FontSize, Padding, Opacity}`
  - `pub fn font_list(v: &str) -> Option<Vec<String>>`: split on `,`, trimmed, empties dropped. `None` if the result is empty.
  - `pub fn font_size(v: &str) -> Option<f32>`: a number in 4..=72, else `None`.
  - `pub fn padding(v: &str) -> Option<(u16, u16)>`: `"n"` → (n, n); `"x,y"` → (x, y); `"l,t,r,b"` → (l, t). Spaces allowed. Each value in 0..=32, else `None`.
  - `pub fn opacity(v: &str) -> Option<f32>`: a number in 0.0..=1.0.
- Produces, in `wraith_config::wt`:
  - `WtProfile` gains `font: Option<Vec<String>>`, `font_size: Option<f32>`, `padding: Option<(u16, u16)>` and `opacity: Option<f32>`. Its derive becomes `#[derive(Clone, Debug, Default, PartialEq)]`, losing `Eq`.
  - `WtSettings` gains `warnings: Vec<String>`.

- [ ] **Step 1: Write the failing tests**
  - `parse.rs` `profile_appearance_keys_round_trip`: parses `profile-font = U:JetBrainsMono NF, Cascadia Mono`, `profile-font-size = U:14`, `profile-padding = U:8,4`, `profile-opacity = U:0.85` and `profile-font-size = U:13` (the second size line replaces the first). It asserts no diagnostics, and that `profile_extras` holds `Font "JetBrainsMono NF, Cascadia Mono"`, `Padding "8,4"`, `Opacity "0.85"` and `FontSize "13"`, in the existing last-line-wins order.
  - `parse.rs` `profile_appearance_bad_values_are_diagnostics`: `profile-font-size = U:200`, `profile-opacity = U:1.5`, `profile-padding = U:40` and `profile-font = U: , ` each give one line diagnostic, so `d.len() == 4`.
  - `wt.rs` `appearance_fields`: extend the fixture.
    - The PowerShell entry gets `"font": {"face": "Cascadia Mono, Consolas", "size": 11}`, `"padding": "8, 4, 8, 4"` and `"opacity": 85`.
    - cmd gets the old `"fontFace": "Consolas"` and `"fontSize": 13` and `"padding": 6` (a number).
    - `profiles.defaults` gets `"font": {"size": 10}`.

    Assert: PowerShell has `font == Some(vec!["Cascadia Mono", "Consolas"])`, `font_size == Some(11.0)`, `padding == Some((8, 4))` and `opacity == Some(0.85)`. cmd has `font == Some(vec!["Consolas"])`, `font_size == Some(13.0)` and `padding == Some((6, 6))`. A profile with no font size gets `Some(10.0)` from the defaults.
  - `wt.rs` `appearance_ranges`: inline JSON with `"opacity": 150` → `opacity == None` and a warning. `"font": {"size": 200}` → `Some(72.0)`, clamped.
  - `wt.rs` `wrong_typed_appearance_values_are_ignored` (Review Focus 4): `"opacity": "high"` and `"padding": true` → both `None`, the profile is still listed, and `warnings.len() == 2`.
- [ ] **Step 2: Run them to see them fail** — `rtk proxy cargo test -p wraith-config; echo EXIT=$?`. Expected: compile errors for the missing variants and fields. EXIT≠0.
- [ ] **Step 3: Implement.**
  - The `profile-*` arm in `keys.rs` matches the four new keys and checks each value with its parser before storing the string.
  - In `wt.rs`, add a `fn appearance(m, warnings) -> (font, font_size, padding, opacity)` helper. `font.face`/`font.size` win over `fontFace`/`fontSize`. `padding` is a string, or a number via `to_string()`. Each value is clamped to 32 before the `padding()` rules apply, because WT values are clamped while config lines are rejected (spec §2.1). Add a `"padding": "40"` → `(32, 32)` case to `appearance_ranges`. `opacity` must be an integer 0–100, divided by 100. The font size is clamped with `.clamp(4.0, 72.0)`, not rejected.
  - Update every `WtProfile { .. }` literal (in `wt.rs` and in `crates/client/tests/profiles.rs::wt_profile`) to end with `..WtProfile::default()`.
- [ ] **Step 4: Run them** — the same command. Expected EXIT=0.
- [ ] **Step 5: Commit** — `feat(config): per-profile font, size, padding and opacity keys and WT parsing`

### Task 3: Profile fields, merge and import

**Files:**
- Modify: `crates/server/src/profile.rs:16-28` (`Profile`)
- Modify: `crates/client/src/profiles.rs` (`merge`)
- Modify: `crates/config/src/wt.rs` (`plan_import`)
- Test: `crates/client/tests/profiles.rs`, `crates/config/tests/import.rs`

**Interfaces:**
- Consumes: Task 2's parsers, `ProfileExtra` variants and `WtProfile` fields.
- Produces: `Profile` gains `font: Option<Vec<String>>`, `font_size: Option<f32>`, `padding: Option<(u16, u16)>` and `opacity: Option<f32>`. Its derive drops `Eq`, keeping `Clone, Debug, Default, PartialEq`. If a `HashSet<Profile>` or another `Eq` bound breaks, compare by name at that site instead.

- [ ] **Step 1: Write the failing tests**
  - `profiles.rs` `wt_appearance_reaches_profile`: a WT Ubuntu profile with `font_size: Some(14.0)` and `opacity: Some(0.9)` → the merged profile has the same.
  - `profiles.rs` `profile_lines_override_wt_appearance`: the same WT profile, plus `profile_extras` with `("Ubuntu 22.04", FontSize, "11")` and `(…, Padding, "3")` → `font_size == Some(11.0)` and `padding == Some((3, 3))`. The WT opacity of 0.9 is kept.
  - `import.rs` `import_writes_appearance_lines`: a WT profile with all four set → the output contains `profile-font = P:A, B`, `profile-font-size = P:14`, `profile-padding = P:8,4` and `profile-opacity = P:0.85`. Padding (6, 6) is written `P:6`. A second `plan_import` over that output adds nothing.
  - `import.rs` `import_skips_unset_appearance`: a profile with none set → no `profile-font*`, `profile-padding` or `profile-opacity` lines.
- [ ] **Step 2: Run** — `rtk proxy cargo test -p wraith-client --test profiles; rtk proxy cargo test -p wraith-config --test import; echo EXIT=$?`. Expected: fails.
- [ ] **Step 3: Implement.**
  - `merge` copies the WT fields.
  - The `profile_extras` loop sets each typed field with Task 2's parser. A value that does not parse cannot happen, because keys.rs already rejected it; ignore it if it does.
  - The "not a profile" warning's key-name match gets the four new key names.
  - `plan_import` appends the four lines after `profile-icon`, formatted as in the test: fonts joined with `", "`, size with `{}`, opacity with `{:.2}`.
- [ ] **Step 4: Run** — the same command. Expected EXIT=0. Also run `rtk proxy cargo test --workspace; echo EXIT=$?`, which must give EXIT=0, because the `Profile` derive changed.
- [ ] **Step 5: Commit** — `feat: per-profile appearance in the merged profile list and wr import-wt`

### Task 4: Font table and font-keyed atlas

**Files:**
- Create: `crates/client/src/font_table.rs` (`pub mod font_table;` in `lib.rs`)
- Modify: `crates/client/src/atlas.rs:20-26` (`GlyphKey`)
- Modify: `crates/client/src/grid.rs:536,602`, `crates/client/src/paint.rs:41` and `crates/client/tests/font.rs:75,80,91` (`GlyphKey` literals)
- Test: `crates/client/tests/font_table.rs`

**Interfaces:**
- Produces:

```rust
pub type FontId = u16;
#[derive(Clone, Debug, PartialEq)]
pub struct FontKey { pub families: Vec<String>, pub size_pt: f32, pub features: Vec<String> }
/// Generic over the loaded font so it is testable without DirectWrite; the app uses `FontTable<FontStack>`.
pub struct FontTable<F> { /* entries: Vec<(FontId, FontKey, F)>, next: FontId */ }
impl<F> FontTable<F> {
    pub fn new(window: FontKey, font: F) -> Self;               // entry 0
    pub fn get_or_load<E>(&mut self, key: &FontKey, load: impl FnOnce(&FontKey) -> Result<F, E>) -> Result<FontId, E>;
    pub fn get(&self, id: FontId) -> &F;                         // unknown id → entry 0
    pub fn retain(&mut self, used: &std::collections::HashSet<FontId>); // never drops 0
    /// True when the next new id would wrap; the caller then rebuilds table and atlas together.
    pub fn exhausted(&self) -> bool;
}
```

- `GlyphKey` gains `pub font: FontId` as its first field. `paint.rs` (chrome text) always passes `0`. `grid.rs` passes the pane's font id: add `font: FontId` to `DrawOpts`, so `push_fg` reads `opts.font`.

- [ ] **Step 1: Write the failing tests** in `font_table.rs`, with `F = &'static str`:
  - `same_key_same_id`: two `get_or_load` calls with an equal key return the same id, and the loader runs once.
  - `retain_drops_unused_but_not_window`: load ids 1 and 2, `retain({2})` → `get(1)` returns entry 0's value, and `get(2)` still works.
  - `ids_are_not_reused_after_prune` (Review Focus 1): load key A → 1, retain `{}`, load key B → 2 (not 1).
  - `load_error_is_returned_and_nothing_inserted`.
  - In `tests/font.rs` (GPU), extend the existing atlas test as `two_fonts_same_glyph_do_not_collide`. Keys `{font: 0, ..k}` and `{font: 1, ..k}` with two `FontStack`s at 12 and 24 pt give entries of different `h`.
- [ ] **Step 2: Run** — `rtk proxy cargo test -p wraith-client --test font_table --test font; echo EXIT=$?`. Expected: fails.
- [ ] **Step 3: Implement** `font_table.rs` and the `GlyphKey`/`DrawOpts` field. Ids grow monotonically from 1, and `exhausted()` is `next == FontId::MAX`.
- [ ] **Step 4: Run** — the same command, plus `rtk proxy cargo test -p wraith-client; echo EXIT=$?`. Expected EXIT=0.
- [ ] **Step 5: Commit** — `feat(client): font table and font-keyed glyph atlas`

### Task 5: Per-pane look in the view

**Files:**
- Modify: `crates/client/src/view.rs` (`Look`, `WindowView` look state, `cells_for`, `min_px`, `relayout`, spawn paths at lines ~467, 907, 955, 975, 1043; `est_cells`, `cell_px`)
- Test: `crates/client/tests/view.rs`

**Interfaces:**
- Consumes: `FontId` (Task 4).
- Produces, in `wraith_client::view`:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look { pub font: FontId, pub cell: (u32, u32), pub pad: (i32, i32), pub opacity: f32 }
impl WindowView {
    /// Window look plus one per profile name (zoom 0); relayout follows.
    pub fn set_looks(&mut self, window: Look, by_profile: HashMap<String, Look>) -> Vec<ClientMsg>;
    /// A zoomed pane's delta (points) and the look the app built for it; delta 0 clears both.
    pub fn set_zoom(&mut self, pane: PaneId, delta: f32, look: Look) -> Vec<ClientMsg>;
    pub fn zoom(&self, pane: PaneId) -> f32;                    // 0 when unset
    pub fn look(&self, pane: PaneId) -> Look;                   // zoom override, else profile, else window
    pub fn profile_look(&self, profile: &str) -> Look;          // for spawns
    pub fn zoomed_panes(&self) -> Vec<(PaneId, String, f32)>;   // (pane, profile, delta), for rebuilds
    /// Every font a look refers to (window, each profile, each zoom override): what the font table must keep.
    pub fn used_fonts(&self) -> HashSet<FontId>;
}
```

- Until `set_looks` is called, the window look is `Look { font: 0, cell: geom.cell, pad: geom.pad, opacity: 1.0 }`, so existing tests keep passing unchanged. `set_geometry` updates the window look's `cell` and `pad` while no `set_looks` has been made.
- `cells_for(rect, &Look)`. The tree layout (`layout`/`dividers`) takes one minimum: pass the per-axis maximum of `2*cell.0 + 2*pad.0` and `cell.1 + 2*pad.1` over the active tab's panes. The `Resize` and `sent_size` value uses the pane's `look.cell`. Zoom deltas and overrides are dropped for panes gone from the snapshot, next to `sent_size.retain`.
- Spawn: new tab, workspace and restart use `profile_look(prof.name)` for both the estimated cells (`cells_for(content, &look)`) and `cell_px`. A split halves the focused pane's rect, not its cells, and computes cells with the new profile's look.

- [ ] **Step 1: Write the failing tests** (`view.rs`, using the existing `snap`/`ctx` helpers; pane 1 is pwsh and pane 2 is cmd):
  - `panes_resize_with_their_own_cell`: `set_looks(window 8×16, {"cmd": cell 10×20, pad 4})`, then `on_snapshot` → pane 1's `Resize` has `cell_w_px == 8`, and pane 2's has 10/20, with cols/rows equal to `((w - 8) / 10, (h - 8) / 20)` of its rect.
  - `split_spawns_with_new_profiles_cell`: splitting into "cmd" gives `SpawnOpts.cell_w_px == 10`.
  - `zoom_changes_only_that_pane`: `set_zoom(2, 2.0, look 12×24)` → only pane 2 gets a new `Resize`, `zoom(2) == 2.0` and `zoom(1) == 0.0`. Then `set_zoom(2, 0.0, _)` restores the cmd look.
  - `zoom_past_min_size_keeps_at_least_2x1` (Review Focus 2): a look with cell 400×400 in a 1000×600 window → that pane's `Resize` has cols ≥ 2 and rows ≥ 1.
  - `tab_min_size_uses_largest_pane_look` (Review Focus 5): with looks of 8×16 and 20×40, a ratio dragged to 0.01 still leaves each pane at least `2*20+2*pad` wide.
  - `used_fonts_covers_profiles_and_zooms`: with looks using fonts 1 (cmd) and 2 (a zoom override on pane 1) → `{0, 1, 2}`. Clearing the zoom → `{0, 1}`.
  - `no_looks_matches_today`: all existing view tests pass unchanged. Run the whole file.
- [ ] **Step 2: Run** — `rtk proxy cargo test -p wraith-client --test view; echo EXIT=$?`. Expected: fails.
- [ ] **Step 3: Implement** as described in Interfaces.
- [ ] **Step 4: Run** — the same command. Expected EXIT=0.
- [ ] **Step 5: Commit** — `feat(client): per-pane look for layout, resize and spawn`

### Task 6: App wiring: build looks, draw per pane, zoom, reload

**Files:**
- Modify: `crates/client/src/profiles.rs` (pure `appearance`)
- Modify: `crates/client/src/app.rs` (`fonts` → `FontTable<FontStack>`, `rebuild_fonts`, `render` pane loop ~1451-1466, IME preedit ~1479, `mouse` ~723/901, `action` font-size arms 599-601, remove `font_size` and `set_font_size`, `geometry`)
- Modify: `crates/client/src/reload.rs:220-233` (`apply_config`)
- Modify: `crates/client/src/grid.rs` (`apply`/`set_overlay` keep taking `&FontStack`; the caller passes the pane's)
- Test: `crates/client/tests/profiles.rs`, `crates/client/tests/reload.rs`

**Interfaces:**
- Consumes: Tasks 3–5.
- Produces, in `wraith_client::profiles`:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance { pub families: Vec<String>, pub size_pt: f32, pub pad: (u16, u16), pub opacity: f32 }
/// `p`'s appearance over the global config, plus a zoom delta; size clamped 4–72. `installed` says
/// whether a family exists; when none of the profile's does, the window families are used.
pub fn appearance(p: Option<&Profile>, c: &Config, zoom: f32, installed: &dyn Fn(&str) -> bool) -> Appearance;
```

- Families: the profile's list followed by `c.font_family`, duplicates removed. If none of the profile's families is installed, just `c.font_family`, and log `tracing::warn!` once per family (keep a `HashSet` in `App`).
- `App` keeps the window `FontKey` at entry 0. For each profile and each zoomed pane, it builds a `Look` from `appearance()`: `font` = `get_or_load`, `cell` = that stack's `metrics()` w/h, `pad` = the pt padding scaled by `dpi/96` as `pad_px` does, `opacity`.
- `rebuild_fonts`:
  1. creates a new table and atlas (also when `exhausted()`);
  2. calls `view.set_looks`;
  3. re-applies every `zoomed_panes()` entry through `set_zoom` with a rebuilt look;
  4. calls `fonts.retain(&view.used_fonts())`. It also does this after every `set_zoom`, so old zoom steps are dropped.
  5. sends `Redraw` to the visible panes.
- `render`: each pane uses `fonts.get(look.font)` and its metrics, `origin = rect + look.pad`, and `DrawOpts { opacity: look.opacity, font: look.font, .. }`. The plain fill shown before a pane's first frame (`None => Painter.rect(rect, bg, ..)`) also uses `look.opacity`. `GridRenderer::apply`/`set_overlay` (in `on_pane_event` ~1235) gets the pane's stack. Chrome keeps `fonts.get(0)`.
- `mouse`: `pixel_to_cell` uses the hovered pane's metrics and pad.
- Zoom actions: `IncreaseFontSize(d)`/`DecreaseFontSize(d)` → `delta = view.zoom(focused) ± d`; `ResetFontSize` → `delta = 0`. Build the look and call `view.set_zoom`. No chrome change.
- `apply_config`:
  - `fonts_changed` also fires when any profile's `(font, font_size, padding)` differs between the old and new `spawn_ctx.profiles`.
  - When only an opacity differs, rebuild the looks (`view.set_looks`) without a new table.
  - Remove the `app.font_size` lines.

- [ ] **Step 1: Write the failing tests**
  - `profiles.rs` `appearance_defaults_to_global`: `appearance(None, &c, 0.0, &|_| true)` gives `c.font_family`, `c.font_size`, `(c.window_padding_x, c.window_padding_y)` and `c.background_opacity`.
  - `profiles.rs` `appearance_profile_wins_and_zoom_adds`: a profile with font `["A"]`, size 14, pad (8, 4) and opacity 0.8, at zoom 2 → families `["A", <global…>]`, size 16, pad (8, 4), opacity 0.8. At zoom 100 → size 72.
  - `profiles.rs` `uninstalled_family_falls_back_to_window_families` (Review Focus 3): `installed = |f| f != "Cascadia Mono"` with a profile font `["Cascadia Mono"]` at size 14 → families `== c.font_family`, size 14.
  - `reload.rs` `profile_font_change_rebuilds`: whatever helper the existing `reload.rs` tests use to compare configs gets a `profile-font-size` line change, and `fonts_changed` must be true. If `fonts_changed` is inline in `apply_config`, first extract it as `pub fn fonts_changed(old: &Config, new: &Config, old_p: &[Profile], new_p: &[Profile]) -> bool` in `reload.rs`.
- [ ] **Step 2: Run** — `rtk proxy cargo test -p wraith-client --test profiles --test reload; echo EXIT=$?`. Expected: fails.
- [ ] **Step 3: Implement** as described in Interfaces.
- [ ] **Step 4: Run** — `rtk proxy cargo test -p wraith-client; echo EXIT=$?` and `rtk proxy cargo clippy -p wraith-client -- -D warnings; echo EXIT=$?`. Expected EXIT=0 for both.
- [ ] **Step 5: Smoke run.** Run the debug build per Global Constraints with `profile-font-size = <default profile>:20`. A `PrintWindow` capture shows the pane text larger than the tab titles.
- [ ] **Step 6: Commit** — `feat(client): per-pane fonts, padding, opacity and zoom`

### Task 7: Composition swap chain when any opacity is below 1

**Files:**
- Modify: `crates/client/src/gpu.rs:78-105` (`create_swapchain`), `SwapChain`
- Modify: `crates/client/src/window.rs:80-83` (remove the Mica call and the `mica` parameter)
- Modify: `crates/client/src/app.rs:2230` (`Window::create` call) and `apply_config`/`rebuild_spawn_ctx` (recreate on crossing)
- Test: `crates/client/tests/app.rs` (pure choice), GPU test in `crates/client/tests/font.rs` or a new `tests/transparency.rs`

**Interfaces:**
- Consumes: `Profile.opacity` (Task 3).
- Produces:
  - `pub fn needs_transparency(c: &Config, profiles: &[Profile]) -> bool` in `app.rs`: true when `c.background_opacity < 1.0` or any `p.opacity` is `< 1.0`.
  - `Gpu::create_swapchain(&self, hwnd: HWND, w: u32, h: u32, composition: bool) -> Result<SwapChain>`. The `SwapChain` keeps the `IDCompositionDevice`, target and visual alive when `composition` is true. `SwapChain::is_composition(&self) -> bool`.

- [ ] **Step 1: Write the failing tests**
  - `app.rs` `transparency_choice`: default config and opaque profiles → false. `background_opacity = 0.9` → true. One profile with `opacity: Some(0.5)` → true.
  - GPU `pane_opacity_is_premultiplied`: draw a `GridRenderer` pane with `DrawOpts.opacity = 0.5` and a white background into `gpu.offscreen`, cleared to `[0.0; 4]`. Then `read_pixel` in the padding area `== [128, 128, 128, 128]` (±1).
- [ ] **Step 2: Run** — `rtk proxy cargo test -p wraith-client --test app --test transparency; echo EXIT=$?`. Expected: fails.
- [ ] **Step 3: Implement.**
  - The composition path is Task 1's call sequence. `Commit()` happens once after `SetContent`, and again after each `ResizeBuffers` only if the visual changes.
  - `App` creates the swap chain with `needs_transparency(&config, &spawn_ctx.profiles)`.
  - After `rebuild_spawn_ctx`, if the answer differs from `sc.is_composition()`, drop the old swap chain (RTV first) and create the new one at `size_px` between frames: from `apply_config`, never inside `render`. Then set `dirty = true`.
- [ ] **Step 4: Run** — the same command, plus `rtk proxy cargo test --workspace; echo EXIT=$?`. Expected EXIT=0.
- [ ] **Step 5: Commit** — `feat(client): composition swap chain for real transparency; drop Mica`

### Task 8: Live acceptance

**Files:**
- Modify: `docs/per-profile-appearance-acceptance.md`
- Modify: `docs/superpowers/specs/2026-11-03-per-profile-appearance-design.md`: Status line → "implemented (date); see `docs/per-profile-appearance-acceptance.md`"

**Interfaces:** Consumes everything above.

- [ ] **Step 1:** Set up per Global Constraints and spec §5 (Live check): a copy of the real WT `settings.json` and fragments under the fake `LOCALAPPDATA`; the originals are only read. In the fake WT copy, give one profile `"font": {"size": 16}`, `"padding": "10"` and `"opacity": 85`.
- [ ] **Step 2:** Run each check in spec §5 (Live check) and fill in a table in the acceptance doc's format (Check | Result | Evidence):
  - a split shows two cell sizes, and each shell reports its own size;
  - zoom changes one pane, and `Ctrl+0` resets it. Posted messages carry no Ctrl state, so invoke the action through the palette and say so in the doc;
  - `wr import-wt` into the fake config writes the four keys, and with `windows-terminal-profiles = false` the look is the same;
  - opacity 0.85 blends the helper window's colour in the pane, but not in the tab bar or the text (captured with `CopyFromScreen`, no activation);
  - input-to-present time from the frame overlay, HWND against composition.
- [ ] **Step 3:** List any fixes found live under "Found and fixed", each with the test that now pins it, as spec 1's acceptance doc does.
- [ ] **Step 4: Commit** — `docs: per-profile appearance acceptance`
