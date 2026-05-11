//! Detect interrupted operations and restore from manifests.
//!
//! - [`recover`] scans a backup root directory for `manifest.json` files
//!   missing a sibling `manifest.json.done`. Each is surfaced as an
//!   [`IncompleteOperation`] for the caller to ask the user about.
//! - [`undo`] takes a manifest path and restores every backed-up file to its
//!   original location, deleting any files the operation wrote that did not
//!   exist before.

use std::path::{Path, PathBuf};

use super::types::{Manifest, Mutation};

#[derive(Debug, thiserror::Error)]
pub enum RecoverError {
    #[error("could not read backup root {path}: {source}")]
    ReadBackupRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not read manifest {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("could not read manifest {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not restore {dst} from backup {backup}: {source}")]
    Restore {
        backup: PathBuf,
        dst: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not delete written file {path}: {source}")]
    DeleteWritten {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not undo MoveFile ({dst} -> {src}): {source}")]
    UndoMove {
        src: PathBuf,
        dst: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct IncompleteOperation {
    pub manifest_path: PathBuf,
    pub backup_dir: PathBuf,
    pub manifest: Manifest,
}

/// Scan `backup_root` for manifest files lacking a `.done` sibling.
///
/// Non-existent `backup_root` returns an empty `Vec` (not an error — first run).
pub fn recover(backup_root: &Path) -> Result<Vec<IncompleteOperation>, RecoverError> {
    if !backup_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = std::fs::read_dir(backup_root).map_err(|e| RecoverError::ReadBackupRoot {
        path: backup_root.to_path_buf(),
        source: e,
    })?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = Manifest::path_in(&dir);
        if !manifest_path.is_file() {
            continue;
        }
        let done = Manifest::done_marker_in(&dir);
        if done.exists() {
            continue;
        }
        let bytes = std::fs::read(&manifest_path).map_err(|e| RecoverError::ReadManifest {
            path: manifest_path.clone(),
            source: e,
        })?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|e| RecoverError::ParseManifest {
                path: manifest_path.clone(),
                source: e,
            })?;
        out.push(IncompleteOperation {
            manifest_path,
            backup_dir: dir,
            manifest,
        });
    }
    Ok(out)
}

/// Roll back the operation described by `manifest_path`.
///
/// Restores every backed-up file and reverses every mutation (deleting any
/// newly-written file that wasn't backed up, moving back any archived file).
pub fn undo(manifest_path: &Path) -> Result<(), UndoError> {
    let bytes = std::fs::read(manifest_path).map_err(|e| UndoError::ReadManifest {
        path: manifest_path.to_path_buf(),
        source: e,
    })?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| UndoError::ParseManifest {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;

    // Reverse mutations in reverse order. We don't rely on which mutations
    // actually executed — most reversals are idempotent or no-ops if the
    // partial state doesn't match.
    for mutation in manifest.mutations.iter().rev() {
        match mutation {
            Mutation::WriteFile { dst, .. } => {
                // If `dst` wasn't in backups, it didn't exist before — delete it.
                // If it was, the next loop will restore it on top.
                let was_backed_up = manifest.backups.iter().any(|b| b.src == *dst);
                if !was_backed_up && dst.exists() {
                    std::fs::remove_file(dst).map_err(|e| UndoError::DeleteWritten {
                        path: dst.clone(),
                        source: e,
                    })?;
                }
            }
            Mutation::MoveFile { src, dst } => {
                // Move dst back to src, if dst exists.
                if dst.exists() {
                    if let Some(parent) = src.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::rename(dst, src).map_err(|e| UndoError::UndoMove {
                        src: src.clone(),
                        dst: dst.clone(),
                        source: e,
                    })?;
                }
            }
            Mutation::StringReplace { .. } => {
                // No special handling; the backup restore below puts the file
                // back to its pre-mutation bytes.
            }
        }
    }

    // Restore every backup.
    for entry in &manifest.backups {
        if let Some(parent) = entry.src.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if entry.backup.exists() {
            std::fs::copy(&entry.backup, &entry.src).map_err(|e| UndoError::Restore {
                backup: entry.backup.clone(),
                dst: entry.src.clone(),
                source: e,
            })?;
        }
    }

    // Drop a `.done` marker so a future `recover` doesn't re-surface this.
    let done = Manifest::done_marker_in(&manifest.backup_dir);
    let _ = std::fs::write(&done, b"undone");

    Ok(())
}
