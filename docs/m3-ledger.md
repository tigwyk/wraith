# M3 execution ledger (every deviation from the plan, and the final-review outcome)
Spec: docs/superpowers/specs/2026-11-01-wraith-terminal-design.md
Plan: docs/superpowers/plans/2026-11-01-wraith-m3.md
Branch: m3 (from main 10b5be4)
Pre-flight:
- T1 model types -> T3 Model, T4 wire, T8 view: consistent (PaneId stays in wire; model.rs imports it).
- T3 ModelCmd/SpawnOpts.image_storage_limit -> T4 session, T12 pane: consistent.
- T4 v2 messages -> T6 ServerLink, T8 view, cli: consistent.
- T7 Pipeline/Batch -> T8 draw, T13 images: consistent.
- T12 ImagePlacement/PaneEvent -> T13 store/draw: consistent.
Task 1: Ruling: layout_never_below_min asserts bounds/count when the area cannot fit the minimums (plan text said 'min or exactly min'); found+fixed a b-rect offset past the area when the usable span is 0 — cost if wrong: none.
Task 1: complete (commits 90c5735..79d55d5, tests: rtk proxy cargo test -p wraith-proto → exit 0, 15 model tests pass)
Task 2: Ruling: added shell-integration key (detect|none|pwsh|bash|zsh → bool) and Config.shell_integration here, since Task 5 reads it — cost if wrong: none.
Task 2: Ruling: client app.rs gets a temporary wildcard arm for the new Actions so the workspace builds; Task 8 replaces it.
Task 2: Note: test source with Windows paths uses forward slashes (tool heredocs halve backslashes); parser still tested for drive-colon handling.
Task 2: complete (commits f2a69e3..997ee9b, tests: rtk proxy cargo test --workspace → exit 0; config 22+15 tests)
Task 3: Ruling: ModelCmd lives in wraith_proto::model; SpawnOpts gained image_storage_limit now (all constructors updated) — needed by NewWorkspace restore defaults.
Task 3: Ruling: a window whose workspace is removed (last tab closed) needs a re-attach; Model drops the workspace and creates a fresh default (test pins this). Session (Task 4) must handle 'my workspace vanished' by attaching the most recent detached one.
Task 3: Ruling: from_json also drops panes not referenced by any tab and resets attached/exit; any dangling reference rejects the whole file.
Task 3: complete (commits 4195838..f1584d1, tests: rtk proxy cargo test --workspace → exit 0; model 17/17)
