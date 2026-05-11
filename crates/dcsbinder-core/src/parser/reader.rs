//! Parse a `.diff.lua` or `modifiers.lua` source into a [`LuaFile`].
//!
//! Uses `full_moon` to build a Lua AST, then walks the narrow subset DCS emits:
//! `local <name> = <table>; return <name>`.

use full_moon::ast::{Expression, Field, Stmt, TableConstructor};

use super::types::{LuaFile, LuaKey, LuaTable, LuaTableEntry, LuaValue};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("full_moon parse error: {0}")]
    FullMoon(String),
    #[error("could not find `local <name> = {{...}}` table at top level")]
    NoTopLevelTable,
    #[error("unsupported key form at {context}: {detail}")]
    UnsupportedKey { context: String, detail: String },
    #[error("unsupported value form at {context}: {detail}")]
    UnsupportedValue { context: String, detail: String },
}

/// Parse a DCS Lua config file source string into a typed [`LuaFile`].
pub fn parse(source: &str) -> Result<LuaFile, ParseError> {
    let ast = full_moon::parse(source).map_err(|errs| {
        ParseError::FullMoon(
            errs.iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    let (var_name, table) = find_top_table(&ast)?;
    let value = parse_table(table, "<root>")?;
    Ok(LuaFile { var_name, value })
}

fn find_top_table(ast: &full_moon::ast::Ast) -> Result<(String, &TableConstructor), ParseError> {
    for stmt in ast.nodes().stmts() {
        if let Stmt::LocalAssignment(la) = stmt {
            // We accept any single-name local assignment whose RHS is a table
            // constructor. There should only be one such statement in a DCS file.
            let name = la
                .names()
                .iter()
                .next()
                .map(|tk| tk.token().to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            if let Some(Expression::TableConstructor(tc)) = la.expressions().iter().next() {
                return Ok((name, tc));
            }
        }
    }
    Err(ParseError::NoTopLevelTable)
}

fn parse_table(table: &TableConstructor, context: &str) -> Result<LuaTable, ParseError> {
    let mut entries = Vec::new();
    for field in table.fields() {
        let (key, value_expr) = parse_field_key(field, context)?;
        let context_child = match &key {
            LuaKey::Str(s) => format!("{context}.{s}"),
            LuaKey::Int(n) => format!("{context}[{n}]"),
        };
        let value = parse_value(value_expr, &context_child)?;
        entries.push(LuaTableEntry { key, value });
    }
    Ok(LuaTable { entries })
}

fn parse_field_key<'a>(
    field: &'a Field,
    context: &str,
) -> Result<(LuaKey, &'a Expression), ParseError> {
    match field {
        Field::ExpressionKey { key, value, .. } => {
            if let Some(s) = expression_as_string(key) {
                Ok((LuaKey::Str(s), value))
            } else if let Some(n) = expression_as_int(key) {
                Ok((LuaKey::Int(n), value))
            } else {
                Err(ParseError::UnsupportedKey {
                    context: context.into(),
                    detail: format!("expected [\"..\"] or [N], got {key:?}"),
                })
            }
        }
        _ => Err(ParseError::UnsupportedKey {
            context: context.into(),
            detail: format!("expected [..] = .., got {field:?}"),
        }),
    }
}

fn parse_value(expr: &Expression, context: &str) -> Result<LuaValue, ParseError> {
    if let Some(s) = expression_as_string(expr) {
        return Ok(LuaValue::Str(s));
    }
    if let Some(n) = expression_as_number_text(expr) {
        return Ok(LuaValue::Number(n));
    }
    if let Some(b) = expression_as_bool(expr) {
        return Ok(LuaValue::Bool(b));
    }
    if expression_is_nil(expr) {
        return Ok(LuaValue::Nil);
    }
    if let Expression::TableConstructor(tc) = expr {
        return Ok(LuaValue::Table(parse_table(tc, context)?));
    }
    Err(ParseError::UnsupportedValue {
        context: context.into(),
        detail: format!("{expr:?}"),
    })
}

fn expression_as_string(expr: &Expression) -> Option<String> {
    if let Expression::String(token_ref) = expr {
        if let full_moon::tokenizer::TokenType::StringLiteral { literal, .. } =
            token_ref.token_type()
        {
            return Some(literal.to_string());
        }
    }
    None
}

fn expression_as_number_text(expr: &Expression) -> Option<String> {
    if let Expression::Number(token_ref) = expr {
        if let full_moon::tokenizer::TokenType::Number { text } = token_ref.token_type() {
            return Some(text.to_string());
        }
    }
    None
}

fn expression_as_int(expr: &Expression) -> Option<i64> {
    expression_as_number_text(expr).and_then(|s| s.parse::<i64>().ok())
}

fn expression_as_bool(expr: &Expression) -> Option<bool> {
    if let Expression::Symbol(token_ref) = expr {
        if let full_moon::tokenizer::TokenType::Symbol { symbol } = token_ref.token_type() {
            return match symbol {
                full_moon::tokenizer::Symbol::True => Some(true),
                full_moon::tokenizer::Symbol::False => Some(false),
                _ => None,
            };
        }
    }
    None
}

fn expression_is_nil(expr: &Expression) -> bool {
    if let Expression::Symbol(token_ref) = expr {
        if let full_moon::tokenizer::TokenType::Symbol { symbol } = token_ref.token_type() {
            return matches!(symbol, full_moon::tokenizer::Symbol::Nil);
        }
    }
    false
}
