# Per-profile appearance: acceptance

## Probe

Throwaway example (deleted): a 400x300 popup shown with `SW_SHOWNOACTIVATE` over a solid #FF0000 helper window, cleared each frame to premultiplied `[0,0,0,0.5]`. Pixel read from the screen centre with `Graphics.CopyFromScreen`, no window activated.

| Mode | Swap chain | Centre pixel | Median | p99 |
| --- | --- | --- | --- | --- |
| Composition | `CreateSwapChainForComposition`, `ALPHA_MODE_PREMULTIPLIED`, `WS_EX_NOREDIRECTIONBITMAP`, DirectComposition target/visual | #800000 | 0.043 ms | 0.309 ms |
| HWND (today) | `CreateSwapChainForHwnd`, `ALPHA_MODE_IGNORE` | #000000 | 0.081 ms | 0.293 ms |

Latency is `WaitForSingleObjectEx` on the frame-latency waitable returning to `Present(1)` returning, over 600 frames per mode, unoptimised (dev profile) build. Both modes: B8G8R8A8, flip-discard, `SetMaximumFrameLatency(1)`.

Machine: Windows 11 Enterprise 10.0.26300, NVIDIA GeForce RTX 5070 Ti.

Verdict: a composition swap chain with premultiplied alpha blends with what is behind the window (red at 50% black gives #800000), so spec section 4 holds. Composition p99 is 0.016 ms above HWND p99, well under the 1 ms threshold. The opt-in rule (Task 7) stands.

## Live check

Date: 2026-11-03. Branch `feat/per-profile-appearance`.

**Setup:**
- Debug build. A private pipe, `WRAITH_LAYOUT_PATH`, and a fake `APPDATA` and `LOCALAPPDATA` under `%TEMP%`, holding a copy of the real Windows Terminal `settings.json` and fragments. The originals were only read; the real `%APPDATA%\wraith\config` and `target\release` were not touched.
- In the WT copy, `cmd` got `"font": {"size": 16}`, `"padding": "10"` and, for the transparency runs, `"opacity": 85`. Windows PowerShell kept its `CaskaydiaCove Nerd Font` face at the global 12 pt.
- Test config: `sidebar-visible = false`, `font-family = CaskaydiaCove NFM`, `font-size = 12`, `background = 000000`, and the bare-key bindings `f1=command_palette`, `f6=increase_font_size:2`, `f7=reset_font_size`, `f8=toggle_frame_overlay`, `f9=new_split:right`.
- The window was shown with `SW_SHOWNOACTIVATE`. Keys and clicks were posted to its queue only (`WM_KEYDOWN`/`WM_KEYUP` with scan codes, `WM_LBUTTON*`). Posted messages carry no Ctrl or Alt state, so zoom was invoked through the bare-key bindings above and through the command palette (`F1`, typed filter, Enter), not `Ctrl+=` or `Ctrl+0`.
- Fonts and padding were captured with `PrintWindow` (`PW_RENDERFULLCONTENT`) on the HWND swap chain (no opacity set). Transparency was captured with `CopyFromScreen`, with a solid #FF0000 helper window (also shown without activation) placed behind Wraith. `PrintWindow` returns black for a composition window, which has no redirection bitmap.
- A split of two profiles: Alt+click and Alt+Enter on the menu cannot be posted, so the split was made with `new_split:right`, and between two server runs the second pane's `profile` and `cmdline` were edited to `cmd` in the private `layout.json`.
- Captures and the scripts that took them were session-local and were not kept. The pixel values and sizes below are the readings taken from them.

| Check | Result | Evidence / notes |
|---|---|---|
| A split of two profiles shows two cell sizes, and each shell reports its own size | Pass | The PowerShell pane (12 pt) reports `mode con` Lines 31, Columns 48; the `cmd` pane (16 pt, padding 10) reports Lines 23, Columns 36 |
| Padding | Pass | Same `PrintWindow` capture: the first text pixel is 2 px in from the PowerShell pane's edge (global padding 2) and 11 px in from the `cmd` pane's edge (padding 10 plus 1 px of glyph bearing) |
| Zoom changes one pane | Pass | `F6` with the `cmd` pane focused: the pane reports 20 x 31 (18 pt). The PowerShell pane stayed 31 x 48. The overlay's atlas stayed at 1-2 % with both fonts and the zoom |
| Reset | Pass | Palette (`F1`, "reset font", Enter) back to 23 x 36. `F7` (`reset_font_size`) did the same. `Ctrl+0` itself was not run (see below) |
| `wr import-wt` into the fake config | Pass | Wrote `profile-font = Windows PowerShell:CaskaydiaCove Nerd Font`, `profile-font = Ubuntu 22.04:CaskaydiaCove NF`, `profile-font-size = cmd:16`, `profile-padding = cmd:10`, `profile-opacity = cmd:0.85`, and `windows-terminal-profiles = false`. A second run printed `exists` for each profile and left the file's hash unchanged |
| The same look with `windows-terminal-profiles = false` | Pass | After the import, the fake WT settings and fragments were deleted. 31 x 48 and 23 x 36 as before, `cmd` pane #48252C over the red helper, PowerShell pane #000000, tab bar #000000 |
| Opacity 0.85 blends the helper's colour in the pane | Pass | `cmd` pane background (900,500) #48252C and its padding strip (505,300) #48252C. One Half Dark's #282C34 at 85 % over #FF0000 is #48252C. With opacity 1 the same pane was #282C34 |
| ...but not in the tab bar or the text | Pass | Tab bar (600,14) and (700,6) both #000000, no red. The brightest text pixel in the `cmd` pane is #DCDFE4 at (591,67), exactly One Half Dark's foreground, with no red tint. The PowerShell pane (250,500) stays #000000. No near-pure-red pixel inside the window except the outer frame strip, which is red with the HWND swap chain too (#D00000 at x=3 in both modes) |
| Transparency at reload (spec 4.2) | Pass | Started opaque, then `profile-opacity = cmd:0.85` was appended to the config: the pane stayed #282C34 (before, 3 s after and 7 s after), and the toast "Transparency applies to new windows" showed at the bottom right. Started with the line, then removed it: the pane went from #48252C to #282C34, so the swap chain switched live. That live switch was later removed: a composition window now stays one at opacity 1 (spec 4.2, final review) |
| Input-to-present, HWND against composition | Pass | Frame overlay (`F8`) readings after a posted key, debug build. HWND: 0.7 ms, 0.6 ms and 0.5 ms; the very first key of one run read 10.3 ms (cold start). Composition: 0.6, 0.7 and 0.6 ms and 0.5 ms. Frame time 0.2-0.4 ms in both. The difference is within the 1 ms threshold of spec 4.4 |

**Before the first frame (composition path):** with the red helper behind, the centre of the `cmd` pane read #DC0708 at 35 ms after the window was shown (the helper's red), #661F25 at 90 ms and #48252C from 153 ms. The tab bar read #FF0000, #BA2525, then #000000 in the same samples. So the window shows what is behind it until its first frame, DWM's show animation blends the middle sample, and it settles within about 150 ms. It did not persist beyond the first frame. This is an observation, not a fix.

Follow-up (2026-11-03): a see-through window now draws and presents its first frame before it is shown, so it is never visible with nothing drawn. A live re-run confirmed the window still starts and draws. The finer before/after timing could not be re-measured: screen reads under DWM take about 250 ms each, and a helper window from another process did not stay below Wraith reliably. What remains is likely Windows' own show animation, which every window gets.

**Found and fixed during the check:**
- **Palette rows for zoom, copy, paste, scroll, clear and reload did nothing.** The palette passed every action to `WindowView::action`, which ignores the ones only the app can run, so "Reset font size" left a pane zoomed (after a palette reset the pane still read 18 x 29 at 20 pt). The bug dates from the palette's first commit; zoom made it visible. The palette now hands every action back in `Out::app`, and `App::run_out` runs it through `App::action`, which passes what it does not handle to the view (`palette_hands_every_action_to_the_app`).

**Final review follow-up (zoom with fallback glyphs):** same setup, one PowerShell pane, CaskaydiaCove NFM. A posted command printed U+1F600 and U+263A three times (the emoji comes from the Segoe UI Emoji fallback face). With the fix: `F6` twice, `F7`, the palette's "increase font" and "reset font", a reload adding `font-size = 14` (a font rebuild), then `F6` again; the client stayed up after every step and the emoji redrew at the zoomed size. With both parts of the fix reverted in the working tree (renderer drop and the atlas guard), the client exited on the first `F6`. Captures were session-local and not kept.

**Not run live:**
- **`Ctrl+=`, `Ctrl+-` and `Ctrl+0`:** posted messages carry no Ctrl state. The same actions ran through bare-key bindings and the palette. The default bindings are in the keybind defaults.
- **Alt+click and Alt+Enter (split with a chosen profile):** no Alt state. The two-profile split came from editing the private layout (see Setup). Covered by `split_profile_uses_chosen_profile_along_longer_side`.
- **The toast on a second crossing:** only the first crossing was observed.
- **DPI change with zoomed panes:** not exercised.
