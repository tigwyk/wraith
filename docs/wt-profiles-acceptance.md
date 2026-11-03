# Windows Terminal profiles acceptance

Date: 2026-11-02. Branch `feat/wt-profiles`.

**Setup:**
- Debug build.
- A private pipe and layout file. A fake `APPDATA` and `LOCALAPPDATA`, holding a copy of the real Windows Terminal `settings.json` and fragments (the originals were only read).
- Test config: `sidebar-visible = false` and `background = 000000`, so per-profile colours stand out.
- The window was shown without activation. Clicks and keys were posted to its queue only, and it was captured with `PrintWindow`. There was no focus stealing and no global input.

| Check | Result | Evidence / notes |
|---|---|---|
| The first tab uses WT's default profile | Pass | WT's `defaultProfile` is "Windows PowerShell", whose command line is PowerShell 7; the tab showed `PowerShell 7.6.9` |
| The `⌄` menu lists WT's visible, launchable profiles in WT's order | Pass | Windows PowerShell, cmd, Ubuntu 22.04, Developer PowerShell for VS 2022, then Settings and Command palette. Azure Cloud Shell (WT-only) and the hidden profiles were left out |
| Bindings on the rows | Pass | `Ctrl+Shift+1…4`, `Ctrl+,`, `Ctrl+Shift+P` |
| The selection follows the pointer | Pass | Hovering cmd moved the selection there |
| A profile's colour scheme | Pass | cmd (One Half Dark) opened with background #282c34, while the PowerShell tab kept the black global background |
| Keys | Pass | End then Enter opened the command palette; Esc closed it |
| `wr import-wt` | Pass | "4 profiles imported"; Azure listed as skipped with its reason. The config gained the `profile` lines, `profile-theme = cmd:One Half Dark`, `default-profile`, `profile-detect = false` and `windows-terminal-profiles = false` |
| After the import, the same menu | Pass | Same four profiles in the same order, read from the config alone |
| Image icons from WT's package | Pass | Windows PowerShell, cmd and Developer PowerShell for VS 2022 showed WT's own icons (`ProfileIcons\…scale-100.png`, found through the package family). Ubuntu's icon is an `.exe` in WT, so it shows its letter tile |

**Found and fixed during the check:**
- **Unquoted program paths with spaces.** WT accepts `%PROGRAMFILES%/PowerShell/7/pwsh.exe`. Expanded, Wraith read the program as `C:\Program`, so the profile was not recognised as pwsh and got no shell integration. Such paths are now quoted (`unquoted_exe_path_with_spaces_is_quoted`).
- **Detected shells came back after the import.** With `windows-terminal-profiles = false`, the detected shells were listed again above the imported ones. The import now also writes `profile-detect = false` (`profile_detect_off_skips_detection`). An empty list still falls back to detection (`empty_list_falls_back_to_detected`).
- **Hover did not move the selection,** so a hovered row was not highlighted. Fixed (`hovering_a_menu_row_selects_it`).

**Not run live:**
- **Alt+click and Alt+Enter (split):** posted messages carry no Alt state. Covered by `click_profile_alt_click_split`, `enter_opens_alt_enter_splits` and `split_profile_uses_chosen_profile_along_longer_side`.
- **`Ctrl+Shift+Space` and `Ctrl+Shift+1…9`:** posted messages carry no Ctrl state. Covered by `ctrl_shift_space_matches_menu` and `ctrl_shift_digit_matches_new_tab_profile`.
- **Icons from `.exe` and `.ico` files:** not supported; those profiles show a letter tile.
