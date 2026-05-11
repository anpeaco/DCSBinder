# Architecture

This document describes the module boundaries, the safety-critical remap protocol, and the error taxonomy.

## Crate boundaries

```
dcsbinder-core   pure Rust + SDL2 (no Slint dep)
   ↑    ↑
   │    └──── dcsbinder-ui   Slint front-end
   └────────── dcsbinder-cli  headless tool
```

`dcsbinder-core` is the source of truth for all behavior. `cli` and `ui` are thin shells that drive the core. Anything that mutates files lives in `core::remap`.

## Module map (`dcsbinder-core`)

| Module | Responsibility | Milestone |
|---|---|---|
| `parser` | Read `.diff.lua` via `full_moon`; write via bespoke serializer | M1 |
| `scanner` | Walk DCS Input tree; classify files (Active / UserArchived / Malformed) | M1 |
| `conflict` | Group by `(install, aircraft, subtype, normalized_device_name)`; flag groups with >1 distinct GUID | M1 |
| `device` | Enumerate via SDL2; convert SDL GUID → DirectInput form | M2 |
| `remap` | Two-phase commit engine: plan → manifest → backup → write → move-stale → finalize | M3 |
| `history` | JSONL append-only audit log | M5 |
| `config` | Install discovery, `%APPDATA%/DCSBinder` paths, alias map, "DCS running?" check | M2/M3 |

## Two-phase commit (remap)

Every remap is a single transaction with this fixed protocol:

```
1. Plan          (pure: build RemapPlan { backups, writes, moves, manifest_path })
2. Manifest      (write tmp → persist atomically; existence = "started")
3. Backup        (copy every affected file into backup dir; record size + blake3)
4. Write         (for each target: write .tmp then fs::rename — atomic within NTFS dir)
5. Move-stale    (stale-GUID files → backup dir; NEVER delete)
6. Finalize      (write manifest.json.done sibling)
```

**Recovery**: on startup, `core::remap::recover()` scans `%APPDATA%/DCSBinder/backups/*/manifest.json` for siblings missing `.done`. Any hit triggers the rollback prompt; rollback = "restore every file in the manifest from the backup dir."

**Crash matrix**:

| Crash after step | State on disk | Recovery |
|---|---|---|
| 1 | Nothing written | No-op (plan was pure) |
| 2 | Manifest only | Delete manifest folder |
| 3 | Manifest + backup, no source mutations | Delete manifest folder |
| 4 (partial) | Some sources written, some original | Restore from backup |
| 5 (partial) | All sources new, some stale still in place | Restore from backup or finalize forward |
| 6 | All done but no `.done` marker | User confirms forward-finalize |

## Error taxonomy

`core` exports one top-level `Error` enum (via `thiserror`), with variants:

- `Io(std::io::Error)` — wrap with context using a small `Context` wrapper
- `LuaParse { path, source: full_moon::Error }` — file failed to parse
- `MalformedFilename { path }` — regex didn't match
- `GuidFormat { input }` — GUID didn't match the 8-4-4-4-12 DirectInput shape
- `DcsRunning { pid }` — refuse to operate; report PID for the user
- `ManifestCorrupt { path, reason }` — JSON parse failed or hash mismatch
- `BackupFailed { path, source }` — backup copy or hash verify failed
- `Sdl2(String)` — SDL2 returns string errors; wrap them
- `AliasConflict { name }` — alias map has contradictory entries

Binaries (`cli`, `ui`) use `anyhow::Error` and convert from `core::Error` with full context.

## Filesystem layout

### DCS user config (read/write)

```
%USERPROFILE%/Saved Games/<DCS_root>/Config/Input/
├── <Aircraft>/
│   ├── modifiers.lua                  (per-aircraft, optional)
│   ├── joystick/
│   │   └── <DeviceName> {GUID}.diff.lua
│   ├── keyboard/
│   ├── mouse/
│   └── trackir/
```

`<DCS_root>` is one of `DCS`, `DCS.openbeta`, `DCS.dcs_serverbeta`. Multiple installs are supported simultaneously.

### App data (read/write)

```
%APPDATA%/DCSBinder/
├── settings.json         (window state, retention, configured install roots)
├── aliases.json          (manual device-name aliases for fuzzy-matched cases)
├── history.jsonl         (append-only audit log)
└── backups/
    └── <utc-timestamp>/
        ├── manifest.json
        ├── manifest.json.done       (finalize marker)
        └── <Aircraft>/<subtype>/...  (snapshotted bytes)
```

## Threading model

- **CLI**: single-threaded synchronous I/O. Scans are linear; 1200 files = ~ms.
- **UI**: scans run on a `std::thread::spawn` background worker. Results are pushed to the Slint event loop via `slint::invoke_from_event_loop`. Never block the UI thread on filesystem walks.
- **SDL2**: init on the main thread of the binary that uses it. CLI uses the joystick subsystem in headless mode (no video).

## Safety invariants enforced by `core::remap`

1. The remap engine never deletes a file. "Delete" in the plan means "move to backup dir."
2. Every file written has its post-write blake3 hash compared against the plan's intended bytes; mismatch = abort.
3. The engine refuses to start if `DCS.exe` is running (via `sysinfo`).
4. The engine refuses to start if the target paths resolve under a OneDrive sync root (file-lock risk). User can override with explicit `--allow-onedrive` flag (CLI) or checkbox (UI), but with a loud warning.
5. The engine refuses to operate on a `UserArchived` file (e.g. `…XXX.diff.lua`); the user must un-archive it first.

## Dependencies (anticipated)

Mandatory: `thiserror`, `indexmap`, `walkdir`, `regex`, `full_moon`, `serde`, `serde_json`, `uuid`, `time`, `blake3`, `tempfile`, `sysinfo`, `directories`, `sdl2`, `similar`.

UI-only: `slint`, `slint-build`.

CLI-only: `clap`, `anyhow`, `tracing`, `tracing-subscriber`.

Test-only: `pretty_assertions`, `tempfile`, `assert_fs`.

## See also

- [`FILE_FORMAT.md`](FILE_FORMAT.md) — `.diff.lua` structure and the GUID format.
- [`ROADMAP.md`](ROADMAP.md) — milestone breakdown.
- [`DECISIONS/`](DECISIONS/) — Architecture Decision Records.
