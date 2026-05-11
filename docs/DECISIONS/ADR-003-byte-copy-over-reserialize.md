# ADR-003: M1–M3 remap path copies file bytes verbatim, never re-serializes

**Status**: Accepted (M0)
**Date**: 2026-05-11

## Context

When DCSBinder remaps a binding to a new GUID, the natural implementation is to:

1. Parse the source `.diff.lua` into a typed model.
2. Write the typed model to the new filename via the bespoke serializer.

This has an obvious failure mode: if the serializer's output differs from DCS's expected format by even one byte (a missing trailing comma, the wrong indentation character, a re-ordered key), DCS may reject the file or — worse — silently misinterpret it.

## Decision

For M1 through M3, the remap engine **copies file bytes verbatim** from the source path to the destination path. It never invokes the serializer.

Re-serialization is gated behind:
1. A future "merge two binds" feature that genuinely needs to produce a new file from typed-model inputs.
2. A green byte-equal round-trip test on the entire fixture corpus (parse → serialize → assert ==).

## Rationale

- The v1 remap operation is, at its core, a **rename**: same content, different filename. There is no semantic reason to re-serialize.
- DCS's exact formatting style is undocumented. We are reverse-engineering it from fixtures. There is no oracle that says "this serializer output is correct" except DCS itself accepting the file at load time.
- Until the round-trip suite is green across a corpus that includes every edge case (Unicode device names, oddball command IDs, modifier-wrapped keys), re-serialization is a foot-gun.
- Byte-copy is also faster, simpler, and has zero correctness risk.

## Consequences

- The serializer is written and tested as part of M1, but only the *test* (round-trip equality) gates progress. The serializer is not on the remap critical path until a feature genuinely needs it.
- The `modifiers.lua` rewrite path in M3 is the exception — it has to rewrite content because device names appear inline. Mitigations:
  - Use the same bespoke serializer.
  - Round-trip-test `modifiers.lua` fixtures byte-equal before rewriting them.
  - Refuse the operation if round-trip fails for that specific fixture; surface to the user as a known limitation.

## Plan B (not chosen)

Always re-serialize. Rejected because the cost of one bad write (DCS refuses to launch with the bind file → user has to manually delete from `%USERPROFILE%/Saved Games/`) is higher than the cost of the constraint.

## Supersedes / superseded by

None. This ADR may be superseded in M5+ when the merge feature lands and the round-trip suite is robust enough to trust.
