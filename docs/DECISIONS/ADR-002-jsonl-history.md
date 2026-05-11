# ADR-002: JSONL audit log, not SQLite

**Status**: Accepted (M0)
**Date**: 2026-05-11

## Context

DCSBinder must record every remap operation for: (a) showing the user a history view, (b) supporting one-click undo, (c) post-hoc forensics if a remap goes wrong. Two natural choices:

1. **JSONL** — append-only newline-delimited JSON at `%APPDATA%/DCSBinder/history.jsonl`.
2. **SQLite** — embedded database at `%APPDATA%/DCSBinder/history.db`.

## Decision

Use **JSONL** for v1. Migrate to SQLite only if cross-aircraft queries become a real need.

## Rationale

- **Crash safety**: a JSONL append is one `write` + one `fsync`. Idempotent and trivially correct. SQLite is also correct but the moving parts (WAL, journal modes, locking) are overkill for an audit log.
- **Auditability**: users can open `history.jsonl` in Notepad. Power users can pipe through `jq`. SQLite requires a CLI install to inspect.
- **Sharing for bug reports**: a single text file copy-pastes into a GitHub issue. A `.db` does not.
- **Volume**: a heavy user might generate a few hundred entries per year. Linear scan in Rust is microseconds; no query engine needed.
- **Migration is cheap**: if SQLite is needed later, a 50-line script imports JSONL into SQLite. The path is one-way only and easy.

## Consequences

- No SQL queries — features like "show all remaps for device X" require a full-file scan in Rust. Fine at the expected scale.
- The undo feature reads the manifest, not the JSONL — JSONL is for display and forensics only. The source of truth for restoring files is the per-operation backup folder.
- `history.jsonl` must be written atomically. We use `OpenOptions::append + write_all + sync_data` per line. Crash mid-line is detectable by JSON parse failure on read.

## Schema versioning

Every entry has `"version": 1`. When (if) we change the schema, bump the version field and write a reader that handles both. Never re-write old entries in place.
