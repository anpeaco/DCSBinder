//! Execute a [`Manifest`]. Two-phase commit:
//!
//! 1. Write manifest atomically (tempfile + persist) into `manifest.backup_dir`.
//! 2. Snapshot every file in `manifest.backups` into the backup folder,
//!    verifying its blake3 matches the planned hash.
//! 3. Apply every mutation in order, each atomic (`tmp + rename` for writes).
//! 4. Write `manifest.json.done` finalize marker.
//!
//! If `dcs_running()` returns `Some`, refuse to start.
//!
//! Aborting mid-phase leaves a recoverable state (manifest exists, `.done`
//! does not) — see `recover::recover` for the recovery flow.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use super::hash;
use super::types::{BackupEntry, Manifest, Mutation};

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error("DCS.exe is running (PID {pid}); refusing to mutate files. Close DCS and retry.")]
    DcsRunning { pid: u32 },
    #[error("could not create backup directory {path}: {source}")]
    CreateBackupDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write manifest at {path}: {source}")]
    WriteManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize manifest: {0}")]
    SerializeManifest(#[from] serde_json::Error),
    #[error("could not back up {src} to {backup}: {source}")]
    Backup {
        src: PathBuf,
        backup: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("blake3 mismatch backing up {src}: expected {expected}, got {actual}")]
    BackupHashMismatch {
        src: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("blake3 mismatch reading source {src}: expected {expected}, got {actual}")]
    SourceHashMismatch {
        src: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("could not perform mutation `WriteFile` (dst={dst}): {source}")]
    WriteFile {
        dst: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not perform mutation `MoveFile` ({src} -> {dst}): {source}")]
    MoveFile {
        src: PathBuf,
        dst: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "string-replace on {path} found {actual} occurrences, expected {expected} \
         (file mutated between plan and execute?)"
    )]
    StringReplaceMismatch {
        path: PathBuf,
        actual: u32,
        expected: u32,
    },
    #[error("string-replace I/O on {path}: {source}")]
    StringReplaceIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write `.done` marker at {path}: {source}")]
    Finalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Execute the manifest. Returns the path to the `.done` marker on success.
///
/// If `check_dcs_running` is `true`, refuses to start when `DCS.exe` is up.
/// (Set to `false` only in tests against synthetic fixtures.)
pub fn execute(manifest: &Manifest, check_dcs_running: bool) -> Result<PathBuf, ExecuteError> {
    if check_dcs_running {
        if let Some(pid) = crate::config::dcs_running() {
            return Err(ExecuteError::DcsRunning { pid });
        }
    }

    // Phase 1: ensure backup_dir exists and write manifest atomically.
    std::fs::create_dir_all(&manifest.backup_dir).map_err(|e| ExecuteError::CreateBackupDir {
        path: manifest.backup_dir.clone(),
        source: e,
    })?;
    write_manifest(manifest)?;

    // Phase 2: snapshot every existing file.
    for entry in &manifest.backups {
        snapshot_one(entry)?;
    }

    // Phase 3: apply mutations.
    for mutation in &manifest.mutations {
        apply_mutation(mutation)?;
    }

    // Phase 4: finalize.
    let done = Manifest::done_marker_in(&manifest.backup_dir);
    std::fs::write(&done, b"done").map_err(|e| ExecuteError::Finalize {
        path: done.clone(),
        source: e,
    })?;
    Ok(done)
}

fn write_manifest(manifest: &Manifest) -> Result<(), ExecuteError> {
    let path = Manifest::path_in(&manifest.backup_dir);
    let json = serde_json::to_vec_pretty(manifest)?;
    atomic_write(&path, &json).map_err(|e| ExecuteError::WriteManifest {
        path: path.clone(),
        source: e,
    })?;
    Ok(())
}

fn snapshot_one(entry: &BackupEntry) -> Result<(), ExecuteError> {
    if let Some(parent) = entry.backup.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ExecuteError::Backup {
            src: entry.src.clone(),
            backup: entry.backup.clone(),
            source: e,
        })?;
    }
    std::fs::copy(&entry.src, &entry.backup).map_err(|e| ExecuteError::Backup {
        src: entry.src.clone(),
        backup: entry.backup.clone(),
        source: e,
    })?;
    let actual = hash::file_blake3(&entry.backup).map_err(|e| ExecuteError::Backup {
        src: entry.src.clone(),
        backup: entry.backup.clone(),
        source: e,
    })?;
    if actual != entry.blake3 {
        return Err(ExecuteError::BackupHashMismatch {
            src: entry.src.clone(),
            expected: entry.blake3.clone(),
            actual,
        });
    }
    Ok(())
}

fn apply_mutation(mutation: &Mutation) -> Result<(), ExecuteError> {
    match mutation {
        Mutation::WriteFile {
            dst,
            source,
            source_blake3,
        } => {
            // Verify source hash before writing (defense against TOCTOU).
            let actual = hash::file_blake3(source).map_err(|e| ExecuteError::WriteFile {
                dst: dst.clone(),
                source: e,
            })?;
            if &actual != source_blake3 {
                return Err(ExecuteError::SourceHashMismatch {
                    src: source.clone(),
                    expected: source_blake3.clone(),
                    actual,
                });
            }
            let bytes = std::fs::read(source).map_err(|e| ExecuteError::WriteFile {
                dst: dst.clone(),
                source: e,
            })?;
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ExecuteError::WriteFile {
                    dst: dst.clone(),
                    source: e,
                })?;
            }
            atomic_write(dst, &bytes).map_err(|e| ExecuteError::WriteFile {
                dst: dst.clone(),
                source: e,
            })?;
        }
        Mutation::MoveFile { src, dst } => {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ExecuteError::MoveFile {
                    src: src.clone(),
                    dst: dst.clone(),
                    source: e,
                })?;
            }
            // `fs::rename` is atomic on Windows when source and destination
            // are on the same volume; cross-volume falls back to copy+delete.
            std::fs::rename(src, dst).map_err(|e| ExecuteError::MoveFile {
                src: src.clone(),
                dst: dst.clone(),
                source: e,
            })?;
        }
        Mutation::StringReplace {
            path,
            find,
            replace,
            expected_replacements,
        } => {
            let bytes = std::fs::read(path).map_err(|e| ExecuteError::StringReplaceIo {
                path: path.clone(),
                source: e,
            })?;
            let text = std::str::from_utf8(&bytes).map_err(|_| ExecuteError::StringReplaceIo {
                path: path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "file is not valid UTF-8",
                ),
            })?;
            let actual = u32::try_from(text.matches(find.as_str()).count()).unwrap_or(u32::MAX);
            if actual != *expected_replacements {
                return Err(ExecuteError::StringReplaceMismatch {
                    path: path.clone(),
                    actual,
                    expected: *expected_replacements,
                });
            }
            let replaced = text.replace(find.as_str(), replace.as_str());
            atomic_write(path, replaced.as_bytes()).map_err(|e| ExecuteError::StringReplaceIo {
                path: path.clone(),
                source: e,
            })?;
        }
    }
    Ok(())
}

/// Write `bytes` to `path` atomically: tempfile in the same directory + rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    // persist overwrites if dst exists on POSIX. On Windows, persist will
    // succeed across an existing file as long as it isn't open elsewhere.
    tmp.persist(path)
        .map_err(|e| std::io::Error::other(e.error))?;
    Ok(())
}
