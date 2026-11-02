# M4 ledger — plan: docs/superpowers/plans/2026-11-02-wraith-m4.md

Plan: Ruling: no sidecar OSC parser (vendored Ghostty has the callbacks); OSC 7501 program status used as a passive source; toast activation in-process (no --activate COM path); no agent-tab-pulse; WSLENV gains WRAITH_PIPE; Codex tested by payload only (not installed).
Task 1: complete — agent crate (harness table, step engine, sanitize); state 8/8, harness 5/5. Note: tests and code were written in one pass, so RED was only 'crate missing', not each behavior. Ruling: state signals never create an agent; the server sends Attached first for hook events (keeps step pure) — cost if wrong: none. Ruling: typed() and became_visible() return the agent to Idle (not just clear attention), so a seen or answered wait shows no badge.
