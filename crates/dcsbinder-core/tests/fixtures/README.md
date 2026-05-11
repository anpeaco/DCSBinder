# Fixture corpus

Real, sanitized `.diff.lua` and `modifiers.lua` files captured from the project owner's
`Saved Games\DCS.openbeta\Config\Input\` tree on 2026-05-11. These are the ground truth
for round-trip serialization, scanner classification, and conflict-detection tests.

Files contain only key/axis bindings — no PII, no DCS IP. Safe to commit.

## Contents

- `A-10C II/joystick/MFDLeft {4E50F3B0-…}.diff.lua` — **old** GUID, 33 bindings.
- `A-10C II/joystick/MFDLeft {CD3E4960-…}.diff.lua` — **new** GUID, 30 bindings (subset).
  - This pair is the reference conflict for M1 scanner + M3 remap tests.
- `A-10C II/joystick/Throttle - HOTAS Warthog {4E50F3B0-…}.diff.lua` — used for the GUID
  format-conversion test in M2 (paired with the user's actual SDL2-reported GUID).
- `A-10C II/keyboard/Keyboard.diff.lua` — non-GUID-suffixed bind file; tests scanner's
  ability to classify keyboard binds (different filename pattern than joystick).
- `AH-64D_BLK_II_CPG/modifiers.lua` — aircraft-level modifiers file; tests M3's
  modifiers-rewrite path.

## Adding to the corpus

When a new edge case appears (Unicode device name, malformed file, novel modifier shape),
copy the relevant file in, add a one-liner above describing what it tests, and reference
it from the test that exercises it. Never edit a fixture in place — fixtures are
treated as immutable ground truth.
