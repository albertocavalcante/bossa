//! Helpers for parsing diskutil plist output.

use anyhow::{Context, Result};
use plist::{Dictionary, Value};
use std::io::Cursor;

/// Parse a plist document expected to be a top-level dictionary.
pub fn parse_plist_dict(plist: &str) -> Result<Dictionary> {
    let value = Value::from_reader(Cursor::new(plist.as_bytes()))
        .context("Failed to parse plist output")?;

    match value {
        Value::Dictionary(dict) => Ok(dict),
        _ => anyhow::bail!("Expected plist dictionary at root"),
    }
}

/// Get a string value for a key.
pub fn dict_get_string(dict: &Dictionary, key: &str) -> Option<String> {
    match dict.get(key) {
        Some(Value::String(value)) => Some(value.clone()),
        _ => None,
    }
}

/// Get a boolean value for a key.
pub fn dict_get_bool(dict: &Dictionary, key: &str) -> Option<bool> {
    match dict.get(key) {
        Some(Value::Boolean(value)) => Some(*value),
        Some(Value::String(value)) => match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Get an unsigned integer value for a key.
pub fn dict_get_u64(dict: &Dictionary, key: &str) -> Option<u64> {
    match dict.get(key) {
        Some(Value::Integer(value)) => value.as_unsigned(),
        Some(Value::String(value)) => value.parse::<u64>().ok(),
        _ => None,
    }
}

/// Get an array value for a key.
pub fn dict_get_array<'a>(dict: &'a Dictionary, key: &str) -> Option<&'a Vec<Value>> {
    match dict.get(key) {
        Some(Value::Array(value)) => Some(value),
        _ => None,
    }
}
