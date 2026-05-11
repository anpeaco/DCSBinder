//! Manifest schema and in-memory plan types.
//!
//! The manifest is the single source of truth for a remap operation. It is
//! written atomically before any mutation, so its existence on disk means
//! "this operation has started"; its `.done` sibling marker means "this
//! operation finished cleanly." Anything in between is recoverable by
//! iterating the manifest's `backups` and restoring each one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Current manifest schema version. Bump if the shape changes.
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    /// `UUIDv7` — chronologically sortable.
    pub operation_id: String,
    pub operation: OperationKind,
    /// RFC 3339 UTC timestamp.
    pub timestamp: String,
    /// Directory containing this manifest, its `.done` sibling, and every backup.
    pub backup_dir: PathBuf,
    /// DCS install root that was the target.
    pub install_root: PathBuf,
    /// Device name being remapped (e.g. `"MFDLeft"`).
    pub device_name: String,
    /// Subtype string (`"joystick"`, `"keyboard"`, etc.).
    pub subtype: String,
    /// GUID whose bytes are the source of truth (the bind we're keeping).
    pub source_guid: String,
    /// GUID we're remapping the content under (typically the live one).
    pub target_guid: String,
    /// Every existing file the operation may touch. Each is **copied** into
    /// `backup_dir` before any mutation. On undo, restore by copying back.
    pub backups: Vec<BackupEntry>,
    /// Ordered sequence of mutations to apply after every backup completes.
    pub mutations: Vec<Mutation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Remap,
    Undo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    /// Absolute path of the original file.
    pub src: PathBuf,
    /// Absolute path inside `backup_dir` where the snapshot lives.
    pub backup: PathBuf,
    /// Blake3 hex of the bytes at `src` at planning time. Verified on backup.
    pub blake3: String,
    /// Size in bytes (lets undo sanity-check before restoring).
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mutation {
    /// Write `dst` with the bytes currently at `source`.
    /// `source_blake3` is the expected hash of those bytes for verification.
    /// If `dst` existed at planning time it must also appear in `backups`.
    WriteFile {
        dst: PathBuf,
        source: PathBuf,
        source_blake3: String,
    },
    /// Move `src` to `dst`. Typically `dst` is inside the backup folder
    /// (archiving a stale file). `src` must appear in `backups`.
    MoveFile { src: PathBuf, dst: PathBuf },
    /// In-place byte-string replacement on `path`. Used for `modifiers.lua`
    /// per ADR-003 (no parser re-serialization in M3). `path` must appear
    /// in `backups`.
    StringReplace {
        path: PathBuf,
        find: String,
        replace: String,
        /// Sanity: how many occurrences we expected at planning time. Execute
        /// refuses to proceed if reality differs (file mutated between plan
        /// and execute).
        expected_replacements: u32,
    },
}

impl Manifest {
    /// Path where the manifest itself lives, given its `backup_dir`.
    #[must_use]
    pub fn path_in(backup_dir: &std::path::Path) -> PathBuf {
        backup_dir.join("manifest.json")
    }

    /// Path of the `.done` finalize marker.
    #[must_use]
    pub fn done_marker_in(backup_dir: &std::path::Path) -> PathBuf {
        backup_dir.join("manifest.json.done")
    }
}
