# CLAUDE.md — DCSBinder navigator for AI sessions

You (a future Claude Code session) are continuing work on **DCSBinder**, a Rust + Slint Windows desktop app. This file gives you the minimum context to act safely; for depth, follow the pointers at the bottom.

## What DCSBinder does

Windows reassigns controller GUIDs (USB hub change, driver reinstall, etc.) and DCS World keys its bindings by GUID, so the user has to re-bind every aircraft after a reassignment. DCSBinder scans the DCS Input tree, detects GUID conflicts, identifies which GUID is currently live via SDL2, lets the user diff the old and new bindings, then atomically remaps the chosen content under the live GUID across every aircraft folder — backing up everything first and writing a tamper-evident history log.

## Tech stack (locked)

- **Rust 2021** (workspace, three crates: `dcsbinder-core`, `dcsbinder-cli`, `dcsbinder-ui`).
- **Slint** for UI (pinned exact version when added).
- **DirectInput** via the `windows` crate for joystick **discovery** (ADR-004).
  DCS uses Windows DirectInput instance GUIDs in filenames; SDL2's joystick GUID
  is a different (vendor/product) format and cannot match. SDL2 is deferred to M4+
  for runtime/UI input handling.
- **`full_moon`** for Lua AST parsing; **bespoke serializer** for writing.
- **`indexmap`, `walkdir`, `regex`, `tempfile`, `blake3`, `serde`/`serde_json`, `uuid` (v7), `sysinfo`, `similar`** for supporting modules.
- **No alternative stacks.** If a future session is tempted to swap (e.g. `mlua` for `full_moon`, SQLite for JSONL, `egui` for Slint), read the relevant ADR in `docs/DECISIONS/` first.

## Hard invariants (do not break)

1. **Byte-equal round-trip.** Every `.diff.lua` fixture must parse and re-serialize byte-identical. Tested in `crates/dcsbinder-core/tests/roundtrip.rs`. This guards against DCS rejecting files written by us.
2. **M1–M3 remap path copies file bytes verbatim.** Re-serialization is gated behind a future "merge" feature and a green round-trip suite. See `docs/DECISIONS/ADR-003-byte-copy-over-reserialize.md`.
3. **Two-phase commit with manifest.** Remap operations write a manifest *before* any user-data mutation, and finalize with a `.done` sibling marker. On startup, un-`.done` manifests trigger a rollback prompt.
4. **Never delete user files.** Stale-GUID files are *moved* into the backup folder, never deleted.
5. **Refuse to operate while `DCS.exe` is running** (sharing-violation risk). Detect via `sysinfo`.
6. **`#![deny(unsafe_code)]` in core.** No exceptions without an ADR.

## Coding conventions

- `rustfmt` defaults. `cargo fmt --check` runs in CI.
- `cargo clippy --workspace --all-targets -- -D warnings` runs in CI.
- Prefer `thiserror` for library error types, `anyhow` only in `dcsbinder-cli` and `dcsbinder-ui` (binaries).
- Tests live alongside source as `#[cfg(test)] mod tests` for unit, and in `tests/` for integration.
- Fixture corpus is in `crates/dcsbinder-core/tests/fixtures/` — real sanitized DCS files, treat as ground truth.

## Repo layout (high-level)

```
DCSBinder/
├── Cargo.toml                workspace root
├── CLAUDE.md                 (this file)
├── README.md                 user-facing intro
├── LICENSE-MIT / LICENSE-APACHE
├── docs/
│   ├── ARCHITECTURE.md       module map, two-phase commit, error taxonomy
│   ├── FILE_FORMAT.md        annotated .diff.lua + GUID format reference
│   ├── ROADMAP.md            milestones M0–M6, deferred features
│   └── DECISIONS/            ADRs (read these before reversing decisions)
├── .github/workflows/ci.yml  fmt + clippy + test
└── crates/
    ├── dcsbinder-core/       pure Rust + SDL2 (no Slint dep)
    ├── dcsbinder-cli/        headless, M1-shippable
    └── dcsbinder-ui/         Slint front-end (M4+)
```

## Current milestone

See `docs/ROADMAP.md` for the M0–M6 plan and what each milestone ships. Use GitHub issues labeled `milestone:m1`…`milestone:m6` for active work.

## When you're unsure

- **About file format**: read `docs/FILE_FORMAT.md` and the fixtures in `crates/dcsbinder-core/tests/fixtures/`.
- **About architecture**: read `docs/ARCHITECTURE.md`.
- **About a past decision**: read the relevant ADR in `docs/DECISIONS/`. If you want to reverse a decision, add a new ADR that supersedes the old one — do not delete or edit the original.
- **About scope**: read `docs/ROADMAP.md`. If a request is outside v1 scope, surface it as a GitHub issue instead of silently expanding M1–M6.

## Upstream

GitHub: <https://github.com/anpeaco/DCSBinder> — issues, releases, CI. Repo owner is `anpeaco`.
