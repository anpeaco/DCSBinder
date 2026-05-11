# Roadmap

Milestones map 1:1 to GitHub milestones (`milestone:m0`…`milestone:m6`). Open work lives there; this document is the long-form rationale.

## M0 — scaffold

**Exit criteria**: `cargo check --workspace` is green. CI runs on every push. Docs and ADRs published. First commit pushed to `https://github.com/anpeaco/DCSBinder`.

Deliverables:
- Workspace `Cargo.toml`, three crates (`core`, `cli`, `ui`).
- `rust-toolchain.toml`, `.gitignore`, dual `LICENSE-MIT` / `LICENSE-APACHE`.
- `CLAUDE.md`, `README.md`, `docs/{ARCHITECTURE,FILE_FORMAT,ROADMAP}.md`, three ADRs.
- `.github/workflows/ci.yml` (fmt, clippy `-D warnings`, test on `windows-latest`).
- `.github/ISSUE_TEMPLATE/{bug,feature}.md`.
- Sanitized fixture corpus copied from the user's real DCS Input tree (incl. the `MFDLeft` conflict pair and an AH-64D `modifiers.lua`).
- GitHub milestones M1–M6 and labels (`area:*`, `risk:safety-critical`, `good-first-issue`) created via `gh`.

## M1 — parse + scan + CLI

**Exit criteria**: `dcsbinder scan` reports every device in every aircraft folder, flagging GUID conflicts. Byte-equal round-trip test passes for every fixture. **Tag `v0.1.0`.**

Deliverables:
- `core::parser` reads `.diff.lua` via `full_moon` into typed model (`DiffFile { axis_diffs, key_diffs }`).
- Bespoke serializer matching DCS's exact formatting style.
- `tests/roundtrip.rs` — every fixture parses and re-emits byte-identical.
- `core::scanner` walks the Input tree, classifies files via the filename regex.
- `core::conflict` groups by `(install, aircraft, subtype, name)` and flags >1-GUID groups.
- `dcsbinder-cli scan` subcommand prints a human-readable report.

## M2 — SDL2 + device matching

**Exit criteria**: `dcsbinder devices` lists every connected joystick with its DCS-format GUID, matched to the filenames in the Input tree.

Deliverables:
- `core::device` initializes SDL2 (joystick subsystem only, headless).
- `core::device::guid` converts SDL form ↔ DirectInput form. **Load-bearing test**: pair the user's real Warthog throttle's SDL GUID with the bind filename `Throttle - HOTAS Warthog {4E50F3B0-2309-11ee-8016-…}`.
- `core::config::dcs_running()` returns the PID of `DCS.exe` if running.
- `core::config::discover_installs()` finds `DCS`, `DCS.openbeta`, `DCS.dcs_serverbeta`.
- `dcsbinder-cli devices` subcommand.

## M3 — backup + remap engine

**Exit criteria**: `dcsbinder remap --device <name> --to-guid <new>` performs a full crash-safe remap across all aircraft for one device. **Crash recovery integration test passes** (kill the writer mid-write, restart, restore is offered and succeeds).

Deliverables:
- `core::remap` two-phase commit with manifest.
- Manifest schema (JSON, versioned).
- Backup copy with blake3 hash verification.
- Atomic `tmp → rename` within target directory.
- Stale-GUID file move-to-backup (never delete).
- `core::remap::recover()` for un-`.done` manifests.
- `modifiers.lua` rewritten when affected device is referenced.
- `dcsbinder-cli remap`, `dcsbinder-cli undo`.

## M4 — Slint UI

**Exit criteria**: All six screens functional. Dry-run preview is mandatory before any mutating action.

Deliverables:
- Slint dep added; `slint = "=1.x.y"` pinned exactly.
- Six screens: Dashboard, Devices, Conflicts, Diff View, History, Settings.
- Background-thread scan with progress reporting to the UI thread.
- Mandatory dry-run preview dialog before any remap confirmation.
- Diff view computes highlights in Rust via `similar`, feeds Slint as a model.

## M5 — history + undo + aliases

**Exit criteria**: Every operation appended to `history.jsonl`. Undo restores files byte-identical to pre-remap. Alias map editor functional. Fuzzy device-name suggestions appear (suggest-only).

Deliverables:
- `core::history` JSONL appender with UUIDv7 op IDs.
- Undo path: read manifest → restore from backup → write inverse history entry.
- `aliases.json` editor in Settings.
- `strsim` Jaro-Winkler suggestions on the Devices screen ("These look similar — link?"). User confirms every link.

## M6 — polish + installer

**Exit criteria**: MSI installer produced via `cargo-wix`. App icon, About dialog, signed if budget allows.

Deliverables:
- `cargo-wix` config + MSI build target.
- `SDL2.dll` bundled via `sdl2` crate's `bundled` feature.
- App icon (`.ico`) and Windows resource.
- About dialog with version, GitHub link, license summary.
- Optional code signing.

## Deferred (post-v1)

- Linux/macOS support (DCS doesn't run on either, so cost-benefit is poor).
- Importing/exporting DCS user profiles.
- Editing individual bindings (out of scope — DCSBinder is a remap tool, not a bind editor).
- "Merge" of two conflicting binds (requires green re-serialization path; pinned to a green round-trip suite).
- SQLite history (only if cross-aircraft query needs emerge).
- Auto-fuzzy device-name matching (always suggest-only).
- Auto-launching DCS after remap, or auto-detecting that a remap is needed on DCS startup.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md), [`FILE_FORMAT.md`](FILE_FORMAT.md), [`DECISIONS/`](DECISIONS/).
- GitHub milestones: <https://github.com/anpeaco/DCSBinder/milestones>.
