//! Serialize a [`LuaFile`] to DCS's exact `.diff.lua` / `modifiers.lua` byte format.
//!
//! Per ADR-003, the M1–M3 remap path never calls this writer — it byte-copies
//! source files. The writer exists so the round-trip test can prove parser
//! correctness, and so future "merge" features have a writer ready.

use std::fmt::Write as _;

use super::types::{LuaFile, LuaKey, LuaTable, LuaTableEntry, LuaValue};

/// Write `file` in DCS's canonical format. Returns a `String` with LF endings
/// and **no trailing newline** (DCS files end with the `f` of `return <name>`).
#[must_use]
pub fn write(file: &LuaFile) -> String {
    let mut out = String::new();
    let _ = write!(out, "local {} = ", file.var_name);
    write_table(&mut out, 0, &file.value);
    let _ = write!(out, "\nreturn {}", file.var_name);
    out
}

fn write_table(out: &mut String, depth: usize, table: &LuaTable) {
    if table.entries.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    for entry in &table.entries {
        write_entry(out, depth + 1, entry);
    }
    indent(out, depth);
    out.push('}');
}

fn write_entry(out: &mut String, depth: usize, entry: &LuaTableEntry) {
    indent(out, depth);
    write_key(out, &entry.key);
    out.push_str(" = ");
    write_value(out, depth, &entry.value);
    out.push_str(",\n");
}

fn write_key(out: &mut String, key: &LuaKey) {
    out.push('[');
    match key {
        LuaKey::Str(s) => write_string_literal(out, s),
        LuaKey::Int(n) => {
            let _ = write!(out, "{n}");
        }
    }
    out.push(']');
}

fn write_value(out: &mut String, depth: usize, value: &LuaValue) {
    match value {
        LuaValue::Str(s) => write_string_literal(out, s),
        LuaValue::Number(text) => out.push_str(text),
        LuaValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        LuaValue::Nil => out.push_str("nil"),
        LuaValue::Table(t) => write_table(out, depth, t),
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push('\t');
    }
}

fn write_string_literal(out: &mut String, s: &str) {
    out.push('"');
    // Escape `\` and `"`; DCS appears to leave other characters as-is even when
    // they're non-printable, but we never observed any in fixtures.
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out.push('"');
}
