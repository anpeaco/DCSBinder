# ADR-001: Use `full_moon` for Lua parsing

**Status**: Accepted (M0)
**Date**: 2026-05-11

## Context

DCSBinder must read and write `.diff.lua` files. The files are Lua scripts that build and return a single table. Three options:

1. **`mlua` / `rlua`** — embed a Lua runtime, execute the file, walk the returned table.
2. **Hand-rolled parser** — purpose-built for DCS's narrow Lua subset (literal tables, strings, numbers, booleans).
3. **`full_moon`** — pure-Rust Lua AST parser with trivia (whitespace/comment) preservation.

## Decision

Use **`full_moon`** for reading. Implement a bespoke serializer for writing (see ADR-003).

## Rationale

`mlua` / `rlua`:
- **Rejected.** Executes untrusted Lua from `Saved Games`, which is a needless attack surface for a tool that opens whatever the user has.
- Loses all formatting information (key order, whitespace, `[1] =` notation), so round-trip is impossible.
- Drags a Lua C runtime in via `libloading` — fussy on Windows installers.

Hand-rolled parser:
- **Rejected.** DCS's subset is tiny, but error recovery, unicode escapes, and edge cases (e.g. user-pasted characters in device names) accumulate maintenance debt.
- The win — perfect source-fidelity — is captured by the bespoke serializer anyway (see ADR-003).

`full_moon`:
- **Accepted.** Pure Rust, no FFI. Full Lua 5.x grammar. Preserves trivia at the AST level so we can validate that what we *wrote* re-parses identically.
- Used in production by `selene` (Lua linter) and `StyLua` (Lua formatter), both mature tooling.

## Consequences

- We pay `full_moon`'s compile cost (one-time, then cached).
- We get a strongly-typed AST to walk; conversion to our typed model (`DiffFile { axis_diffs, key_diffs }`) is straightforward.
- Writing does **not** use `full_moon`'s printer — we serialize from our typed model via a hand-written, DCS-style-matching writer (see ADR-003).
- Future feature "preserve user-added comments in `.diff.lua` files" is supported (full_moon retains them).

## Plan B (not chosen)

If `full_moon` proves unmaintained or hits a Lua-syntax case it can't represent: vendor a snapshot of the relevant subset and finish hand-rolling. Cost ≈ 1–2 weeks. Migration would be encapsulated in `core::parser::reader` so the rest of the codebase is unaffected.
