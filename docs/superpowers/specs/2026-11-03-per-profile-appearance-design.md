# Wraith per-profile appearance (font, size, padding, opacity) — design

Status: approved design (2026-11-03), not implemented
Parent specs: `docs/superpowers/specs/2026-11-01-wraith-terminal-design.md` (§6.3 latency), `docs/superpowers/specs/2026-11-02-wt-profiles-design.md` (spec 1, on branch `feat/wt-profiles`; this is its "spec 2")

## 1. Intent

**Goal.** A Windows Terminal (WT) user's profiles look in Wraith as they do in WT: each profile's font face, font size, padding and background opacity, per pane.

**Decided with the user**
- **Per pane.** In one tab, split panes of different profiles each draw in their own profile's font, size and padding, as in WT. The chrome (tab bar, sidebar, palette, menus, settings panel, toasts) keeps the window font.
- **Zoom is per pane.** `Ctrl+=`, `Ctrl+-` and `Ctrl+0` change only the focused pane.
- **Transparency must be made to work.** `background-opacity < 1` has never made Wraith see-through, so this spec fixes window transparency as well as making opacity per profile.
- **What shows through is the windows behind**, as with WT's `opacity` without acrylic. No Mica, no acrylic.
- **One shared glyph atlas** (approach A below), not one atlas per font.

**Success criteria**
- A split of two profiles with different fonts and sizes shows two cell sizes. Each shell gets the right column and row count, and the right cell size in pixels (so Kitty images are the right size in each pane).
- Zoom in one pane leaves the other panes and the chrome unchanged. `Ctrl+0` returns that pane to its profile's size.
- A pane at opacity 0.85 shows the windows behind it, while its text and the tab bar stay opaque.
- With every opacity at 1, rendering and presentation work exactly as today, with no new latency (parent spec §6.3).
- A profile that sets none of the four values gives a pane identical to today's.
- `wr import-wt` writes the four values, and running it twice changes the config once.

**Non-goals**
- WT's `font.weight`, `font.features`/`axes`, `cellHeight`/`cellWidth`, `useAcrylic`, `acrylicOpacity`, background images, cursor shape, `antialiasingMode`, `useMica`.
- Per-profile `font-feature`, which stays global.
- The four values in the Settings panel. They are config lines only.
- Keeping a zoom across window close or reattach.
- A switch to make the composition swap chain the only path (§4.4).

**Approaches considered**
- **A. Font table + one shared atlas (chosen).** `GlyphKey` gains a font index, and all fonts share the 2048² atlas. The renderer change is the smallest: `GlyphAtlas::get` already takes the `FontStack` to rasterize with, and `GridRenderer` already gets fonts per call. Cost: several fonts fill the atlas sooner, and when it fills, the existing clear-and-redraw covers every font.
- **B. One atlas per font.** Each font's cache is isolated, but this costs 16 MB of VRAM per font and changes `Batch`, `Pipeline::submit` and the shader binding. Rejected because A costs less.
- **C. The window font follows the focused pane or tab.** Rejected because it breaks "per pane".

## 2. Configuration, profiles and import

### 2.1 Reading WT

`WtProfile` (`crates/config/src/wt.rs`) gains four fields. Each is merged from `profiles.defaults` and fragments like the existing keys (spec 1 §2.2):

| WT key | Wraith field | Rule |
|---|---|---|
| `font.face`, else `fontFace` | `font: Option<Vec<String>>` | Split on `,`, trimmed, empty names dropped |
| `font.size`, else `fontSize` | `font_size: Option<f32>` | A number, clamped to 4–72 (`clamp_font_size`) |
| `padding` | `padding: Option<(u16, u16)>` | A string or a number. `"n"` → (n, n); `"x, y"` → (x, y); `"l, t, r, b"` → (l, t). Wraith padding is symmetric, so right and bottom are dropped. Values above 32 are clamped to 32, the settings panel's limit. |
| `opacity` | `opacity: Option<f32>` | An integer from 0 to 100, stored as 0.0–1.0 |

A value of the wrong type, or an opacity outside 0–100, is ignored with a warning, and the rest of the profile is kept.

### 2.2 New config keys

These keys take `name:value`, like `profile-theme`. They extend `ProfileExtra` in `crates/config/src/keys.rs`, and setting one again replaces the earlier line, as the existing `profile-*` keys do.

```
profile-font = Ubuntu 22.04:JetBrainsMono NF, Cascadia Mono
profile-font-size = Ubuntu 22.04:14
profile-padding = Ubuntu 22.04:8        # or 8,4 (x,y)
profile-opacity = Ubuntu 22.04:0.85     # 0.0–1.0, like background-opacity
```

Values are checked when parsed: a size from 4 to 72, padding from 0 to 32 per axis, and opacity from 0.0 to 1.0, as `background-opacity` checks it. A bad value is a line diagnostic, as for other bad lines. A name that is not a profile gives the existing merge warning (`profiles.rs`).

### 2.3 The profile

`Profile` gains `font`, `font_size`, `padding` and `opacity`, with the types of §2.1. In `profiles::merge`, WT values fill them, and `profile-*` lines win (spec 1 §3).

**Font resolution.** A pane's family list is the profile's families followed by the global `font-family` list, which act as fallback. `font-feature` is global. When none of the profile's families is installed (`FontStack::has_family`), the pane uses the window's families at the profile's size, and Wraith logs a warning once per family. This case is common: WT's default "Cascadia Mono" ships inside WT's package and is often not installed system-wide.

### 2.4 Import

`wr import-wt` (and the palette command) also writes `profile-font`, `profile-font-size`, `profile-padding` and `profile-opacity` for a profile when it sets them, directly or through WT's `defaults`. A font list is written comma-separated. Padding is written as `n` when x equals y, else `x,y`. Opacity is written with two decimals. The rest of spec 1 §4 is unchanged: a profile that already has a `profile` line is skipped with all its lines, so a second run writes nothing.

## 3. Rendering, layout and zoom

### 3.1 Font table

`App.fonts` becomes a table of `FontStack`s, indexed by a `FontId` (`u16`), keyed by (family list, size in points, features), all at the window's DPI.
- **Entry 0** is the window font (`font-family`, `font-size`). All chrome draws with entry 0, and only entry 0.
- **Loading.** An entry loads when a pane that needs it first gets a look (§3.3), when the pane appears or is spawned. Nothing loads on a keystroke or a frame. The same key always gives the same id.
- **Pruning.** At relayout, entries that no visible pane uses are dropped, except entry 0. Their glyphs stay in the atlas until its next clear.
- **Rebuild.** A DPI change, or a reload that changes `font-family`, `font-feature`, `font-size` or any `profile-font`/`profile-font-size`, rebuilds the table and the atlas (as `rebuild_fonts` does today), then re-sends every visible pane (`PaneCmd::Redraw`).

### 3.2 Atlas

`GlyphKey` gains `font: FontId`. `GlyphAtlas::get` rasterizes with the table entry for `key.font`. There is still one 2048² atlas, and `Batch` and `Pipeline::submit` are unchanged. When the atlas fills, the existing clear (`generation` bump, rows re-sent) covers all fonts.

### 3.3 Pane look

`WindowView` keeps one `Look` per pane:

```rust
pub struct Look { pub font: FontId, pub cell: (u32, u32), pub pad: (i32, i32), pub opacity: f32 }
```

- **Source.** The look comes from the pane's profile (`PaneMeta.profile`) plus the pane's zoom delta (§3.4). Font and size come from §2.3. Padding is the profile's `padding`, else `window-padding-x`/`-y`. Opacity is the profile's `opacity`, else `background-opacity`. Padding is scaled for DPI as `pad_px` does today.
- **Who builds it.** `App` builds looks, because it owns the font table. It gives `WindowView` a look per profile name (zoom 0), and `WindowView` adds each pane's zoom on top. A pane whose profile is unknown, or sets nothing, gets the window look, which is exactly today's `Geometry.cell`, `Geometry.pad` and `background-opacity`.
- **What reads it.** Everything that read `Geometry.cell` or `Geometry.pad` for a pane:
  - `cells_for`, the minimum pane size (2×1 cells plus padding) and `relayout`. `PaneCmd::Resize` already carries `cell_w_px`/`cell_h_px` per pane, and the server and Kitty image sizes already use them, so the server needs no change. `sent_size` already includes the cell size, so a look change re-sends `Resize` only to the panes it affects.
  - Spawn: `new_tab`, splits and the restart path ask for the cell size and estimated cells of the profile they start, instead of `self.cell_px()`/`est_cells()`.
  - In `app.rs`: `pixel_to_cell` and the mouse path, selection, the cursor, IME preedit, find highlights, the exit overlay, and `push_layers` (origin = rect + the pane's pad, metrics = the pane's font, `DrawOpts.opacity` = the pane's opacity).
- **What does not.** Dividers and split ratios stay in pixels (`PaneNode::Split.ratio`), as now. Chrome geometry (`bar_h`, tab widths, sidebar, overlays) keeps using `Geometry.cell`, which stays the window font's cell.

### 3.4 Zoom

- `Action::IncreaseFontSize(d)` and `DecreaseFontSize(d)` add or subtract `d` points on the focused pane's zoom delta. `ResetFontSize` sets that pane's delta to 0. The resulting size is clamped with `clamp_font_size`.
- Deltas live in `WindowView`, per pane, on the client. They are lost when the window closes or reattaches, and kept across config reloads. A pane that closes takes its delta with it.
- **Visible change from today:** zoom no longer resizes the chrome. Chrome size follows `font-size` only. `App.font_size` and `App::set_font_size` go away. `rebuild_fonts` and `reload::apply_config` read `config.font_size` instead.

### 3.5 Reload

`reload::apply_config` treats a change to any `profile-font`, `profile-font-size`, `profile-padding` or `profile-opacity` line, or to a WT value behind them, the way it treats `font-*` and `window-padding-*` today. It rebuilds what §3.1 says, recomputes every look, runs relayout and redraws. An opacity-only change recomputes looks and redraws, and may switch the swap chain (§4.2).

## 4. Transparency

### 4.1 Why it does not work today

The swap chain is a flip-model swap chain on the HWND (`CreateSwapChainForHwnd`) with `DXGI_ALPHA_MODE_IGNORE`. DWM shows it opaque whatever alpha is drawn, and the Mica backdrop set in `Window::create` never shows through it.

### 4.2 Composition swap chain

- **When.** The window needs transparency when `background-opacity`, or the opacity of any profile in the list, is below 1. It is a pure function of the config and the profile list, unit tested.
- **With transparency:** `CreateSwapChainForComposition` with `DXGI_ALPHA_MODE_PREMULTIPLIED`, the same format, flip-discard and the frame-latency waitable flag, with `SetMaximumFrameLatency(1)`. It is shown through a DirectComposition device, a target for the HWND, and one visual whose content is the swap chain. The window keeps `WS_EX_NOREDIRECTIONBITMAP`.
- **Without:** the HWND swap chain stays exactly as today, so the common case takes no latency risk (parent §6.3).
- **Switching.** A reload or WT change that crosses the line in either direction recreates the swap chain at the current size. The `DWMSBT_MAINWINDOW` (Mica) call is removed in both cases.

### 4.3 What is see-through

- Drawn at the pane's opacity: the pane's default background (the grid clear), its padding, and the plain fill shown before its first frame.
- Opaque: text, cell backgrounds with explicit SGR colours, the cursor, selection, images, dividers, the unfocused-pane dim, and all chrome (tab bar, sidebar, palette, menus, settings, toasts, tooltips).
- All output is premultiplied. The grid clear already multiplies by opacity. The pipeline's blend state and every other quad must agree, and the probe in §4.4 checks that.

### 4.4 Probe first

The first implementation task is a throwaway probe. It draws a composition swap chain with a half-transparent rectangle over a solid-colour helper window and captures the screen region to read the blended pixel. It also compares input-to-present time from the frame overlay with the HWND path. The result is recorded in the acceptance doc. If composition costs latency, the opt-in rule of §4.2 stands, as designed. If it costs nothing, using it always is a separate follow-up, not part of this spec.

## 5. Testing

- **Config** (`config/tests/parse.rs`): the four keys round-trip. An out-of-range size, opacity or padding is a line diagnostic. Setting a key twice for one profile keeps the last.
- **WT parser** (`config/tests/wt.rs`, fixture extended): `font.face`/`font.size`; the old `fontFace`/`fontSize`; a font inherited from `defaults`; a comma face list; all three padding forms, as string and number; opacity 0–100 → 0–1, and 150 ignored; a font size of 200 clamped to 72; a wrong-typed value ignored with a warning and the profile kept.
- **Merge and import** (`client/tests/profiles.rs`, `config/tests/import.rs`, CLI test): WT values reach `Profile`, and `profile-*` lines win. The import writes the four keys when set and nothing when unset, and a second run writes nothing.
- **View** (`client/tests/view.rs`, pure, looks injected):
  - two panes with different cells each get `Resize` with their own cell counts and `cell_w_px`/`cell_h_px`;
  - a split spawns with the new profile's cell size and estimated cells;
  - the minimum pane size uses the pane's cell and padding;
  - `pixel_to_cell` uses the pane's look;
  - zoom changes only the focused pane, `Ctrl+0` resets it, and chrome geometry is unchanged;
  - a pane without profile appearance gets exactly today's numbers.
- **Font table:** the same key gives the same id; unused entries are dropped at relayout, but entry 0 never is; an uninstalled family falls back to the window families at the profile's size. Two fonts' `GlyphKey`s with the same face index and glyph do not collide.
- **Swap chain choice:** all opacities at 1 → HWND; any below 1 → composition; crossing on reload → recreate.
- **Live check** (`docs/per-profile-appearance-acceptance.md`). The setup is spec 1's: a debug build, a private `WRAITH_PIPE_NAME` and `WRAITH_LAYOUT_PATH`, a fake `APPDATA` and `LOCALAPPDATA` holding a copy of the WT settings (the originals are only read), the window shown without activation, and only posted input. `PrintWindow` captures fonts and padding. For transparency, a solid-colour helper window is placed behind Wraith without activation, and the screen region is captured. `scripts/keys.ps1` and `scripts/shot.ps1` are not used. The real `%APPDATA%\wraith\config` and `target\release` are not touched. Checks:
  - a split of two profiles shows two cell sizes, and each shell reports its own size (`tput cols`/`lines` or `$Host.UI.RawUI.WindowSize`);
  - zoom changes one pane, and `Ctrl+0` resets it;
  - `wr import-wt` into the fake config writes the four keys and gives the same look with `windows-terminal-profiles = false`;
  - opacity 0.85 shows the helper's colour, blended, through the pane but not through the tab bar or text;
  - input-to-present time, HWND against composition (§4.4).

## 6. Review focus

The inputs the tests above are least likely to catch:
1. **Atlas churn.** Many fonts and zoom steps at once refill and clear the 2048² atlas more often. Watch for a clear every frame: the existing shortcut comment in `atlas.rs` (clear instead of grow) is the place to upgrade if it happens.
2. **Zoom that crosses the minimum pane size.** A pane zoomed until 2×1 cells no longer fit must clamp, as a window resize does, not panic or send 0 rows.
3. **DPI change with zoomed panes.** The rebuild must keep each pane's delta and re-send `Resize` with the new cell pixels.
4. **A profile removed from WT while its panes are open.** Those panes fall back to the window look on the next reload, as their colours do (spec 1 §7.5).
5. **Premultiplied output.** Any quad drawn with straight alpha over a transparent clear shows as a bright fringe. This is checked live, not in unit tests.
6. **Swap chain recreation during a resize or present.** It must happen between frames, with the frame-latency waitable object replaced.
