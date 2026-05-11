//! Enumerate currently-attached controllers via Windows `DirectInput` and
//! handle the DCS-form GUID `{8-4-4-4-12}` they use in filenames.
//!
//! See `ADR-004` for why we go straight to `DirectInput` instead of using SDL2:
//! DCS filenames carry the **instance** GUID Windows assigns at enumeration time,
//! which SDL2's joystick GUID (a bus/VID/PID-based format) cannot match.

pub mod guid;

pub use guid::{Guid, GuidParseError};

#[cfg(windows)]
pub mod enumerator;

#[cfg(windows)]
pub use enumerator::{enumerate, EnumError, LiveDevice};

/// Public stub for non-Windows builds (CI matrices, dev on Linux/macOS).
#[cfg(not(windows))]
pub fn enumerate() -> Result<Vec<()>, &'static str> {
    Err("device enumeration requires Windows + DirectInput")
}
