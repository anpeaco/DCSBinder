//! Read and write DCS Lua config files (`.diff.lua` and `modifiers.lua`).
//!
//! Reading uses `full_moon` (AST-level, no Lua runtime). Writing uses a bespoke
//! serializer that matches DCS's exact formatting style. See ADR-001 and ADR-003.

pub mod reader;
pub mod types;
pub mod writer;

pub use reader::{parse, ParseError};
pub use types::{LuaFile, LuaKey, LuaTable, LuaTableEntry, LuaValue};
pub use writer::write;
