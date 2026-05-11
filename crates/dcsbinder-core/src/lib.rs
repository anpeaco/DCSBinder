//! `DCSBinder` core: parsing, scanning, conflict detection, and the remap engine.
//!
//! See `docs/ARCHITECTURE.md` in the workspace root for the high-level design.

pub mod config;
pub mod conflict;
pub mod device;
pub mod history;
pub mod parser;
pub mod remap;
pub mod scanner;
