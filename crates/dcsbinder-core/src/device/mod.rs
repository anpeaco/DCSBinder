//! Enumerate currently-attached controllers via SDL2 and convert their GUIDs
//! from SDL form to the `DirectInput` form DCS uses in filenames.
//!
//! The GUID format-conversion test (SDL form → DCS filename match) is the most
//! load-bearing correctness test in the project. See `docs/FILE_FORMAT.md`.
//!
//! Until M2 lands, this module is a stub.
