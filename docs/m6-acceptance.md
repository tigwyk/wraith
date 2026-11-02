# M6 acceptance — hover feedback and polish

Date: 2026-11-02. Branch `m6`.

**Setup:**
- Debug build, plus a release build in a separate target directory (your running Wraith holds `target\release`).
- A private pipe, layout file and `APPDATA`, with this test config: `f7=new_tab`, `f6=toggle_sidebar`, `f9=command_palette`, `f10=toggle_frame_overlay`, `cursor-style-blink = false`.
- The test window was shown without activation. Keys and mouse messages were posted to its queue only, and it was captured with `PrintWindow`.
- There was no focus stealing and no global input. Because of that, the real pointer never sat over the window: hover clears as soon as Windows sends `WM_MOUSELEAVE`, and hover-only visuals could not be captured.

| Check | Result | Evidence / notes |
|---|---|---|
| Hover logic: target changes only, drops vanished targets, clears on leave, press clears on capture loss | Pass | `hover_reports_only_target_changes`, `hover_drops_target_that_disappears`, `pointer_left_clears_hover`, `capture_lost_clears_press` |
| Pointer shape (resize arrows on dividers, I-beam on panes, arrow when the program owns the mouse) | Pass (logic) | `cursor_for_divider_is_resize`, `cursor_over_tracking_pane_is_arrow`. Live: needs the real pointer. |
| Hover washes on tabs, `×`, `+`, sidebar rows, agent rows; divider highlight | Not seen live | Needs the real pointer over the window (see setup) |
| Palette: hovering a row selects it | Pass | Posted move over row 4 → "Previous workspace" selected |
| Palette: a click runs the row | Pass | Click on "Tabs › New tab" opened a second tab |
| Palette: a click outside closes it | Pass | Click in the pane closed it; the click did not reach the pane |
| Palette: the wheel scrolls; hovering never scrolls the list | Pass (logic) | `scroll_moves_selection_by_rows_and_clamps` |
| Tab underline slides to the new tab | Pass | On target 300 ms after a new tab |
| Floating sidebar slides in; palette and find bar fade in from 40%; new attention dots flare once | Pass (logic) | `motion` tests; palette settled fully drawn |
| Overlays show on the key press's first frame | Pass | `overlay_shows_on_its_first_frame`. Found live: opening at 0% drew nothing on the first frame. |
| Slides jump on resize, DPI, font or dock change | Pass | `snap_jumps_without_motion`, `motion_snaps_on_layout_change_and_pulses_new_dots_once` |
| Reduced motion ("Show animations" off) | Pass (logic) | `reduced_motion_completes_at_once`, and the `Motion::new(false)` case. Live: needs you to flip the Windows setting. |
| Accents dim in an inactive window | Pass | The unactivated test window draws its underline in the dim text color |
| Tooltips: 500 ms rest, name and key, follow a rebind | Pass (logic) | `tooltip_names_action_and_binding`, `tooltip_follows_rebinding`, `tooltip_due_after_rest_and_resets_on_move`. Live: needs the real pointer. |
| Idle | Pass | 0 ms client CPU over 3 s once motion ended. Measured twice, before and after the pacing fix. |
| Frame cost with motion (release) | Pass | 0.1–0.2 ms at rest, 0.2–0.3 ms during the palette fade (frame-time overlay) |
| Input-to-present (release) | Partly measured | 0.1, 0.5 and 0.3 ms at rest. During a fade the overlay rewrites itself every frame, which drops the input figure, so it could not be read there. A key during motion waits for at most one frame of about 0.3 ms. |

Bugs found during acceptance:
1. **First frame of an overlay was empty.** The palette opened at 0% opacity, so the key press showed nothing for a frame. Overlays now start at 40%.
2. **Animations stalled.** After each frame, the loop slept the full idle timeout (1 s with blink off) before drawing the next motion frame, so a 120 ms slide took seconds. A running motion now asks for the next vsync frame.

Not run: hover washes, the pointer shape, tooltips and reduced motion, live. These need the real pointer or a Windows setting changed, so they are left for you.
