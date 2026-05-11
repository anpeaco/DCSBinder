# ADR-004: DirectInput (via `windows` crate), not SDL2, for device discovery

**Status**: Accepted (M2)
**Date**: 2026-05-11
**Supersedes**: the SDL2-only device-discovery decision implied in the M0 plan.

## Context

DCS Input filenames use a Windows **DirectInput instance GUID** for every joystick:

```
<DeviceName> {4E50F3B0-2309-11ee-8015-444553540000}.diff.lua
```

These are time-based UUIDs (note the `11ee` version-1 marker) that Windows generates when it enumerates a DirectInput device. The whole point of DCSBinder is that **Windows reassigns these GUIDs** on hub changes / driver reinstalls / etc. — so for a tool to identify *which* GUID is currently live for a connected device, it has to read the DirectInput instance GUID directly.

The M0 plan called for SDL2 (`sdl2` crate) to enumerate joysticks. But SDL2's `Joystick::guid()` returns a **different** GUID, constructed internally by SDL from bus type + USB VID + USB PID + version + CRC. It is not the Windows DirectInput instance GUID, and there is no documented conversion between the two. Empirically, two physical copies of the same controller will get *identical* SDL GUIDs but **different** DirectInput instance GUIDs — exactly the case DCSBinder must distinguish (e.g. `MFDLeft` and `MFDRight` are two of the same physical MFD device).

## Decision

Use the **`windows` crate** (official Microsoft Rust bindings for the Win32 API) to drive **DirectInput 8** directly:

1. `DirectInput8Create` to obtain the `IDirectInput8` interface.
2. `IDirectInput8::EnumDevices(DI8DEVCLASS_GAMECTRL, …)` to enumerate joysticks.
3. For each `DIDEVICEINSTANCEW` in the callback, read `guidInstance` (the instance GUID DCS uses) and `guidProduct` (vendor+product encoded the standard PIDVID way).
4. Format both as DCS-style `{8-4-4-4-12}` strings via the same `Guid::to_dcs_string()` helper used by the filename parser.

Feature-gate `windows` to keep compile times reasonable:

```toml
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_System_Com",
    "Win32_Devices_HumanInterfaceDevice",
] }
```

## SDL2 stays in the plan, just not for this

SDL2 remains the right choice for things it's actually good at: rich human-readable button/axis labels for the M4 Diff view, runtime input handling, hot-plug events. ADR-001/SDL2 dependency is **deferred to M4**, not removed.

## Rationale

- **Correctness over convenience.** We have to talk to the same enumerator DCS talks to (DirectInput). Anything else is a guessing game.
- **The `windows` crate is the official, maintained Microsoft path.** Alternatives like `multiinput` exist but are unmaintained; raw FFI is unnecessary because windows-rs covers DirectInput.
- **Cross-platform is a non-goal** (per ROADMAP "deferred features"). DCS is Windows-only.
- **Compile time is acceptable.** With only three feature flags enabled, `windows` adds ~10–15s to a clean build — comparable to SDL2.

## Consequences

- `core::device` is now a Windows-only module. The crate uses `#[cfg(windows)]` to gate the enumerator; on non-Windows it returns a "not supported" error. CI runs on `windows-latest` already so this doesn't break anything.
- `core::device::guid::Guid` is a 16-byte newtype with `parse_dcs` / `to_dcs_string`. It uses Windows's `GUID` struct under the hood when interfacing with DirectInput, but exposes a portable API.
- We never need a "SDL ↔ DCS GUID converter." The whole conversion problem is sidestepped by going straight to DirectInput.
- The load-bearing M2 test (correlate a currently-attached controller's GUID with a DCS filename's GUID) now uses DirectInput enumeration, not SDL2 enumeration.

## Plan B (not chosen)

Wrap raw FFI to DirectInput by hand, avoiding the `windows` crate. Rejected because windows-rs is already feature-gated, well-tested, and means we don't carry our own COM-vtable boilerplate.

## References

- DirectInput documentation: <https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ee416842(v=vs.85)>
- `windows` crate: <https://crates.io/crates/windows>
- SDL2 joystick GUID format (for the record, this is what we're *not* using): the layout is described in `SDL_joystick.c` in the SDL2 source, but the takeaway is that it's vendor/product-based, not instance-based.
