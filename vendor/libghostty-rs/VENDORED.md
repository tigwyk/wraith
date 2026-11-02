# Vendored libghostty-rs

Source: https://github.com/uzaaft/libghostty-rs at `575ed6b8ceb79ffaf247ecf7e34333876b0d5be4` (2026-10-30), crates only.
License: MIT OR Apache-2.0 (see LICENSE).

Why vendored: Wraith tracks a newer Ghostty than any published crate. The raw bindings are
regenerated here against that Ghostty.

Local changes:
- `crates/libghostty-vt-sys/build.rs`: `GHOSTTY_COMMIT` pinned to Ghostty `7cd6fa0215a1299c69c27fe543f48ecab45b775f` (2026-11-02).
- `crates/libghostty-vt-sys/src/{bindings,layouts}.rs`: regenerated against that commit:

      set GHOSTTY_SOURCE_DIR=<ghostty checkout at the pinned commit>
      set LIBCLANG_PATH=<dir with libclang.dll>
      cargo run --manifest-path vendor/libghostty-rs/Cargo.toml -p libghostty-vt-sys --features bindgen-tool --bin gen-bindings
- `crates/libghostty-vt/src/terminal.rs`: `vt_write_until_ground` calls `ghostty_terminal_vt_write_until` with the GROUND boundary (upstream Ghostty removed `ghostty_terminal_vt_write_until_ground`).

Building needs Zig 0.17.x on PATH (Ghostty's `minimum_zig_version`).
