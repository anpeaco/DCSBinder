# DCS Input File Format

Reference for the files DCSBinder reads, writes, and rewrites. Source of truth is the fixture corpus under `crates/dcsbinder-core/tests/fixtures/`; this document explains what's in those files.

## Path structure

```
%USERPROFILE%/Saved Games/<DCS_root>/Config/Input/<Aircraft>/<subtype>/<DeviceName> {GUID}.diff.lua
```

- `<DCS_root>` ∈ { `DCS`, `DCS.openbeta`, `DCS.dcs_serverbeta` }.
- `<Aircraft>` is the DCS internal aircraft name, e.g. `A-10C II`, `F-16C_50`, `AH-64D_BLK_II_PLT`.
- `<subtype>` ∈ { `joystick`, `keyboard`, `mouse`, `trackir` }.
- `<DeviceName>` is the controller's reported product name (or user-edited).
- `{GUID}` is the **DirectInput GUID** in canonical 8-4-4-4-12 form with braces.

A per-aircraft `modifiers.lua` may also live in `<Aircraft>/` (next to the subtype folders), referencing devices by name in modifier definitions. Remap must rewrite it alongside filename changes.

## Filename regex

```regex
^(?P<name>.+?) \{(?P<guid>[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12})\}(?P<suffix>[^.]*)\.diff\.lua$
```

| Capture | Meaning |
|---|---|
| `name` | Device name. Lazy match so it doesn't gobble `{…}` if a user pasted braces. |
| `guid` | Strict DirectInput shape. Anything else → `Malformed`. |
| `suffix` | Any text between `}` and `.diff.lua`. **Non-empty = `UserArchived`** (e.g. `MFDLeft {…}XXX.diff.lua`). |

Files ending in `.bak`, `.disabled`, `.old`, or `.diff.lua.<anything>` are also `UserArchived`.

`.lua` files that don't match the regex but don't have a `.diff` suffix (e.g. DCS-exported user profiles) are ignored, not classified as malformed.

## GUID format

Two forms exist for the same physical device:

- **DirectInput / DCS form** (what filenames use): `{4E50F3B0-2309-11ee-8015-444553540000}` — 8-4-4-4-12 hex with braces.
- **SDL form** (what the `sdl2` crate returns): `4e50f3b0230911ee8015444553540000` — 32 hex chars, no braces, no dashes, lowercase.

The trailing `444553540000` (= ASCII `DESS`) is the DirectInput vendor marker on Windows. Devices enumerated via SDL2 from a DirectInput backend will have this marker; XInput devices use a different layout. DCS files always have the DirectInput form.

Conversion lives in `dcsbinder-core::device::guid`. The single most important test in the project is the pair (SDL GUID string, DCS filename) for a known-real controller round-trip — without that test green, nothing else in the device-matching path is trustworthy.

## `.diff.lua` body

A `.diff.lua` is a Lua script that builds and returns a single table:

```lua
local diff = {
    ["axisDiffs"] = {
        ["a2001cdnil"] = {
            ["name"] = "Pitch",
            ["removed"] = {
                [1] = { ["key"] = "JOY_Y" },
            },
        },
        ["a2002cdnil"] = {
            ["name"] = "Roll",
            ["added"] = {
                [1] = { ["key"] = "JOY_X" },
            },
        },
    },
    ["keyDiffs"] = {
        ["d1527pnilunilcdnilvdnilvpnilvunil"] = {
            ["name"] = "Left MFCD Disable power",
            ["added"] = {
                [1] = { ["key"] = "JOY_BTN28" },
            },
        },
    },
}
return diff
```

### Top-level keys

- `axisDiffs` — entries for analog axes (sticks, throttles, pedals, knobs).
- `keyDiffs` — entries for buttons / digital inputs.

Both are dictionaries keyed by a **command ID** (DCS's internal identifier for the bindable action).

### Command IDs

Command IDs encode the action plus modifier configuration. Two prefixes observed:

- `a<num>cd<mod>` — axis command. `num` is the axis action ID; `cd` segment encodes the "curve" / "deadzone" / modifier (`nil` = none).
- `d<num>p<mod>u<mod>cd<mod>vd<mod>vp<mod>vu<mod>` — key command. `num` is the key action ID; the trailing segments encode press/unpress/cooldown/voice modifiers.

DCSBinder treats command IDs as opaque strings — we never parse them. They are stable across runs for the same action.

### Entry shape

Each entry has:

- `name` — human-readable label DCS uses in the bind UI. Often duplicated across files; not unique.
- `added` — array of `{ key = "..." }` objects representing bindings the user added relative to DCS's default.
- `removed` — array of `{ key = "..." }` objects representing bindings the user removed from DCS's default.

Both `added` and `removed` are optional and may co-exist. A "key" is a string like `JOY_BTN1`, `JOY_Y`, `JOY_POV1_U`, or modifier-wrapped objects (full shape TBD when modifier fixtures land).

### Formatting style DCS emits

The bespoke serializer must replicate this byte-for-byte:

- **Tabs** for indentation (1 tab per level).
- **LF** line endings (`\n`), even on Windows. Never `\r\n`.
- String keys quoted as `["string"]`. Strings use `"` (double quote), not `'`.
- Numeric array keys as `[1]`, `[2]`, …, **one-indexed**, sequential.
- Spaces around `=` (`] = `, not `]=`).
- **Trailing comma** on every entry inside braces.
- Outer wrapper exactly:
  ```
  local diff = {
  …
  }
  return diff
  ```
  **No trailing newline at EOF** — the file ends with the `f` of `diff`.
- **Top-level key order**: alphabetical. Observed in fixtures: `axisDiffs` before `keyDiffs`.
- **Within each command**, keys appear in alphabetical order (`added` < `name` < `removed`).
- **Command IDs** within `axisDiffs` / `keyDiffs` appear in lexical order.

The parser preserves the actual order found in each file via `IndexMap`; the
serializer emits whatever order was parsed. This means we follow DCS automatically
if DCS ever changes its sort order.

The byte-equal round-trip test in `tests/roundtrip.rs` is the enforcement.

## `modifiers.lua`

Lives at `<Aircraft>/modifiers.lua` (not inside a subtype folder). Schema differs from `.diff.lua` — references devices by *name* in modifier definitions. **When DCSBinder remaps a device, `modifiers.lua` must be rewritten alongside the filename change** if it references the renamed device. Fixture: `crates/dcsbinder-core/tests/fixtures/AH-64D_BLK_II_CPG/modifiers.lua` (added in M0).

## Subtype-specific notes

- **joystick**: most common; the only subtype where GUID conflicts are common.
- **keyboard**: usually a single file (e.g. `Keyboard.diff.lua`) with no GUID braces. Different filename regex; classify carefully.
- **mouse**: similar to keyboard.
- **trackir**: typically TrackIR head-tracking axis bindings; behaves like joystick for our purposes.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) for how the parser and scanner consume these files.
- [`DECISIONS/ADR-001-full_moon-parser.md`](DECISIONS/ADR-001-full_moon-parser.md) for why we use `full_moon` not `mlua`.
- [`DECISIONS/ADR-003-byte-copy-over-reserialize.md`](DECISIONS/ADR-003-byte-copy-over-reserialize.md) for why M1–M3 never re-serialize.
