//! Build a [`Manifest`] from `(install_root, device_name, source_guid, target_guid)`.
//!
//! No filesystem mutations happen here. The planner reads existing files (to
//! hash and discover affected paths) but never writes.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::hash;
use super::types::{BackupEntry, Manifest, Mutation, OperationKind, MANIFEST_VERSION};
use crate::device::guid::Guid;
use crate::scanner::{FileStatus, ScannedFile, Subtype};

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("no source file found for device `{device_name}` with GUID {source_guid}")]
    NoSourceFile {
        device_name: String,
        source_guid: String,
    },
    #[error("source and target GUID are equal — nothing to do ({source_guid})")]
    SourceEqualsTarget { source_guid: String },
    #[error("could not hash {path}: {source}")]
    Hash {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read modifiers.lua at {path}: {source}")]
    ReadModifiers {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid GUID `{0}`: {1}")]
    BadGuid(String, String),
}

/// Build the manifest for a remap operation.
///
/// `backup_root` is the parent dir under which a per-operation `<utc-ts>-<id>/`
/// folder is allocated. `files` should be the result of [`crate::scanner::scan`].
///
/// The returned `Manifest` lists:
/// - Every existing file that needs to be copied into the backup folder (with blake3).
/// - The ordered mutations: `WriteFile` (for each affected target), `MoveFile`
///   (for archiving the source), and `StringReplace` (for each `modifiers.lua`
///   that references the source GUID).
pub fn plan(
    install_root: &Path,
    device_name: &str,
    subtype: Subtype,
    source_guid: &Guid,
    target_guid: &Guid,
    files: &[ScannedFile],
    backup_root: &Path,
) -> Result<Manifest, PlanError> {
    plan_with_scope(
        install_root,
        device_name,
        subtype,
        source_guid,
        target_guid,
        files,
        backup_root,
        None,
    )
}

/// Like [`plan`] but restricts the operation to a single aircraft folder.
/// Pass `Some(aircraft_name)` to remap that one aircraft only, or `None` to
/// remap every aircraft that has the source file (the default).
#[allow(clippy::too_many_arguments)]
pub fn plan_with_scope(
    install_root: &Path,
    device_name: &str,
    subtype: Subtype,
    source_guid: &Guid,
    target_guid: &Guid,
    files: &[ScannedFile],
    backup_root: &Path,
    restrict_to_aircraft: Option<&str>,
) -> Result<Manifest, PlanError> {
    if source_guid == target_guid {
        return Err(PlanError::SourceEqualsTarget {
            source_guid: source_guid.to_dcs_string(),
        });
    }

    let operation_id = Uuid::now_v7().to_string();
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string());
    let backup_dir = allocate_backup_dir(backup_root, &timestamp, &operation_id);

    let mut backups: Vec<BackupEntry> = Vec::new();
    let mut mutations: Vec<Mutation> = Vec::new();

    // Walk only Active files for this device+subtype in this install.
    let candidates: Vec<&ScannedFile> = files
        .iter()
        .filter(|f| f.install_root == install_root && f.subtype == Some(subtype))
        .filter(
            |f| matches!(&f.status, FileStatus::Active { device_name: n, .. } if n == device_name),
        )
        .collect();

    // Group by aircraft (optionally filtered to a single aircraft).
    let aircrafts: std::collections::BTreeSet<&str> = candidates
        .iter()
        .map(|f| f.aircraft.as_str())
        .filter(|a| match restrict_to_aircraft {
            Some(r) => r == *a,
            None => true,
        })
        .collect();

    let mut had_any_source = false;

    for aircraft in &aircrafts {
        let did_emit = plan_for_aircraft(
            install_root,
            aircraft,
            device_name,
            subtype,
            source_guid,
            target_guid,
            &candidates,
            &backup_dir,
            &mut backups,
            &mut mutations,
        )?;
        if did_emit {
            had_any_source = true;
        }
    }

    if !had_any_source {
        return Err(PlanError::NoSourceFile {
            device_name: device_name.to_string(),
            source_guid: source_guid.to_dcs_string(),
        });
    }

    Ok(Manifest {
        version: MANIFEST_VERSION,
        operation_id,
        operation: OperationKind::Remap,
        timestamp,
        backup_dir,
        install_root: install_root.to_path_buf(),
        device_name: device_name.to_string(),
        subtype: subtype.as_str().to_string(),
        source_guid: source_guid.to_dcs_string(),
        target_guid: target_guid.to_dcs_string(),
        backups,
        mutations,
    })
}

/// Plan all backups + mutations for one aircraft. Returns `true` if a source
/// file was found and steps were emitted, `false` if the aircraft was skipped.
#[allow(clippy::too_many_arguments)]
fn plan_for_aircraft(
    install_root: &Path,
    aircraft: &str,
    device_name: &str,
    subtype: Subtype,
    source_guid: &Guid,
    target_guid: &Guid,
    candidates: &[&ScannedFile],
    backup_dir: &Path,
    backups: &mut Vec<BackupEntry>,
    mutations: &mut Vec<Mutation>,
) -> Result<bool, PlanError> {
    let in_this_aircraft: Vec<&ScannedFile> = candidates
        .iter()
        .copied()
        .filter(|f| f.aircraft == aircraft)
        .collect();

    let Some(source_file) = in_this_aircraft.iter().find(|f| match &f.status {
        FileStatus::Active { guid, .. } => guid_matches(guid, source_guid),
        _ => false,
    }) else {
        return Ok(false);
    };

    let source_path = source_file.path.clone();
    let source_hash = file_hash(&source_path)?;
    let source_size = file_size(&source_path)?;

    let subtype_dir = source_path.parent().map_or_else(
        || install_root.join(aircraft).join(subtype.as_str()),
        Path::to_path_buf,
    );
    let target_filename = format!("{device_name} {}.diff.lua", target_guid.to_dcs_string());
    let target_path = subtype_dir.join(target_filename);

    // Backup source.
    backups.push(BackupEntry {
        src: source_path.clone(),
        backup: backup_path_for(backup_dir, install_root, &source_path),
        blake3: source_hash.clone(),
        size: source_size,
    });

    // Backup target if it exists.
    if let Some(target_file) = in_this_aircraft.iter().find(|f| match &f.status {
        FileStatus::Active { guid, .. } => guid_matches(guid, target_guid),
        _ => false,
    }) {
        backups.push(BackupEntry {
            src: target_file.path.clone(),
            backup: backup_path_for(backup_dir, install_root, &target_file.path),
            blake3: file_hash(&target_file.path)?,
            size: file_size(&target_file.path)?,
        });
    }

    // Backup + archive any other stale-GUID files for this device.
    for other in in_this_aircraft.iter().filter(|f| match &f.status {
        FileStatus::Active { guid, .. } => {
            !guid_matches(guid, source_guid) && !guid_matches(guid, target_guid)
        }
        _ => false,
    }) {
        backups.push(BackupEntry {
            src: other.path.clone(),
            backup: backup_path_for(backup_dir, install_root, &other.path),
            blake3: file_hash(&other.path)?,
            size: file_size(&other.path)?,
        });
        mutations.push(Mutation::MoveFile {
            src: other.path.clone(),
            dst: archive_path_for(backup_dir, install_root, &other.path),
        });
    }

    mutations.push(Mutation::WriteFile {
        dst: target_path,
        source: source_path.clone(),
        source_blake3: source_hash,
    });
    mutations.push(Mutation::MoveFile {
        src: source_path.clone(),
        dst: archive_path_for(backup_dir, install_root, &source_path),
    });

    plan_modifiers_rewrite(
        install_root,
        aircraft,
        source_guid,
        target_guid,
        backup_dir,
        backups,
        mutations,
    )?;

    Ok(true)
}

fn plan_modifiers_rewrite(
    install_root: &Path,
    aircraft: &str,
    source_guid: &Guid,
    target_guid: &Guid,
    backup_dir: &Path,
    backups: &mut Vec<BackupEntry>,
    mutations: &mut Vec<Mutation>,
) -> Result<(), PlanError> {
    let modifiers_lua = install_root.join(aircraft).join("modifiers.lua");
    if !modifiers_lua.is_file() {
        return Ok(());
    }
    let bytes = std::fs::read(&modifiers_lua).map_err(|e| PlanError::ReadModifiers {
        path: modifiers_lua.clone(),
        source: e,
    })?;
    let occurrences = collect_guid_occurrences(&bytes, source_guid);
    if occurrences.is_empty() {
        return Ok(());
    }
    backups.push(BackupEntry {
        src: modifiers_lua.clone(),
        backup: backup_path_for(backup_dir, install_root, &modifiers_lua),
        blake3: hash::bytes_blake3(&bytes),
        size: bytes.len() as u64,
    });
    let target_dcs = target_guid.to_dcs_string();
    for (exact_text, count) in occurrences {
        mutations.push(Mutation::StringReplace {
            path: modifiers_lua.clone(),
            find: exact_text,
            replace: target_dcs.clone(),
            expected_replacements: count,
        });
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String, PlanError> {
    hash::file_blake3(path).map_err(|e| PlanError::Hash {
        path: path.to_path_buf(),
        source: e,
    })
}

fn file_size(path: &Path) -> Result<u64, PlanError> {
    Ok(std::fs::metadata(path)
        .map_err(|e| PlanError::Hash {
            path: path.to_path_buf(),
            source: e,
        })?
        .len())
}

fn allocate_backup_dir(backup_root: &Path, timestamp: &str, op_id: &str) -> PathBuf {
    // Use the first 8 chars of UUIDv7 (post-dashes-stripped) for a short ident.
    let short = op_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>();
    // Replace ':' / '.' from timestamp to keep it FS-safe.
    let safe_ts = timestamp.replace([':', '.'], "-");
    backup_root.join(format!("{safe_ts}_{short}"))
}

fn backup_path_for(backup_dir: &Path, install_root: &Path, original: &Path) -> PathBuf {
    let rel = original.strip_prefix(install_root).unwrap_or(original);
    backup_dir.join("snapshots").join(rel)
}

fn archive_path_for(backup_dir: &Path, install_root: &Path, original: &Path) -> PathBuf {
    let rel = original.strip_prefix(install_root).unwrap_or(original);
    backup_dir.join("archived").join(rel)
}

fn guid_matches(guid_str: &str, guid: &Guid) -> bool {
    Guid::parse_dcs(&format!("{{{guid_str}}}")).is_ok_and(|parsed| parsed == *guid)
}

fn guid_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\{[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\}",
        )
        .expect("guid regex compiles")
    })
}

/// Walk `bytes` (as UTF-8 text) and collect every `{GUID}` substring whose
/// 16 bytes equal `target`. Returns a map of `exact-surface-form` -> `count`,
/// preserving the original casing DCS used in the file.
fn collect_guid_occurrences(bytes: &[u8], target: &Guid) -> Vec<(String, u32)> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for m in guid_regex().find_iter(text) {
        if let Ok(parsed) = Guid::parse_dcs(m.as_str()) {
            if parsed == *target {
                *counts.entry(m.as_str().to_string()).or_insert(0) += 1;
            }
        }
    }
    counts.into_iter().collect()
}
