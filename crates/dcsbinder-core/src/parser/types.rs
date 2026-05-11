//! Typed model of a DCS Lua config file (`.diff.lua` or `modifiers.lua`).
//!
//! The model is intentionally general — it captures the narrow Lua subset DCS
//! actually emits without over-specializing. Number values are stored as their
//! original source text (e.g. `"0.3"`, `"1"`, `"-0.15"`) to guarantee byte-equal
//! round-trip; we never parse them into `f64`.
//!
//! Higher-level semantic wrappers (e.g. "this is the `axisDiffs` section, those
//! are commands with `added`/`removed` arrays") layer on top of [`LuaFile`].

/// A whole `.diff.lua` or `modifiers.lua` file: a single `local <name> = <table>`
/// assignment followed by `return <name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaFile {
    /// The bound variable name (`"diff"` for `.diff.lua`, `"modifiers"` for `modifiers.lua`).
    pub var_name: String,
    pub value: LuaTable,
}

/// A Lua table. Entries are stored as a `Vec` to preserve source order exactly
/// (DCS sorts keys, but we don't depend on that — we follow whatever the file used).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaTable {
    pub entries: Vec<LuaTableEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaTableEntry {
    pub key: LuaKey,
    pub value: LuaValue,
}

/// Either a string key (`["name"]`) or an integer key (`[1]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaKey {
    Str(String),
    Int(i64),
}

/// A Lua value within a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaValue {
    /// A string literal. Stored unescaped.
    Str(String),
    /// A number literal. Stored **as its original source text** (`"0.3"`, `"1"`,
    /// `"-0.15"`) so round-trip is byte-equal without floating-point precision concerns.
    Number(String),
    Bool(bool),
    Nil,
    Table(LuaTable),
}

impl LuaTable {
    /// Look up a string-keyed entry's value, by key. Returns `None` if the key
    /// is absent or is an integer key.
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&LuaValue> {
        self.entries.iter().find_map(|e| match &e.key {
            LuaKey::Str(k) if k == key => Some(&e.value),
            _ => None,
        })
    }
}
