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
