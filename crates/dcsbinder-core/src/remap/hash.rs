//! Blake3 hashing helper for backup verification.

use std::path::Path;

/// Compute the blake3 hex digest of a file's bytes.
pub fn file_blake3(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

/// Compute the blake3 hex digest of an in-memory byte slice.
#[must_use]
pub fn bytes_blake3(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
