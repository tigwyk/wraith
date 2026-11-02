# Settings GUI acceptance

Date: 2026-11-02. Branch `feat/settings-gui`.

**Setup:**
- Debug build.
- A private pipe, layout file, `APPDATA` and `LOCALAPPDATA`.
- Test config, with CRLF line endings: `# mine`, `font-size = 12`, `copy-on-select = false`.
- The window was shown without activation. Clicks and keys were posted to its queue only, and it was captured with `PrintWindow`. There was no focus stealing and no global input.

| Check | Result | Evidence / notes |
|---|---|---|
| The gear opens the panel | Pass | A posted click on the gear opened it. The tooltip reads `Settings  Ctrl+,` |
| Three changes rewrite only their lines | Pass | Theme (list, ↓↓, Enter), Font size (→→) and Copy on select (Tab, Space). The file became `# mine`, `font-size = 14`, `copy-on-select = true`, `theme = 13th Floor`. CRLF and the comment were kept, and there was no other change |
| Changes show at once | Pass | The capture showed the 13th Floor colours and 14 pt text right after the keys |
| Moving through the font list previews; Esc reverts | Pass | The font list opened on the current font, and ↓ previewed the next one. After Esc, the file was byte-for-byte unchanged and the font was back |
| A read-only file | Pass | → on Font size showed the toast "Couldn't save settings: the config file is read-only". The file was unchanged |
| Esc closes; a closed panel draws nothing | Pass | Capture after Esc: the terminal only |
| Reset dots | Pass | Shown on Theme, Font size and Copy on select (changed); hidden on rows at their default |
| Writes keep the encoding: BOM, CRLF, UTF-16; refuse invalid UTF-8 | Pass (logic) | `crates/config/tests/edit.rs`: 13 tests |
| Every control's value round-trips through the parser | Pass (logic) | `crates/config/tests/settings.rs` |
| Keys, lists, quiet hours, read-only rows, layout | Pass (logic) | `crates/client/tests/settings.rs`: 21 tests |
| `agent-notify = none` | Pass (logic) | `agent_notify_none_turns_all_off` |

**Found and fixed during the check:** long labels ran into their controls at 14 pt ("Padding, horizontal‹ 2 ›"). The panel is now 66 cells wide, capped by the window, and labels truncate before the control.

**Not run live:**
- **Ctrl+,:** posted messages carry no Ctrl state. The binding is covered by `ctrl_comma_opens_settings`.
- **IME text in a list query:** needs an IME.
- **"Open config file":** it launches an editor outside the test.
- **A key set in an included file:** covered by `read_only_row_gives_no_edit` and `origins_follow_includes`.
- **Reduced motion:** needs the Windows setting changed. The panel uses the same `Motion::overlay` as the palette.
